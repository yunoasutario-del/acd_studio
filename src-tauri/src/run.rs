// 通常実行（Phase 2 作業 4）とサンプルテスト（作業 5）：ビルドして、できたプログラムを input.txt を標準入力にして実行する
//
// 手順（D-089）：対象と版を示す（D-088・D-097）→ cargo build --release --bin <名前> →
// cargo の出力から実行ファイルの場所を受け取る → test のフォルダの input.txt を標準入力にして直接起動する（D-091・D-106）
// 実行ログは Tauri のチャンネルで画面に送る（09 K-07、P2-04-1）

use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;

use crate::compare::{self, Mode, TestSettings};
use crate::cpp::{self, CppSettings};
use crate::language::LangSettings;
use crate::language::Language;
use crate::process::{self, Limits, OUTPUT_LIMIT, Outcome, RUN_TIME_LIMIT, Side, Stream};
use crate::python;
use crate::toolchain::{self, TOOLCHAIN_HINT, Target};
use crate::workspace::{CaseKey, case_key};

// 画面に出す量（D-096）。標準出力・標準エラーそれぞれ先頭 256 KiB
pub const DISPLAY_LIMIT: usize = 256 * 1024;

// ランダムテストの 1 回目の seed は 0 以上この値未満にする（D-143）。回数の上限 10000 を足しても 2^31 を超えないので、
// 生成器が seed を 32 ビットの符号つき整数で読んでも収まる
pub const SEED_RANGE: u64 = 2_000_000_000;

// 実行中の処理の停止の印。IDE 全体で同時に 1 つだけ（D-013）
#[derive(Default)]
pub struct RunControl(Mutex<Option<Arc<AtomicBool>>>);

// 処理の間だけ持つ印。落ちたときも含めて、手放すと「実行中」を解く
struct RunGuard<'a>(&'a RunControl);

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        self.0.end();
    }
}

// アプリが終わるとき、止まるのを待つ時間の上限
const EXIT_WAIT: Duration = Duration::from_secs(3);

impl RunControl {
    // 処理を始める。すでに動いていればエラー
    fn begin(&self) -> Result<Arc<AtomicBool>, String> {
        let mut current = self
            .0
            .lock()
            .map_err(|_| "実行の状態を読めません".to_string())?;
        if current.is_some() {
            return Err("別の処理を実行中です".to_string());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *current = Some(cancel.clone());
        Ok(cancel)
    }

    fn end(&self) {
        if let Ok(mut current) = self.0.lock() {
            *current = None;
        }
    }

    // 実行中の処理を止める（停止の操作、D-009）
    pub fn stop(&self) {
        if let Ok(current) = self.0.lock()
            && let Some(cancel) = current.as_ref()
        {
            cancel.store(true, Ordering::SeqCst);
        }
    }

    pub fn running(&self) -> bool {
        self.0
            .lock()
            .map(|current| current.is_some())
            .unwrap_or(false)
    }

    // 実行中なら止め、止まるまで待つ（上限 EXIT_WAIT）。アプリが終わるときに使う
    pub fn stop_and_wait(&self) {
        self.stop();
        let until = Instant::now() + EXIT_WAIT;
        while self.running() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

// 塊ごとに届くバイト列を文字列にする。塊の境目で切れた UTF-8 の文字は、次の塊まで持ち越す
#[derive(Default)]
struct Utf8Carry(Vec<u8>);

impl Utf8Carry {
    fn decode(&mut self, bytes: &[u8]) -> String {
        self.0.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&self.0) {
            Ok(_) => self.0.len(),
            // 末尾の文字が途中で切れているだけなら、そこまでを使う
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            // 正しくない並びは、置き換え文字にしてすべて使う
            Err(_) => self.0.len(),
        };
        let text = String::from_utf8_lossy(&self.0[..valid]).into_owned();
        self.0.drain(..valid);
        text
    }
}

// 実行ログの行の種類
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogKind {
    // IDE からの知らせ
    Info,
    // cargo の進み具合（標準エラー）
    Build,
    Stdout,
    Stderr,
}

// コンパイルエラー・警告（エラー一覧に出す）
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub level: String,
    pub message: String,
    pub rendered: String,
    pub file: Option<String>,
    pub line: Option<u64>,
}

// 終わり方
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum Finish {
    // 準備やビルドの前で失敗した（cargo が見つからないなど）
    Failed {
        message: String,
    },
    // ビルドに失敗した
    CompileError,
    // ビルド中に止めた
    BuildStopped,
    // サンプルテストを実行した（各ケースの結果は Case で送る）
    Tested {
        total: usize,
        accepted: usize,
        // 不正解・実行時エラー・タイムアウトなど、正解でも目視確認でもないもの
        failed: usize,
        // 比較不可・目視確認
        unjudged: usize,
        // 止めた（D-099）
        interrupted: bool,
        // 実行したケースのうち、最も長い実行時間（ミリ秒。止めたケースは含めない）。実行履歴に出す（Phase 4a 作業 4）
        max_elapsed_ms: Option<u64>,
    },
    // 対話テストをした（Phase 4c 作業 1。D-144）。各ケースの結果は Case で送る
    InteractiveTested {
        total: usize,
        accepted: usize,
        // WA・RE・TLE・MLE・OLE など、AC でないもの
        failed: usize,
        // 止めた（D-099）
        interrupted: bool,
        // 実行したケースのうち、最も長い実行時間（ミリ秒。止めたケースは含めない）
        max_elapsed_ms: Option<u64>,
    },
    // ランダムテストをした（Phase 4b 作業 2。D-143）
    RandomTested {
        // 予定の回数（compare.json の randomCount）
        total: usize,
        // 解答を判定した回数（失敗した回を含む。止めた回は含めない）
        done: usize,
        // 1 回目の seed。i 回目（0 から）の seed は開始の seed + i
        start_seed: u64,
        // 解答が AC でなかった最初の回。なければ None
        failure: Option<RandomFailure>,
        // 止めた（D-099）
        interrupted: bool,
        // 解答の実行時間のうち最長（ミリ秒。止めた回は含めない）
        max_elapsed_ms: Option<u64>,
    },
    // デバッグをした（Phase 5 作業 1。D-145・D-146）。実行履歴には残さない
    Debugged {
        // プログラムの終了コード（`exited` の知らせ）。届かなかった（止めた・アダプターが落ちた）なら None
        exit_code: Option<i32>,
        // 停止の操作（またはアプリの終了）で止めた
        stopped: bool,
    },
    // AtCoder に提出した（Phase 6 作業 2。D-147・D-148）。実行履歴の「提出」に残す。
    // ほかの終わり方より大きいので、箱に入れる（JSON の形は、ほかと同じく kind と項目が並ぶ）
    Submitted(Box<SubmittedRecord>),
    // プログラムを実行した
    Ran {
        outcome: RunOutcome,
        // 実時間（ミリ秒。ローカルの計測）
        elapsed_ms: u64,
        // 出力のパイプが閉じなかった（出力の全体を得られなかった）
        output_left_open: bool,
    },
}

// 提出の記録（Phase 6 作業 2）。判定が決まったら verdict・score を書き足す
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmittedRecord {
    pub contest: String,
    pub task: String,
    // 提出の ID（自分の提出の一覧から決める）。決められなかったら None
    pub submission_id: Option<u64>,
    // 判定（例：AC、WA）と得点（ページの表記のまま）。決まるまで None
    pub verdict: Option<String>,
    pub score: Option<String>,
    // 送れなかったときの理由
    pub error: Option<String>,
}

// ランダムテストで見つかった失敗（D-143）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RandomFailure {
    pub seed: u64,
    // 解答の判定の略語（WA・RE・TLE・MLE・OLE など。D-140）。全体の結果と履歴に出す
    pub verdict: String,
    // テストのフォルダに保存したケースの名前（`random-N`）。保存できなかったら None（理由は実行ログに出す）
    pub case: Option<String>,
    // 同じ入力がすでに保存されていたので、新しくは保存しなかった（case はその名前）
    pub already_saved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum RunOutcome {
    Exited { code: i32 },
    Signaled { signal: i32 },
    TimedOut,
    OutputLimit { stream: LogKind },
    Stopped,
}

// チャンネルで画面に送るもの
#[derive(Debug, Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum RunEvent {
    Log {
        kind: LogKind,
        text: String,
    },
    Diagnostics {
        items: Vec<Diagnostic>,
    },
    // サンプルテストのケースを始めた
    CaseStarted {
        name: String,
    },
    // サンプルテストのケースの結果。ランダムテストでは、解答が AC でなかったケース（Phase 4b 作業 2）
    Case {
        result: CaseResult,
    },
    // ランダムテストの index 回目（1 から）を始めた。seed は生成器に渡す値（D-143）
    RandomProgress {
        index: usize,
        total: usize,
        seed: u64,
    },
    // デバッグの知らせ（Phase 5 作業 1。D-146）
    Debug {
        event: crate::debug::DebugEvent,
    },
    Finished {
        finish: Finish,
    },
}

