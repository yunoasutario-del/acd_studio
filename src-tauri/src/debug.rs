// デバッグ（Phase 5 作業 1。D-145・D-146）：利用者の Mac の Command Line Tools の lldb-dap を、Rust 側から DAP で使う
//
// - アダプターは `xcrun --find lldb-dap` で見つける。IDE には同梱せず、取得もしない（D-145 の 1）
// - DAP のクライアントは Rust 側に置く（D-145 の 2）。画面には決まった操作（続行・ステップ・ブレークポイント・スタック・
//   変数・式の評価）だけを出し、任意の DAP の要求や LLDB の初期化コマンドを画面から送る経路は作らない
// - 起動設定は Rust 側が作る（D-146 2.3）。~/.lldbinit を読まない（$__lldb_sourceInitFile: false）。
//   rustc に付属する表示スクリプトを initCommands で読み込み、標準入力は target.input-path で渡す
//   （Apple 版の 21.0.0 には起動設定の stdio がなく、runInTerminal は使えない。docs/phase5/01-pre-start-research.md §4）
// - メッセージの見出し（Content-Length）は LSP と同じなので、lsp.rs の encode・Decoder を使う
// - ビルドは dev（D-146 2.2）。ほかの処理と同時には動かさない（run.rs の RunControl。D-013）
// - C++（Phase 17 作業 6。D-211・D-214）は、g++ の -g -O0 -D_GLIBCXX_ASSERTIONS でビルドし、Rust の表示スクリプトを使わない。
//   panic の代わりに、例外（__cxa_throw）と abort で止める。ブレークポイントは .cpp・.hpp・.h などにも置ける
// - 止めるときは disconnect（terminateDebuggee: true）を送り、2 秒で終わらなければアダプターのグループに SIGKILL、
//   デバッグ対象の pid が残っていればそれにも SIGKILL を送る。terminate は送らない（宣言されておらず、送ると
//   アダプターが答えなくなった。01 §5）
//
// - Python（Phase 20 作業 6。D-226）は、debugpy（利用者がアプリのフォルダの python/cpython・python/pypy に
//   `pip install --target` で入れたもの）を使う。IDE が `<処理系> … -m debugpy --listen 127.0.0.1:<ポート> --wait-for-client main.py`
//   にあたる命令でデバッグ対象を起動し（標準入力は入力のファイル）、TCP で DAP をつなぐ（attach）。捕まえていない例外で止める。
//   SystemExit（exit()）で止まったときは、IDE がそのまま続ける。docs/phase19/07-phase20-trial.md §3・§5
//
// 限界：debugserver とデバッグ対象は、それぞれ別のプロセスグループに入る（debugserver の --setsid。01 §5）。
// アダプターが落ちると debugserver がデバッグ対象を止めることは手元で確かめたが、debugserver の pid は受け取らないので、
// 残っていないかは IDE では確かめない。デバッグ対象の pid は process の知らせで受け取り、止めるときに確かめる
// （pid が別のプロセスに再利用される短い間の競合は残る）。macOS（Unix）だけを対象にする

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::ipc::Channel;

use crate::language::LangSettings;
use crate::language::Language;
use crate::lsp::{Decoder, encode, file_uri};
#[cfg(windows)]
use crate::process::ProcessGroupShim;
use crate::process::{Pid, exited_without_reaping, kill_group};
use crate::python::{self, Interpreter};
use crate::run::{self, Finish, LogKind, Prepared, RunEvent, info, prepare, relative, send};
use crate::toolchain::{self, RustcPaths, Target};
use crate::workspace::{case_key, resolve};

// 要求の答えを待つ時間の上限
const REQUEST_WAIT: Duration = Duration::from_secs(10);
// 起動（launch。表示スクリプトの読み込みとプログラムの起動）を待つ時間の上限
const LAUNCH_WAIT: Duration = Duration::from_secs(30);
// 止めるときに、disconnect のあとアダプターが終わるのを待つ時間（D-146 2.10）
const DISCONNECT_WAIT: Duration = Duration::from_secs(2);
// 止めたあと、デバッグ対象が終わるのを確かめる時間
const DEBUGGEE_WAIT: Duration = Duration::from_millis(500);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
// デバッグコンソールに出すプログラムの出力などの合計の上限（D-146 2.8）
pub const OUTPUT_LIMIT: usize = 1024 * 1024;
// 1 つのファイルに置けるブレークポイントの数の上限
const MAX_BREAKPOINTS: usize = 1000;
// 1 回に受け取る変数の数の上限
const MAX_VARIABLES: usize = 1000;
// 式の長さの上限
const MAX_EXPRESSION: usize = 64 * 1024;
// panic で止めるための関数のブレークポイント（D-146 2.9）
const PANIC_FUNCTION: &str = "rust_panic";
// C++ で止めるための関数のブレークポイント：例外を投げる（__cxa_throw）・abort（D-211。範囲外の読み書きの検査〔D-214〕も abort に来る）
const THROW_FUNCTION: &str = "__cxa_throw";
const ABORT_FUNCTION: &str = "abort";
// ブレークポイントを置けるファイルの拡張子
const SOURCE_EXTENSIONS: &[&str] = &["rs", "cpp", "cc", "cxx", "hpp", "hh", "h", "py"];

const ADAPTER_MISSING: &str = "デバッグには Xcode の Command Line Tools の lldb-dap が必要です。ターミナルで xcode-select --install を実行して入れてください";
const NOT_DEBUGGING: &str = "デバッグ中ではありません";
const NOT_STOPPED: &str = "プログラムが止まっていません（止まっているときだけ使えます）";
pub const DEV_BUILD_NOTE: &str = "デバッグ用のビルド（dev：最適化なし、オーバーフローの検査あり）で動かします。ジャッジ（--release）とは速さや panic の起き方が違うことがあります。\n";
pub const PYTHON_DEBUG_NOTE: &str = "debugpy の中で動かすので、ふつうの実行よりかなり遅くなります。捕まえていない例外で止まります（exit() では止まりません）。\n";
// debugpy がつなぎを待ち始めるまで待つ時間の上限
const CONNECT_WAIT: Duration = Duration::from_secs(15);
// debugpy を動かす小さな命令。1 つ目の引数（debugpy を入れた場所）を sys.path の先頭に足し、残りの引数で debugpy を動かす
const DEBUGPY_BOOT: &str = "import sys, runpy; sys.path.insert(0, sys.argv.pop(1)); runpy.run_module('debugpy', run_name='__main__', alter_sys=True)";
pub const CPP_DEBUG_BUILD_NOTE: &str = "デバッグ用のビルド（-g -O0 -D_GLIBCXX_ASSERTIONS：最適化なし、vector の範囲外の読み書きなどで abort）で動かします。ジャッジ（-O2）とは速さや、範囲外の読み書きのときの動きが違います。\n";

// ---- 画面とのやり取りの形 ----

// 始めるときのブレークポイント（ファイルは作業領域のルートからの相対パス）
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBreakpoints {
    pub path: String,
    pub lines: Vec<u32>,
}

// アダプターが置いたブレークポイント
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BreakpointView {
    pub id: Option<i64>,
    pub verified: bool,
    // アダプターが動かしたあとの行（空行に置いたときなど）
    pub line: Option<u64>,
    // 作業領域の中のファイルなら、ルートからの相対パス
    pub path: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FrameView {
    pub id: i64,
    // 関数の名前（末尾の `::h<16 桁>` を外したもの）
    pub name: String,
    // ソースの絶対パス（あれば）
    pub source: Option<String>,
    // ソースの file: の URI（作業領域の外のソースを読み取り専用のタブで開くのに使う。D-133）
    pub source_uri: Option<String>,
    // 作業領域の中なら、ルートからの相対パス
    pub workspace_path: Option<String>,
    pub line: u64,
    pub column: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScopeView {
    pub name: String,
    pub variables_reference: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VariableView {
    pub name: String,
    pub value: String,
    pub type_name: Option<String>,
    // 0 でなければ、開いて中を見られる（variables にこの値を渡す）
    pub variables_reference: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateView {
    pub result: String,
    pub type_name: Option<String>,
    pub variables_reference: i64,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum StepAction {
    Continue,
    Pause,
    Next,
    StepIn,
    StepOut,
}

impl StepAction {
    fn command(self) -> &'static str {
        match self {
            StepAction::Continue => "continue",
            StepAction::Pause => "pause",
            StepAction::Next => "next",
            StepAction::StepIn => "stepIn",
            StepAction::StepOut => "stepOut",
        }
    }
}

// 式を評価する場所（デバッグコンソール・ウォッチ式）。hover などほかの値は受け付けない
#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum EvaluateContext {
    Repl,
    Watch,
}

// 画面に送るデバッグの知らせ（RunEvent::Debug で送る）
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum DebugEvent {
    // 止まった。reason は DAP の値（breakpoint・step・pause・exception など）。panic で止まったときは "panic"、
    // C++ で例外を投げたときは "throw"、abort のときは "abort"
    Stopped {
        reason: String,
        description: Option<String>,
        thread_id: Option<i64>,
    },
    Continued,
    // プログラムの出力など。category は DAP の値（stdout・stderr・console・important）。
    // target.input-path のときは、標準出力と標準エラーが混ざって stdout で届く（01 §4）
    Output {
        category: String,
        text: String,
    },
    // 始めるときに置いたブレークポイント（ファイルごと）
    Breakpoints {
        path: String,
        breakpoints: Vec<BreakpointView>,
    },
    // アダプターがブレークポイントを変えた（reason は changed・new・removed）
    BreakpointChanged {
        reason: String,
        breakpoint: BreakpointView,
    },
    // プログラムが終わった
    Exited {
        code: Option<i32>,
    },
}

// ---- アダプターを見つける ----

// テストで差し替えられる、外の環境
pub struct Environment {
    // （Windows では読まない。段階 6 の C++ W8）
    #[cfg_attr(windows, allow(dead_code))]
    pub xcrun: PathBuf,
    // アダプターに渡す HOME（テストで ~/.lldbinit を読まないことを確かめるときだけ使う）
    pub home: Option<PathBuf>,
    // debugpy を入れる場所（アプリのフォルダの python/。その下の cpython・pypy。D-226）
    pub python_tools: Option<PathBuf>,
}

impl Environment {
    pub fn real(python_tools: Option<PathBuf>) -> Self {
        Environment {
            xcrun: PathBuf::from("/usr/bin/xcrun"),
            home: None,
            python_tools,
        }
    }
}

// ---- debugpy（D-226） ----

// 処理系ごとの debugpy の場所（CPython 用はビルドされたもの、PyPy 用は純粋な Python のものが入るので分ける）
fn debugpy_dir(tools: &Path, kind: Interpreter) -> PathBuf {
    tools.join(match kind {
        Interpreter::Cpython => "cpython",
        Interpreter::Pypy => "pypy",
    })
}

// debugpy を見つける。なければ、入れ方（その処理系の pip で --target に入れる命令）を返す
#[cfg(not(windows))]
pub fn find_debugpy(
    tools: Option<&Path>,
    kind: Interpreter,
    interpreter: &Path,
) -> Result<PathBuf, String> {
    let tools = tools.ok_or_else(|| "アプリのフォルダが分かりません".to_string())?;
    let dir = debugpy_dir(tools, kind);
    if dir.join("debugpy").join("__init__.py").is_file() {
        return Ok(dir);
    }
    Err(format!(
        "Python のデバッグには debugpy が要ります（IDE は入れません）。ターミナルで次を実行して入れてください：\n\"{}\" -m pip install --target \"{}\" debugpy",
        interpreter.display(),
        dir.display()
    ))
}

// Windows（段階 6 W5）：探し方は同じ。入れ方の文は PowerShell 向けにし、引用符で始まる命令の前に & を付ける
// （付けないと、PowerShell は引用符の中を文字列として読み、命令として動かさない。推測。docs/windows-port-plan.md 4.8）
#[cfg(windows)]
pub fn find_debugpy(
    tools: Option<&Path>,
    kind: Interpreter,
    interpreter: &Path,
) -> Result<PathBuf, String> {
    let tools = tools.ok_or_else(|| "アプリのフォルダが分かりません".to_string())?;
    let dir = debugpy_dir(tools, kind);
    if dir.join("debugpy").join("__init__.py").is_file() {
        return Ok(dir);
    }
    Err(format!(
        "Python のデバッグには debugpy が要ります（IDE は入れません）。PowerShell で次を実行して入れてください：\n& \"{}\" -m pip install --target \"{}\" debugpy",
        interpreter.display(),
        dir.display()
    ))
}

// 空いているポートを借りて、すぐ返す（debugpy に渡す。借りてから debugpy が使うまでの短い間の競合は残る）
fn free_port() -> Result<u16, String> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|error| format!("debugpy に渡すポートを借りられません：{error}"))
}

// デバッグ対象を debugpy の中で動かす命令（パッケージのディレクトリで、script は相対パス）。
// -E・-B・-X int_max_str_digits=0 は実行と同じ。-X frozen_modules=off は debugpy がブレークポイントを見落とさないように
fn debugpy_command(interpreter: &Path, debugpy: &Path, port: u16, script: &str) -> Command {
    let mut command = Command::new(interpreter);
    // Windows は -X utf8 も付ける（段階 6 W2。実行の python::program・check_command と同じ。付けないと、CPython は
    // パイプへ cp932 で出し、デバッグの出力の日本語が化ける）。
    // -u も付ける（段階 6 W7。付けないと、パイプへの標準出力がまとめて書かれ、止まっている間や強制的に止めたときに届かない）。
    // -E があるので PYTHONUNBUFFERED は効かない。そのため -u で渡す
    #[cfg(windows)]
    {
        command.args(python::UTF8_MODE);
        command.arg("-u");
    }
    command
        .args([
            "-E",
            "-B",
            "-X",
            "int_max_str_digits=0",
            "-X",
            "frozen_modules=off",
            "-c",
            DEBUGPY_BOOT,
        ])
        .arg(debugpy)
        .args([
            "--listen".to_string(),
            format!("127.0.0.1:{port}"),
            "--wait-for-client".to_string(),
            script.to_string(),
        ]);
    toolchain::clear_ide_env(&mut command);
    command
}

fn debugpy_initialize_arguments() -> Value {
    json!({
        "clientID": "atcoder-studio",
        "clientName": "AtCoder Studio",
        "adapterID": "debugpy",
        "locale": "ja",
        "linesStartAt1": true,
        "columnsStartAt1": true,
        "pathFormat": "path",
        "supportsVariableType": true,
    })
}

// 自分のコードだけを止める・見る。出力は、IDE がデバッグ対象の標準出力・標準エラーから読む。子プロセスはデバッグしない
fn debugpy_attach_arguments() -> Value {
    json!({
        "name": "AtCoder Studio",
        "justMyCode": true,
        "redirectOutput": false,
        "subProcess": false,
    })
}

// SystemExit（exit()・sys.exit()）で止まったか。debugpy は 0 以外の SystemExit を捕まえていない例外として止めるが、
// 競技プログラミングでは途中で終えるのに使うので、IDE がそのまま続ける（exceptionOptions では外せなかった。07 §5）
fn is_system_exit(body: &Value) -> bool {
    body["reason"] == "exception"
        && body["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("SystemExit"))
}

// `xcrun --find lldb-dap` で lldb-dap の場所を見つける（シェルは使わない。D-146 2.1）
// （Windows では使わない。段階 6 の C++ W8）
#[cfg_attr(windows, allow(dead_code))]
pub fn find_adapter(xcrun: &Path) -> Result<PathBuf, String> {
    let mut command = Command::new(xcrun);
    command.args(["--find", "lldb-dap"]).stdin(Stdio::null());
    toolchain::clear_ide_env(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("{ADAPTER_MISSING}（xcrun を起動できません：{error}）"))?;
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if output.status.success() && path.is_absolute() && path.is_file() {
        Ok(path)
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if detail.is_empty() {
            ADAPTER_MISSING.to_string()
        } else {
            format!("{ADAPTER_MISSING}（xcrun：{detail}）")
        })
    }
}

// `lldb-dap --version` のうち、版の行（例：LLVM version 21.0.0）
fn adapter_version(adapter: &Path) -> Option<String> {
    let output = Command::new(adapter)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.lines()
        .map(str::trim)
        .find(|line| line.contains("version"))
        .map(str::to_string)
}

// ---- 起動設定（D-146 2.3） ----

// LLDB のコマンドの引数として、パスを二重引用符で囲む
fn lldb_quote(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

// rustc に付属する表示スクリプトの読み込み（rust-lldb と同じ 2 行）と、標準ライブラリのソースの場所の対応付け
fn init_commands(rustc: &RustcPaths) -> Vec<String> {
    let etc = rustc.sysroot.join("lib/rustlib/etc");
    let mut commands = vec![
        format!(
            "command script import {}",
            lldb_quote(&etc.join("lldb_lookup.py"))
        ),
        format!(
            "command source -s 0 {}",
            lldb_quote(&etc.join("lldb_commands"))
        ),
    ];
    // 標準ライブラリのフレームのパス（/rustc/<コミット>/library/…）を、rust-src の場所に読み替える
    let source = rustc.sysroot.join("lib/rustlib/src/rust");
    if let Some(hash) = &rustc.commit_hash
        && source.is_dir()
    {
        commands.push(format!(
            "settings set target.source-map /rustc/{hash} {}",
            lldb_quote(&source)
        ));
    }
    commands
}

fn initialize_arguments() -> Value {
    json!({
        "clientID": "atcoder-studio",
        "clientName": "AtCoder Studio",
        "adapterID": "lldb-dap",
        "locale": "ja",
        "linesStartAt1": true,
        "columnsStartAt1": true,
        "pathFormat": "path",
        "supportsVariableType": true,
        // ~/.lldbinit を読まない（lldb-dap の拡張。01 §8）
        "$__lldb_sourceInitFile": false,
    })
}

fn launch_arguments(program: &Path, dir: &Path, input: &Path, rustc: Option<&RustcPaths>) -> Value {
    json!({
        "type": "lldb-dap",
        "request": "launch",
        "name": "AtCoder Studio",
        "program": program,
        "args": [],
        "cwd": dir,
        "stopOnEntry": false,
        "initCommands": rustc.map(init_commands).unwrap_or_default(),
        "preRunCommands": [
            // 標準入力（01 §4）
            format!("settings set target.input-path {}", lldb_quote(input)),
            // デバッグの対象に、IDE（lldb-dap の親）の「書類」などへのアクセスの許可を引き継がせる。既定では対象が自分で
            // 責任を持つプロセスになり、問題（実行ファイル）が変わるたびに macOS が確認を出す（Phase 18 作業 3。D-217）
            "settings set target.inherit-tcc true".to_string(),
        ],
    })
}

// ---- gdb（Windows。段階 6 の C++。W8〜W10。docs/windows-port-plan.md の「段階 6 の C++（gdb）の計画」） ----
// 利用者の手元の MinGW-w64 の gdb（16.3 で確かめた）を DAP（--interpreter=dap）で使う。IDE には同梱せず、取得もしない

// gdb の引数（9.1 の追加 50）：利用者のフォルダの初期化ファイルを読まない（システムの gdbinit は読む。libstdc++ の表示の登録）、
// 始めの案内を出さない、DAP で話す、作業フォルダの .gdbinit とプログラム・DLL の隣のスクリプトを自動で読まない
#[cfg(windows)]
const GDB_ARGS: &[&str] = &[
    "-nh",
    "-q",
    "--interpreter=dap",
    "-iex",
    "set auto-load off",
];

// gdb の最初のスレッド（main）の番号（gdb のスレッドの番号は 1 から数え、1 つの gdb で 1 つのプロセスだけを動かす）。
// pause の threadId（W13。gdb の pause は threadId を見ずに `interrupt -a` で全部のスレッドを止める。DAP では要る項目）と、
// 一時停止のあとに扱うスレッド（W14）に使う
#[cfg(windows)]
const GDB_PAUSE_THREAD: i64 = 1;

#[cfg(windows)]
const GDB_MISSING: &str = "C++ のデバッグには gdb（MinGW-w64 の gdb.exe）が要ります。g++.exe と同じフォルダにも PATH にも gdb.exe が見つかりません。MinGW-Builds の GCC（bin に gdb.exe が入っています）を使うか、gdb.exe のあるフォルダを PATH に足してください";

// 起動する gdb と、gdb の子に付ける環境（W8）
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq)]
struct GdbLaunch {
    // 起動する exe（MinGW-Builds の形なら gdborig.exe、そうでなければ gdb.exe）
    program: PathBuf,
    // 子の PATH の先頭に、この順に足すフォルダ
    path_prefix: Vec<PathBuf>,
    // PYTHONHOME（MinGW-Builds の形のときだけ <mingw64>\opt）
    python_home: Option<PathBuf>,
}

// 設定の C++ のコンパイラ（なければ見つけた g++）と同じフォルダ → PATH の順に gdb.exe を探す（W8）
#[cfg(windows)]
fn find_gdb(cpp: &crate::cpp::CppSettings) -> Result<GdbLaunch, String> {
    let compiler_dir = crate::cpp::find_compiler(cpp)
        .ok()
        .and_then(|compiler| compiler.parent().map(Path::to_path_buf));
    find_gdb_in(compiler_dir.as_deref(), std::env::var_os("PATH").as_deref())
}