// サンプルテストのケースの判定（AC2-02・AC2-03・AC2-10・AC2-18）
#[derive(Debug, Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum Verdict {
    // 正解（ローカルの比較。D-094）
    Accepted,
    WrongAnswer,
    // 比較不可・目視確認（.out がない、または compare.json で none。D-090・D-092）
    Unjudged,
    // 終了コード 0 以外、シグナル（RE）
    RuntimeError { outcome: RunOutcome },
    // 時間の制限（compare.json の timeLimitMs。既定 2500 ms）を超えたので止めた（TLE。D-141）
    TimedOut,
    // 最大使用メモリが制限（compare.json の memoryLimitMiB。既定 1024 MiB）を超えた（MLE。終わってから判定する。D-141）
    MemoryLimit,
    // 出力が上限を超えた。比較しない（D-096）
    OutputLimit,
    // 出力の全体を得られなかった。比較しない
    OutputIncomplete,
    // 止めたケース（中断。D-099）
    Interrupted,
    // 止めたので実行しなかったケース（未実行。D-099）
    NotRun,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseResult {
    pub name: String,
    pub verdict: Verdict,
    // 実時間（ミリ秒。ローカルの計測。D-094）
    pub elapsed_ms: Option<u64>,
    // 画面に出す分（それぞれ先頭 DISPLAY_LIMIT まで）
    pub input: String,
    pub expected: Option<String>,
    pub actual: String,
    pub stderr: String,
    // 期待出力のファイル（`sample-N.out`・`random-N.out`）の、パッケージのフォルダからの相対パス。なくても、置く場所を示す
    pub out_file: String,
    // 実際の出力を期待出力として保存できない理由。保存できるなら None（Phase 4a 作業 3。D-137）
    pub save_blocked: Option<String>,
    // 最大使用メモリ（最大 RSS。KiB。ローカルの計測。D-141）。受け取れなかったら None
    pub memory_kib: Option<u64>,
    // 対話テストの、やり取りなど（Phase 4c。D-144）。サンプルテスト・ランダムテストでは None
    pub interaction: Option<Interaction>,
}

// 対話テストの 1 ケースの、解答とジャッジのやり取りと補足
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Interaction {
    // 時系列で並べたやり取り（合わせて先頭 DISPLAY_LIMIT まで）。同じ側が続けて出力したものは 1 つ
    pub turns: Vec<Turn>,
    // やり取りを省略した（DISPLAY_LIMIT を超えた）
    pub turns_omitted: bool,
    // ジャッジの標準エラー。解答の標準エラーは CaseResult.stderr
    pub judge_stderr: String,
    // 判定の補足（ジャッジの終わり方、時間切れの注意など）
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub from: Side,
    pub text: String,
}

// 経過時間をミリ秒で表す。実行履歴（history.json）から読み直すために u64 にする
// （serde の、タグ付きの enum の読み込みは u128 を扱えない。Phase 4a 作業 4）
pub(crate) fn millis(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

// 画面に送る。ウィンドウが閉じて送れなくても、実行は止めない
pub(crate) fn send(channel: &Channel<RunEvent>, event: RunEvent) {
    let _ = channel.send(event);
}

pub(crate) fn info(channel: &Channel<RunEvent>, text: impl Into<String>) {
    send(
        channel,
        RunEvent::Log {
            kind: LogKind::Info,
            text: text.into(),
        },
    );
}

// cargo build の JSON の出力（--message-format=json）のうち、使うところだけ
#[derive(Deserialize)]
struct CargoMessage {
    reason: String,
    message: Option<CompilerMessage>,
    target: Option<ArtifactTarget>,
    executable: Option<PathBuf>,
}

#[derive(Deserialize)]
struct CompilerMessage {
    level: String,
    message: String,
    rendered: Option<String>,
    spans: Vec<Span>,
}

#[derive(Deserialize)]
struct Span {
    file_name: String,
    line_start: u64,
    is_primary: bool,
}

#[derive(Deserialize)]
struct ArtifactTarget {
    name: String,
    kind: Vec<String>,
}

// cargo build の標準出力から、種類（`bin`・`example`）と名前が合う実行ファイルの場所を取り出す
pub(crate) fn find_executable(stdout: &str, name: &str, kind: &str) -> Option<PathBuf> {
    stdout.lines().find_map(|line| {
        let message = serde_json::from_str::<CargoMessage>(line).ok()?;
        let target = message.target?;
        (message.reason == "compiler-artifact"
            && target.name == name
            && target.kind.iter().any(|item| item == kind))
        .then_some(message.executable)?
    })
}

// cargo build の標準出力（1 行に 1 つの JSON）から、診断と実行ファイルの場所を取り出す
fn parse_build_output(stdout: &str, bin: &str) -> (Vec<Diagnostic>, Option<PathBuf>) {
    let mut diagnostics = Vec::new();
    let mut executable = None;
    for line in stdout.lines() {
        let Ok(message) = serde_json::from_str::<CargoMessage>(line) else {
            continue;
        };
        match message.reason.as_str() {
            "compiler-message" => {
                if let Some(compiler) = message.message {
                    let primary = compiler.spans.iter().find(|span| span.is_primary);
                    diagnostics.push(Diagnostic {
                        level: compiler.level,
                        rendered: compiler
                            .rendered
                            .unwrap_or_else(|| compiler.message.clone()),
                        message: compiler.message,
                        file: primary.map(|span| span.file_name.clone()),
                        line: primary.map(|span| span.line_start),
                    });
                }
            }
            "compiler-artifact" => {
                if let Some(target) = message.target
                    && target.name == bin
                    && target.kind.iter().any(|kind| kind == "bin")
                    && message.executable.is_some()
                {
                    executable = message.executable;
                }
            }
            _ => {}
        }
    }
    (diagnostics, executable)
}

// 1 つの出力を、画面に出す量（DISPLAY_LIMIT）まで送る。超えたら一度だけ知らせる
struct Display<'a> {
    channel: &'a Channel<RunEvent>,
    kind: LogKind,
    name: &'static str,
    shown: usize,
    omitted: bool,
    carry: Utf8Carry,
}

impl<'a> Display<'a> {
    fn new(channel: &'a Channel<RunEvent>, kind: LogKind, name: &'static str) -> Self {
        Display {
            channel,
            kind,
            name,
            shown: 0,
            omitted: false,
            carry: Utf8Carry::default(),
        }
    }

    fn output(&mut self, bytes: &[u8]) {
        let room = DISPLAY_LIMIT.saturating_sub(self.shown);
        let part = &bytes[..bytes.len().min(room)];
        self.shown += part.len();
        if !part.is_empty() {
            let text = self.carry.decode(part);
            if !text.is_empty() {
                send(
                    self.channel,
                    RunEvent::Log {
                        kind: self.kind,
                        text,
                    },
                );
            }
        }
        if bytes.len() > room && !self.omitted {
            self.omitted = true;
            info(
                self.channel,
                format!(
                    "\n（{}の表示を省略しました。先頭 {} KiB だけを表示しています）\n",
                    self.name,
                    DISPLAY_LIMIT / 1024
                ),
            );
        }
    }
}