// テストのために、コンパイラのフォルダと PATH を受け取る形（相対パスの PATH の項目は見ない。cpp.rs の find_compiler_on_windows と同じ）
#[cfg(windows)]
fn find_gdb_in(
    compiler_dir: Option<&Path>,
    path_var: Option<&std::ffi::OsStr>,
) -> Result<GdbLaunch, String> {
    let in_path: Vec<PathBuf> = path_var
        .map(|paths| {
            std::env::split_paths(paths)
                .filter(|dir| dir.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    let gdb = compiler_dir
        .map(Path::to_path_buf)
        .into_iter()
        .chain(in_path)
        .map(|dir| dir.join("gdb.exe"))
        .find(|path| path.is_file())
        .ok_or_else(|| GDB_MISSING.to_string())?;
    let gdb_dir = gdb.parent().map(Path::to_path_buf).unwrap_or_default();
    // デバッグ対象の DLL は、ふだんの実行と同じくコンパイラの bin から（なければ gdb のフォルダ）
    let dll_dir = compiler_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| gdb_dir.clone());
    // MinGW-Builds の形：gdb.exe は包みで、本体は隣の gdborig.exe、Python は ..\opt（包みの中身は文字列から見た推測。9.1 の追加 50）。
    // 包みを通すと、空白のある引数が分かれ、デバッグ対象が opt\bin の libgcc_s_seh-1.dll で動く（P2 で確かめた）ので、本体を直接起動する
    let orig = gdb_dir.join("gdborig.exe");
    match gdb_dir.parent().map(|mingw| mingw.join("opt")) {
        Some(opt) if orig.is_file() && opt.is_dir() => Ok(GdbLaunch {
            program: orig,
            path_prefix: vec![dll_dir, opt.join("bin")],
            python_home: Some(opt),
        }),
        _ => Ok(GdbLaunch {
            program: gdb,
            path_prefix: vec![dll_dir],
            python_home: None,
        }),
    }
}

// gdb の子の環境（W9）：PATH の先頭、PYTHONHOME、PYTHONUTF8=1（付けないと、デバッグ対象の出力が 1 文字も届かない。P2 の 1 回目）
#[cfg(windows)]
fn apply_gdb_env(command: &mut Command, launch: &GdbLaunch) {
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let paths = launch
        .path_prefix
        .iter()
        .cloned()
        .chain(std::env::split_paths(&inherited));
    if let Ok(joined) = std::env::join_paths(paths) {
        command.env("PATH", joined);
    }
    if let Some(home) = &launch.python_home {
        command.env("PYTHONHOME", home);
    }
    command.env("PYTHONUTF8", "1");
}

// gdb を DAP で起動する命令（W9。作業フォルダ・process_group・標準入出力・D-104 の変数は Client::spawn_gdb で付ける）
#[cfg(windows)]
fn gdb_command(launch: &GdbLaunch) -> Command {
    let mut command = Command::new(&launch.program);
    command.args(GDB_ARGS);
    apply_gdb_env(&mut command, launch);
    command
}

// `gdb --version` の 1 行目（例：GNU gdb (GDB) 16.3）。gdborig.exe は libpython のため同じ環境で動かす。窓を出さない
#[cfg(windows)]
fn gdb_version(launch: &GdbLaunch) -> Option<String> {
    let mut command = crate::process::command(&launch.program);
    command.arg("--version").stdin(Stdio::null());
    apply_gdb_env(&mut command, launch);
    let output = command.output().ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(|line| line.trim().to_string())
}

// gdb の initialize（W10）：lldb-dap の拡張（$__lldb_sourceInitFile）を送らない
#[cfg(windows)]
fn gdb_initialize_arguments() -> Value {
    json!({
        "clientID": "atcoder-studio",
        "clientName": "AtCoder Studio",
        "adapterID": "gdb",
        "locale": "ja",
        "linesStartAt1": true,
        "columnsStartAt1": true,
        "pathFormat": "path",
        "supportsVariableType": true,
    })
}

// gdb の launch（W10）：program・cwd は `\\?\` を外す。標準入力は args に引用符つきの 1 つの文字列（配列で送ると gdb が `<` を引用して
// 入らない。P2）。入力のファイルがないとき（choose_input の NUL）も同じ形。Windows のファイル名に `"` は使えないので逃がさない。
// preRunCommands（target.input-path・inherit-tcc）は macOS の LLDB だけなので送らない
#[cfg(windows)]
fn gdb_launch_arguments(program: &Path, dir: &Path, input: &Path) -> Value {
    json!({
        "program": crate::trust::without_verbatim(program),
        "cwd": crate::trust::without_verbatim(dir),
        "args": format!("< \"{}\"", crate::trust::without_verbatim(input).display()),
        "stopOnEntry": false,
    })
}

// gdb の一時停止（interrupt）は、割り込みのためのスレッドで、理由のない停止（reason "stopped"）として届く（W14。A9 の記録：
// threadId 5、スタックは ntdll!DbgBreakPoint）。利用者のコードが見えるように、main のスレッド（GDB_PAUSE_THREAD）の pause として扱う。
// デバッグコンソールで利用者が gdb の命令（interrupt など）を打って止まったときも、同じく pause として扱う
#[cfg(windows)]
fn gdb_pause_stop(adapter: &str, message: &Value) -> Option<Value> {
    if adapter != "gdb" || message["event"] != "stopped" || message["body"]["reason"] != "stopped" {
        return None;
    }
    let mut message = message.clone();
    message["body"]["reason"] = json!("pause");
    message["body"]["threadId"] = json!(GDB_PAUSE_THREAD);
    Some(message)
}

// ---- lldb-dap（Windows。段階 6 の Rust。W15〜W23。docs/windows-port-plan.md の「段階 6 の Rust（lldb-dap）の計画」） ----
// LLVM の公式版の lldb-dap（23.1.6 で確かめた）を DAP で使う。IDE には同梱せず、取得もしない（D-145）

#[cfg(windows)]
const LLDB_DAP_MISSING: &str = "Rust のデバッグには LLVM の lldb-dap（lldb-dap.exe）が要ります。PATH にも %ProgramFiles%\\LLVM\\bin にも見つかりません。LLVM の公式版（23.1 以降。llvm-project の GitHub の Releases にある Windows 用のインストーラー）を入れるか、lldb-dap.exe のあるフォルダを PATH に足してください";

#[cfg(windows)]
const LLDB_PYTHON_MISSING: &str = "Rust のデバッグ（lldb-dap）には CPython の python3.dll が要ります。見つけた CPython のフォルダに python3.dll がありません。python.org の Windows 版の Python 3.13 を入れてください（例：`winget install --id Python.Python.3.13 --scope user`）。別の場所のものを使うときは、settings.json の python.cpython に python.exe のパスを書いてください";

// 起動する lldb-dap と、子の PATH の先頭に足す Python のフォルダ（W15・W16）
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq)]
struct LldbLaunch {
    program: PathBuf,
    python_dir: PathBuf,
}

// PATH → %ProgramFiles%\LLVM\bin の順に lldb-dap.exe を探す（W15。9.1 の追加 52。試しのために取り出した %LOCALAPPDATA%\Programs の下は探さない）
#[cfg(windows)]
fn find_lldb_dap() -> Result<PathBuf, String> {
    let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
    find_lldb_dap_in(
        std::env::var_os("PATH").as_deref(),
        program_files.as_deref(),
    )
}

// テストのために、PATH と %ProgramFiles% を受け取る形（相対パスの PATH の項目は見ない。find_gdb_in と同じ）
#[cfg(windows)]
fn find_lldb_dap_in(
    path_var: Option<&std::ffi::OsStr>,
    program_files: Option<&Path>,
) -> Result<PathBuf, String> {
    let in_path: Vec<PathBuf> = path_var
        .map(|paths| {
            std::env::split_paths(paths)
                .filter(|dir| dir.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    in_path
        .into_iter()
        .chain(program_files.map(|dir| dir.join("LLVM").join("bin")))
        .map(|dir| dir.join("lldb-dap.exe"))
        .find(|path| path.is_file())
        .ok_or_else(|| LLDB_DAP_MISSING.to_string())
}

// CPython の exe のフォルダに python3.dll があれば、そのフォルダ（W16。lldb-dap の liblldb.dll が python3.dll を読む。
// lldb-dap は自分では Python を探さない〔--check-python で確かめた〕）
#[cfg(windows)]
fn lldb_python_dir(interpreter: &Path) -> Result<PathBuf, String> {
    interpreter
        .parent()
        .filter(|dir| dir.join("python3.dll").is_file())
        .map(Path::to_path_buf)
        .ok_or_else(|| LLDB_PYTHON_MISSING.to_string())
}

// lldb-dap と、その子に足す Python のフォルダ（W15・W16）。Python は、設定の python.interpreter が pypy でも CPython を探す
#[cfg(windows)]
fn find_lldb_launch(settings: &python::PythonSettings) -> Result<LldbLaunch, String> {
    let program = find_lldb_dap()?;
    let cpython = python::PythonSettings {
        interpreter: Interpreter::Cpython,
        ..settings.clone()
    };
    let interpreter = python::find_interpreter(&cpython)
        .map_err(|error| format!("Rust のデバッグ（lldb-dap）には CPython が要ります。{error}"))?;
    let python_dir = lldb_python_dir(&interpreter)?;
    Ok(LldbLaunch {
        program,
        python_dir,
    })
}

// lldb-dap の子の環境（W16・W20）：PATH の先頭に Python のフォルダ。デバッグ対象の起動のたびに出るコンソールの窓を隠す
// （LLDB の ProcessLauncherWindows が読む変数。LLDB の自前のテストの道具も付けている。9.1 の追加 52）
#[cfg(windows)]
fn apply_lldb_dap_env(command: &mut Command, launch: &LldbLaunch) {
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(launch.python_dir.clone()).chain(std::env::split_paths(&inherited));
    if let Ok(joined) = std::env::join_paths(paths) {
        command.env("PATH", joined);
    }
    command.env("LLDB_LAUNCH_INFERIORS_WITHOUT_CONSOLE", "true");
}

// lldb-dap を DAP で起動する命令（W16・W20。--no-lldbinit：家とシステムの .lldbinit を読まない。initialize の $__lldb_sourceInitFile: false と両方）
#[cfg(windows)]
fn lldb_dap_command(launch: &LldbLaunch) -> Command {
    let mut command = Command::new(&launch.program);
    command.arg("--no-lldbinit");
    apply_lldb_dap_env(&mut command, launch);
    command
}

// `lldb-dap --version` の最初の行。Python を足した子の環境で、窓を出さず（CREATE_NO_WINDOW）、上限つきで動かす。答えがなければ None
#[cfg(windows)]
const LLDB_DAP_VERSION_WAIT: Duration = Duration::from_secs(5);

#[cfg(windows)]
fn lldb_dap_version(launch: &LldbLaunch) -> Option<String> {
    let mut command = crate::process::command(&launch.program);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    apply_lldb_dap_env(&mut command, launch);
    let mut child = command.spawn().ok()?;
    let until = Instant::now() + LLDB_DAP_VERSION_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < until => thread::sleep(POLL_INTERVAL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut text = String::new();
    child.stdout.take()?.read_to_string(&mut text).ok()?;
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

// Windows の Rust の initCommands（W17）：lldb_lookup.py の import と source-map だけ（Windows の rustc の etc に lldb_commands がない）。
// パスは Windows の区切りでつなぐ（Unix の init_commands の行は変えない）
#[cfg(windows)]
fn lldb_dap_init_commands_windows(rustc: &RustcPaths) -> Vec<String> {
    let rustlib = rustc.sysroot.join("lib").join("rustlib");
    let mut commands = vec![format!(
        "command script import {}",
        lldb_quote(&rustlib.join("etc").join("lldb_lookup.py"))
    )];
    let source = rustlib.join("src").join("rust");
    if let Some(hash) = &rustc.commit_hash
        && source.is_dir()
    {
        commands.push(format!(
            "settings set target.source-map /rustc/{hash} {}",
            lldb_quote(&source)
        ));
    }
    commands
}

// Windows の Rust の launch（W18）：program・cwd・input-path は `\\?\` を外す。preRunCommands は標準入力だけ
// （target.inherit-tcc は macOS の LLDB だけ）。Unix の launch_arguments の行は変えない
#[cfg(windows)]
fn lldb_dap_launch_arguments_windows(
    program: &Path,
    dir: &Path,
    input: &Path,
    rustc: Option<&RustcPaths>,
) -> Value {
    json!({
        "type": "lldb-dap",
        "request": "launch",
        "name": "AtCoder Studio",
        "program": crate::trust::without_verbatim(program),
        "args": [],
        "cwd": crate::trust::without_verbatim(dir),
        "stopOnEntry": false,
        "initCommands": rustc.map(lldb_dap_init_commands_windows).unwrap_or_default(),
        "preRunCommands": [
            format!(
                "settings set target.input-path {}",
                lldb_quote(&crate::trust::without_verbatim(input))
            ),
        ],
    })
}

// ---- 標準入力（D-146 2.4） ----

// デバッグの標準入力にするファイルと、実行ログに出す文。case が None なら input.txt（なければ /dev/null）、
// Some なら、テストのフォルダの `<case>.in`（`sample-N`・`random-N` だけ）
fn choose_input(
    dir: &Path,
    test_dir: Option<&Path>,
    folder: &str,
    case: Option<&str>,
) -> Result<(PathBuf, String), String> {
    match case {
        Some(name) => {
            if case_key(name).is_none() {
                return Err(format!(
                    "デバッグに使えるケースは sample-N・random-N だけです（{name}）"
                ));
            }
            let path = test_dir
                .map(|test| test.join(format!("{name}.in")))
                .filter(|path| path.is_file())
                .ok_or_else(|| format!("{folder}/{name}.in が見つかりません"))?;
            let shown = format!("標準入力：{}\n", relative(dir, &path));
            Ok((path, shown))
        }
        None => match test_dir
            .map(|test| test.join("input.txt"))
            .filter(|path| path.is_file())
        {
            Some(path) => {
                let shown = format!("標準入力：{}\n", relative(dir, &path));
                Ok((path, shown))
            }
            // 指定しないと lldb-dap の端末になり、入力を待ち続けるので、空の入力を渡す（01 §4）
            None => Ok((
                #[cfg(not(windows))]
                PathBuf::from("/dev/null"),
                // Windows に /dev/null はなく、開けない（段階 6 W1。テスト A1 で確かめた）。NUL は空の入力になる
                #[cfg(windows)]
                PathBuf::from("NUL"),
                format!(
                    "入力ファイルなし（空の標準入力で実行）。入力が必要なら {folder}/input.txt を作ってください\n"
                ),
            )),
        },
    }
}

// ---- パスと名前 ----

// ブレークポイントを置けるファイル：作業領域の中の .rs と C++ のソース・ヘッダー（D-211）と .py（D-226）
fn breakpoint_source(root: &Path, path: &str) -> Result<PathBuf, String> {
    let file = resolve(root, path).map_err(|error| error.to_string())?;
    let allowed = file
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| SOURCE_EXTENSIONS.contains(&extension));
    if !file.is_file() || !allowed {
        return Err(format!(
            "ブレークポイントを置けるのは .rs・C++ のファイル（.cpp・.hpp・.h など）・.py だけです（{path}）"
        ));
    }
    Ok(file)
}

// 止める関数のブレークポイントと、止まったときの理由（Rust は panic、C++ は例外と abort。D-146 2.9・D-211）
fn stop_functions(language: Language) -> &'static [(&'static str, &'static str)] {
    match language {
        Language::Rust => &[(PANIC_FUNCTION, "panic")],
        Language::Cpp => &[(THROW_FUNCTION, "throw"), (ABORT_FUNCTION, "abort")],
        // Python は関数ではなく、捕まえていない例外で止める（setExceptionBreakpoints。D-226）
        Language::Python => &[],
    }
}

// アダプターが返したパスが作業領域の中なら、ルートからの相対パスにする
fn in_workspace(root: &Path, path: &str) -> Option<String> {
    let real = Path::new(path).canonicalize().ok()?;
    real.starts_with(root).then(|| relative(root, &real))
}

// 関数の名前の末尾の `::h` と 16 桁の 16 進数（シンボルのハッシュ）を外す
fn strip_hash(name: &str) -> String {
    if let Some(index) = name.rfind("::h") {
        let hash = &name[index + 3..];
        if hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return name[..index].to_string();
        }
    }
    name.to_string()
}

fn breakpoint_view(root: &Path, value: &Value) -> BreakpointView {
    BreakpointView {
        id: value["id"].as_i64(),
        verified: value["verified"].as_bool().unwrap_or(false),
        line: value["line"].as_u64(),
        path: value["source"]["path"]
            .as_str()
            .and_then(|path| in_workspace(root, path)),
        message: value["message"].as_str().map(str::to_string),
    }
}

// 失敗の答えの説明
fn failure_message(response: &Value) -> String {
    response["body"]["error"]["format"]
        .as_str()
        .or_else(|| response["message"].as_str())
        .unwrap_or("理由は分かりません")
        .to_string()
}

// ---- クライアント ----

// 読み取りのスレッドからセッションへの知らせ
enum Notice {
    Initialized,
    Terminated,
}

#[derive(Default)]
struct State {
    // 止まっているスレッド（止まっていなければ None）
    stopped_thread: Option<i64>,
    // デバッグ対象の pid（process の知らせ）
    debuggee_pid: Option<i32>,
    exit_code: Option<i32>,
    // launch の答えが届いた（それより前の console の出力は、表示スクリプトの読み込みなので捨てる）
    launched: bool,
    output_sent: usize,
    output_omitted: bool,
    // panic・例外・abort で止める関数のブレークポイントの id と、止まった理由（"panic"・"throw"・"abort"）
    stop_breakpoints: Vec<(i64, &'static str)>,
}

struct Shared {
    // アダプターの名前（lldb-dap・debugpy。知らせの文に使う）
    adapter: &'static str,
    // 要求の番号（読み取りのスレッドが SystemExit で続けるときにも使う）
    seq: AtomicI64,
    root: PathBuf,
    pending: Mutex<HashMap<i64, SyncSender<Value>>>,
    state: Mutex<State>,
    channel: Channel<RunEvent>,
}

pub struct Client {
    shared: Arc<Shared>,
    outgoing: Sender<String>,
}

fn debug_event(channel: &Channel<RunEvent>, event: DebugEvent) {
    send(channel, RunEvent::Debug { event });
}

impl Shared {
    fn lock_state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    // 出力を上限まで送る。超えたら一度だけ知らせる
    fn output(&self, category: &str, text: &str) {
        let mut state = self.lock_state();
        if category == "console" && !state.launched {
            return;
        }
        // Windows（段階 6 の C++ W11）：gdb は自分の文（No symbol table is loaded. など）を、デバッグ対象の出力と同じ stdout の知らせで送る。
        // デバッグ対象が出力するのは process の知らせ（debuggee_pid が入る）より後なので、それより前の出力は gdb の文として捨てる
        #[cfg(windows)]
        if self.adapter == "gdb" && state.debuggee_pid.is_none() {
            return;
        }
        let room = OUTPUT_LIMIT.saturating_sub(state.output_sent);
        if text.len() <= room {
            state.output_sent += text.len();
            drop(state);
            debug_event(
                &self.channel,
                DebugEvent::Output {
                    category: category.to_string(),
                    text: text.to_string(),
                },
            );
        } else if !state.output_omitted {
            state.output_omitted = true;
            state.output_sent = OUTPUT_LIMIT;
            drop(state);
            debug_event(
                &self.channel,
                DebugEvent::Output {
                    category: "important".to_string(),
                    text: format!(
                        "\n（出力が {} MiB を超えたので、これより後は表示しません）\n",
                        OUTPUT_LIMIT / 1024 / 1024
                    ),
                },
            );
        }
    }

    fn event(&self, message: &Value, notices: &Sender<Notice>) {
        // Windows（段階 6 の C++ W14）：gdb の一時停止を、main のスレッドの pause として扱う（gdb_pause_stop）
        #[cfg(windows)]
        let rewritten = gdb_pause_stop(self.adapter, message);
        #[cfg(windows)]
        let message = rewritten.as_ref().unwrap_or(message);
        let body = &message["body"];
        match message["event"].as_str().unwrap_or_default() {
            "initialized" => {
                let _ = notices.send(Notice::Initialized);
            }
            "stopped" => {
                let thread_id = body["threadId"].as_i64();
                let mut state = self.lock_state();
                state.stopped_thread = thread_id.or(state.stopped_thread);
                let special = state.stop_breakpoints.iter().find_map(|(id, reason)| {
                    body["hitBreakpointIds"]
                        .as_array()
                        .is_some_and(|ids| ids.iter().any(|hit| hit.as_i64() == Some(*id)))
                        .then_some(*reason)
                });
                drop(state);
                let reason = special
                    .unwrap_or_else(|| body["reason"].as_str().unwrap_or("unknown"))
                    .to_string();
                debug_event(
                    &self.channel,
                    DebugEvent::Stopped {
                        reason,
                        description: body["description"].as_str().map(str::to_string),
                        thread_id,
                    },
                );
            }
            "continued" => {
                self.lock_state().stopped_thread = None;
                debug_event(&self.channel, DebugEvent::Continued);
            }
            "output" => {
                let category = body["category"].as_str().unwrap_or("console");
                if category != "telemetry"
                    && let Some(text) = body["output"].as_str()
                {
                    self.output(category, text);
                }
            }
            "breakpoint" => {
                debug_event(
                    &self.channel,
                    DebugEvent::BreakpointChanged {
                        reason: body["reason"].as_str().unwrap_or("changed").to_string(),
                        breakpoint: breakpoint_view(&self.root, &body["breakpoint"]),
                    },
                );
            }
            "process" => {
                self.lock_state().debuggee_pid = body["systemProcessId"]
                    .as_i64()
                    .and_then(|pid| i32::try_from(pid).ok());
            }
            "exited" => {
                let code = body["exitCode"]
                    .as_i64()
                    .and_then(|c| i32::try_from(c).ok());
                {
                    let mut state = self.lock_state();
                    state.exit_code = code;
                    state.stopped_thread = None;
                }
                debug_event(&self.channel, DebugEvent::Exited { code });
            }
            "terminated" => {
                let _ = notices.send(Notice::Terminated);
            }
            _ => {}
        }
    }
}

// 書き込みのスレッド（lsp.rs の spawn_writer と同じ形）。lldb-dap は標準入力、debugpy は TCP
fn spawn_writer(mut stdin: impl Write + Send + 'static, outgoing: Receiver<String>) {
    thread::spawn(move || {
        for body in outgoing {
            if stdin
                .write_all(&encode(&body))
                .and_then(|_| stdin.flush())
                .is_err()
            {
                break;
            }
        }
    });
}

// 標準出力を読み、答えは待っている要求へ、知らせはセッションと画面へ回す。逆向きの要求には失敗で答える
fn spawn_reader(
    stdout: impl Read + Send + 'static,
    shared: Arc<Shared>,
    outgoing: Sender<String>,
    notices: Sender<Notice>,
) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut decoder = Decoder::default();
        let mut buffer = vec![0u8; 64 * 1024];
        'read: loop {
            let n = match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let Ok(messages) = decoder.push(&buffer[..n]) else {
                info(
                    &shared.channel,
                    format!("{} の出力を読めません\n", shared.adapter),
                );
                break;
            };
            for text in messages {
                let Ok(message) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                match message["type"].as_str().unwrap_or_default() {
                    "response" => {
                        if message["command"] == "launch" || message["command"] == "attach" {
                            shared.lock_state().launched = true;
                        }
                        let waiting = message["request_seq"].as_i64().and_then(|seq| {
                            shared
                                .pending
                                .lock()
                                .ok()
                                .and_then(|mut pending| pending.remove(&seq))
                        });
                        if let Some(waiting) = waiting {
                            let _ = waiting.send(message);
                        }
                    }
                    // SystemExit で止まったら、画面には知らせずに続ける（D-226）
                    "event"
                        if shared.adapter == "debugpy"
                            && message["event"] == "stopped"
                            && is_system_exit(&message["body"]) =>
                    {
                        let seq = shared.seq.fetch_add(1, Ordering::SeqCst);
                        let request = json!({
                            "seq": seq,
                            "type": "request",
                            "command": "continue",
                            "arguments": { "threadId": message["body"]["threadId"] },
                        });
                        if outgoing.send(request.to_string()).is_err() {
                            break 'read;
                        }
                    }
                    "event" => shared.event(&message, &notices),
                    "request" => {
                        let answer = json!({
                            "seq": 0,
                            "type": "response",
                            "request_seq": message["seq"],
                            "command": message["command"],
                            "success": false,
                            "message": "AtCoder Studio はこの要求に対応していません",
                        });
                        if outgoing.send(answer.to_string()).is_err() {
                            break 'read;
                        }
                    }
                    _ => {}
                }
            }
        }
        // 待っている要求を終わらせる（送り手を捨てると、待つ側は「終わりました」になる）
        if let Ok(mut pending) = shared.pending.lock() {
            pending.clear();
        }
    });
}