pub(crate) fn outcome_of(outcome: Outcome) -> RunOutcome {
    match outcome {
        Outcome::Exited(code) => RunOutcome::Exited { code },
        Outcome::Signaled(signal) => RunOutcome::Signaled { signal },
        Outcome::TimedOut => RunOutcome::TimedOut,
        Outcome::OutputLimit(Stream::Stdout) => RunOutcome::OutputLimit {
            stream: LogKind::Stdout,
        },
        Outcome::OutputLimit(Stream::Stderr) => RunOutcome::OutputLimit {
            stream: LogKind::Stderr,
        },
        Outcome::Stopped => RunOutcome::Stopped,
    }
}

// 止めたときの知らせ（D-095 の条件 (2)）
pub(crate) const NOT_CHECKED: &str =
    "止めた後に子孫のプロセスが残っていないかは、IDE では確かめていません。";

// 通常実行とサンプルテストの準備：対象と版を示し、ビルドする。dir はパッケージのディレクトリ（canonicalize 済み）
// before_build は、バイナリと example の一覧が分かった後、ビルドの前に呼ぶ（サンプルテストのケースの確かめ）。
// 返す値の 2 つ目は、あわせてビルドする example の名前（ランダムテストの生成器・愚直解、対話テストのジャッジ。D-143・D-144）。
// Rust は cargo build に `--example <名前>` を足し、C++ は `examples/<名前>.cpp` を g++ でビルドする（Phase 17 作業 7。D-212）。
// ビルドした example の場所は Prepared の examples に入る。
// release が false なら `--release` を付けない（デバッグ用の dev のビルド。Phase 5 作業 1。D-146）
pub(crate) fn prepare<T>(
    dir: &Path,
    target: &Target,
    release: bool,
    langs: &LangSettings,
    cancel: &Arc<AtomicBool>,
    channel: &Channel<RunEvent>,
    before_build: impl FnOnce(&[String], &[String]) -> Result<(T, Vec<String>), Finish>,
) -> Result<(Prepared, T), Finish> {
    let package = if target.package.is_empty() {
        "（作業領域のルート）"
    } else {
        &target.package
    };
    let what = match target.language {
        Language::Rust => target.bin.clone(),
        Language::Cpp => format!("{}（C++）", cpp::SOURCE),
        Language::Python => format!("{}（Python）", python::SOURCE),
    };
    info(channel, format!("実行の対象：{package} の {what}\n"));
    match target.language {
        Language::Rust => {}
        Language::Cpp => {
            return prepare_cpp(dir, release, &langs.cpp, cancel, channel, before_build);
        }
        Language::Python => {
            return prepare_python(dir, &langs.python, cancel, channel, before_build);
        }
    }

    let Some(cargo) = toolchain::find_cargo() else {
        return Err(Finish::Failed {
            message: "cargo が見つかりません".to_string(),
        });
    };

    // 版の表示（D-097）
    let (rustc_version, cargo_version) = toolchain::versions(&cargo, dir);
    match &rustc_version {
        Ok(version) => info(channel, format!("ローカル：{version}\n")),
        Err(error) => info(
            channel,
            format!("ローカルの rustc の版を確かめられません：{error}\n{TOOLCHAIN_HINT}\n"),
        ),
    }
    if let Ok(version) = &cargo_version {
        info(channel, format!("ローカル：{version}\n"));
    }
    info(channel, format!("ジャッジ：{}\n", toolchain::JUDGE_NOTE));
    if let Ok(version) = &rustc_version
        && !toolchain::same_as_judge(version)
    {
        info(channel, "ローカルの rustc の版がジャッジと異なります。\n");
    }

    // バイナリの一覧（D-106 のテストのフォルダに使う）と example の一覧（ランダムテストの生成器・愚直解。D-143）
    let (bins, examples) = match toolchain::metadata_json(&cargo, dir).and_then(|json| {
        Ok((
            toolchain::bin_names(&json, dir)?,
            toolchain::example_names(&json, dir)?,
        ))
    }) {
        Ok(names) => names,
        Err(message) => return Err(Finish::Failed { message }),
    };
    if !bins.contains(&target.bin) {
        return Err(Finish::Failed {
            message: format!("バイナリ「{}」が見つかりません", target.bin),
        });
    }

    let (checked, wanted_examples) = before_build(&bins, &examples)?;

    // ビルド（D-089）。時間の上限は設けない（D-096）
    info(channel, "\nビルドしています…\n");
    let mut build = toolchain::cargo_command(&cargo, dir);
    build.arg("build");
    if release {
        build.arg("--release");
    }
    build.args(["--bin", &target.bin, "--message-format=json"]);
    for name in &wanted_examples {
        build.args(["--example", name]);
    }
    let mut build_log = Display::new(channel, LogKind::Build, "ビルドの出力");
    let built = process::run(
        build,
        None,
        &Limits {
            time: None,
            output: OUTPUT_LIMIT,
            memory: None,
        },
        cancel,
        |stream, bytes| {
            if stream == Stream::Stderr {
                build_log.output(bytes);
            }
        },
    );
    let built = match built {
        Ok(built) => built,
        Err(message) => return Err(Finish::Failed { message }),
    };
    let build_stdout = String::from_utf8_lossy(&built.stdout).into_owned();
    let (diagnostics, executable) = parse_build_output(&build_stdout, &target.bin);
    let has_errors = diagnostics.iter().any(|item| item.level == "error");
    send(channel, RunEvent::Diagnostics { items: diagnostics });
    match built.outcome {
        Outcome::Exited(0) => {}
        Outcome::Stopped => {
            info(channel, format!("ビルドを止めました。{NOT_CHECKED}\n"));
            return Err(Finish::BuildStopped);
        }
        Outcome::Exited(_) if has_errors => {
            info(
                channel,
                "ビルドに失敗しました。エラー一覧を見てください。\n",
            );
            return Err(Finish::CompileError);
        }
        // コンパイルエラーではない失敗（ツールチェーンの解決、Cargo.toml の誤りなど）。
        // cargo の出力は実行ログに出ている
        Outcome::Exited(_) => {
            info(
                channel,
                format!("ビルドに失敗しました。\n{TOOLCHAIN_HINT}\n"),
            );
            return Err(Finish::Failed {
                message: "ビルドに失敗しました（実行ログを見てください）".to_string(),
            });
        }
        other => {
            return Err(Finish::Failed {
                message: format!("ビルドが異常に終わりました（{other:?}）"),
            });
        }
    }
    let Some(executable) = executable else {
        return Err(Finish::Failed {
            message: "ビルドした実行ファイルの場所が分かりません".to_string(),
        });
    };
    let mut built_examples = Vec::new();
    for name in wanted_examples {
        let Some(path) = find_executable(&build_stdout, &name, "example") else {
            return Err(Finish::Failed {
                message: format!("ビルドした example（{name}）の実行ファイルの場所が分かりません"),
            });
        };
        built_examples.push((name, Program::native(path)));
    }

    Ok((
        Prepared {
            executable: Program::native(executable),
            bins,
            examples: built_examples,
        },
        checked,
    ))
}