// デバッグ対象の出力を、デバッグの出力として出す（debugpy。上限は lldb-dap の出力と同じ）
fn spawn_output_reader(
    reader: impl Read + Send + 'static,
    category: &'static str,
    shared: Arc<Shared>,
) {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut buffer = vec![0u8; 16 * 1024];
        while let Ok(n) = reader.read(&mut buffer) {
            if n == 0 {
                break;
            }
            shared.output(category, &String::from_utf8_lossy(&buffer[..n]));
        }
    });
}

// 標準エラーは実行ログに出す（行頭にアダプターの名前）
fn spawn_stderr_reader(
    stderr: impl Read + Send + 'static,
    adapter: &'static str,
    channel: Channel<RunEvent>,
) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut buffer = vec![0u8; 16 * 1024];
        while let Ok(n) = reader.read(&mut buffer) {
            if n == 0 {
                break;
            }
            send(
                &channel,
                RunEvent::Log {
                    kind: LogKind::Stderr,
                    text: format!("{adapter}：{}", String::from_utf8_lossy(&buffer[..n])),
                },
            );
        }
    });
}

// 起動した子のグループ（Windows は Job）を止めてから回収する。debugpy につなげなかったときに使う
// （Windows 移植の段階 6。テストのために spawn_debugpy の中の 2 行をここに移した。動きは同じ）
fn stop_and_reap(child: &mut Child) {
    kill_group(child.id() as Pid);
    let _ = child.wait();
    // Windows：回収したので、Job の記録（プロセスのハンドルの写し）を消す（段階 6 W3。lsp.rs の Server::stop と同じ）
    #[cfg(windows)]
    let _ = crate::win::release_job(child.id());
}

impl Client {
    fn spawn(
        adapter: &Path,
        dir: &Path,
        root: &Path,
        home: Option<&Path>,
        channel: &Channel<RunEvent>,
    ) -> Result<(Arc<Client>, Child, Receiver<Notice>), String> {
        let mut command = Command::new(adapter);
        command
            .current_dir(dir)
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // デバッグ対象は lldb-dap の環境を引き継ぐので、今の実行と同じく D-104 の変数を外す
        toolchain::clear_ide_env(&mut command);
        if let Some(home) = home {
            command.env("HOME", home);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("lldb-dap を起動できません：{error}"))?;
        #[cfg(windows)]
        crate::win::start_in_job(&child)
            .map_err(|error| format!("lldb-dap を起動できません：{error}"))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        spawn_stderr_reader(
            child.stderr.take().expect("piped"),
            "lldb-dap",
            channel.clone(),
        );
        let (client, notices) = Client::connect("lldb-dap", stdin, stdout, root, channel);
        Ok((client, child, notices))
    }

    // Windows（段階 6 の C++。W9）：gdb を DAP で起動する。流れは spawn と同じ（Unix の spawn の行を変えないことを優先して分けた。
    // Python の W5 と同じ理由）。gdb の子の環境（PATH・PYTHONHOME・PYTHONUTF8）は gdb_command で付ける
    #[cfg(windows)]
    fn spawn_gdb(
        launch: &GdbLaunch,
        dir: &Path,
        root: &Path,
        channel: &Channel<RunEvent>,
    ) -> Result<(Arc<Client>, Child, Receiver<Notice>), String> {
        let mut command = gdb_command(launch);
        command
            .current_dir(dir)
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // デバッグ対象は gdb の環境を引き継ぐので、今の実行と同じく D-104 の変数を外す
        toolchain::clear_ide_env(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("gdb を起動できません：{error}"))?;
        crate::win::start_in_job(&child)
            .map_err(|error| format!("gdb を起動できません：{error}"))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        spawn_stderr_reader(child.stderr.take().expect("piped"), "gdb", channel.clone());
        let (client, notices) = Client::connect("gdb", stdin, stdout, root, channel);
        Ok((client, child, notices))
    }

    // Windows（段階 6 の Rust。W16・W20）：lldb-dap を DAP で起動する。流れは spawn_gdb と同じ（Unix の spawn と C++ の spawn_gdb の行は変えない）。
    // 引数と子の環境（--no-lldbinit・Python のフォルダ・窓を隠す変数）は lldb_dap_command で付ける
    #[cfg(windows)]
    fn spawn_lldb_dap(
        launch: &LldbLaunch,
        dir: &Path,
        root: &Path,
        channel: &Channel<RunEvent>,
    ) -> Result<(Arc<Client>, Child, Receiver<Notice>), String> {
        let mut command = lldb_dap_command(launch);
        command
            .current_dir(dir)
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // デバッグ対象は lldb-dap の環境を引き継ぐので、今の実行と同じく D-104 の変数を外す
        toolchain::clear_ide_env(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("lldb-dap を起動できません：{error}"))?;
        crate::win::start_in_job(&child)
            .map_err(|error| format!("lldb-dap を起動できません：{error}"))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        spawn_stderr_reader(
            child.stderr.take().expect("piped"),
            "lldb-dap",
            channel.clone(),
        );
        let (client, notices) = Client::connect("lldb-dap", stdin, stdout, root, channel);
        Ok((client, child, notices))
    }

    // 書き込み先と読み取り元から、クライアントを作る
    fn connect(
        adapter: &'static str,
        writer: impl Write + Send + 'static,
        reader: impl Read + Send + 'static,
        root: &Path,
        channel: &Channel<RunEvent>,
    ) -> (Arc<Client>, Receiver<Notice>) {
        let shared = Arc::new(Shared {
            adapter,
            seq: AtomicI64::new(1),
            root: root.to_path_buf(),
            pending: Mutex::new(HashMap::new()),
            state: Mutex::new(State::default()),
            channel: channel.clone(),
        });
        let (outgoing, to_write) = mpsc::channel();
        let (notify, notices) = mpsc::channel();
        spawn_writer(writer, to_write);
        spawn_reader(reader, shared.clone(), outgoing.clone(), notify);
        (Arc::new(Client { shared, outgoing }), notices)
    }