// C++ の準備（Phase 16 作業 1。D-205）：コンパイラと版を示し、g++ でビルドする。
// バイナリの一覧は main だけ。example は examples/*.cpp（ランダムテスト・対話テスト。Phase 17 作業 7。D-212）
fn prepare_cpp<T>(
    dir: &Path,
    release: bool,
    settings: &CppSettings,
    cancel: &Arc<AtomicBool>,
    channel: &Channel<RunEvent>,
    before_build: impl FnOnce(&[String], &[String]) -> Result<(T, Vec<String>), Finish>,
) -> Result<(Prepared, T), Finish> {
    let compiler = cpp::find_compiler(settings).map_err(|message| Finish::Failed { message })?;
    // 版の表示（D-097 と同じ）。GCC でなければビルドしない
    let (line, number) = cpp::version(&compiler).map_err(|message| Finish::Failed { message })?;
    info(
        channel,
        format!("ローカル：{line}（{}）\n", compiler.display()),
    );
    info(channel, format!("ジャッジ：{}\n", cpp::JUDGE_NOTE));
    if number != cpp::JUDGE_VERSION {
        info(channel, "ローカルの GCC の版がジャッジと異なります。\n");
    }
    info(channel, format!("{}\n", cpp::NOT_LIKE_JUDGE));
    // AC Library（D-206）
    match &settings.ac_library {
        Some(dir) => info(channel, format!("AC Library：{}\n", dir.display())),
        None => info(
            channel,
            format!(
                "AC Library：なし（使うときは、アプリのフォルダの {}/atcoder/ に置いてください）\n",
                cpp::AC_LIBRARY_DIR
            ),
        ),
    }

    let bins = vec![cpp::BIN.to_string()];
    let examples = cpp::example_names(dir);
    let (checked, wanted_examples) = before_build(&bins, &examples)?;

    // ビルド。時間の上限は設けない（D-096 と同じ）。main.cpp のあと、要る example を 1 つずつ（D-212）。
    // エラー一覧には、それまでのビルドの診断を合わせて出す
    info(channel, "\nビルドしています…\n");
    let mut diagnostics = Vec::new();
    let executable = cpp::output_path(dir, release);
    build_cpp(
        &compiler,
        dir,
        cpp::build_args(release, &executable, settings),
        &executable,
        &mut diagnostics,
        cancel,
        channel,
    )?;
    let mut built_examples = Vec::new();
    for name in wanted_examples {
        let output = cpp::example_output_path(dir, &name);
        let source = format!("examples/{name}.cpp");
        build_cpp(
            &compiler,
            dir,
            cpp::build_args_for(&source, &output, settings),
            &output,
            &mut diagnostics,
            cancel,
            channel,
        )?;
        built_examples.push((name, Program::native(output)));
    }
    Ok((
        Prepared {
            executable: Program::native(executable),
            bins,
            examples: built_examples,
        },
        checked,
    ))
}

// Python の準備（Phase 19 作業 1。D-220〜D-222）：処理系と版を示し、ジャッジと同じ版でなければ動かさない。
// ビルドはなく、main.py と要る example の構文を確かめる（誤りなら CE）。example は examples/*.py
fn prepare_python<T>(
    dir: &Path,
    settings: &python::PythonSettings,
    cancel: &Arc<AtomicBool>,
    channel: &Channel<RunEvent>,
    before_build: impl FnOnce(&[String], &[String]) -> Result<(T, Vec<String>), Finish>,
) -> Result<(Prepared, T), Finish> {
    let kind = settings.interpreter;
    let interpreter =
        python::find_interpreter(settings).map_err(|message| Finish::Failed { message })?;
    let version = python::version(&interpreter).map_err(|message| Finish::Failed { message })?;
    info(
        channel,
        format!("ローカル：{}（{}）\n", version.line, interpreter.display()),
    );
    info(channel, format!("ジャッジ：{}\n", kind.judge_note()));
    match python::check_version(kind, &version) {
        Ok(Some(note)) => info(channel, format!("{note}\n")),
        Ok(None) => {}
        Err(message) => return Err(Finish::Failed { message }),
    }

    let bins = vec![python::BIN.to_string()];
    let examples = python::example_names(dir);
    let (checked, wanted_examples) = before_build(&bins, &examples)?;

    // 構文の確かめ（D-222）。main.py のあと、要る example を 1 つずつ。エラー一覧には、それまでの診断を合わせて出す
    info(channel, "\n構文を確かめています…\n");
    let mut diagnostics = Vec::new();
    check_python(
        &interpreter,
        dir,
        python::SOURCE,
        &mut diagnostics,
        cancel,
        channel,
    )?;
    let mut built_examples = Vec::new();
    for name in wanted_examples {
        let source = format!("examples/{name}.py");
        check_python(
            &interpreter,
            dir,
            &source,
            &mut diagnostics,
            cancel,
            channel,
        )?;
        built_examples.push((name, python::program(&interpreter, &source)));
    }
    Ok((
        Prepared {
            executable: python::program(&interpreter, python::SOURCE),
            bins,
            examples: built_examples,
        },
        checked,
    ))
}

// Python のソースを 1 つ確かめる（パッケージのディレクトリで動かす）。誤りなら CE
fn check_python(
    interpreter: &Path,
    dir: &Path,
    source: &str,
    diagnostics: &mut Vec<Diagnostic>,
    cancel: &Arc<AtomicBool>,
    channel: &Channel<RunEvent>,
) -> Result<(), Finish> {
    let mut check_log = Display::new(channel, LogKind::Build, "構文の確かめの出力");
    let checked = process::run(
        python::check_command(interpreter, dir, source),
        None,
        &Limits {
            time: None,
            output: OUTPUT_LIMIT,
            memory: None,
        },
        cancel,
        |stream, bytes| {
            if stream == Stream::Stderr {
                check_log.output(bytes);
            }
        },
    )
    .map_err(|message| Finish::Failed { message })?;
    let found = python::parse_check(&String::from_utf8_lossy(&checked.stdout), source);
    let has_errors = !found.is_empty();
    diagnostics.extend(found);
    send(
        channel,
        RunEvent::Diagnostics {
            items: diagnostics.clone(),
        },
    );
    match checked.outcome {
        Outcome::Exited(0) => Ok(()),
        Outcome::Stopped => {
            info(
                channel,
                format!("構文の確かめを止めました。{NOT_CHECKED}\n"),
            );
            Err(Finish::BuildStopped)
        }
        Outcome::Exited(_) if has_errors => {
            info(
                channel,
                "構文の誤りがあります。エラー一覧を見てください。\n",
            );
            Err(Finish::CompileError)
        }
        other => Err(Finish::Failed {
            message: format!("{source} の構文を確かめられません（{other:?}）"),
        }),
    }
}

// g++ で 1 つビルドする（パッケージのディレクトリで動かす）。診断は diagnostics に足してエラー一覧に送る
fn build_cpp(
    compiler: &Path,
    dir: &Path,
    args: Vec<String>,
    output: &Path,
    diagnostics: &mut Vec<Diagnostic>,
    cancel: &Arc<AtomicBool>,
    channel: &Channel<RunEvent>,
) -> Result<(), Finish> {
    if let Some(parent) = output.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        return Err(Finish::Failed {
            message: format!("ビルドの置き場所（target/）を作れません：{error}"),
        });
    }
    let mut build = Command::new(compiler);
    build.current_dir(dir).args(args);
    toolchain::clear_ide_env(&mut build);
    let mut build_log = Display::new(channel, LogKind::Build, "ビルドの出力");
    let built = process::run(
        build,
        None,
        &Limits {
            time: None,
            output: OUTPUT_LIMIT,
            memory: None,
        },
        cancel,
        |stream, bytes| {
            if stream == Stream::Stderr {
                build_log.output(bytes);
            }
        },
    )
    .map_err(|message| Finish::Failed { message })?;
    let found = cpp::parse_diagnostics(&String::from_utf8_lossy(&built.stderr));
    let has_errors = found.iter().any(|item| item.level == "error");
    diagnostics.extend(found);
    send(
        channel,
        RunEvent::Diagnostics {
            items: diagnostics.clone(),
        },
    );
    match built.outcome {
        Outcome::Exited(0) => {}
        Outcome::Stopped => {
            info(channel, format!("ビルドを止めました。{NOT_CHECKED}\n"));
            return Err(Finish::BuildStopped);
        }
        Outcome::Exited(_) if has_errors => {
            info(
                channel,
                "ビルドに失敗しました。エラー一覧を見てください。\n",
            );
            return Err(Finish::CompileError);
        }
        Outcome::Exited(_) => {
            return Err(Finish::Failed {
                message: "ビルドに失敗しました（実行ログを見てください）".to_string(),
            });
        }
        other => {
            return Err(Finish::Failed {
                message: format!("ビルドが異常に終わりました（{other:?}）"),
            });
        }
    }
    Ok(())
}

// 動かすもの：実行ファイルと、その引数（Python は処理系とソース。Phase 19 作業 1。D-221）
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Program {
    pub path: PathBuf,
    pub args: Vec<OsString>,
}