    // デバッグ対象を debugpy の中で起動し（新しいプロセスグループ。標準入力は input）、TCP でつなぐ（D-226）。
    // つなげるまで、debugpy が終わる・止める印・時間切れを見る
    fn spawn_debugpy(
        interpreter: &Path,
        debugpy: &Path,
        dir: &Path,
        root: &Path,
        input: &Path,
        cancel: &Arc<AtomicBool>,
        channel: &Channel<RunEvent>,
    ) -> Result<(Arc<Client>, Child, Receiver<Notice>), String> {
        let port = free_port()?;
        let stdin = std::fs::File::open(input).map_err(|error| {
            format!(
                "標準入力のファイルを開けません（{}）：{error}",
                input.display()
            )
        })?;
        let mut command = debugpy_command(interpreter, debugpy, port, python::SOURCE);
        command
            .current_dir(dir)
            .process_group(0)
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| format!("debugpy を起動できません：{error}"))?;
        #[cfg(windows)]
        crate::win::start_in_job(&child)
            .map_err(|error| format!("debugpy を起動できません：{error}"))?;
        // 出力は、つないでから読む（それまでは debugpy がつなぎを待つので、デバッグ対象は出力しない）
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let pid = child.id() as Pid;
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let until = Instant::now() + CONNECT_WAIT;
        let stream = loop {
            if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_millis(200)) {
                break Ok(stream);
            }
            if cancel.load(Ordering::SeqCst) {
                break Err("止めました".to_string());
            }
            if exited_without_reaping(pid) {
                break Err("debugpy が起動の途中で終わりました".to_string());
            }
            if Instant::now() >= until {
                break Err("debugpy につなげません".to_string());
            }
            thread::sleep(Duration::from_millis(100));
        };
        let stream = match stream {
            Ok(stream) => stream,
            Err(message) => {
                stop_and_reap(&mut child);
                return Err(message);
            }
        };
        let _ = stream.set_nodelay(true);
        let writer = stream
            .try_clone()
            .map_err(|error| format!("debugpy につなげません：{error}"))?;
        let (client, notices) = Client::connect("debugpy", writer, stream, root, channel);
        // デバッグ対象の標準出力・標準エラーを、デバッグの出力として出す（redirectOutput は使わない。使うと、DAP の
        // output と、本来の出力の両方に出て二重になる）
        spawn_output_reader(stdout, "stdout", client.shared.clone());
        spawn_output_reader(stderr, "stderr", client.shared.clone());
        Ok((client, child, notices))
    }

    // 要求を送る。答えは返した受け手で待つ
    fn begin_request(&self, command: &str, arguments: Value) -> Result<Receiver<Value>, String> {
        let seq = self.shared.seq.fetch_add(1, Ordering::SeqCst);
        let (answer, waiting) = mpsc::sync_channel(1);
        self.shared
            .pending
            .lock()
            .map_err(|_| "デバッグの状態を読めません".to_string())?
            .insert(seq, answer);
        let body =
            json!({ "seq": seq, "type": "request", "command": command, "arguments": arguments });
        if self.outgoing.send(body.to_string()).is_err() {
            if let Ok(mut pending) = self.shared.pending.lock() {
                pending.remove(&seq);
            }
            return Err(format!(
                "{} に送れません（終わっています）",
                self.shared.adapter
            ));
        }
        Ok(waiting)
    }

    fn finish_request(
        &self,
        waiting: Receiver<Value>,
        command: &str,
        wait: Duration,
    ) -> Result<Value, String> {
        let adapter = self.shared.adapter;
        match waiting.recv_timeout(wait) {
            Ok(response) if response["success"] == true => Ok(response["body"].clone()),
            Ok(response) => Err(format!(
                "{adapter} が {command} を断りました：{}",
                failure_message(&response)
            )),
            Err(RecvTimeoutError::Timeout) => Err(format!(
                "{adapter} が {command} に {} 秒答えませんでした",
                wait.as_secs()
            )),
            Err(RecvTimeoutError::Disconnected) => Err(format!("{adapter} が終わりました")),
        }
    }

    fn request(&self, command: &str, arguments: Value, wait: Duration) -> Result<Value, String> {
        let waiting = self.begin_request(command, arguments)?;
        self.finish_request(waiting, command, wait)
    }

    fn stopped_thread(&self) -> Result<i64, String> {
        self.shared
            .lock_state()
            .stopped_thread
            .ok_or_else(|| NOT_STOPPED.to_string())
    }

    // 続行・一時停止・ステップ（D-146 2.5）
    pub fn step(&self, action: StepAction) -> Result<(), String> {
        // Windows（段階 6 の C++ W13）：gdb は動いている間の threads を断る（notStopped）ので、threads を送らずに pause を送る
        #[cfg(windows)]
        if action == StepAction::Pause && self.shared.adapter == "gdb" {
            return self
                .request(
                    "pause",
                    json!({ "threadId": GDB_PAUSE_THREAD }),
                    REQUEST_WAIT,
                )
                .map(|_| ());
        }
        let thread_id = match action {
            // 動いている間も止められるように、スレッドの一覧の最初（main）を使う
            StepAction::Pause => {
                let threads = self.request("threads", json!({}), REQUEST_WAIT)?;
                threads["threads"][0]["id"]
                    .as_i64()
                    .ok_or_else(|| "スレッドが見つかりません".to_string())?
            }
            _ => self.stopped_thread()?,
        };
        // 止まっている印は、要求を送る前に消す。答えを待つ間に次の「止まった」の知らせが届くことがあり、
        // 答えのあとに消すと、その知らせまで消してしまう（Phase 10 作業 6。D-169）
        if action != StepAction::Pause {
            self.shared.lock_state().stopped_thread = None;
        }
        let result = self.request(
            action.command(),
            json!({ "threadId": thread_id }),
            REQUEST_WAIT,
        );
        if result.is_err() && action != StepAction::Pause {
            // 送れなかったときは止まったまま。その間に新しい「止まった」が届いていれば、そちらを使う
            let mut state = self.shared.lock_state();
            if state.stopped_thread.is_none() {
                state.stopped_thread = Some(thread_id);
            }
        }
        result.map(|_| ())
    }

    // 1 つのファイルのブレークポイントを、lines のとおりに置き直す（D-146 2.6）
    pub fn set_breakpoints(
        &self,
        path: &str,
        lines: &[u32],
    ) -> Result<Vec<BreakpointView>, String> {
        let file = breakpoint_source(&self.shared.root, path)?;
        if lines.len() > MAX_BREAKPOINTS {
            return Err(format!(
                "1 つのファイルに置けるブレークポイントは {MAX_BREAKPOINTS} までです"
            ));
        }
        if lines.contains(&0) {
            return Err("行の番号は 1 からです".to_string());
        }
        let breakpoints: Vec<Value> = lines.iter().map(|line| json!({ "line": line })).collect();
        // Windows（段階 6 W6）：canonicalize の結果の `\\?\C:\…` のままだと、debugpy は verified で答えるのに止まらない
        // （動いている main.py は、`\\?\` のない cwd からの相対で扱われるため。調べで確かめた）。`\\?\` を外して送る
        #[cfg(windows)]
        let file = crate::trust::without_verbatim(&file);
        let body = self.request(
            "setBreakpoints",
            json!({ "source": { "path": file }, "breakpoints": breakpoints }),
            REQUEST_WAIT,
        )?;
        Ok(body["breakpoints"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| breakpoint_view(&self.shared.root, item))
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn stack(&self) -> Result<Vec<FrameView>, String> {
        let thread_id = self.stopped_thread()?;
        let body = self.request(
            "stackTrace",
            json!({ "threadId": thread_id, "startFrame": 0, "levels": 100 }),
            REQUEST_WAIT,
        )?;
        Ok(body["stackFrames"]
            .as_array()
            .map(|frames| {
                frames
                    .iter()
                    // debugpy を動かす小さな命令（-c）のフレームは出さない（D-226）
                    .filter(|frame| frame["source"]["path"] != "<string>")
                    .map(|frame| {
                        let source = frame["source"]["path"].as_str().map(str::to_string);
                        FrameView {
                            id: frame["id"].as_i64().unwrap_or_default(),
                            name: strip_hash(frame["name"].as_str().unwrap_or_default()),
                            workspace_path: source
                                .as_deref()
                                .and_then(|path| in_workspace(&self.shared.root, path)),
                            source_uri: source.as_deref().map(|path| file_uri(Path::new(path))),
                            source,
                            line: frame["line"].as_u64().unwrap_or_default(),
                            column: frame["column"].as_u64().unwrap_or_default(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn scopes(&self, frame_id: i64) -> Result<Vec<ScopeView>, String> {
        self.stopped_thread()?;
        let body = self.request("scopes", json!({ "frameId": frame_id }), REQUEST_WAIT)?;
        Ok(body["scopes"]
            .as_array()
            .map(|scopes| {
                scopes
                    .iter()
                    .map(|scope| ScopeView {
                        name: scope["name"].as_str().unwrap_or_default().to_string(),
                        variables_reference: scope["variablesReference"]
                            .as_i64()
                            .unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn variables(&self, reference: i64) -> Result<Vec<VariableView>, String> {
        self.stopped_thread()?;
        let body = self.request(
            "variables",
            json!({ "variablesReference": reference, "start": 0, "count": MAX_VARIABLES }),
            REQUEST_WAIT,
        )?;
        Ok(body["variables"]
            .as_array()
            .map(|variables| {
                variables
                    .iter()
                    .take(MAX_VARIABLES)
                    .map(|variable| VariableView {
                        name: variable["name"].as_str().unwrap_or_default().to_string(),
                        value: variable["value"].as_str().unwrap_or_default().to_string(),
                        type_name: variable["type"].as_str().map(str::to_string),
                        variables_reference: variable["variablesReference"]
                            .as_i64()
                            .unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    // 式を評価する（デバッグコンソール・ウォッチ式。D-145 の 3）。止まっているときだけ
    pub fn evaluate(
        &self,
        expression: &str,
        frame_id: Option<i64>,
        context: EvaluateContext,
    ) -> Result<EvaluateView, String> {
        self.stopped_thread()?;
        if expression.len() > MAX_EXPRESSION {
            return Err("式が長すぎます".to_string());
        }
        let mut arguments = json!({
            "expression": expression,
            "context": match context {
                EvaluateContext::Repl => "repl",
                EvaluateContext::Watch => "watch",
            },
        });
        if let Some(frame_id) = frame_id {
            arguments["frameId"] = json!(frame_id);
        }
        let body = self.request("evaluate", arguments, REQUEST_WAIT)?;
        Ok(EvaluateView {
            result: body["result"].as_str().unwrap_or_default().to_string(),
            type_name: body["type"].as_str().map(str::to_string),
            variables_reference: body["variablesReference"].as_i64().unwrap_or_default(),
        })
    }

    // 止める（D-146 2.10）：disconnect → 待つ → アダプターのグループに SIGKILL → デバッグ対象に SIGKILL → 回収。
    // terminate は送らない。強制的に止めたなら true。debugpy では child がデバッグ対象そのもので、exited の知らせが
    // 来ないので、終了コードを child から受け取る（D-226）
    fn shutdown(&self, child: &mut Child) -> bool {
        let pid = child.id() as Pid;
        // 止める前に、自分で終わっていたか（debugpy の終了コードは、このときだけ使う）
        let ended = exited_without_reaping(pid);
        if !ended {
            let _ = self.begin_request("disconnect", json!({ "terminateDebuggee": true }));
            let until = Instant::now() + DISCONNECT_WAIT;
            while !exited_without_reaping(pid) && Instant::now() < until {
                thread::sleep(POLL_INTERVAL);
            }
        }
        let forced = !exited_without_reaping(pid);
        // 回収する前に、アダプターのグループの残りを止める
        kill_group(pid);
        if let Some(debuggee) = self.shared.lock_state().debuggee_pid
            && debuggee != pid
        {
            stop_debuggee(debuggee);
        }
        let status = child.wait().ok();
        // Windows：回収したので、Job の記録（プロセスのハンドルの写し）を消す（段階 6 W4。lsp.rs の Server::stop と同じ）
        #[cfg(windows)]
        let _ = crate::win::release_job(child.id());
        if self.shared.adapter == "debugpy" && ended {
            let mut state = self.shared.lock_state();
            if state.exit_code.is_none() {
                state.exit_code = status.and_then(|status| status.code());
                let code = state.exit_code;
                drop(state);
                debug_event(&self.shared.channel, DebugEvent::Exited { code });
            }
        }
        forced
    }
}

// pid のプロセスがあるか（回収されていないものを含む）
#[cfg(unix)]
fn alive(pid: i32) -> bool {
    // SAFETY: シグナル 0 は送らずに、あるかだけを確かめる
    unsafe { libc::kill(pid, 0) == 0 }
}

// Windows：まだ動いているか。デバッグは今回の移植に含めないので、段階 1 ではコンパイルが通るようにするだけ（docs/windows-port-plan.md 9.1 の 12）
#[cfg(windows)]
fn alive(pid: i32) -> bool {
    crate::win::is_alive(pid as u32)
}

// Windows：デバッグ対象が残っていれば止める（Unix の stop_debuggee と同じ流れ）
#[cfg(windows)]
fn stop_debuggee(pid: i32) {
    let until = Instant::now() + DEBUGGEE_WAIT;
    while alive(pid) && Instant::now() < until {
        thread::sleep(POLL_INTERVAL);
    }
    if alive(pid) {
        crate::win::terminate(pid as u32);
    }
}

// デバッグ対象が残っていれば SIGKILL を送り、少しの間、なくなるのを待つ
#[cfg(unix)]
fn stop_debuggee(pid: i32) {
    let until = Instant::now() + DEBUGGEE_WAIT;
    while alive(pid) && Instant::now() < until {
        thread::sleep(POLL_INTERVAL);
    }
    if alive(pid) {
        // SAFETY: process の知らせで受け取ったデバッグ対象の pid に送る（再利用の短い間の競合は残る。上の「限界」）
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

// ---- 今のセッション ----

// 今のデバッグのクライアント。動いていなければ None。決まった操作のコマンドがこれを使う
#[derive(Default)]
pub struct DebugControl(Mutex<Option<Arc<Client>>>);

impl DebugControl {
    fn set(&self, client: Option<Arc<Client>>) {
        if let Ok(mut current) = self.0.lock() {
            *current = client;
        }
    }

    pub fn current(&self) -> Result<Arc<Client>, String> {
        self.0
            .lock()
            .map_err(|_| "デバッグの状態を読めません".to_string())?
            .clone()
            .ok_or_else(|| NOT_DEBUGGING.to_string())
    }
}

// セッションの間だけ持つ。手放すと、今のクライアントを外す
struct SessionGuard<'a>(&'a DebugControl);

impl Drop for SessionGuard<'_> {
    fn drop(&mut self) {
        self.0.set(None);
    }
}

pub struct Request<'a> {
    pub dir: &'a Path,
    pub root: &'a Path,
    pub target: &'a Target,
    pub case: Option<&'a str>,
    pub breakpoints: &'a [SourceBreakpoints],
    // C++ のコンパイラと AC Library（D-205・D-206）
    pub langs: &'a LangSettings,
}

// デバッグの本体：アダプターを見つけ、dev でビルドし、起動して、終わる・止めるまで待つ
pub fn debug_target(
    request: &Request,
    environment: &Environment,
    control: &DebugControl,
    cancel: &Arc<AtomicBool>,
    channel: &Channel<RunEvent>,
) -> Finish {
    let Request {
        dir,
        root,
        target,
        case,
        breakpoints,
        langs,
    } = request;
    // アダプター（Python は debugpy）は、ビルド・構文の確かめの前に見つける（なければ進めない）
    let adapter = match find_session_adapter(target.language, langs, environment) {
        Ok(adapter) => adapter,
        Err(message) => {
            info(channel, format!("{message}\n"));
            return Finish::Failed { message };
        }
    };
    match &adapter {
        Adapter::Lldb(path) => info(
            channel,
            format!(
                "デバッグアダプター：{}（{}）\n",
                path.display(),
                adapter_version(path).unwrap_or_else(|| "版は分かりません".to_string())
            ),
        ),
        Adapter::Debugpy(dir) => info(
            channel,
            format!(
                "デバッグアダプター：debugpy {}（{}）\n",
                debugpy_version(dir).unwrap_or_else(|| "版は分かりません".to_string()),
                dir.display()
            ),
        ),
        #[cfg(windows)]
        Adapter::Gdb(launch) => info(
            channel,
            format!(
                "デバッグアダプター：{}（{}）\n",
                launch.program.display(),
                gdb_version(launch).unwrap_or_else(|| "版は分かりません".to_string())
            ),
        ),
        // Windows（段階 6 の Rust）：版が取れなかったときは、版の部分を出さずに場所だけ
        #[cfg(windows)]
        Adapter::LldbDap(launch) => info(
            channel,
            format!(
                "デバッグアダプター：{}{}\n",
                launch.program.display(),
                lldb_dap_version(launch)
                    .map(|version| format!("（{version}）"))
                    .unwrap_or_default()
            ),
        ),
    }

    let (
        Prepared {
            executable, bins, ..
        },
        (),
    ) = match prepare(dir, target, false, langs, cancel, channel, |_, _| {
        Ok(((), Vec::new()))
    }) {
        Ok(prepared) => prepared,
        Err(finish) => return finish,
    };
    info(
        channel,
        match target.language {
            Language::Rust => DEV_BUILD_NOTE,
            Language::Cpp => CPP_DEBUG_BUILD_NOTE,
            Language::Python => PYTHON_DEBUG_NOTE,
        },
    );

    // 標準入力
    let tests = toolchain::test_dir(dir, &bins, &target.bin);
    if let Some(notice) = &tests.notice {
        info(channel, format!("{notice}\n"));
    }
    let folder = tests
        .dir
        .as_ref()
        .map(|test| relative(dir, test))
        .unwrap_or_else(|| {
            if bins.len() == 1 {
                "test".to_string()
            } else {
                format!("test/{}", target.bin)
            }
        });
    let input = match choose_input(dir, tests.dir.as_deref(), &folder, *case) {
        Ok((path, shown)) => {
            info(channel, shown);
            path
        }
        Err(message) => {
            info(channel, format!("{message}\n"));
            return Finish::Failed { message };
        }
    };

    // 表示スクリプトと標準ライブラリのソースの場所（Rust だけ。取れなくても続ける）
    let rustc = match target.language {
        Language::Rust => Some(
            toolchain::find_cargo()
                .ok_or_else(|| "cargo が見つかりません".to_string())
                .and_then(|cargo| toolchain::rustc_paths(&cargo, dir)),
        ),
        Language::Cpp | Language::Python => None,
    };
    let rustc = match rustc {
        None => None,
        Some(Ok(paths)) => Some(paths),
        Some(Err(error)) => {
            info(
                channel,
                format!(
                    "rustc の sysroot を確かめられないので、変数の表示を整えずに続けます：{error}\n"
                ),
            );
            None
        }
    };

    info(channel, "\nデバッグを始めています…\n");
    let spawned = match &adapter {
        Adapter::Lldb(path) => Client::spawn(path, dir, root, environment.home.as_deref(), channel),
        // debugpy では、実行の命令の処理系（executable.path）でデバッグ対象を起動する
        Adapter::Debugpy(debugpy) => Client::spawn_debugpy(
            &executable.path,
            debugpy,
            dir,
            root,
            &input,
            cancel,
            channel,
        ),
        #[cfg(windows)]
        Adapter::Gdb(launch) => Client::spawn_gdb(launch, dir, root, channel),
        #[cfg(windows)]
        Adapter::LldbDap(launch) => Client::spawn_lldb_dap(launch, dir, root, channel),
    };
    let (client, mut child, notices) = match spawned {
        Ok(spawned) => spawned,
        Err(message) => {
            info(channel, format!("{message}\n"));
            return Finish::Failed { message };
        }
    };
    control.set(Some(client.clone()));
    let _session = SessionGuard(control);

    let plan = match &adapter {
        Adapter::Lldb(_) => SessionPlan {
            initialize: initialize_arguments(),
            start: "launch",
            start_arguments: launch_arguments(&executable.path, dir, &input, rustc.as_ref()),
            stop_functions: stop_functions(target.language),
            uncaught_exceptions: false,
        },
        Adapter::Debugpy(_) => SessionPlan {
            initialize: debugpy_initialize_arguments(),
            start: "attach",
            start_arguments: debugpy_attach_arguments(),
            stop_functions: &[],
            uncaught_exceptions: true,
        },
        // Windows（段階 6 の C++ W10）：止める関数は lldb-dap と同じ（__cxa_throw・abort。gdb でも止まることを P2 で確かめた）
        #[cfg(windows)]
        Adapter::Gdb(_) => SessionPlan {
            initialize: gdb_initialize_arguments(),
            start: "launch",
            start_arguments: gdb_launch_arguments(&executable.path, dir, &input),
            stop_functions: stop_functions(target.language),
            uncaught_exceptions: false,
        },
        // Windows（段階 6 の Rust W17・W18）：initialize は lldb-dap と同じ。launch は Windows 用
        #[cfg(windows)]
        Adapter::LldbDap(_) => SessionPlan {
            initialize: initialize_arguments(),
            start: "launch",
            start_arguments: lldb_dap_launch_arguments_windows(
                &executable.path,
                dir,
                &input,
                rustc.as_ref(),
            ),
            stop_functions: stop_functions(target.language),
            uncaught_exceptions: false,
        },
    };
    let started = start_session(&client, &notices, plan, breakpoints, cancel);
    let mut stopped = cancel.load(Ordering::SeqCst);
    match started {
        Ok(()) => {
            info(channel, "デバッグを始めました。\n");
            stopped = wait_until_end(&child, &notices, cancel);
            // debugpy は、デバッグ対象が終わる少し前に terminated を送る。自分で終わるのを少し待ち、終了コードを受け取る
            if !stopped && matches!(adapter, Adapter::Debugpy(_)) {
                let pid = child.id() as Pid;
                let until = Instant::now() + DISCONNECT_WAIT;
                while !exited_without_reaping(pid) && Instant::now() < until {
                    thread::sleep(POLL_INTERVAL);
                }
            }
        }
        Err(message) if !stopped => {
            info(channel, format!("{message}\n"));
            let forced = client.shutdown(&mut child);
            if forced {
                info(
                    channel,
                    format!(
                        "{} が終わらなかったので、強制的に止めました。\n",
                        client.shared.adapter
                    ),
                );
            }
            return Finish::Failed { message };
        }
        Err(_) => {}
    }

    let forced = client.shutdown(&mut child);
    if stopped {
        info(
            channel,
            format!("\nデバッグを止めました。{}\n", run::NOT_CHECKED),
        );
    } else {
        info(channel, "\nデバッグが終わりました。\n");
    }
    if forced {
        info(
            channel,
            format!(
                "{} が終わらなかったので、強制的に止めました。\n",
                client.shared.adapter
            ),
        );
    }
    Finish::Debugged {
        exit_code: client.shared.lock_state().exit_code,
        stopped,
    }
}

// 使うアダプター
enum Adapter {
    // lldb-dap の場所
    // （Windows では作らない。Rust は LldbDap、C++ は gdb。段階 6 の C++ W8・Rust W15）
    #[cfg_attr(windows, allow(dead_code))]
    Lldb(PathBuf),
    // debugpy を入れた場所（D-226）
    Debugpy(PathBuf),
    // Windows（段階 6 の C++ W8）：起動する gdb と、gdb の子の環境
    #[cfg(windows)]
    Gdb(GdbLaunch),
    // Windows（段階 6 の Rust W15）：起動する lldb-dap と、その子の PATH に足す Python のフォルダ
    #[cfg(windows)]
    LldbDap(LldbLaunch),
}

// 言語に合うアダプターを見つける。Python は設定の処理系と、その処理系の debugpy
fn find_session_adapter(
    language: Language,
    langs: &LangSettings,
    environment: &Environment,
) -> Result<Adapter, String> {
    match language {
        #[cfg(not(windows))]
        Language::Rust | Language::Cpp => find_adapter(&environment.xcrun).map(Adapter::Lldb),
        // Windows：Rust は LLVM の lldb-dap（段階 6 の Rust W15）。C++ は gdb（段階 6 の C++ W8）
        #[cfg(windows)]
        Language::Rust => find_lldb_launch(&langs.python).map(Adapter::LldbDap),
        #[cfg(windows)]
        Language::Cpp => find_gdb(&langs.cpp).map(Adapter::Gdb),
        Language::Python => {
            let interpreter = python::find_interpreter(&langs.python)?;
            find_debugpy(
                environment.python_tools.as_deref(),
                langs.python.interpreter,
                &interpreter,
            )
            .map(Adapter::Debugpy)
        }
    }
}

// 入れた debugpy の版（`debugpy-<版>.dist-info` の名前から）。分からなければ None
fn debugpy_version(dir: &Path) -> Option<String> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        name.strip_prefix("debugpy-")
            .and_then(|rest| rest.strip_suffix(".dist-info"))
            .map(str::to_string)
    })
}

// セッションの始め方（lldb-dap は launch、debugpy は attach。D-226）
struct SessionPlan {
    initialize: Value,
    start: &'static str,
    start_arguments: Value,
    // 止める関数（lldb-dap）
    stop_functions: &'static [(&'static str, &'static str)],
    // 捕まえていない例外で止める（debugpy）
    uncaught_exceptions: bool,
}

// initialize → launch（attach）→ initialized → ブレークポイント → configurationDone → launch（attach）の答え
fn start_session(
    client: &Client,
    notices: &Receiver<Notice>,
    plan: SessionPlan,
    breakpoints: &[SourceBreakpoints],
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    let channel = &client.shared.channel;
    let adapter = client.shared.adapter;
    let stop_functions = plan.stop_functions;
    client.request("initialize", plan.initialize, REQUEST_WAIT)?;
    let launch = client.begin_request(plan.start, plan.start_arguments)?;
    // initialized を待つ（止める印も見る）
    let until = Instant::now() + LAUNCH_WAIT;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("止めました".to_string());
        }
        match notices.recv_timeout(POLL_INTERVAL) {
            Ok(Notice::Initialized) => break,
            Ok(Notice::Terminated) | Err(RecvTimeoutError::Disconnected) => {
                return Err(format!("{adapter} が起動の途中で終わりました"));
            }
            Err(RecvTimeoutError::Timeout) if Instant::now() >= until => {
                return Err(format!("{adapter} の起動が終わりません"));
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
    for item in breakpoints {
        match client.set_breakpoints(&item.path, &item.lines) {
            Ok(views) => debug_event(
                channel,
                DebugEvent::Breakpoints {
                    path: item.path.clone(),
                    breakpoints: views,
                },
            ),
            Err(error) => info(
                channel,
                format!("ブレークポイントを置けません（{}）：{error}\n", item.path),
            ),
        }
    }
    // Python は、捕まえていない例外で止める（D-226）
    if plan.uncaught_exceptions {
        if let Err(error) = client.request(
            "setExceptionBreakpoints",
            json!({ "filters": ["uncaught"] }),
            REQUEST_WAIT,
        ) {
            info(channel, format!("例外で止める設定ができません：{error}\n"));
        }
        client.request("configurationDone", json!({}), REQUEST_WAIT)?;
        client.finish_request(launch, plan.start, LAUNCH_WAIT)?;
        return Ok(());
    }
    // panic（Rust）、例外と abort（C++）で止める（D-146 2.9・D-211）。答えのブレークポイントは、送った順に並ぶ
    let names: Vec<Value> = stop_functions
        .iter()
        .map(|(name, _)| json!({ "name": name }))
        .collect();
    match client.request(
        "setFunctionBreakpoints",
        json!({ "breakpoints": names }),
        REQUEST_WAIT,
    ) {
        Ok(body) => {
            client.shared.lock_state().stop_breakpoints = stop_functions
                .iter()
                .enumerate()
                .filter_map(|(index, (_, reason))| {
                    body["breakpoints"][index]["id"]
                        .as_i64()
                        .map(|id| (id, *reason))
                })
                .collect();
        }
        Err(error) => info(
            channel,
            format!("panic・例外で止める設定ができません：{error}\n"),
        ),
    }
    client.request("configurationDone", json!({}), REQUEST_WAIT)?;
    client.finish_request(launch, plan.start, LAUNCH_WAIT)?;
    Ok(())
}

// 終わる（terminated）・止める印・アダプターの終了を待つ。止める印で終えたなら true
fn wait_until_end(child: &Child, notices: &Receiver<Notice>, cancel: &Arc<AtomicBool>) -> bool {
    let pid = child.id() as Pid;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return true;
        }
        match notices.recv_timeout(POLL_INTERVAL) {
            Ok(Notice::Terminated) | Err(RecvTimeoutError::Disconnected) => return false,
            Ok(Notice::Initialized) | Err(RecvTimeoutError::Timeout) => {}
        }
        if exited_without_reaping(pid) {
            return false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tauri::ipc::InvokeResponseBody;

    struct Dir(PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn workspace(name: &str) -> Dir {
        let path = std::env::temp_dir().join(format!(
            "atcoder-studio-debug-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Dir(path.canonicalize().unwrap())
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn strips_symbol_hashes() {
        assert_eq!(
            strip_hash("solution::main::h4b2b0ff0f5cdd15b"),
            "solution::main"
        );
        assert_eq!(
            strip_hash("core::result::Result$LT$T$C$E$GT$::expect::h6254b53b2560e935"),
            "core::result::Result$LT$T$C$E$GT$::expect"
        );
        // ハッシュでないものは残す
        assert_eq!(strip_hash("main"), "main");
        assert_eq!(strip_hash("a::hello"), "a::hello");
        assert_eq!(strip_hash("a::h123"), "a::h123");
        assert_eq!(strip_hash("a::h4b2b0ff0f5cdd15z"), "a::h4b2b0ff0f5cdd15z");
    }

    #[cfg(unix)]
    #[test]
    fn accepts_breakpoints_only_in_workspace_rust_files() {
        let ws = workspace("paths");
        write(&ws.0.join("pkg/src/main.rs"), "fn main() {}\n");
        write(&ws.0.join("pkg/Cargo.toml"), "");
        let outside = workspace("paths-outside");
        write(&outside.0.join("x.rs"), "");
        assert_eq!(
            breakpoint_source(&ws.0, "pkg/src/main.rs").unwrap(),
            ws.0.join("pkg/src/main.rs")
        );
        assert!(breakpoint_source(&ws.0, "pkg/Cargo.toml").is_err());
        assert!(breakpoint_source(&ws.0, "pkg/src").is_err());
        // C++ のソースとヘッダーにも置ける（D-211）
        write(&ws.0.join("cpp/main.cpp"), "");
        write(&ws.0.join("cpp/lib.hpp"), "");
        write(&ws.0.join("cpp/compile_flags.txt"), "");
        assert!(breakpoint_source(&ws.0, "cpp/main.cpp").is_ok());
        assert!(breakpoint_source(&ws.0, "cpp/lib.hpp").is_ok());
        assert!(breakpoint_source(&ws.0, "cpp/compile_flags.txt").is_err());
        // 止める関数：Rust は panic、C++ は例外と abort
        assert_eq!(stop_functions(Language::Rust), &[("rust_panic", "panic")]);
        assert_eq!(
            stop_functions(Language::Cpp),
            &[("__cxa_throw", "throw"), ("abort", "abort")]
        );
        assert!(breakpoint_source(&ws.0, "../x.rs").is_err());
        assert!(breakpoint_source(&ws.0, &outside.0.join("x.rs").to_string_lossy()).is_err());
        // 外を指すリンク
        std::os::unix::fs::symlink(outside.0.join("x.rs"), ws.0.join("pkg/src/link.rs")).unwrap();
        assert!(breakpoint_source(&ws.0, "pkg/src/link.rs").is_err());
        // 作業領域の中かの判定
        assert_eq!(
            in_workspace(&ws.0, &ws.0.join("pkg/src/main.rs").to_string_lossy()),
            Some("pkg/src/main.rs".to_string())
        );
        assert_eq!(
            in_workspace(&ws.0, &outside.0.join("x.rs").to_string_lossy()),
            None
        );
        assert_eq!(
            in_workspace(&ws.0, "/rustc/abc/library/std/src/rt.rs"),
            None
        );
    }

    #[test]
    fn chooses_input() {
        let ws = workspace("input");
        let dir = ws.0.join("pkg");
        let test = dir.join("test");
        fs::create_dir_all(&test).unwrap();
        // input.txt がなければ /dev/null
        let (path, shown) = choose_input(&dir, Some(&test), "test", None).unwrap();
        #[cfg(not(windows))]
        assert_eq!(path, PathBuf::from("/dev/null"));
        // Windows は NUL（段階 6 W1）
        #[cfg(windows)]
        assert_eq!(path, PathBuf::from("NUL"));
        assert!(shown.contains("test/input.txt を作ってください"), "{shown}");
        let (path, _) = choose_input(&dir, None, "test", None).unwrap();
        #[cfg(not(windows))]
        assert_eq!(path, PathBuf::from("/dev/null"));
        #[cfg(windows)]
        assert_eq!(path, PathBuf::from("NUL"));
        write(&test.join("input.txt"), "1\n");
        let (path, shown) = choose_input(&dir, Some(&test), "test", None).unwrap();
        assert_eq!(path, test.join("input.txt"));
        assert_eq!(shown, "標準入力：test/input.txt\n");
        // ケース
        write(&test.join("sample-2.in"), "2\n");
        write(&test.join("random-3.in"), "3\n");
        write(&test.join("interactive-1.in"), "4\n");
        assert_eq!(
            choose_input(&dir, Some(&test), "test", Some("sample-2"))
                .unwrap()
                .0,
            test.join("sample-2.in")
        );
        assert_eq!(
            choose_input(&dir, Some(&test), "test", Some("random-3"))
                .unwrap()
                .0,
            test.join("random-3.in")
        );
        for wrong in [
            "interactive-1",
            "../sample-2",
            "sample-2.in",
            "input",
            "sample-",
        ] {
            assert!(
                choose_input(&dir, Some(&test), "test", Some(wrong)).is_err(),
                "{wrong}"
            );
        }
        let error = choose_input(&dir, Some(&test), "test", Some("sample-9")).unwrap_err();
        assert!(error.contains("test/sample-9.in"), "{error}");
        assert!(choose_input(&dir, None, "test", Some("sample-2")).is_err());
    }

    // ---- Windows 移植の段階 6（Python。docs/windows-port-plan.md「段階 6 の Python（debugpy）の計画」） ----

    // A1：input.txt もケースもないとき、標準入力にするパスを開けて、読むと空（入力を待たない）
    #[cfg(windows)]
    #[test]
    fn opens_an_empty_input_without_input_files_on_windows() {
        let ws = workspace("empty-input");
        let dir = ws.0.join("pkg");
        let test = dir.join("test");
        fs::create_dir_all(&test).unwrap();
        for test_dir in [Some(test.as_path()), None] {
            let (path, _) = choose_input(&dir, test_dir, "test", None).unwrap();
            let mut file = fs::File::open(&path).unwrap_or_else(|error| {
                panic!(
                    "標準入力にするパス {} を開けません：{error}",
                    path.display()
                )
            });
            let mut read = Vec::new();
            file.read_to_end(&mut read).unwrap();
            assert!(read.is_empty(), "空でない：{} バイト", read.len());
        }
    }

    // A2：debugpy を動かす命令に -X utf8 が付く（付けないと、CPython はパイプへ cp932 で出す。4.4）
    #[cfg(windows)]
    #[test]
    fn passes_utf8_mode_to_debugpy_on_windows() {
        let command = debugpy_command(Path::new("python.exe"), Path::new("tools"), 5678, "main.py");
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "-X" && pair[1] == "utf8"),
            "-X utf8 がない：{args:?}"
        );
    }

    // A6：debugpy を動かす命令に -u が、-c より前（処理系への指定）に付く（段階 6 W7。付けないと、パイプへの標準出力が
    // まとめて書かれ、止まっている間や強制的に止めたときに出力が届かない）
    #[cfg(windows)]
    #[test]
    fn passes_unbuffered_to_debugpy_on_windows() {
        let command = debugpy_command(Path::new("python.exe"), Path::new("tools"), 5678, "main.py");
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let unbuffered = args.iter().position(|arg| arg == "-u");
        let script = args.iter().position(|arg| arg == "-c");
        assert!(
            matches!((unbuffered, script), (Some(u), Some(c)) if u < c),
            "-c より前に -u がない：{args:?}"
        );
    }

    // テスト用の子のプログラム（30 秒眠るだけ）を、IDE と同じく Job に入れて起動する
    #[cfg(windows)]
    fn sleeping_child_in_job() -> Child {
        let mut command = crate::process::command(crate::process::test_child_path());
        command
            .arg("sleep:30000")
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command.spawn().unwrap();
        crate::win::start_in_job(&child).unwrap();
        assert!(
            crate::win::has_job_record(child.id()),
            "起動したのに記録がない"
        );
        child
    }

    // A3：shutdown で止めたあと、Job の記録（win.rs の JOBS。プロセスのハンドルの写しを持つ）が残らない。
    // debugpy の代わりに、テスト用の子のプログラムと、どこにもつながらないクライアントを使う（disconnect に答えないので約 2 秒）
    #[cfg(windows)]
    #[test]
    fn forgets_the_job_record_after_debug_shutdown_on_windows() {
        let mut child = sleeping_child_in_job();
        let pid = child.id();
        let channel = Channel::new(|_| Ok(()));
        let (client, _notices) = Client::connect(
            "debugpy",
            std::io::sink(),
            std::io::empty(),
            Path::new("."),
            &channel,
        );
        assert!(client.shutdown(&mut child), "強制的に止めたはず");
        assert!(
            !crate::win::has_job_record(pid),
            "止めたあとも JOBS に pid {pid} の記録が残っている"
        );
    }

    // A4：debugpy につなげなかったときの止め方（stop_and_reap）のあと、Job の記録が残らない
    #[cfg(windows)]
    #[test]
    fn forgets_the_job_record_when_debugpy_does_not_start_on_windows() {
        let mut child = sleeping_child_in_job();
        let pid = child.id();
        stop_and_reap(&mut child);
        assert!(
            !crate::win::has_job_record(pid),
            "止めたあとも JOBS に pid {pid} の記録が残っている"
        );
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "Windows：デバッグは段階 6（今回の移植には含めない）。docs/windows-port-plan.md 7 章"
    )]
    fn builds_launch_configuration() {
        let init = initialize_arguments();
        assert_eq!(init["$__lldb_sourceInitFile"], false);
        assert!(init.get("supportsRunInTerminalRequest").is_none());
        let ws = workspace("launch");
        let sysroot = ws.0.join("sys root");
        fs::create_dir_all(sysroot.join("lib/rustlib/src/rust")).unwrap();
        let rustc = RustcPaths {
            sysroot: sysroot.clone(),
            commit_hash: Some("29483883eed69d5fb4db01964cdf2af4d86e9cb2".to_string()),
        };
        let launch = launch_arguments(
            Path::new("/p/target/debug/solution"),
            Path::new("/p"),
            Path::new("/p/test/a \"b\".in"),
            Some(&rustc),
        );
        assert_eq!(launch["program"], "/p/target/debug/solution");
        assert_eq!(launch["cwd"], "/p");
        assert!(launch.get("stdio").is_none());
        assert!(launch.get("console").is_none());
        assert_eq!(
            launch["preRunCommands"][0],
            "settings set target.input-path \"/p/test/a \\\"b\\\".in\""
        );
        assert_eq!(
            launch["preRunCommands"][1],
            "settings set target.inherit-tcc true"
        );
        let commands = launch["initCommands"].as_array().unwrap();
        assert_eq!(commands.len(), 3);
        assert!(
            commands[0]
                .as_str()
                .unwrap()
                .ends_with("sys root/lib/rustlib/etc/lldb_lookup.py\"")
        );
        assert!(
            commands[1]
                .as_str()
                .unwrap()
                .starts_with("command source -s 0 \"")
        );
        assert_eq!(
            commands[2],
            format!(
                "settings set target.source-map /rustc/29483883eed69d5fb4db01964cdf2af4d86e9cb2 \"{}\"",
                sysroot.join("lib/rustlib/src/rust").display()
            )
        );
        // rust-src がない・コミットが分からないときは対応付けをしない。sysroot が分からなければ表示スクリプトもなし
        let no_hash = RustcPaths {
            sysroot: sysroot.clone(),
            commit_hash: None,
        };
        assert_eq!(init_commands(&no_hash).len(), 2);
        let launch = launch_arguments(
            Path::new("/a"),
            Path::new("/"),
            Path::new("/dev/null"),
            None,
        );
        assert_eq!(launch["initCommands"], json!([]));
    }

    // アダプターが止めなかったデバッグ対象を、pid で止める
    #[cfg(unix)]
    #[test]
    fn stops_leftover_debuggee() {
        let mut child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let pid = child.id() as i32;
        // 回収するのは別のスレッド（デバッグ対象は IDE の子ではないので、ここでは回収を待たない）
        let waiter = thread::spawn(move || child.wait().unwrap());
        let started = Instant::now();
        stop_debuggee(pid);
        let status = waiter.join().unwrap();
        assert_eq!(
            std::os::unix::process::ExitStatusExt::signal(&status),
            Some(libc::SIGKILL)
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "Windows：デバッグは段階 6（今回の移植には含めない）。docs/windows-port-plan.md 7 章"
    )]
    fn reports_missing_adapter() {
        let error = find_adapter(Path::new("/nonexistent/xcrun")).unwrap_err();
        assert!(error.starts_with(ADAPTER_MISSING), "{error}");
        // xcrun が失敗した（見つからない）とき：false は何も出さずに 1 で終わる
        let error = find_adapter(Path::new("/usr/bin/false")).unwrap_err();
        assert_eq!(error, ADAPTER_MISSING);
    }

    // ---- 本物の lldb-dap を使うテスト。Command Line Tools と cargo が要る ----

    const SOLUTION: &str = r#"use std::collections::HashMap;
use std::io::Read;

fn sum(v: &[i64]) -> i64 {
    let mut s = 0;
    for x in v {
        s += x;
    }
    s
}

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let mut it = input.split_whitespace();
    let n: usize = it.next().unwrap_or("0").parse().unwrap();
    let a: Vec<i64> = (0..n).map(|_| it.next().unwrap().parse().unwrap()).collect();
    let name = String::from("atcoder");
    let mut count: HashMap<i64, usize> = HashMap::new();
    for &x in &a {
        *count.entry(x).or_insert(0) += 1;
    }
    let total = sum(&a);
    println!("{} {} {}", total, name, count.len());
    if let Ok(mark) = std::env::var("LLDBINIT_MARK") {
        println!("MARK {mark}");
    }
    if input.contains("spin") {
        let mut i: u64 = 0;
        loop {
            i = i.wrapping_add(1);
        }
    }
    if input.contains("boom") {
        let x: i32 = 2147483600 + n as i32;
        println!("{}", x + 100);
    }
}
"#;
    // 行番号（SOLUTION の中）
    const LINE_S_ADD: u32 = 7;
    const LINE_TOTAL: u32 = 23;

    struct Session {
        events: Arc<Mutex<Vec<Value>>>,
        control: Arc<DebugControl>,
        cancel: Arc<AtomicBool>,
        handle: Option<thread::JoinHandle<Finish>>,
    }

    impl Session {
        fn start(
            pkg: &Path,
            root: &Path,
            case: Option<&str>,
            breakpoints: Vec<SourceBreakpoints>,
            home: Option<PathBuf>,
        ) -> Session {
            let target = Target {
                package: "pkg".to_string(),
                bin: "solution".to_string(),
                language: Language::Rust,
            };
            Session::start_target(pkg, root, case, breakpoints, home, target)
        }

        fn start_target(
            pkg: &Path,
            root: &Path,
            case: Option<&str>,
            breakpoints: Vec<SourceBreakpoints>,
            home: Option<PathBuf>,
            target: Target,
        ) -> Session {
            let environment = Environment {
                home,
                ..Environment::real(None)
            };
            Session::start_with(
                pkg,
                root,
                case,
                breakpoints,
                target,
                LangSettings::default(),
                environment,
            )
        }

        fn start_with(
            pkg: &Path,
            root: &Path,
            case: Option<&str>,
            breakpoints: Vec<SourceBreakpoints>,
            target: Target,
            langs: LangSettings,
            environment: Environment,
        ) -> Session {
            let events = Arc::new(Mutex::new(Vec::new()));
            let sink = events.clone();
            let channel = Channel::new(move |body| {
                if let InvokeResponseBody::Json(text) = body {
                    sink.lock()
                        .unwrap()
                        .push(serde_json::from_str(&text).unwrap());
                }
                Ok(())
            });
            let control = Arc::new(DebugControl::default());
            let cancel = Arc::new(AtomicBool::new(false));
            let (pkg, root, case) = (
                pkg.to_path_buf(),
                root.to_path_buf(),
                case.map(str::to_string),
            );
            let (control2, cancel2) = (control.clone(), cancel.clone());
            let handle = thread::spawn(move || {
                debug_target(
                    &Request {
                        dir: &pkg,
                        root: &root,
                        target: &target,
                        case: case.as_deref(),
                        breakpoints: &breakpoints,
                        langs: &langs,
                    },
                    &environment,
                    &control2,
                    &cancel2,
                    &channel,
                )
            });
            Session {
                events,
                control,
                cancel,
                handle: Some(handle),
            }
        }

        // 条件に合う Debug の知らせを、since 番目より後から待つ
        fn wait(&self, since: usize, wanted: impl Fn(&Value) -> bool) -> (usize, Value) {
            let until = Instant::now() + Duration::from_secs(120);
            loop {
                {
                    let events = self.events.lock().unwrap();
                    if let Some((index, event)) = events
                        .iter()
                        .enumerate()
                        .skip(since)
                        .find(|(_, event)| event["type"] == "debug" && wanted(&event["event"]))
                    {
                        return (index + 1, event["event"].clone());
                    }
                }
                #[cfg(not(windows))]
                assert!(
                    Instant::now() < until,
                    "待っている知らせが来ません：{:?}",
                    self.events.lock().unwrap()
                );
                // Windows（段階 6 T3）：失敗の文に、アプリの識別子とユーザーのフォルダの名前を出さない
                #[cfg(windows)]
                assert!(
                    Instant::now() < until,
                    "待っている知らせが来ません：{}",
                    hide_names(&format!("{:?}", self.events.lock().unwrap()))
                );
                thread::sleep(Duration::from_millis(20));
            }
        }

        fn wait_stopped(&self, since: usize) -> (usize, Value) {
            self.wait(since, |event| event["kind"] == "stopped")
        }

        fn client(&self) -> Arc<Client> {
            self.control.current().unwrap()
        }

        fn finish(mut self) -> Finish {
            self.handle.take().unwrap().join().unwrap()
        }

        fn output(&self) -> String {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event["type"] == "debug" && event["event"]["kind"] == "output")
                .map(|event| event["event"]["text"].as_str().unwrap().to_string())
                .collect()
        }

        fn log(&self) -> String {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event["type"] == "log")
                .map(|event| event["text"].as_str().unwrap().to_string())
                .collect()
        }

        fn debuggee_pid(&self) -> Option<i32> {
            self.control
                .current()
                .ok()?
                .shared
                .lock_state()
                .debuggee_pid
        }
    }

    fn package(name: &str) -> (Dir, PathBuf) {
        let ws = workspace(name);
        let pkg = ws.0.join("pkg");
        write(
            &pkg.join("Cargo.toml"),
            "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"solution\"\npath = \"src/solution.rs\"\n",
        );
        write(&pkg.join("src/solution.rs"), SOLUTION);
        (ws, pkg)
    }

    fn breakpoints(lines: &[u32]) -> Vec<SourceBreakpoints> {
        vec![SourceBreakpoints {
            path: "pkg/src/solution.rs".to_string(),
            lines: lines.to_vec(),
        }]
    }

    fn assert_gone(pid: i32) {
        let until = Instant::now() + Duration::from_secs(3);
        while alive(pid) && Instant::now() < until {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!alive(pid), "デバッグ対象 {pid} が残っています");
    }

    // 止まる → スタック → 変数 → ステップ → 式 → 続行して終わる
    #[test]
    #[cfg_attr(
        windows,
        ignore = "Windows：デバッグは段階 6（今回の移植には含めない）。docs/windows-port-plan.md 7 章"
    )]
    fn debugs_with_real_lldb_dap() {
        let (ws, pkg) = package("e2e");
        write(&pkg.join("test/input.txt"), "5\n3 1 4 1 5\n");
        let session = Session::start(&pkg, &ws.0, None, breakpoints(&[LINE_TOTAL]), None);
        let (since, stopped) = session.wait_stopped(0);
        assert_eq!(stopped["reason"], "breakpoint");
        let initial = session.wait(0, |event| event["kind"] == "breakpoints").1;
        assert_eq!(initial["path"], "pkg/src/solution.rs");
        assert_eq!(initial["breakpoints"][0]["verified"], true);
        assert_eq!(initial["breakpoints"][0]["line"], LINE_TOTAL);
        assert_eq!(initial["breakpoints"][0]["path"], "pkg/src/solution.rs");
        let log = session.log();
        assert!(log.contains("デバッグアダプター："), "{log}");
        assert!(log.contains(DEV_BUILD_NOTE), "{log}");
        assert!(log.contains("標準入力：test/input.txt"), "{log}");

        let client = session.client();
        let stack = client.stack().unwrap();
        assert_eq!(stack[0].name, "solution::main");
        assert_eq!(
            stack[0].workspace_path.as_deref(),
            Some("pkg/src/solution.rs")
        );
        assert_eq!(stack[0].line, LINE_TOTAL as u64);
        // 標準ライブラリのフレームは作業領域の外。rust-src があれば、その場所に読み替わる
        let outside = stack
            .iter()
            .find(|frame| frame.name.starts_with("std::rt::lang_start"))
            .unwrap();
        assert_eq!(outside.workspace_path, None);
        assert!(
            outside
                .source
                .as_deref()
                .is_some_and(|path| !path.starts_with("/rustc/")),
            "{outside:?}"
        );
        assert!(
            outside
                .source_uri
                .as_deref()
                .is_some_and(|uri| uri.starts_with("file:///")),
            "{outside:?}"
        );

        let scopes = client.scopes(stack[0].id).unwrap();
        let locals = client.variables(scopes[0].variables_reference).unwrap();
        let find = |name: &str| locals.iter().find(|v| v.name == name).unwrap().clone();
        assert_eq!(find("n").value, "5");
        assert_eq!(find("a").value, "size=5");
        assert_eq!(find("name").value, "\"atcoder\"");
        assert_eq!(find("count").value, "size=4");
        let elements = client.variables(find("a").variables_reference).unwrap();
        assert_eq!(elements[2].name, "[2]");
        assert_eq!(elements[2].value, "4");

        // ステップイン（Vec から &[i64] への変換か sum に入る）→ ステップアウトで main に戻る
        client.step(StepAction::StepIn).unwrap();
        let (since, stopped) = session.wait_stopped(since);
        assert_eq!(stopped["reason"], "step");
        assert_ne!(client.stack().unwrap()[0].name, "solution::main");
        client.step(StepAction::StepOut).unwrap();
        let (since, _) = session.wait_stopped(since);
        assert_eq!(client.stack().unwrap()[0].name, "solution::main");
        // sum の中で式を評価する
        client
            .set_breakpoints("pkg/src/solution.rs", &[LINE_S_ADD])
            .unwrap();
        client.step(StepAction::Continue).unwrap();
        let (since, _) = session.wait_stopped(since);
        let top = client.stack().unwrap()[0].clone();
        assert_eq!(top.line, LINE_S_ADD as u64);
        let watch = client
            .evaluate("s + 1", Some(top.id), EvaluateContext::Watch)
            .unwrap();
        assert_eq!(watch.result, "1");
        let backtrace = client
            .evaluate("`bt", Some(top.id), EvaluateContext::Repl)
            .unwrap();
        assert!(backtrace.result.contains("solution::sum"), "{backtrace:?}");
        // デバッグの対象は IDE の許可を引き継ぐ（D-217）
        let tcc = client
            .evaluate(
                "`settings show target.inherit-tcc",
                Some(top.id),
                EvaluateContext::Repl,
            )
            .unwrap();
        assert!(tcc.result.contains("= true"), "{tcc:?}");
        client.step(StepAction::Next).unwrap();
        let (since, stopped) = session.wait_stopped(since);
        assert_eq!(stopped["reason"], "step");

        // 動いている間は、式・スタックを断る
        let pid = session.debuggee_pid().unwrap();
        client.set_breakpoints("pkg/src/solution.rs", &[]).unwrap();
        client.step(StepAction::Continue).unwrap();
        assert_eq!(client.stack().unwrap_err(), NOT_STOPPED);
        assert_eq!(
            client
                .evaluate("1", None, EvaluateContext::Repl)
                .unwrap_err(),
            NOT_STOPPED
        );
        session.wait(since, |event| {
            event["kind"] == "exited" && event["code"] == 0
        });
        let output = session.output();
        let finish = session.finish();
        assert!(
            matches!(
                finish,
                Finish::Debugged {
                    exit_code: Some(0),
                    stopped: false
                }
            ),
            "{finish:?}"
        );
        assert!(output.contains("14 atcoder 4"), "{output}");
        // 表示スクリプトの読み込みの出力は出さない
        assert!(!output.contains("type synthetic add"), "{output}");
        assert_gone(pid);
    }

    // ---- C++（Phase 17 作業 6。D-211・D-214）。g++-15 も要る ----

    const CPP_SOLUTION: &str = r#"#include <bits/stdc++.h>
using namespace std;

long long sum(const vector<long long>& v) {
    long long s = 0;
    for (auto x : v) s += x;
    return s;
}

int main() {
    int n;
    cin >> n;
    vector<long long> a(n);
    for (auto& x : a) cin >> x;
    string name = "atcoder";
    long long total = sum(a);
    cout << total << " " << name << endl;
    if (total == 7) throw runtime_error("boom");
    if (total == 8) cout << a[n] << endl;
}
"#;
    const CPP_LINE_S_ADD: u32 = 6;
    const CPP_LINE_TOTAL: u32 = 16;

    fn cpp_session(pkg: &Path, root: &Path, case: Option<&str>, lines: &[u32]) -> Session {
        let target = Target {
            package: "pkg".to_string(),
            bin: crate::cpp::BIN.to_string(),
            language: Language::Cpp,
        };
        let breakpoints = if lines.is_empty() {
            Vec::new()
        } else {
            vec![SourceBreakpoints {
                path: "pkg/main.cpp".to_string(),
                lines: lines.to_vec(),
            }]
        };
        Session::start_target(pkg, root, case, breakpoints, None, target)
    }

    // 止まる → 変数（vector・string）→ ステップ → 式 → 続行。例外・範囲外の読み書きで止まる
    #[test]
    #[cfg_attr(
        windows,
        ignore = "Windows：デバッグは段階 6（今回の移植には含めない）。docs/windows-port-plan.md 7 章"
    )]
    fn debugs_cpp_with_real_lldb_dap() {
        if crate::cpp::find_compiler(&crate::cpp::CppSettings::default()).is_err() {
            eprintln!("g++-15 がないので飛ばします");
            return;
        }
        let ws = workspace("cpp");
        let pkg = ws.0.join("pkg");
        write(&pkg.join("main.cpp"), CPP_SOLUTION);
        write(&pkg.join("test/input.txt"), "3\n1 2 3\n");
        write(&pkg.join("test/sample-1.in"), "2\n3 4\n");
        write(&pkg.join("test/sample-2.in"), "2\n3 5\n");

        let session = cpp_session(&pkg, &ws.0, None, &[CPP_LINE_TOTAL]);
        let (since, stopped) = session.wait_stopped(0);
        assert_eq!(stopped["reason"], "breakpoint");
        let log = session.log();
        assert!(log.contains(CPP_DEBUG_BUILD_NOTE), "{log}");
        assert!(!log.contains("sysroot"), "{log}");
        let client = session.client();
        let stack = client.stack().unwrap();
        assert_eq!(stack[0].workspace_path.as_deref(), Some("pkg/main.cpp"));
        assert_eq!(stack[0].line, CPP_LINE_TOTAL as u64);
        let scopes = client.scopes(stack[0].id).unwrap();
        let locals = client.variables(scopes[0].variables_reference).unwrap();
        let find = |name: &str| locals.iter().find(|v| v.name == name).unwrap().clone();
        assert_eq!(find("n").value, "3");
        assert_eq!(find("a").value, "size=3");
        assert_eq!(find("name").value, "\"atcoder\"");
        let elements = client.variables(find("a").variables_reference).unwrap();
        assert_eq!(elements[2].value, "3");
        // sum の中で止めて、式を評価する
        client
            .set_breakpoints("pkg/main.cpp", &[CPP_LINE_S_ADD])
            .unwrap();
        client.step(StepAction::Continue).unwrap();
        let (since, _) = session.wait_stopped(since);
        let top = client.stack().unwrap()[0].clone();
        assert_eq!(top.line, CPP_LINE_S_ADD as u64);
        let watch = client
            .evaluate("s + 10", Some(top.id), EvaluateContext::Watch)
            .unwrap();
        assert_eq!(watch.result, "10");
        client.step(StepAction::Next).unwrap();
        let (since, stopped) = session.wait_stopped(since);
        assert_eq!(stopped["reason"], "step");
        client.set_breakpoints("pkg/main.cpp", &[]).unwrap();
        client.step(StepAction::Continue).unwrap();
        session.wait(since, |event| {
            event["kind"] == "exited" && event["code"] == 0
        });
        let output = session.output();
        assert!(output.contains("6 atcoder"), "{output}");
        assert!(matches!(
            session.finish(),
            Finish::Debugged {
                exit_code: Some(0),
                stopped: false
            }
        ));

        // 例外を投げると止まり、呼んだ行が main.cpp に見える
        let session = cpp_session(&pkg, &ws.0, Some("sample-1"), &[]);
        let (_, stopped) = session.wait_stopped(0);
        assert_eq!(stopped["reason"], "throw", "{stopped}");
        let stack = session.client().stack().unwrap();
        assert!(
            stack
                .iter()
                .any(|frame| frame.workspace_path.as_deref() == Some("pkg/main.cpp")),
            "{stack:?}"
        );
        session.cancel.store(true, Ordering::SeqCst);
        assert!(matches!(
            session.finish(),
            Finish::Debugged { stopped: true, .. }
        ));

        // 範囲外の読み書きは、デバッグ用のビルドでは abort になって止まる（D-214）
        let session = cpp_session(&pkg, &ws.0, Some("sample-2"), &[]);
        let (_, stopped) = session.wait_stopped(0);
        assert_eq!(stopped["reason"], "abort", "{stopped}");
        session.cancel.store(true, Ordering::SeqCst);
        assert!(matches!(
            session.finish(),
            Finish::Debugged { stopped: true, .. }
        ));
    }

    // 動いている間に止める・一時停止・panic
    #[test]
    #[cfg_attr(
        windows,
        ignore = "Windows：デバッグは段階 6（今回の移植には含めない）。docs/windows-port-plan.md 7 章"
    )]
    fn stops_pauses_and_catches_panics() {
        let (ws, pkg) = package("stop");
        write(&pkg.join("test/sample-1.in"), "1 7 spin\n");
        write(&pkg.join("test/sample-2.in"), "1 7 boom\n");

        // 一時停止してから、止める印を立てる
        let session = Session::start(&pkg, &ws.0, Some("sample-1"), Vec::new(), None);
        session.wait(0, |event| {
            event["kind"] == "output" && event["text"].as_str().unwrap().contains("7 atcoder 1")
        });
        let client = session.client();
        let pid = session.debuggee_pid().unwrap();
        client.step(StepAction::Pause).unwrap();
        let (since, stopped) = session.wait_stopped(0);
        // Apple 版の 21.0.0 では、一時停止の理由が exception で届く（01 §3）
        assert!(
            ["pause", "exception"].contains(&stopped["reason"].as_str().unwrap()),
            "{stopped}"
        );
        let stack = client.stack().unwrap();
        assert!(
            stack.iter().any(|frame| frame.name == "solution::main"),
            "{stack:?}"
        );
        client.step(StepAction::Continue).unwrap();
        session.wait(since, |event| event["kind"] == "continued");
        let started = Instant::now();
        session.cancel.store(true, Ordering::SeqCst);
        let control = session.control.clone();
        let finish = session.finish();
        assert!(
            matches!(finish, Finish::Debugged { stopped: true, .. }),
            "{finish:?}"
        );
        assert!(
            // disconnect で止まれば、アダプターを待つ 2 秒（DISCONNECT_WAIT）より十分に早い
            started.elapsed() < Duration::from_millis(1500),
            "{:?}",
            started.elapsed()
        );
        assert_gone(pid);
        assert_eq!(control.current().err().as_deref(), Some(NOT_DEBUGGING));

        // panic で止まり、続行すると 101 で終わる
        let session = Session::start(&pkg, &ws.0, Some("sample-2"), Vec::new(), None);
        let (since, stopped) = session.wait_stopped(0);
        assert_eq!(stopped["reason"], "panic");
        let client = session.client();
        let stack = client.stack().unwrap();
        assert!(
            stack
                .iter()
                .any(|frame| frame.workspace_path.as_deref() == Some("pkg/src/solution.rs")),
            "{stack:?}"
        );
        client.step(StepAction::Continue).unwrap();
        session.wait(since, |event| event["kind"] == "exited");
        let output = session.output();
        let finish = session.finish();
        assert!(
            matches!(
                finish,
                Finish::Debugged {
                    exit_code: Some(101),
                    stopped: false
                }
            ),
            "{finish:?}"
        );
        assert!(output.contains("attempt to add with overflow"), "{output}");
    }

    // ~/.lldbinit と作業フォルダの .lldbinit を読まない（AC5-12）。input.txt がなくても入力を待たない。
    // 初期化ファイルの出力は launch の答えより前なので画面に出さない（捨てる）。そこで、読んだらプログラムの
    // 環境変数に目印が付くようにして、プログラムの出力で確かめる
    #[test]
    #[cfg_attr(
        windows,
        ignore = "Windows：デバッグは段階 6（今回の移植には含めない）。docs/windows-port-plan.md 7 章"
    )]
    fn ignores_lldbinit_and_does_not_wait_for_input() {
        let (ws, pkg) = package("init");
        let home = ws.0.join("home");
        write(
            &home.join(".lldbinit"),
            "settings set target.env-vars LLDBINIT_MARK=home\n",
        );
        write(
            &pkg.join(".lldbinit"),
            "settings set target.env-vars LLDBINIT_MARK=cwd\n",
        );
        let session = Session::start(&pkg, &ws.0, None, Vec::new(), Some(home));
        session.wait(0, |event| event["kind"] == "exited");
        let output = session.output();
        let log = session.log();
        let finish = session.finish();
        assert!(
            matches!(
                finish,
                Finish::Debugged {
                    exit_code: Some(0),
                    stopped: false
                }
            ),
            "{finish:?}"
        );
        assert!(output.contains("0 atcoder 0"), "{output}");
        assert!(log.contains("入力ファイルなし"), "{log}");
        assert!(!output.contains("MARK"), "{output}");
    }

    // ---- 本物の debugpy を使うテスト（D-226）。処理系と、アプリのフォルダの python/<処理系> の debugpy が要る ----

    // （PYTHON_SOLUTION・PY_LINE_*・python_session は、Windows 移植の段階 6 T3 で cfg(unix) を外し、Windows のテストでも使う）
    const PYTHON_SOLUTION: &str = "import sys


def solve(n):
    xs = [i * i for i in range(n)]
    s = \"abc\"
    total = sum(xs)
    return total


def main():
    n = int(input())
    t = solve(n)
    print(t)
    if n == 7:
        d = {}
        print(d[\"x\"])
    if n == 9:
        sys.exit(3)
    if n == 11:
        while True:
            pass


main()
";
    // 行番号（PYTHON_SOLUTION の中）
    const PY_LINE_TOTAL: u32 = 7;
    const PY_LINE_RETURN: u32 = 8;
    const PY_LINE_KEY_ERROR: u32 = 17;

    // 利用者がアプリのフォルダに入れた debugpy（なければ None）
    #[cfg(unix)]
    fn installed_python_tools() -> Option<PathBuf> {
        let tools = PathBuf::from(std::env::var_os("HOME")?)
            .join("Library/Application Support/io.github.ishigaki892.acdstudio/python");
        tools.is_dir().then_some(tools)
    }

    // Windows（段階 6 T3）：アプリの識別子（tauri.conf.json の identifier。ここには書き写さない）
    #[cfg(windows)]
    fn app_identifier() -> Option<String> {
        let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).ok()?;
        config["identifier"].as_str().map(str::to_string)
    }

    // Windows（段階 6 T3）：利用者がアプリの設定のフォルダ（%APPDATA%\<識別子>）の python に入れた debugpy（なければ None）
    #[cfg(windows)]
    fn installed_python_tools() -> Option<PathBuf> {
        let tools = PathBuf::from(std::env::var_os("APPDATA")?)
            .join(app_identifier()?)
            .join("python");
        tools.is_dir().then_some(tools)
    }

    // Windows（段階 6 T3）：テストの失敗の文に、アプリの識別子とユーザーのフォルダの名前を出さない
    #[cfg(windows)]
    fn hide_names(text: &str) -> String {
        let mut text = text.to_string();
        if let Some(identifier) = app_identifier() {
            text = text.replace(&identifier, "<識別子>");
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            let home = home.to_string_lossy();
            // {:?} で書くと `\` が 2 つになるので、その形も伏せる
            text = text.replace(&home.replace('\\', "\\\\"), "%USERPROFILE%");
            text = text.replace(&*home, "%USERPROFILE%");
        }
        text
    }

    fn python_session(
        pkg: &Path,
        root: &Path,
        case: Option<&str>,
        lines: &[u32],
        interpreter: Interpreter,
        tools: &Path,
    ) -> Session {
        let target = Target {
            package: "pkg".to_string(),
            bin: python::BIN.to_string(),
            language: Language::Python,
        };
        let mut langs = LangSettings::default();
        langs.python.interpreter = interpreter;
        let breakpoints = vec![SourceBreakpoints {
            path: "pkg/main.py".to_string(),
            lines: lines.to_vec(),
        }];
        Session::start_with(
            pkg,
            root,
            case,
            breakpoints,
            target,
            langs,
            Environment::real(Some(tools.to_path_buf())),
        )
    }

    #[test]
    fn reports_missing_debugpy() {
        let ws = workspace("no-debugpy");
        let message = find_debugpy(
            Some(&ws.0),
            Interpreter::Pypy,
            Path::new("/opt/homebrew/bin/pypy3.11"),
        )
        .unwrap_err();
        let dir = ws.0.join("pypy");
        assert!(
            message.contains(&format!(
                "\"/opt/homebrew/bin/pypy3.11\" -m pip install --target \"{}\" debugpy",
                dir.display()
            )),
            "{message}"
        );
        // Windows は PowerShell 向けの文で、命令の先頭に & が付く（段階 6 W5）
        #[cfg(windows)]
        assert!(
            message.contains(&format!(
                "PowerShell で次を実行して入れてください：\n& \"/opt/homebrew/bin/pypy3.11\" -m pip install --target \"{}\" debugpy",
                dir.display()
            )),
            "{message}"
        );
        write(&dir.join("debugpy/__init__.py"), "");
        assert_eq!(
            find_debugpy(Some(&ws.0), Interpreter::Pypy, Path::new("/x")).unwrap(),
            dir
        );
        // CPython 用は別の場所
        assert!(find_debugpy(Some(&ws.0), Interpreter::Cpython, Path::new("/x")).is_err());
        // 起動の命令：-X frozen_modules=off、debugpy の場所、127.0.0.1 のポート、つなぎを待つ
        let command = debugpy_command(Path::new("/py"), &dir, 5678, "main.py");
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        // Windows は先頭に -X utf8（段階 6 W2）と -u（段階 6 W7）が付く。残りは Unix と同じ
        #[cfg(windows)]
        let args = {
            assert_eq!(args[..3], ["-X", "utf8", "-u"], "{args:?}");
            args[3..].to_vec()
        };
        assert_eq!(
            args[..6],
            [
                "-E",
                "-B",
                "-X",
                "int_max_str_digits=0",
                "-X",
                "frozen_modules=off"
            ]
        );
        assert_eq!(args[6], "-c");
        assert_eq!(
            args[8..],
            [
                dir.display().to_string(),
                "--listen".to_string(),
                "127.0.0.1:5678".to_string(),
                "--wait-for-client".to_string(),
                "main.py".to_string(),
            ]
        );
        assert!(is_system_exit(
            &json!({ "reason": "exception", "text": "SystemExit       (note: …)" })
        ));
        assert!(!is_system_exit(
            &json!({ "reason": "exception", "text": "KeyError" })
        ));
        assert!(!is_system_exit(
            &json!({ "reason": "breakpoint", "text": "SystemExit" })
        ));
    }

    // 止まる → スタック → 変数 → 式 → ステップ → 続行して終わる。例外で止まる。exit() では止まらない。止める
    #[cfg(unix)]
    #[test]
    fn debugs_python_with_real_debugpy() {
        let Some(tools) = installed_python_tools() else {
            eprintln!("アプリのフォルダに debugpy がないので飛ばします");
            return;
        };
        let mut tried = 0;
        for interpreter in [Interpreter::Cpython, Interpreter::Pypy] {
            let settings = python::PythonSettings {
                interpreter,
                ..python::PythonSettings::default()
            };
            let Ok(found) = python::find_interpreter(&settings) else {
                eprintln!("{} がないので飛ばします", interpreter.label());
                continue;
            };
            if find_debugpy(Some(&tools), interpreter, &found).is_err() {
                eprintln!("{} の debugpy がないので飛ばします", interpreter.label());
                continue;
            }
            tried += 1;
            let ws = workspace(&format!("debugpy-{}", interpreter.label()));
            let pkg = ws.0.join("pkg");
            write(&pkg.join("main.py"), PYTHON_SOLUTION);
            write(&pkg.join("test/input.txt"), "5\n");
            write(&pkg.join("test/sample-1.in"), "7\n");
            write(&pkg.join("test/sample-2.in"), "9\n");
            write(&pkg.join("test/sample-3.in"), "11\n");
            let label = interpreter.label();

            // 1. ブレークポイント
            let session = python_session(&pkg, &ws.0, None, &[PY_LINE_TOTAL], interpreter, &tools);
            let (since, stopped) = session.wait_stopped(0);
            assert_eq!(stopped["reason"], "breakpoint", "{label}");
            let log = session.log();
            assert!(log.contains("デバッグアダプター：debugpy 1."), "{log}");
            assert!(log.contains(PYTHON_DEBUG_NOTE), "{log}");
            assert!(log.contains("標準入力：test/input.txt"), "{log}");
            let client = session.client();
            let stack = client.stack().unwrap();
            assert_eq!(stack[0].name, "solve", "{label}");
            assert_eq!(stack[0].line, PY_LINE_TOTAL as u64);
            assert_eq!(stack[0].workspace_path.as_deref(), Some("pkg/main.py"));
            assert!(
                stack
                    .iter()
                    .all(|frame| frame.source.as_deref() != Some("<string>")),
                "{stack:?}"
            );
            assert_eq!(stack.last().unwrap().name, "<module>");
            let scopes = client.scopes(stack[0].id).unwrap();
            let locals = client.variables(scopes[0].variables_reference).unwrap();
            let value = |name: &str| {
                locals
                    .iter()
                    .find(|v| v.name == name)
                    .map(|v| v.value.clone())
                    .unwrap_or_else(|| panic!("{name} がない：{locals:?}"))
            };
            assert_eq!(value("n"), "5");
            assert_eq!(value("xs"), "[0, 1, 4, 9, 16]");
            let watch = client
                .evaluate("n + len(xs)", Some(stack[0].id), EvaluateContext::Watch)
                .unwrap();
            assert_eq!(watch.result, "10");
            let repl = client
                .evaluate("s.upper()", Some(stack[0].id), EvaluateContext::Repl)
                .unwrap();
            assert_eq!(repl.result, "'ABC'");
            client.step(StepAction::Next).unwrap();
            let (since, _) = session.wait_stopped(since);
            assert_eq!(client.stack().unwrap()[0].line, PY_LINE_RETURN as u64);
            client.set_breakpoints("pkg/main.py", &[]).unwrap();
            client.step(StepAction::Continue).unwrap();
            let (_, exited) = session.wait(since, |event| event["kind"] == "exited");
            assert_eq!(exited["code"], 0, "{label}");
            // 出力は 1 回だけ（DAP の output と本来の出力の両方には出さない）
            assert_eq!(session.output(), "30\n", "{label}");
            assert!(!session.log().contains("debugpy：30"), "{}", session.log());
            drop(client);
            assert!(matches!(
                session.finish(),
                Finish::Debugged {
                    exit_code: Some(0),
                    stopped: false
                }
            ));

            // 2. 捕まえていない例外で、その行で止まる
            let session = python_session(&pkg, &ws.0, Some("sample-1"), &[], interpreter, &tools);
            let (since, stopped) = session.wait_stopped(0);
            assert_eq!(stopped["reason"], "exception", "{label}");
            let client = session.client();
            assert_eq!(client.stack().unwrap()[0].line, PY_LINE_KEY_ERROR as u64);
            client.step(StepAction::Continue).unwrap();
            let (_, exited) = session.wait(since, |event| event["kind"] == "exited");
            assert_eq!(exited["code"], 1, "{label}");
            drop(client);
            session.finish();

            // 3. exit(3) では止まらず、終了コード 3
            let session = python_session(&pkg, &ws.0, Some("sample-2"), &[], interpreter, &tools);
            let (_, exited) = session.wait(0, |event| event["kind"] == "exited");
            assert_eq!(exited["code"], 3, "{label}");
            assert!(
                !session
                    .events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|e| e["type"] == "debug" && e["event"]["kind"] == "stopped"),
                "{label}"
            );
            session.finish();

            // 4. 回り続けているところを止める。デバッグ対象（と debugpy のアダプター）が残らない
            let session = python_session(&pkg, &ws.0, Some("sample-3"), &[], interpreter, &tools);
            let pid = loop {
                if let Some(pid) = session.debuggee_pid() {
                    break pid;
                }
                thread::sleep(Duration::from_millis(20));
            };
            thread::sleep(Duration::from_millis(500));
            session.cancel.store(true, Ordering::SeqCst);
            assert!(matches!(
                session.finish(),
                Finish::Debugged { stopped: true, .. }
            ));
            assert_gone(pid);
            let group = Command::new("/usr/bin/pgrep")
                .args(["-g", &pid.to_string()])
                .output()
                .unwrap();
            assert!(
                String::from_utf8_lossy(&group.stdout).trim().is_empty(),
                "{label}"
            );
            assert!(!pkg.join("__pycache__").exists(), "{label}");
        }
        eprintln!("debugpy で試した処理系：{tried}");
    }

    // 1 つの output の知らせが、標準出力で、text に word を含むか（段階 6 T4。A5 の W7 の確かめ用。
    // wait の条件の中で呼ぶので、events のロックは取らない）
    #[cfg(windows)]
    fn stdout_contains(event: &Value, word: &str) -> bool {
        event["kind"] == "output"
            && event["category"] == "stdout"
            && event["text"]
                .as_str()
                .is_some_and(|text| text.contains(word))
    }

    // A5（Windows 移植の段階 6 T3）：本物の debugpy（Windows）。Unix の debugs_python_with_real_debugpy と同じ 4 つの場面と、
    // 5. 入力のファイルがないとき（W1）・日本語の出力（W2）。止めたあとは、子孫（debugpy のアダプター）と Job の記録が残らない。
    // 処理系と、アプリの設定のフォルダの python\<処理系> の debugpy がなければ飛ぶ（CI では飛ぶ）。
    // 失敗の文には、アプリの識別子とユーザーのフォルダの名前を出さない（hide_names）
    #[cfg(windows)]
    #[test]
    fn debugs_python_with_real_debugpy_on_windows() {
        let Some(tools) = installed_python_tools() else {
            eprintln!("アプリのフォルダに debugpy がないので飛ばします");
            return;
        };
        let mut tried = 0;
        for interpreter in [Interpreter::Cpython, Interpreter::Pypy] {
            let settings = python::PythonSettings {
                interpreter,
                ..python::PythonSettings::default()
            };
            let Ok(found) = python::find_interpreter(&settings) else {
                eprintln!("{} がないので飛ばします", interpreter.label());
                continue;
            };
            if find_debugpy(Some(&tools), interpreter, &found).is_err() {
                eprintln!("{} の debugpy がないので飛ばします", interpreter.label());
                continue;
            }
            tried += 1;
            let ws = workspace(&format!("debugpy-windows-{}", interpreter.label()));
            let pkg = ws.0.join("pkg");
            write(&pkg.join("main.py"), PYTHON_SOLUTION);
            write(&pkg.join("test/input.txt"), "5\n");
            write(&pkg.join("test/sample-1.in"), "7\n");
            write(&pkg.join("test/sample-2.in"), "9\n");
            write(&pkg.join("test/sample-3.in"), "11\n");
            let label = interpreter.label();

            // 1. ブレークポイント（作業領域のルートは `\\?\` の付いた形。W6 で外して送ると止まり、作業領域の中のファイルと分かるか）
            let started = Instant::now();
            let session = python_session(&pkg, &ws.0, None, &[PY_LINE_TOTAL], interpreter, &tools);
            let (since, stopped) = session.wait_stopped(0);
            eprintln!("{label}：始めてから止まるまで {:?}", started.elapsed());
            assert_eq!(stopped["reason"], "breakpoint", "{label}");
            let log = session.log();
            assert!(
                log.contains("デバッグアダプター：debugpy 1."),
                "{}",
                hide_names(&log)
            );
            assert!(log.contains(PYTHON_DEBUG_NOTE), "{}", hide_names(&log));
            assert!(
                log.contains("標準入力：test/input.txt"),
                "{}",
                hide_names(&log)
            );
            let client = session.client();
            let stack = client.stack().unwrap();
            let shown = hide_names(&format!("{stack:?}"));
            assert_eq!(stack[0].name, "solve", "{label}：{shown}");
            assert_eq!(stack[0].line, PY_LINE_TOTAL as u64, "{shown}");
            assert_eq!(
                stack[0].workspace_path.as_deref(),
                Some("pkg/main.py"),
                "{shown}"
            );
            assert!(
                stack
                    .iter()
                    .all(|frame| frame.source.as_deref() != Some("<string>")),
                "{shown}"
            );
            assert_eq!(stack.last().unwrap().name, "<module>", "{shown}");
            let scopes = client.scopes(stack[0].id).unwrap();
            let locals = client.variables(scopes[0].variables_reference).unwrap();
            let value = |name: &str| {
                locals
                    .iter()
                    .find(|v| v.name == name)
                    .map(|v| v.value.clone())
                    .unwrap_or_else(|| panic!("{name} がない：{locals:?}"))
            };
            assert_eq!(value("n"), "5");
            assert_eq!(value("xs"), "[0, 1, 4, 9, 16]");
            let watch = client
                .evaluate("n + len(xs)", Some(stack[0].id), EvaluateContext::Watch)
                .unwrap();
            assert_eq!(watch.result, "10");
            let repl = client
                .evaluate("s.upper()", Some(stack[0].id), EvaluateContext::Repl)
                .unwrap();
            assert_eq!(repl.result, "'ABC'");
            client.step(StepAction::Next).unwrap();
            let (since, _) = session.wait_stopped(since);
            assert_eq!(client.stack().unwrap()[0].line, PY_LINE_RETURN as u64);
            client.set_breakpoints("pkg/main.py", &[]).unwrap();
            client.step(StepAction::Continue).unwrap();
            let (_, exited) = session.wait(since, |event| event["kind"] == "exited");
            assert_eq!(exited["code"], 0, "{label}");
            // 出力は 1 回だけ。CPython の print はパイプで CRLF（docs/windows-port-plan.md 4.3）
            assert_eq!(session.output(), "30\r\n", "{label}");
            assert!(
                !session.log().contains("debugpy：30"),
                "{}",
                hide_names(&session.log())
            );
            drop(client);
            assert!(matches!(
                session.finish(),
                Finish::Debugged {
                    exit_code: Some(0),
                    stopped: false
                }
            ));

            // 2. 捕まえていない例外で、その行で止まる
            let session = python_session(&pkg, &ws.0, Some("sample-1"), &[], interpreter, &tools);
            let (since, stopped) = session.wait_stopped(0);
            assert_eq!(stopped["reason"], "exception", "{label}");
            // 止まっている間に、止まる前の出力（91）が届いている（W7。止まった知らせより前に届くので、0 から探す）
            session.wait(0, |event| stdout_contains(event, "91"));
            let client = session.client();
            assert_eq!(client.stack().unwrap()[0].line, PY_LINE_KEY_ERROR as u64);
            client.step(StepAction::Continue).unwrap();
            let (_, exited) = session.wait(since, |event| event["kind"] == "exited");
            assert_eq!(exited["code"], 1, "{label}");
            drop(client);
            session.finish();

            // 3. exit(3) では止まらず、終了コード 3
            let session = python_session(&pkg, &ws.0, Some("sample-2"), &[], interpreter, &tools);
            let (_, exited) = session.wait(0, |event| event["kind"] == "exited");
            assert_eq!(exited["code"], 3, "{label}");
            assert!(
                !session
                    .events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|e| e["type"] == "debug" && e["event"]["kind"] == "stopped"),
                "{label}"
            );
            session.finish();

            // 4. 回り続けているところを止める。デバッグ対象と、その子孫（debugpy のアダプター）と、Job の記録が残らない
            let session = python_session(&pkg, &ws.0, Some("sample-3"), &[], interpreter, &tools);
            let pid = loop {
                if let Some(pid) = session.debuggee_pid() {
                    break pid;
                }
                thread::sleep(Duration::from_millis(20));
            };
            // 止める前に、回り始める前の出力（385）が届いている（W7）
            session.wait(0, |event| stdout_contains(event, "385"));
            thread::sleep(Duration::from_millis(500));
            session.cancel.store(true, Ordering::SeqCst);
            assert!(matches!(
                session.finish(),
                Finish::Debugged { stopped: true, .. }
            ));
            assert_gone(pid);
            let left = crate::win::descendants(pid as u32);
            assert!(left.is_empty(), "{label}：子孫が残っている：{left:?}");
            assert!(
                !crate::win::has_job_record(pid as u32),
                "{label}：Job の記録が残っている"
            );
            assert!(!pkg.join("__pycache__").exists(), "{label}");

            // 5. 入力のファイルがないとき（NUL。W1）は入力を待たずに終わり、日本語の出力が化けない（-X utf8。W2）
            write(
                &pkg.join("main.py"),
                "import sys\nprint(\"あいう\")\nprint(len(sys.stdin.read()))\n",
            );
            fs::remove_file(pkg.join("test/input.txt")).unwrap();
            let session = python_session(&pkg, &ws.0, None, &[], interpreter, &tools);
            let (_, exited) = session.wait(0, |event| event["kind"] == "exited");
            assert_eq!(exited["code"], 0, "{label}");
            assert!(
                session.log().contains("入力ファイルなし"),
                "{}",
                hide_names(&session.log())
            );
            assert_eq!(session.output(), "あいう\r\n0\r\n", "{label}");
            session.finish();
        }
        eprintln!("debugpy で試した処理系：{tried}");
    }

    // Windows（段階 6 C++ T2）：比べて違えば、ユーザーのフォルダの名前を伏せて落ちる（失敗の文に一時フォルダのパスが出るため）
    #[cfg(windows)]
    fn assert_same<T: PartialEq + std::fmt::Debug>(left: T, right: T) {
        assert!(
            left == right,
            "{}",
            hide_names(&format!("left: {left:?}\nright: {right:?}"))
        );
    }

    // 命令の環境の値（なければ None。名前は大文字・小文字を区別しない）
    #[cfg(windows)]
    fn command_env(command: &Command, name: &str) -> Option<String> {
        command
            .get_envs()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .and_then(|(_, value)| value)
            .map(|value| value.to_string_lossy().into_owned())
    }

    // A7（段階 6 C++ T2。W8）：gdb の見つけ方と起動の形。偽のフォルダとファイルで試す（本物の gdb は使わない）
    #[cfg(windows)]
    #[test]
    fn finds_gdb_next_to_the_compiler_on_windows() {
        let ws = workspace("find-gdb");
        let mingw = ws.0.join("mingw64");
        let bin = mingw.join("bin");
        let opt = mingw.join("opt");
        let other = ws.0.join("other").join("bin");
        write(&bin.join("g++.exe"), "");
        let settings = crate::cpp::CppSettings {
            compiler: bin.join("g++.exe").to_string_lossy().into_owned(),
            ..crate::cpp::CppSettings::default()
        };

        // 1. MinGW-Builds の形（gdb.exe・gdborig.exe・..\opt）：gdborig.exe を直接。PATH はコンパイラの bin → opt\bin、PYTHONHOME は opt
        write(&bin.join("gdb.exe"), "");
        write(&bin.join("gdborig.exe"), "");
        fs::create_dir_all(opt.join("bin")).unwrap();
        let orig = GdbLaunch {
            program: bin.join("gdborig.exe"),
            path_prefix: vec![bin.clone(), opt.join("bin")],
            python_home: Some(opt.clone()),
        };
        assert_same(find_gdb(&settings), Ok(orig));

        // 2. gdborig.exe はあるが ..\opt がない：gdb.exe をそのまま。PATH の先頭はコンパイラの bin だけ、PYTHONHOME なし
        fs::remove_dir_all(&opt).unwrap();
        let plain = GdbLaunch {
            program: bin.join("gdb.exe"),
            path_prefix: vec![bin.clone()],
            python_home: None,
        };
        assert_same(find_gdb_in(Some(&bin), None), Ok(plain.clone()));

        // 3. gdborig.exe がない（gdb.exe が本体の配布）：gdb.exe をそのまま
        fs::remove_file(bin.join("gdborig.exe")).unwrap();
        assert_same(find_gdb_in(Some(&bin), None), Ok(plain));

        // 4. コンパイラの隣になく、PATH にある：PATH の gdb.exe（相対パスの項目は見ない）。デバッグ対象の DLL のため、PATH の先頭はコンパイラの bin
        fs::remove_file(bin.join("gdb.exe")).unwrap();
        write(&other.join("gdb.exe"), "");
        let path_var = std::env::join_paths([PathBuf::from("relative"), other.clone()]).unwrap();
        assert_same(
            find_gdb_in(Some(&bin), Some(&path_var)),
            Ok(GdbLaunch {
                program: other.join("gdb.exe"),
                path_prefix: vec![bin.clone()],
                python_home: None,
            }),
        );

        // 5. gdb.exe がどこにもない：Windows 向けの案内の文
        fs::remove_file(other.join("gdb.exe")).unwrap();
        assert_same(
            find_gdb_in(Some(&bin), Some(&path_var)),
            Err(GDB_MISSING.to_string()),
        );
        assert_same(find_gdb_in(None, None), Err(GDB_MISSING.to_string()));
    }

    // A7b（段階 6 C++ T2。W9）：gdb を起動する命令の引数と環境
    #[cfg(windows)]
    #[test]
    fn builds_gdb_command_on_windows() {
        let orig = GdbLaunch {
            program: PathBuf::from(r"C:\m\bin\gdborig.exe"),
            path_prefix: vec![PathBuf::from(r"C:\m\bin"), PathBuf::from(r"C:\m\opt\bin")],
            python_home: Some(PathBuf::from(r"C:\m\opt")),
        };
        let command = gdb_command(&orig);
        assert_same(
            command.get_program(),
            std::ffi::OsStr::new(r"C:\m\bin\gdborig.exe"),
        );
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_same(
            args,
            [
                "-nh",
                "-q",
                "--interpreter=dap",
                "-iex",
                "set auto-load off",
            ]
            .map(str::to_string)
            .to_vec(),
        );
        assert_same(
            command_env(&command, "PYTHONHOME"),
            Some(r"C:\m\opt".to_string()),
        );
        assert_same(command_env(&command, "PYTHONUTF8"), Some("1".to_string()));
        let path = command_env(&command, "PATH").unwrap_or_default();
        let head: Vec<PathBuf> = std::env::split_paths(&path).take(2).collect();
        assert_same(head, orig.path_prefix.clone());

        // gdb.exe が本体の配布：PYTHONHOME は付けない。PYTHONUTF8 は付ける
        let plain = GdbLaunch {
            program: PathBuf::from(r"C:\g\bin\gdb.exe"),
            path_prefix: vec![PathBuf::from(r"C:\g\bin")],
            python_home: None,
        };
        let command = gdb_command(&plain);
        assert_same(command_env(&command, "PYTHONHOME"), None);
        assert_same(command_env(&command, "PYTHONUTF8"), Some("1".to_string()));
        let path = command_env(&command, "PATH").unwrap_or_default();
        assert_same(
            std::env::split_paths(&path).next(),
            Some(PathBuf::from(r"C:\g\bin")),
        );
    }

    // A8（段階 6 C++ T2。W10）：gdb の initialize と launch。lldb-dap の項目がなく、`\\?\` を外し、標準入力は引用符つきの 1 つの文字列
    #[cfg(windows)]
    #[test]
    fn builds_gdb_launch_configuration_on_windows() {
        let init = gdb_initialize_arguments();
        assert_eq!(init["adapterID"], "gdb");
        assert_eq!(init["linesStartAt1"], true);
        assert_eq!(init["pathFormat"], "path");
        assert!(init.get("$__lldb_sourceInitFile").is_none(), "{init}");

        let launch = gdb_launch_arguments(
            Path::new(r"\\?\C:\w\pkg\target\main-debug.exe"),
            Path::new(r"\\?\C:\w\pkg"),
            Path::new(r"\\?\C:\w\pkg\test\input.txt"),
        );
        assert_eq!(launch["program"], r"C:\w\pkg\target\main-debug.exe");
        assert_eq!(launch["cwd"], r"C:\w\pkg");
        assert_eq!(launch["args"], r#"< "C:\w\pkg\test\input.txt""#);
        assert_eq!(launch["stopOnEntry"], false);
        for key in ["initCommands", "preRunCommands", "stdio", "env"] {
            assert!(launch.get(key).is_none(), "{key}：{launch}");
        }

        // 入力のファイルがないとき（choose_input の NUL）も同じ形
        let launch = gdb_launch_arguments(
            Path::new(r"C:\w\a.exe"),
            Path::new(r"C:\w"),
            Path::new("NUL"),
        );
        assert_eq!(launch["args"], r#"< "NUL""#);
    }

    // A13（段階 6 Rust T2。W15・W16）：lldb-dap の見つけ方（PATH → %ProgramFiles%\LLVM\bin）と、Python のフォルダ（python3.dll があること）。
    // 偽のフォルダとファイルで試す（本物の lldb-dap は使わない）
    #[cfg(windows)]
    #[test]
    fn finds_lldb_dap_on_windows() {
        let ws = workspace("find-lldb-dap");
        let path_dir = ws.0.join("path").join("bin");
        let program_files = ws.0.join("pf");
        let llvm_bin = program_files.join("LLVM").join("bin");
        write(&path_dir.join("lldb-dap.exe"), "");
        write(&llvm_bin.join("lldb-dap.exe"), "");
        let path_var = std::env::join_paths([PathBuf::from("relative"), path_dir.clone()]).unwrap();

        // 1. PATH にある：PATH のもの（%ProgramFiles% より先。相対パスの項目は見ない）
        assert_same(
            find_lldb_dap_in(Some(&path_var), Some(&program_files)),
            Ok(path_dir.join("lldb-dap.exe")),
        );

        // 2. PATH になく、%ProgramFiles%\LLVM\bin にある：そこのもの
        fs::remove_file(path_dir.join("lldb-dap.exe")).unwrap();
        assert_same(
            find_lldb_dap_in(Some(&path_var), Some(&program_files)),
            Ok(llvm_bin.join("lldb-dap.exe")),
        );

        // 3. どこにもない：LLVM の公式版を入れる案内の文
        fs::remove_file(llvm_bin.join("lldb-dap.exe")).unwrap();
        assert_same(
            find_lldb_dap_in(Some(&path_var), Some(&program_files)),
            Err(LLDB_DAP_MISSING.to_string()),
        );
        assert_same(
            find_lldb_dap_in(None, None),
            Err(LLDB_DAP_MISSING.to_string()),
        );

        // 4. Python：CPython の exe の隣に python3.dll がなければ案内の文、あればそのフォルダ
        let python = ws.0.join("Python313");
        write(&python.join("python.exe"), "");
        assert_same(
            lldb_python_dir(&python.join("python.exe")),
            Err(LLDB_PYTHON_MISSING.to_string()),
        );
        write(&python.join("python3.dll"), "");
        assert_same(
            lldb_python_dir(&python.join("python.exe")),
            Ok(python.clone()),
        );
    }

    // A14（段階 6 Rust T2。W16・W20）：lldb-dap を起動する命令の引数と環境
    #[cfg(windows)]
    #[test]
    fn builds_lldb_dap_command_on_windows() {
        let launch = LldbLaunch {
            program: PathBuf::from(r"C:\l\bin\lldb-dap.exe"),
            python_dir: PathBuf::from(r"C:\p\Python313"),
        };
        let command = lldb_dap_command(&launch);
        assert_same(
            command.get_program(),
            std::ffi::OsStr::new(r"C:\l\bin\lldb-dap.exe"),
        );
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        // 家とシステムの .lldbinit を読まない（initialize の $__lldb_sourceInitFile: false と両方）
        assert_same(args, vec!["--no-lldbinit".to_string()]);
        // デバッグ対象の起動のたびに出るコンソールの窓を隠す（K4）
        assert_same(
            command_env(&command, "LLDB_LAUNCH_INFERIORS_WITHOUT_CONSOLE"),
            Some("true".to_string()),
        );
        // liblldb.dll が読む python3.dll のため、子の PATH の先頭に Python のフォルダ
        let path = command_env(&command, "PATH").unwrap_or_default();
        assert_same(
            std::env::split_paths(&path).next(),
            Some(PathBuf::from(r"C:\p\Python313")),
        );
    }

    // A15（段階 6 Rust T2。W17）：Windows の initCommands。lldb_lookup.py の import と source-map だけで、lldb_commands を読まない
    #[cfg(windows)]
    #[test]
    fn builds_rust_init_commands_on_windows() {
        let ws = workspace("rust-init-windows");
        let sysroot = crate::trust::without_verbatim(&ws.0).join("sysroot");
        let rustc = RustcPaths {
            sysroot: sysroot.clone(),
            commit_hash: Some("0123abc".to_string()),
        };
        let import = format!(
            "command script import {}",
            lldb_quote(&sysroot.join(r"lib\rustlib\etc\lldb_lookup.py"))
        );

        // rust-src がない：import だけ
        assert_same(lldb_dap_init_commands_windows(&rustc), vec![import.clone()]);

        // rust-src がある：import と source-map
        let source = sysroot.join(r"lib\rustlib\src\rust");
        fs::create_dir_all(&source).unwrap();
        assert_same(
            lldb_dap_init_commands_windows(&rustc),
            vec![
                import,
                format!(
                    "settings set target.source-map /rustc/0123abc {}",
                    lldb_quote(&source)
                ),
            ],
        );
    }

    // A16（段階 6 Rust T2。W18）：Windows の Rust の launch。inherit-tcc を送らず、program・cwd・input-path の `\\?\` を外す
    #[cfg(windows)]
    #[test]
    fn builds_rust_launch_configuration_on_windows() {
        let init = initialize_arguments();
        assert_eq!(init["$__lldb_sourceInitFile"], false);

        let rustc = RustcPaths {
            sysroot: PathBuf::from(r"C:\r"),
            commit_hash: None,
        };
        let launch = lldb_dap_launch_arguments_windows(
            Path::new(r"\\?\C:\w\pkg\target\debug\a.exe"),
            Path::new(r"\\?\C:\w\pkg"),
            Path::new(r"\\?\C:\w\pkg\test\input.txt"),
            Some(&rustc),
        );
        assert_eq!(launch["type"], "lldb-dap");
        assert_eq!(launch["request"], "launch");
        assert_eq!(launch["program"], r"C:\w\pkg\target\debug\a.exe");
        assert_eq!(launch["cwd"], r"C:\w\pkg");
        assert_eq!(launch["args"], json!([]));
        assert_eq!(launch["stopOnEntry"], false);
        assert_eq!(
            launch["initCommands"],
            json!([r#"command script import "C:\\r\\lib\\rustlib\\etc\\lldb_lookup.py""#])
        );
        // 標準入力だけ。target.inherit-tcc（macOS の LLDB だけ）は送らない
        assert_eq!(
            launch["preRunCommands"],
            json!([r#"settings set target.input-path "C:\\w\\pkg\\test\\input.txt""#])
        );

        // 入力のファイルがないとき（choose_input の NUL）も同じ形。表示スクリプトがないとき（rustc の sysroot が取れない）は initCommands が空
        let launch = lldb_dap_launch_arguments_windows(
            Path::new(r"C:\w\a.exe"),
            Path::new(r"C:\w"),
            Path::new("NUL"),
            None,
        );
        assert_eq!(
            launch["preRunCommands"],
            json!([r#"settings set target.input-path "NUL""#])
        );
        assert_eq!(launch["initCommands"], json!([]));
    }

    // ---- 本物の gdb を使うテスト（Windows 移植の段階 6 の C++ T3）。MinGW-w64 の g++ と gdb が要る ----

    // 試しの C++ のプログラム。行の番号：11 日本語の出力、13・15 標準入力、16 string、17 vector を足す、18 出力（ブレークポイント）、
    // 19 total が 7 なら throw、20 total が 8 なら範囲外の読み書き（デバッグ用のビルドでは abort。D-214）、21–24 total が 9 なら回り続ける
    #[cfg(windows)]
    const CPP_GDB_SOLUTION: &str = r#"#include <bits/stdc++.h>
using namespace std;

long long sum(const vector<long long>& v) {
    long long s = 0;
    for (auto x : v) s += x;
    return s;
}

int main() {
    cout << "あいう" << endl;
    int n = 0;
    cin >> n;
    vector<long long> a(n);
    for (auto& x : a) cin >> x;
    string name = "atcoder";
    long long total = sum(a);
    cout << total << " " << name << endl;
    if (total == 7) throw runtime_error("boom");
    if (total == 8) cout << a[n] << endl;
    if (total == 9) {
        volatile long long i = 0;
        while (true) i = i + 1;
    }
}
"#;
    #[cfg(windows)]
    const CPP_GDB_LINE_PRINT: u32 = 18;

    // このテストのプロセスの子孫に、gdb（gdb.exe・gdborig.exe）とデバッグ対象（main-debug.exe）が残っていないか
    #[cfg(windows)]
    fn gdb_left() -> Vec<(u32, String)> {
        crate::win::descendants(std::process::id())
            .into_iter()
            .filter(|(_, name)| {
                let name = name.to_ascii_lowercase();
                name.starts_with("gdb") || name == "main-debug.exe"
            })
            .collect()
    }

    // 場面のあとに、gdb とデバッグ対象が残っていないかを確かめる。3 秒まで 20 ms ごとに見直し、まだ動いている（alive）ものだけを数える
    // （assert_gone と同じ考え方。もう終わっているプロセスは利用者に害がない。利用者が決めた）。一覧にあるが alive でないものは、数えずに記録する。
    // 残ったものの数は、落ちなくても記録する。落ちるときは、その場面のデバッグ対象の pid と、残ったもの 1 つずつの pid・名前・alive を出す
    // （T3 の 5 つの確かめで、全体の cargo test のとき A9 が場面 2 のあとに 1 回落ちたため。「止めた直後に一覧に残っていた」という見立ては未確認）
    #[cfg(windows)]
    fn assert_no_gdb_left(scene: &str, debuggee: Option<i32>) {
        let until = Instant::now() + Duration::from_secs(3);
        let (running, ended) = loop {
            let (running, ended): (Vec<_>, Vec<_>) = gdb_left()
                .into_iter()
                .partition(|(pid, _)| crate::win::is_alive(*pid));
            if running.is_empty() || Instant::now() >= until {
                break (running, ended);
            }
            thread::sleep(Duration::from_millis(20));
        };
        eprintln!(
            "gdb：{scene}のあと残ったもの（動いている）の数 {}",
            running.len()
        );
        if !ended.is_empty() {
            eprintln!(
                "{}",
                hide_names(&format!(
                    "gdb：{scene}のあと一覧にあるが終わっているもの（数えない。pid・名前）{ended:?}"
                ))
            );
        }
        let detail: Vec<(u32, String, bool)> = running
            .iter()
            .map(|(pid, name)| (*pid, name.clone(), crate::win::is_alive(*pid)))
            .collect();
        assert!(
            running.is_empty(),
            "{}",
            hide_names(&format!(
                "{scene}：デバッグ対象の pid {debuggee:?}、残ったもの（pid・名前・alive）{detail:?}"
            ))
        );
    }

    // 実行ログに「強制的に止めました」が出たか（止めるときに forced になったかの記録。finish の前に session.events を写しておく）
    #[cfg(windows)]
    fn forced_in(events: &Arc<Mutex<Vec<Value>>>) -> bool {
        events.lock().unwrap().iter().any(|event| {
            event["type"] == "log"
                && event["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("強制的に止めました"))
        })
    }

    // A9（段階 6 C++ T3）：本物の gdb（Windows）。debugs_cpp_with_real_lldb_dap と同じ場面に、日本語の出力・gdb 自身の文が出ないこと（W11）・
    // 一時停止（W13）・input.txt がないとき（< "NUL"）・止めたあとに gdb とデバッグ対象が残らないことを足す。
    // CI では飛ぶ（GitHub Actions は環境変数 CI を付ける）：runner の gdb の有無と版を手元で調べられず、落ちたときに切り分けられないため
    // （lldb-dap の本物のテストも CI では --skip。9.1 の追加 50）。gdb が見つからなくても飛ぶ。失敗の文はユーザーのフォルダの名前を伏せる
    #[cfg(windows)]
    #[test]
    fn debugs_cpp_with_real_gdb_on_windows() {
        if std::env::var_os("CI").is_some() {
            eprintln!("CI では本物の gdb のテストを飛ばします");
            return;
        }
        if find_gdb(&crate::cpp::CppSettings::default()).is_err() {
            eprintln!("gdb がないので飛ばします");
            return;
        }
        let ws = workspace("gdb");
        let pkg = ws.0.join("pkg");
        write(&pkg.join("main.cpp"), CPP_GDB_SOLUTION);
        write(&pkg.join("test/input.txt"), "3\n1 2 3\n");
        write(&pkg.join("test/sample-1.in"), "2\n3 4\n");
        write(&pkg.join("test/sample-2.in"), "2\n3 5\n");
        write(&pkg.join("test/sample-3.in"), "2\n4 5\n");

        // 1. ブレークポイント → スタック → 変数（vector・string）→ 式 → デバッグコンソール → ステップ → 続行して終わる
        let started = Instant::now();
        let session = cpp_session(&pkg, &ws.0, None, &[CPP_GDB_LINE_PRINT]);
        let (since, stopped) = session.wait_stopped(0);
        eprintln!("gdb：始めてから止まるまで {:?}", started.elapsed());
        assert_eq!(stopped["reason"], "breakpoint");
        let log = session.log();
        assert!(log.contains("デバッグアダプター："), "{}", hide_names(&log));
        assert!(log.contains("（GNU gdb"), "{}", hide_names(&log));
        assert!(log.contains(CPP_DEBUG_BUILD_NOTE), "{}", hide_names(&log));
        assert!(
            log.contains("標準入力：test/input.txt"),
            "{}",
            hide_names(&log)
        );
        // 件 2（追加 49）：始めたあとに continued が来ないことの記録（assert はしない）
        let continued = session
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event["type"] == "debug" && event["event"]["kind"] == "continued")
            .count();
        eprintln!("gdb：最初に止まるまでの continued の数 {continued}");
        let client = session.client();
        let stack = client.stack().unwrap();
        let shown = hide_names(&format!("{stack:?}"));
        assert_eq!(stack[0].name, "main", "{shown}");
        assert_eq!(
            stack[0].workspace_path.as_deref(),
            Some("pkg/main.cpp"),
            "{shown}"
        );
        assert_eq!(stack[0].line, CPP_GDB_LINE_PRINT as u64, "{shown}");
        let scopes = client.scopes(stack[0].id).unwrap();
        let locals = client.variables(scopes[0].variables_reference).unwrap();
        let find = |name: &str| {
            locals
                .iter()
                .find(|v| v.name == name)
                .cloned()
                .unwrap_or_else(|| panic!("{name} がない：{}", hide_names(&format!("{locals:?}"))))
        };
        assert_eq!(find("n").value, "3");
        assert_eq!(find("total").value, "6");
        let name = find("name");
        assert!(name.value.contains("atcoder"), "{name:?}");
        // 変数の欄での vector の中身（P2 では見ていなかった。10 章の未確認）
        let a = find("a");
        assert!(a.value.contains("length 3"), "{a:?}");
        let elements = client.variables(a.variables_reference).unwrap();
        let values: Vec<&str> = elements.iter().map(|v| v.value.as_str()).collect();
        assert_eq!(values, ["1", "2", "3"], "{elements:?}");
        let watch = client
            .evaluate("total + 1", Some(stack[0].id), EvaluateContext::Watch)
            .unwrap();
        assert_eq!(watch.result, "7");
        // デバッグコンソールは gdb の命令。-nh で読んだシステムの gdbinit の libstdc++ の表示が登録されている
        let printers = client
            .evaluate(
                "info pretty-printer",
                Some(stack[0].id),
                EvaluateContext::Repl,
            )
            .unwrap();
        assert!(printers.result.contains("libstdc++-v6"), "{printers:?}");
        client.step(StepAction::Next).unwrap();
        let (since, stopped) = session.wait_stopped(since);
        assert_eq!(stopped["reason"], "step");
        assert_eq!(
            client.stack().unwrap()[0].line,
            (CPP_GDB_LINE_PRINT + 1) as u64
        );
        client.set_breakpoints("pkg/main.cpp", &[]).unwrap();
        client.step(StepAction::Continue).unwrap();
        let (_, exited) = session.wait(since, |event| event["kind"] == "exited");
        assert_eq!(exited["code"], 0);
        // W11：プログラムの最初の行が消えず、gdb 自身の文が出ない
        let output = session.output();
        assert!(output.starts_with("あいう"), "{}", hide_names(&output));
        assert!(output.contains("6 atcoder"), "{}", hide_names(&output));
        for noise in ["No symbol table", "pending", "No source file"] {
            assert!(!output.contains(noise), "{noise}：{}", hide_names(&output));
        }
        drop(client);
        let debuggee = session.debuggee_pid();
        let events = session.events.clone();
        let finish = session.finish();
        eprintln!("gdb：場面 1 の forced {}", forced_in(&events));
        assert!(
            matches!(
                finish,
                Finish::Debugged {
                    exit_code: Some(0),
                    stopped: false
                }
            ),
            "{finish:?}"
        );
        assert_no_gdb_left("場面 1", debuggee);

        // 2. 例外を投げると止まり、呼んだ行が main.cpp に見える。止まったまま止める
        let session = cpp_session(&pkg, &ws.0, Some("sample-1"), &[]);
        let (_, stopped) = session.wait_stopped(0);
        assert_eq!(stopped["reason"], "throw", "{stopped}");
        let pid = session.debuggee_pid().unwrap();
        let stack = session.client().stack().unwrap();
        assert!(
            stack
                .iter()
                .any(|frame| frame.workspace_path.as_deref() == Some("pkg/main.cpp")),
            "{}",
            hide_names(&format!("{stack:?}"))
        );
        session.cancel.store(true, Ordering::SeqCst);
        let events = session.events.clone();
        let finish = session.finish();
        eprintln!("gdb：場面 2 の forced {}", forced_in(&events));
        assert!(
            matches!(finish, Finish::Debugged { stopped: true, .. }),
            "{finish:?}"
        );
        assert_gone(pid);
        assert_no_gdb_left("場面 2", Some(pid));

        // 3. 範囲外の読み書きは、デバッグ用のビルドでは abort になって止まる（D-214）。続行したあとの動き（W12）は記録だけ
        let session = cpp_session(&pkg, &ws.0, Some("sample-2"), &[]);
        let (since, stopped) = session.wait_stopped(0);
        assert_eq!(stopped["reason"], "abort", "{stopped}");
        let client = session.client();
        let stack = client.stack().unwrap();
        assert!(
            stack
                .iter()
                .any(|frame| frame.workspace_path.as_deref() == Some("pkg/main.cpp")),
            "{}",
            hide_names(&format!("{stack:?}"))
        );
        client.step(StepAction::Continue).unwrap();
        let (since, next) = session.wait(since, |event| {
            event["kind"] == "stopped" || event["kind"] == "exited"
        });
        eprintln!(
            "gdb：abort のあと続行して {}",
            hide_names(&next.to_string())
        );
        if next["kind"] == "stopped" {
            client.step(StepAction::Continue).unwrap();
            let (_, exited) = session.wait(since, |event| event["kind"] == "exited");
            eprintln!("gdb：もう一度続行して {}", hide_names(&exited.to_string()));
        }
        let output = session.output();
        assert!(
            output.contains("__n < this->size()"),
            "{}",
            hide_names(&output)
        );
        drop(client);
        let debuggee = session.debuggee_pid();
        let events = session.events.clone();
        let finish = session.finish();
        eprintln!("gdb：場面 3 の forced {}", forced_in(&events));
        assert!(
            matches!(finish, Finish::Debugged { stopped: false, .. }),
            "{finish:?}"
        );
        assert_no_gdb_left("場面 3", debuggee);

        // 4. 回り続けているところで一時停止する（W13）。止まった理由とスタックは記録だけ。そのあと止める
        let session = cpp_session(&pkg, &ws.0, Some("sample-3"), &[]);
        session.wait(0, |event| {
            event["kind"] == "output"
                && event["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("9 atcoder"))
        });
        let pid = session.debuggee_pid().unwrap();
        thread::sleep(Duration::from_millis(500));
        let since = session.events.lock().unwrap().len();
        let client = session.client();
        client.step(StepAction::Pause).unwrap();
        let (_, stopped) = session.wait_stopped(since);
        // W14：一時停止は main のスレッドの pause として扱い、スタックに利用者のコード（main.cpp）が見える
        assert_eq!(stopped["reason"], "pause", "{stopped}");
        assert_eq!(stopped["threadId"], GDB_PAUSE_THREAD, "{stopped}");
        let stack = client.stack().unwrap();
        assert!(
            stack
                .iter()
                .any(|frame| frame.workspace_path.as_deref() == Some("pkg/main.cpp")),
            "{}",
            hide_names(&format!("{stack:?}"))
        );
        eprintln!(
            "gdb：一時停止 {}、スタック {}",
            hide_names(&stopped.to_string()),
            hide_names(&format!(
                "{:?}",
                client
                    .stack()
                    .map(|stack| stack.iter().map(|f| f.name.clone()).collect::<Vec<_>>())
            ))
        );
        drop(client);
        let started = Instant::now();
        session.cancel.store(true, Ordering::SeqCst);
        let events = session.events.clone();
        let finish = session.finish();
        eprintln!(
            "gdb：一時停止のあと止めるまで {:?}、場面 4 の forced {}",
            started.elapsed(),
            forced_in(&events)
        );
        assert!(
            matches!(finish, Finish::Debugged { stopped: true, .. }),
            "{finish:?}"
        );
        assert_gone(pid);
        assert_no_gdb_left("場面 4", Some(pid));

        // 5. input.txt がないとき（choose_input の NUL を < "NUL" で渡す。10 章の未確認）は、入力を待たずに終わる
        fs::remove_file(pkg.join("test/input.txt")).unwrap();
        let session = cpp_session(&pkg, &ws.0, None, &[]);
        let (_, exited) = session.wait(0, |event| event["kind"] == "exited");
        assert_eq!(exited["code"], 0);
        assert!(
            session.log().contains("入力ファイルなし"),
            "{}",
            hide_names(&session.log())
        );
        let output = session.output();
        assert!(output.starts_with("あいう"), "{}", hide_names(&output));
        assert!(output.contains("0 atcoder"), "{}", hide_names(&output));
        let debuggee = session.debuggee_pid();
        let events = session.events.clone();
        session.finish();
        eprintln!("gdb：場面 5 の forced {}", forced_in(&events));
        assert_no_gdb_left("場面 5", debuggee);
    }

    // A10（段階 6 C++ T3。W11）：gdb のとき、process の知らせより前の出力（gdb 自身の文）は出さず、そのあとの出力（デバッグ対象）は出す。
    // launch の答えのあと・process の前に届く gdb の文も捨てる（境は process の知らせ）。偽の知らせで試す（本物の gdb は使わない。CI でも動く）
    #[cfg(windows)]
    #[test]
    fn drops_gdb_messages_before_the_program_starts_on_windows() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Json(text) = body {
                sink.lock()
                    .unwrap()
                    .push(serde_json::from_str::<Value>(&text).unwrap());
            }
            Ok(())
        });
        let output = |seq: i64, text: &str| {
            json!({
                "seq": seq,
                "type": "event",
                "event": "output",
                "body": { "category": "stdout", "output": text },
            })
        };
        let messages = [
            output(1, "No symbol table is loaded.  Use the \"file\" command.\n"),
            json!({ "seq": 2, "type": "response", "request_seq": 2, "command": "launch", "success": true }),
            output(3, "Breakpoint 2 (-function abort) pending.\n"),
            json!({
                "seq": 4,
                "type": "event",
                "event": "process",
                "body": { "systemProcessId": 4321, "startMethod": "process" },
            }),
            output(5, "あいう\n"),
        ];
        let bytes: Vec<u8> = messages
            .iter()
            .flat_map(|message| encode(&message.to_string()))
            .collect();
        let (_client, notices) = Client::connect(
            "gdb",
            std::io::sink(),
            std::io::Cursor::new(bytes),
            Path::new("."),
            &channel,
        );
        // 読み終えると、読み取りのスレッドが知らせの送り手を捨てる
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            match notices.recv_timeout(Duration::from_millis(20)) {
                Err(RecvTimeoutError::Disconnected) => break,
                _ => assert!(Instant::now() < until, "読み終わりません"),
            }
        }
        let texts: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event["type"] == "debug" && event["event"]["kind"] == "output")
            .map(|event| {
                event["event"]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        assert_same(texts, vec!["あいう\n".to_string()]);
    }

    // 偽のアダプター（段階 6 C++ T3。A11）：IDE が書いた要求を覚え、gdb と同じく threads は notStopped で断り、ほかは成功で答える
    #[cfg(windows)]
    struct FakeAdapterWriter {
        decoder: Decoder,
        requests: Arc<Mutex<Vec<Value>>>,
        answers: Sender<Vec<u8>>,
    }

    #[cfg(windows)]
    impl Write for FakeAdapterWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let texts = self.decoder.push(bytes).map_err(std::io::Error::other)?;
            for text in texts {
                let request: Value = serde_json::from_str(&text).map_err(std::io::Error::other)?;
                let command = request["command"].as_str().unwrap_or_default().to_string();
                let refused = command == "threads";
                let mut answer = json!({
                    "seq": 0,
                    "type": "response",
                    "request_seq": request["seq"],
                    "command": command,
                    "success": !refused,
                });
                if refused {
                    answer["message"] = json!("notStopped");
                }
                self.requests.lock().unwrap().push(request);
                let _ = self.answers.send(encode(&answer.to_string()));
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[cfg(windows)]
    struct FakeAdapterReader {
        answers: Receiver<Vec<u8>>,
        pending: Vec<u8>,
    }

    #[cfg(windows)]
    impl Read for FakeAdapterReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.pending.is_empty() {
                match self.answers.recv() {
                    Ok(bytes) => self.pending = bytes,
                    // 書き手（クライアントの書き込みのスレッド）が終わった
                    Err(_) => return Ok(0),
                }
            }
            let n = self.pending.len().min(buffer.len());
            buffer[..n].copy_from_slice(&self.pending[..n]);
            self.pending.drain(..n);
            Ok(n)
        }
    }

    // A11（段階 6 C++ T3。W13）：gdb のときの一時停止は、threads を送らずに pause だけを送る（gdb は動いている間の threads を断る）。
    // 偽のアダプターで試す（本物の gdb は使わない。CI でも動く）
    #[cfg(windows)]
    #[test]
    fn pauses_gdb_without_asking_for_threads_on_windows() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (answers, incoming) = mpsc::channel();
        let writer = FakeAdapterWriter {
            decoder: Decoder::default(),
            requests: requests.clone(),
            answers,
        };
        let reader = FakeAdapterReader {
            answers: incoming,
            pending: Vec::new(),
        };
        let channel = Channel::new(|_| Ok(()));
        let (client, _notices) = Client::connect("gdb", writer, reader, Path::new("."), &channel);
        assert_same(client.step(StepAction::Pause), Ok(()));
        let commands: Vec<String> = requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request["command"].as_str().unwrap_or_default().to_string())
            .collect();
        assert_same(commands, vec!["pause".to_string()]);
    }

    // A12（段階 6 C++ T3。W14）：gdb の理由のない停止（一時停止）は、main のスレッド（1）の pause として扱う。
    // ほかの理由の停止と、gdb でないアダプターの停止は、そのまま。偽の知らせで試す（本物の gdb は使わない。CI でも動く）
    #[cfg(windows)]
    #[test]
    fn treats_gdb_interrupt_as_pause_of_main_thread_on_windows() {
        let stopped = |seq: i64, reason: &str, thread: i64| {
            json!({
                "seq": seq,
                "type": "event",
                "event": "stopped",
                "body": { "reason": reason, "threadId": thread, "allThreadsStopped": true },
            })
        };
        // 知らせを流し、読み終わるまで待って、画面への Stopped の知らせ（理由・スレッド）と、最後の止まったスレッドを返す
        let run = |adapter: &'static str, messages: Vec<Value>| {
            let events = Arc::new(Mutex::new(Vec::new()));
            let sink = events.clone();
            let channel = Channel::new(move |body| {
                if let InvokeResponseBody::Json(text) = body {
                    sink.lock()
                        .unwrap()
                        .push(serde_json::from_str::<Value>(&text).unwrap());
                }
                Ok(())
            });
            let bytes: Vec<u8> = messages
                .iter()
                .flat_map(|message| encode(&message.to_string()))
                .collect();
            let (client, notices) = Client::connect(
                adapter,
                std::io::sink(),
                std::io::Cursor::new(bytes),
                Path::new("."),
                &channel,
            );
            let until = Instant::now() + Duration::from_secs(10);
            loop {
                match notices.recv_timeout(Duration::from_millis(20)) {
                    Err(RecvTimeoutError::Disconnected) => break,
                    _ => assert!(Instant::now() < until, "読み終わりません"),
                }
            }
            let stops: Vec<(String, Option<i64>)> = events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event["type"] == "debug" && event["event"]["kind"] == "stopped")
                .map(|event| {
                    (
                        event["event"]["reason"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        event["event"]["threadId"].as_i64(),
                    )
                })
                .collect();
            let last = client.shared.lock_state().stopped_thread;
            (stops, last)
        };

        // gdb：理由のない停止（threadId 5）は main（1）の pause。ブレークポイントの停止はそのまま
        assert_same(
            run(
                "gdb",
                vec![stopped(1, "breakpoint", 1), stopped(2, "stopped", 5)],
            ),
            (
                vec![
                    ("breakpoint".to_string(), Some(1)),
                    ("pause".to_string(), Some(1)),
                ],
                Some(1),
            ),
        );
        // gdb の signal の停止（abort のあとの __fastfail。W12）はそのまま
        assert_same(
            run("gdb", vec![stopped(1, "signal", 1)]),
            (vec![("signal".to_string(), Some(1))], Some(1)),
        );
        // gdb でないアダプターは、理由が "stopped" でもそのまま
        assert_same(
            run("lldb-dap", vec![stopped(1, "stopped", 5)]),
            (vec![("stopped".to_string(), Some(5))], Some(5)),
        );
    }
}