impl Program {
    // ビルドした実行ファイル（引数なし）
    pub(crate) fn native(path: PathBuf) -> Self {
        Program {
            path,
            args: Vec::new(),
        }
    }

    pub(crate) fn command(&self) -> Command {
        let mut command = Command::new(&self.path);
        command.args(&self.args);
        command
    }
}

// ビルドした実行ファイルと、パッケージのバイナリの一覧、ビルドした example の名前と実行ファイル
pub(crate) struct Prepared {
    pub executable: Program,
    pub bins: Vec<String>,
    pub examples: Vec<(String, Program)>,
}

impl Prepared {
    // ビルドした example の実行ファイル（what は「生成器」などの説明。見つからなければ Finish::Failed）
    pub(crate) fn example(&self, name: &str, what: &str) -> Result<Program, Finish> {
        self.examples
            .iter()
            .find(|(built, _)| built == name)
            .map(|(_, path)| path.clone())
            .ok_or_else(|| Finish::Failed {
                message: format!("ビルドした{what}（{name}）の実行ファイルの場所が分かりません"),
            })
    }
}

// 通常実行の本体。dir はパッケージのディレクトリ（canonicalize 済み）
fn run_target(
    dir: &Path,
    target: &Target,
    langs: &LangSettings,
    cancel: &Arc<AtomicBool>,
    channel: &Channel<RunEvent>,
) -> Finish {
    let (
        Prepared {
            executable, bins, ..
        },
        (),
    ) = match prepare(dir, target, true, langs, cancel, channel, |_, _| {
        Ok(((), Vec::new()))
    }) {
        Ok(prepared) => prepared,
        Err(finish) => return finish,
    };

    // 標準入力（D-091・D-106）
    let tests = toolchain::test_dir(dir, &bins, &target.bin);
    if let Some(notice) = &tests.notice {
        info(channel, format!("{notice}\n"));
    }
    let input_path = tests.dir.as_ref().map(|test| test.join("input.txt"));
    let stdin = match input_path.as_ref().filter(|path| path.is_file()) {
        Some(path) => match File::open(path) {
            Ok(file) => {
                info(channel, format!("標準入力：{}\n", relative(dir, path)));
                Some(file)
            }
            Err(error) => {
                return Finish::Failed {
                    message: format!("入力ファイルを開けません：{error}"),
                };
            }
        },
        None => {
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
            info(
                channel,
                format!(
                    "入力ファイルなし（空の標準入力で実行）。入力が必要なら {folder}/input.txt を作ってください\n"
                ),
            );
            None
        }
    };

    // 実行（D-089）。D-104 の変数を外し、パッケージのディレクトリで直接起動する
    info(channel, "\n実行しています…\n");
    let mut program = executable.command();
    program.current_dir(dir);
    toolchain::clear_ide_env(&mut program);
    let mut stdout_log = Display::new(channel, LogKind::Stdout, "標準出力");
    let mut stderr_log = Display::new(channel, LogKind::Stderr, "標準エラー");
    let result = process::run(
        program,
        stdin,
        &Limits {
            time: Some(RUN_TIME_LIMIT),
            output: OUTPUT_LIMIT,
            memory: None,
        },
        cancel,
        |stream, bytes| match stream {
            Stream::Stdout => stdout_log.output(bytes),
            Stream::Stderr => stderr_log.output(bytes),
        },
    );
    let result = match result {
        Ok(result) => result,
        Err(message) => return Finish::Failed { message },
    };
    match result.outcome {
        Outcome::Stopped => info(channel, format!("\n止めました。{NOT_CHECKED}\n")),
        Outcome::TimedOut => info(
            channel,
            format!(
                "\nタイムアウト（{} 秒）で止めました。ジャッジの実行時間制限とは別の、IDE の上限です。{NOT_CHECKED}\n",
                RUN_TIME_LIMIT.as_secs()
            ),
        ),
        Outcome::OutputLimit(_) => info(
            channel,
            format!(
                "\n出力が上限（{} MiB）を超えたので、切り詰めてプログラムを止めました。出力の全体は得られていません。{NOT_CHECKED}\n",
                OUTPUT_LIMIT / 1024 / 1024
            ),
        ),
        _ => {}
    }
    if result.output_left_open {
        info(
            channel,
            "\nプログラムが終わった後も出力が閉じられなかったので、読み取りを打ち切りました。出力の全体は得られていない可能性があります。\n",
        );
    }
    Finish::Ran {
        outcome: outcome_of(result.outcome),
        elapsed_ms: millis(result.elapsed),
        output_left_open: result.output_left_open,
    }
}

// 画面に出す分の文字列（先頭 DISPLAY_LIMIT まで）
pub(crate) fn shown(bytes: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(&bytes[..bytes.len().min(DISPLAY_LIMIT)]).into_owned();
    if bytes.len() > DISPLAY_LIMIT {
        text.push_str(&format!(
            "\n（先頭 {} KiB だけを表示しています）",
            DISPLAY_LIMIT / 1024
        ));
    }
    text
}

// サンプルテストのケースがあるか（提出の前のサンプルテスト。Phase 9 作業 2。D-162）
pub fn has_test_cases(test_dir: &Path) -> bool {
    !test_cases(test_dir).is_empty()
}

// テストのフォルダの sample-N.in と random-N.in を並べる（D-090・D-143）。sample が先、それぞれ N の数値の順
fn test_cases(test_dir: &Path) -> Vec<(CaseKey, PathBuf)> {
    let mut cases: Vec<(CaseKey, PathBuf)> = std::fs::read_dir(test_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let key = case_key(name.strip_suffix(".in")?)?;
                    let path = entry.path();
                    path.is_file().then_some((key, path))
                })
                .collect()
        })
        .unwrap_or_default();
    cases.sort();
    cases
}

// 実行するケースを選ぶ（Phase 4a 作業 2。D-137）。wanted が None なら全部。名前（`sample-N`・`random-N`）で指定し、
// テストのフォルダにあるケースの名前と照らす（パスとしては使わない）。ないケースがあれば、実行しない
fn select_cases(
    all: Vec<(CaseKey, PathBuf)>,
    wanted: Option<&[String]>,
) -> Result<Vec<(CaseKey, PathBuf)>, String> {
    let Some(wanted) = wanted else {
        return Ok(all);
    };
    if wanted.is_empty() {
        return Err("実行するケースが指定されていません".to_string());
    }
    let name_of = |path: &Path| {
        path.file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let missing: Vec<&str> = wanted
        .iter()
        .filter(|name| !all.iter().any(|(_, path)| &name_of(path) == *name))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "指定したケースがありません：{}（テストケースのフォルダを確かめてください）",
            missing.join("、")
        ));
    }
    Ok(all
        .into_iter()
        .filter(|(_, path)| wanted.contains(&name_of(path)))
        .collect())
}

// 実際の出力を期待出力として保存できない理由。保存できるなら None（Phase 4a 作業 3。D-137）。
// 画面に出す文字列（`shown`）は、長いと切り詰め、UTF-8 でない部分を置き換えるので、
// 保存できるのは、正常に終わり、出力の全体を得られ、画面に出す文字列が実際の出力そのものであるときだけ
fn save_block_reason(completed: bool, stdout: &[u8]) -> Option<String> {
    if !completed {
        return Some("出力を最後まで得られなかったので保存できません".to_string());
    }
    if stdout.len() > DISPLAY_LIMIT {
        return Some(format!(
            "出力が {} KiB を超えるので保存できません",
            DISPLAY_LIMIT / 1024
        ));
    }
    if std::str::from_utf8(stdout).is_err() {
        return Some("出力が UTF-8 ではないので保存できません".to_string());
    }
    None
}

// 1 つのケースの判定（D-141 の順）。止めた → 中断、時間の制限 → TLE、出力の上限 → OLE、
// メモリの制限 → MLE、異常な終わり方 → RE、出力の全体を得られなかった → 比較しない、そのあとで出力を比べる（AC・WA・目視確認）
pub(crate) fn verdict_of(
    result: &process::RunResult,
    expected: Option<&[u8]>,
    settings: &TestSettings,
) -> Verdict {
    let memory_exceeded = result
        .max_rss_bytes
        .is_some_and(|bytes| bytes > settings.memory_limit_mib * 1024 * 1024);
    match result.outcome {
        Outcome::Stopped => Verdict::Interrupted,
        Outcome::TimedOut => Verdict::TimedOut,
        Outcome::OutputLimit(_) => Verdict::OutputLimit,
        _ if memory_exceeded => Verdict::MemoryLimit,
        Outcome::Exited(0) if result.output_left_open => Verdict::OutputIncomplete,
        Outcome::Exited(0) => match expected
            .and_then(|expected| compare::judge(&settings.mode, expected, &result.stdout))
        {
            Some(true) => Verdict::Accepted,
            Some(false) => Verdict::WrongAnswer,
            None => Verdict::Unjudged,
        },
        other => Verdict::RuntimeError {
            outcome: outcome_of(other),
        },
    }
}

// 1 つのケースを実行して判定する。時間の制限で止める（TLE。D-141）
fn run_case(
    dir: &Path,
    executable: &Program,
    input: &Path,
    settings: &TestSettings,
    cancel: &Arc<AtomicBool>,
) -> Result<CaseResult, String> {
    let name = input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let input_bytes =
        std::fs::read(input).map_err(|error| format!("{name}.in を読めません：{error}"))?;
    let expected_path = input.with_extension("out");
    let expected = if expected_path.is_file() {
        Some(
            std::fs::read(&expected_path)
                .map_err(|error| format!("{name}.out を読めません：{error}"))?,
        )
    } else {
        None
    };
    let mut program = executable.command();
    program.current_dir(dir);
    toolchain::clear_ide_env(&mut program);
    let stdin = File::open(input).map_err(|error| format!("{name}.in を開けません：{error}"))?;
    let result = process::run(
        program,
        Some(stdin),
        &Limits {
            time: Some(Duration::from_millis(settings.time_limit_ms)),
            output: OUTPUT_LIMIT,
            // 実行中もメモリの制限を見て、超えたら止める（D-175）
            memory: Some(settings.memory_limit_mib * 1024 * 1024),
        },
        cancel,
        |_, _| {},
    )?;
    let verdict = verdict_of(&result, expected.as_deref(), settings);
    let completed = matches!(result.outcome, Outcome::Exited(0)) && !result.output_left_open;
    Ok(CaseResult {
        name,
        verdict,
        elapsed_ms: Some(millis(result.elapsed)),
        input: shown(&input_bytes),
        expected: expected.as_deref().map(shown),
        actual: shown(&result.stdout),
        stderr: shown(&result.stderr),
        out_file: relative(dir, &expected_path),
        save_blocked: save_block_reason(completed, &result.stdout),
        memory_kib: result.max_rss_bytes.map(|bytes| bytes.div_ceil(1024)),
        interaction: None,
    })
}

// サンプルテストの本体（D-004・D-090・D-092〜D-094・D-099）。ケースは 1 件ずつ順に実行する（D-013）
// wanted を渡すと、その名前のケースだけ実行する（1 ケースだけ・失敗したケースだけの再実行。D-137）
fn test_target(
    dir: &Path,
    target: &Target,
    langs: &LangSettings,
    wanted: Option<&[String]>,
    cancel: &Arc<AtomicBool>,
    channel: &Channel<RunEvent>,
) -> Finish {
    // テストケースと compare.json は、ビルドの前に確かめる（誤りならビルドしない）
    let (Prepared { executable, .. }, (folder, cases, total, settings)) = match prepare(
        dir,
        target,
        true,
        langs,
        cancel,
        channel,
        |bins, _| {
            let tests = toolchain::test_dir(dir, bins, &target.bin);
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
            let cases = tests.dir.as_deref().map(test_cases).unwrap_or_default();
            if cases.is_empty() {
                return Err(Finish::Failed {
                    message: format!(
                        "テストケースがありません。{folder}/sample-1.in と sample-1.out を作ってください"
                    ),
                });
            }
            let total = cases.len();
            let cases =
                select_cases(cases, wanted).map_err(|message| Finish::Failed { message })?;
            let settings = tests
                .dir
                .as_deref()
                .map(compare::read_settings)
                .unwrap_or_else(|| Ok(TestSettings::default()))
                .map_err(|message| Finish::Failed { message })?;
            Ok(((folder, cases, total, settings), Vec::new()))
        },
    ) {
        Ok(prepared) => prepared,
        Err(finish) => return finish,
    };
    let count = if cases.len() == total {
        format!("{} 件", cases.len())
    } else {
        format!("全 {total} 件のうち {} 件", cases.len())
    };
    info(
        channel,
        format!(
            "\nサンプルテスト：{folder} の {count}（比較の方式：{}、時間の制限 {} ms、メモリの制限 {} MiB。ローカルの判定）\n",
            mode_name(&settings.mode),
            settings.time_limit_ms,
            settings.memory_limit_mib
        ),
    );

    let (mut accepted, mut failed, mut unjudged) = (0, 0, 0);
    let mut interrupted = false;
    let mut max_elapsed_ms: Option<u64> = None;
    for (_, input) in &cases {
        if interrupted {
            // 止めたので、残りのケースは実行しない（D-099）
            let name = input
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            send(
                channel,
                RunEvent::Case {
                    result: CaseResult {
                        name,
                        verdict: Verdict::NotRun,
                        elapsed_ms: None,
                        input: String::new(),
                        expected: None,
                        actual: String::new(),
                        stderr: String::new(),
                        out_file: relative(dir, &input.with_extension("out")),
                        save_blocked: Some("実行していないので保存できません".to_string()),
                        memory_kib: None,
                        interaction: None,
                    },
                },
            );
            continue;
        }
        let name = input
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        send(channel, RunEvent::CaseStarted { name });
        let result = match run_case(dir, &executable, input, &settings, cancel) {
            Ok(result) => result,
            Err(message) => return Finish::Failed { message },
        };
        match result.verdict {
            Verdict::Accepted => accepted += 1,
            Verdict::Unjudged => unjudged += 1,
            Verdict::Interrupted => interrupted = true,
            _ => failed += 1,
        }
        if !matches!(result.verdict, Verdict::Interrupted) {
            max_elapsed_ms = max_elapsed_ms.max(result.elapsed_ms);
        }
        send(channel, RunEvent::Case { result });
    }
    if interrupted {
        info(
            channel,
            format!("\nサンプルテストを止めました。{NOT_CHECKED}\n"),
        );
    }
    Finish::Tested {
        total: cases.len(),
        accepted,
        failed,
        unjudged,
        interrupted,
        max_elapsed_ms,
    }
}

fn mode_name(mode: &Mode) -> &'static str {
    match mode {
        Mode::Whitespace => "空白緩和",
        Mode::Exact => "完全一致",
        Mode::Float { .. } => "浮動小数点",
        Mode::None => "比較しない",
    }
}

// dir からの相対パス（区切りは /）
pub(crate) fn relative(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir)
        .map(|relative| {
            relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

// 通常実行を始め、終わるまで待つ。dir はパッケージのディレクトリ（作業領域の中で確かめ済み）
// test が true ならサンプルテスト、false なら通常実行
// cases は、サンプルテストで実行するケースの名前（None なら全部）。終わり方を返す（実行履歴に残す。Phase 4a 作業 4）
pub fn start(
    dir: PathBuf,
    target: Target,
    langs: LangSettings,
    test: bool,
    cases: Option<Vec<String>>,
    control: &RunControl,
    channel: Channel<RunEvent>,
) -> Result<Finish, String> {
    if cases.is_some() && !test {
        return Err("ケースを指定できるのはサンプルテストだけです".to_string());
    }
    let cancel = control.begin()?;
    let guard = RunGuard(control);
    let finish = if test {
        test_target(&dir, &target, &langs, cases.as_deref(), &cancel, &channel)
    } else {
        run_target(&dir, &target, &langs, &cancel, &channel)
    };
    drop(guard);
    send(
        &channel,
        RunEvent::Finished {
            finish: finish.clone(),
        },
    );
    Ok(finish)
}

// ランダムテストを始め、終わるまで待つ（Phase 4b 作業 2。D-143）。dir はパッケージのディレクトリ、
// root は作業領域のルート（どちらも確かめ済み。失敗したケースの保存先が作業領域の中かを確かめるのに使う）。
// start_seed は 1 回目の seed。終わり方を返す（実行履歴に残す）
pub fn start_random(
    dir: PathBuf,
    root: PathBuf,
    target: Target,
    langs: LangSettings,
    start_seed: u64,
    control: &RunControl,
    channel: Channel<RunEvent>,
) -> Result<Finish, String> {
    let cancel = control.begin()?;
    let guard = RunGuard(control);
    let finish =
        crate::random::random_target(&dir, &root, &target, &langs, start_seed, &cancel, &channel);
    drop(guard);
    send(
        &channel,
        RunEvent::Finished {
            finish: finish.clone(),
        },
    );
    Ok(finish)
}

// 対話テストを始め、終わるまで待つ（Phase 4c 作業 1。D-144）。dir はパッケージのディレクトリ（作業領域の中で確かめ済み）。
// cases は、実行するケースの名前（`interactive-N`。None なら全部）。終わり方を返す（実行履歴に残す）
pub fn start_interactive(
    dir: PathBuf,
    target: Target,
    langs: LangSettings,
    cases: Option<Vec<String>>,
    control: &RunControl,
    channel: Channel<RunEvent>,
) -> Result<Finish, String> {
    let cancel = control.begin()?;
    let guard = RunGuard(control);
    let finish = crate::interactive::interactive_target(
        &dir,
        &target,
        &langs,
        cases.as_deref(),
        &cancel,
        &channel,
    );
    drop(guard);
    send(
        &channel,
        RunEvent::Finished {
            finish: finish.clone(),
        },
    );
    Ok(finish)
}

// デバッグを始め、終わるまで待つ（Phase 5 作業 1。D-145・D-146）。dir はパッケージのディレクトリ、
// root は作業領域のルート（どちらも確かめ済み）。case はデバッグの標準入力にするケースの名前（`sample-N`・`random-N`。
// None なら input.txt）。breakpoints は始めるときのブレークポイント。ほかの処理と同時には動かさない（D-013）
#[allow(clippy::too_many_arguments)]
pub fn start_debug(
    dir: PathBuf,
    root: PathBuf,
    target: Target,
    case: Option<String>,
    breakpoints: Vec<crate::debug::SourceBreakpoints>,
    langs: LangSettings,
    // debugpy を入れる場所（アプリのフォルダの python/。D-226）
    python_tools: Option<PathBuf>,
    control: &RunControl,
    debug: &crate::debug::DebugControl,
    channel: Channel<RunEvent>,
) -> Result<Finish, String> {
    let cancel = control.begin()?;
    let guard = RunGuard(control);
    let finish = crate::debug::debug_target(
        &crate::debug::Request {
            dir: &dir,
            root: &root,
            target: &target,
            case: case.as_deref(),
            breakpoints: &breakpoints,
            langs: &langs,
        },
        &crate::debug::Environment::real(python_tools),
        debug,
        &cancel,
        &channel,
    );
    drop(guard);
    send(
        &channel,
        RunEvent::Finished {
            finish: finish.clone(),
        },
    );
    Ok(finish)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_diagnostics_and_executable() {
        let stdout = [
            r#"{"reason":"compiler-message","message":{"level":"error","message":"mismatched types","rendered":"error[E0308]: mismatched types\n","spans":[{"file_name":"src/main.rs","line_start":3,"is_primary":true}]}}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"a","kind":["bin"]},"executable":"/x/target/release/a"}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"dep","kind":["lib"]},"executable":null}"#,
            "not json",
            r#"{"reason":"build-finished","success":false}"#,
        ]
        .join("\n");
        let (diagnostics, executable) = parse_build_output(&stdout, "a");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].level, "error");
        assert_eq!(diagnostics[0].file.as_deref(), Some("src/main.rs"));
        assert_eq!(diagnostics[0].line, Some(3));
        assert_eq!(executable, Some(PathBuf::from("/x/target/release/a")));
        // ほかのバイナリの実行ファイルは使わない
        assert_eq!(parse_build_output(&stdout, "b").1, None);
    }

    #[test]
    fn finds_example_and_bin_executables_by_kind() {
        let stdout = [
            r#"{"reason":"compiler-artifact","target":{"name":"a","kind":["bin"]},"executable":"/x/release/a"}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"a_gen","kind":["example"]},"executable":"/x/release/examples/a_gen"}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"helper","kind":["example"]},"executable":null}"#,
            "not json",
        ]
        .join("\n");
        assert_eq!(
            find_executable(&stdout, "a_gen", "example"),
            Some(PathBuf::from("/x/release/examples/a_gen"))
        );
        assert_eq!(
            find_executable(&stdout, "a", "bin"),
            Some(PathBuf::from("/x/release/a"))
        );
        // 種類が違う、実行ファイルがない、ない名前は None
        assert_eq!(find_executable(&stdout, "a_gen", "bin"), None);
        assert_eq!(find_executable(&stdout, "helper", "example"), None);
        assert_eq!(find_executable(&stdout, "zzz", "example"), None);
    }

    #[test]
    fn carries_split_utf8_characters() {
        let mut carry = Utf8Carry::default();
        let bytes = "あい".as_bytes();
        assert_eq!(carry.decode(&bytes[..2]), "");
        assert_eq!(carry.decode(&bytes[2..4]), "あ");
        assert_eq!(carry.decode(&bytes[4..]), "い");
        // 正しくない並びは置き換え文字にする
        assert_eq!(carry.decode(&[0xff, b'a']), "\u{fffd}a");
    }

    #[test]
    fn releases_run_even_if_dropped_early() {
        let control = RunControl::default();
        {
            let _cancel = control.begin().unwrap();
            let _guard = RunGuard(&control);
        }
        assert!(!control.running());
        control.stop_and_wait();
    }

    #[test]
    fn lists_test_cases_samples_first_then_random_in_number_order() {
        let dir = std::env::temp_dir().join(format!("atcoder-studio-cases-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            "sample-10.in",
            "sample-2.in",
            "sample-1.in",
            "sample-1.out",
            "sample-x.in",
            "input.txt",
            "random-3.in",
            "random-12.in",
            "random-2.in",
            "random-2.out",
            "random-x.in",
            "random-.in",
            "randomized-1.in",
            "other-1.in",
        ] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        // 名前が `.in` で終わるフォルダは、ケースにしない
        std::fs::create_dir(dir.join("random-9.in")).unwrap();
        let keys: Vec<CaseKey> = test_cases(&dir).into_iter().map(|(key, _)| key).collect();
        assert_eq!(keys, vec![(0, 1), (0, 2), (0, 10), (1, 2), (1, 3), (1, 12)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decides_verdicts_in_order() {
        let settings = TestSettings {
            mode: Mode::Whitespace,
            time_limit_ms: 2500,
            memory_limit_mib: 10,
            random_count: 100,
        };
        let result = |outcome: Outcome, stdout: &str, rss_mib: Option<u64>| process::RunResult {
            outcome,
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
            elapsed: Duration::from_millis(1),
            output_left_open: false,
            max_rss_bytes: rss_mib.map(|mib| mib * 1024 * 1024),
        };
        let verdict = |result: process::RunResult, expected: Option<&str>| {
            verdict_of(&result, expected.map(str::as_bytes), &settings)
        };
        let ok = Outcome::Exited(0);
        assert!(matches!(
            verdict(result(ok, "1\n", Some(5)), Some("1\n")),
            Verdict::Accepted
        ));
        assert!(matches!(
            verdict(result(ok, "2\n", Some(5)), Some("1\n")),
            Verdict::WrongAnswer
        ));
        assert!(matches!(
            verdict(result(ok, "2\n", None), None),
            Verdict::Unjudged
        ));
        // 最大使用メモリが制限ちょうどなら超えていない。超えたら、出力が合っていても MLE
        assert!(matches!(
            verdict(result(ok, "1\n", Some(10)), Some("1\n")),
            Verdict::Accepted
        ));
        let over = process::RunResult {
            max_rss_bytes: Some(10 * 1024 * 1024 + 1),
            ..result(ok, "1\n", None)
        };
        assert!(matches!(verdict(over, Some("1\n")), Verdict::MemoryLimit));
        // 異常な終わり方でも、メモリを超えていれば MLE。超えていなければ RE
        assert!(matches!(
            verdict(result(Outcome::Exited(101), "", Some(11)), Some("1\n")),
            Verdict::MemoryLimit
        ));
        assert!(matches!(
            verdict(result(Outcome::Signaled(11), "", Some(5)), Some("1\n")),
            Verdict::RuntimeError { .. }
        ));
        // 時間の制限・出力の上限・止めたものは、メモリより先
        assert!(matches!(
            verdict(result(Outcome::TimedOut, "", Some(11)), None),
            Verdict::TimedOut
        ));
        assert!(matches!(
            verdict(
                result(Outcome::OutputLimit(process::Stream::Stdout), "", Some(11)),
                None
            ),
            Verdict::OutputLimit
        ));
        assert!(matches!(
            verdict(result(Outcome::Stopped, "", Some(11)), None),
            Verdict::Interrupted
        ));
        // メモリを受け取れなかったときは、MLE にしない
        assert!(matches!(
            verdict(result(ok, "1\n", None), Some("1\n")),
            Verdict::Accepted
        ));
    }

    #[cfg(unix)]
    #[test]
    fn stops_a_case_at_the_time_limit() {
        // 時間の制限で止めて TLE にする（D-141）。sh で 2 秒待つスクリプトを、制限 200 ms で動かす
        let dir = std::env::temp_dir().join(format!("atcoder-studio-tle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("slow.sh");
        std::fs::write(&script, "#!/bin/sh\nsleep 2\necho 1\n").unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        std::fs::write(dir.join("sample-1.in"), "").unwrap();
        std::fs::write(dir.join("sample-1.out"), "1\n").unwrap();
        let settings = TestSettings {
            time_limit_ms: 200,
            ..TestSettings::default()
        };
        let started = Instant::now();
        let case = run_case(
            &dir,
            &Program::native(script.clone()),
            &dir.join("sample-1.in"),
            &settings,
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(matches!(case.verdict, Verdict::TimedOut));
        assert!(started.elapsed() < Duration::from_millis(1500));
        // 使用メモリは受け取れる
        assert!(case.memory_kib.is_some_and(|kib| kib > 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decides_whether_an_output_can_be_saved_as_expected() {
        // 正常に終わり、全体を得られ、UTF-8 で、画面に出せる大きさ
        assert_eq!(save_block_reason(true, "あ\n".as_bytes()), None);
        assert_eq!(save_block_reason(true, b""), None);
        assert_eq!(save_block_reason(true, &vec![b'a'; DISPLAY_LIMIT]), None);
        // 正常に終わらなかった、出力の全体を得られなかった
        assert!(
            save_block_reason(false, b"1\n")
                .unwrap()
                .contains("最後まで")
        );
        // 画面に出す量を超える（切り詰められる）
        let reason = save_block_reason(true, &vec![b'a'; DISPLAY_LIMIT + 1]).unwrap();
        assert!(reason.contains("256 KiB"));
        // UTF-8 でない（置き換えられる）
        assert!(
            save_block_reason(true, &[0xff, 0xfe])
                .unwrap()
                .contains("UTF-8")
        );
        // 保存できるときは、画面に出す文字列が実際の出力そのもの
        let exact = "1 2\nあ\n";
        assert_eq!(shown(exact.as_bytes()), exact);
    }

    #[test]
    fn selects_cases_by_name() {
        let all: Vec<(CaseKey, PathBuf)> = [(0, 1), (0, 2), (0, 10), (1, 1), (1, 7)]
            .iter()
            .map(|&(kind, n)| {
                let prefix = if kind == 0 { "sample" } else { "random" };
                ((kind, n), PathBuf::from(format!("/t/{prefix}-{n}.in")))
            })
            .collect();
        let numbers = |cases: Vec<(CaseKey, PathBuf)>| {
            cases.into_iter().map(|(key, _)| key).collect::<Vec<_>>()
        };
        // 指定なしは全部
        assert_eq!(
            numbers(select_cases(all.clone(), None).unwrap()),
            vec![(0, 1), (0, 2), (0, 10), (1, 1), (1, 7)]
        );
        // 指定した名前だけ。順番は番号の順のまま
        let wanted = vec!["sample-10".to_string(), "sample-1".to_string()];
        assert_eq!(
            numbers(select_cases(all.clone(), Some(&wanted)).unwrap()),
            vec![(0, 1), (0, 10)]
        );
        // random-N も名前で選べる。sample-1 と random-1 は別のケース
        let wanted = vec!["random-1".to_string(), "sample-2".to_string()];
        assert_eq!(
            numbers(select_cases(all.clone(), Some(&wanted)).unwrap()),
            vec![(0, 2), (1, 1)]
        );
        // ない名前、パスのような名前、空は実行しない
        for wanted in [
            vec!["sample-3".to_string()],
            vec!["sample-1".to_string(), "sample-9".to_string()],
            vec!["../sample-1".to_string()],
            vec!["/t/sample-1".to_string()],
            vec![],
        ] {
            assert!(
                select_cases(all.clone(), Some(&wanted)).is_err(),
                "{wanted:?}"
            );
        }
        let error = select_cases(all, Some(&["sample-3".to_string()])).unwrap_err();
        assert!(error.contains("sample-3"));
    }

    #[test]
    fn judges_cases_with_a_real_program() {
        // cat は入力をそのまま出力する
        let dir = std::env::temp_dir().join(format!("atcoder-studio-judge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sample-1.in"), "1 2\n").unwrap();
        std::fs::write(dir.join("sample-1.out"), "1  2\n").unwrap();
        std::fs::write(dir.join("sample-2.in"), "3\n").unwrap();
        std::fs::write(dir.join("sample-2.out"), "4\n").unwrap();
        std::fs::write(dir.join("sample-3.in"), "5\n").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let verdict = |n: u32| {
            run_case(
                &dir,
                &Program::native(PathBuf::from("/bin/cat")),
                &dir.join(format!("sample-{n}.in")),
                &TestSettings::default(),
                &cancel,
            )
            .unwrap()
            .verdict
        };
        let case3 = run_case(
            &dir,
            &Program::native(PathBuf::from("/bin/cat")),
            &dir.join("sample-3.in"),
            &TestSettings::default(),
            &cancel,
        )
        .unwrap();
        // .out がなくても、置く場所は相対パスで示す。cat の出力は保存できる
        assert_eq!(case3.out_file, "sample-3.out");
        assert_eq!(case3.save_blocked, None);
        assert_eq!(case3.actual, "5\n");
        assert!(matches!(verdict(1), Verdict::Accepted));
        assert!(matches!(verdict(2), Verdict::WrongAnswer));
        // .out がなければ比較不可・目視確認
        assert!(matches!(verdict(3), Verdict::Unjudged));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 画面（TypeScript）は、各値の中の名前も camelCase で受け取る（elapsedMs など）
    #[test]
    fn sends_field_names_in_camel_case() {
        let finish = Finish::Ran {
            outcome: RunOutcome::Exited { code: 0 },
            elapsed_ms: 12,
            output_left_open: false,
        };
        let json = serde_json::to_value(&finish).unwrap();
        assert_eq!(json["kind"], "ran");
        assert_eq!(json["elapsedMs"], 12);
        assert_eq!(json["outputLeftOpen"], false);
    }

    #[test]
    fn allows_only_one_run_at_a_time() {
        let control = RunControl::default();
        let cancel = control.begin().unwrap();
        assert!(control.begin().is_err());
        control.stop();
        assert!(cancel.load(Ordering::SeqCst));
        control.end();
        assert!(control.begin().is_ok());
    }
}
