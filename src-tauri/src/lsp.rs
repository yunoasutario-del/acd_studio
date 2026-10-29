// 言語サーバー（rust-analyzer・clangd・basedpyright）の起動・停止と、LSP のメッセージの中継（Phase 3b-1 作業 2。D-123・D-124）
//
// - C++ のパッケージでは clangd を動かす（Phase 17 作業 5。D-210）。rust-analyzer と合わせて 1 つだけ動かし、
//   言語が変われば止めてから起動する。clangd へのコンパイルの命令は initializationOptions で渡す（cpp.rs）
// - Python のパッケージでは basedpyright を動かす（Phase 20 作業 5。D-225）。処理系と型の検査の強さは
//   workspace/configuration の答えで渡す
// - 開いている .rs の最寄りの Cargo.toml のディレクトリで、rust-analyzer を子プロセスとして起動する（D-124）。
//   動かすのは IDE 全体で 1 つだけ。別のパッケージで起動するときは、前のものを止めてから起動する
// - 標準入出力で、Content-Length の見出しを付けた LSP のメッセージをやり取りする。見出しの付け外しはここで行う
// - 画面からサーバーへは lsp_send の 1 つのコマンドで、サーバーから画面へはチャンネルで送る（09 K-07）
// - initialize の rootUri・workspaceFolders・initializationOptions は、ここで決めて上書きする。
//   能力のうち pull 診断（textDocument.diagnostic）は取り除く（D-129。画面のライブラリは宣言するのに pull しないため）。
//   サーバーからの workspace/configuration の問い合わせにも、ここで答える（画面には送らない）
// - 画面から送るメッセージは許可リストで検査する（D-131）。通すのは画面のライブラリが使う方法と、
//   サーバーの要求への応答だけ。textDocument.uri は作業領域の中のファイルだけ。本文の大きさは MAX_FILE_SIZE まで
// - 起動の環境は Phase 2 と同じ（D-104 の変数を外し、RUSTUP_AUTO_INSTALL=0。D-107）
// - 新しいプロセスグループで起動し、止めるときは LSP の正式な終了（shutdown → exit）を送って待ち、
//   残っていればグループ全体に SIGKILL を送ってから回収する（D-095 と同じ方法）。
//   rust-analyzer が起動する子（proc-macro-srv、cargo check とその子）も同じグループに入る（推測。AC3b-02 で確かめる）
// - rust-analyzer が自分で終わったときも、終わるのを待つスレッドが画面に知らせる（回収は次の停止・起動のとき）
// - 画面のチャンネルが変わったら（画面の再読み込みなど）、同じパッケージでも起動し直す。
//   新しい画面は initialize からやり直すため（Tauri のチャンネルの id は、画面のチャンネル 1 つごとに決まる）
//
// 限界：textDocument.uri 以外の場所（completionItem/resolve の data など）にある URI は検査しない。
// didChange の本文の上限は 1 つのメッセージごとで、少しずつ編集して大きくすることは止めない。標準入力への書き込みには時間の上限がない
// （rust-analyzer が読まずにパイプが満杯になると、書き込みのスレッドが待ち、積んだメッセージがたまる）。
// setsid などでグループから抜けた子孫は止められない（D-095）。macOS（Unix）だけを対象にする

use std::io::{BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

use crate::language::Language;
#[cfg(windows)]
use crate::process::ProcessGroupShim;
use crate::process::{Pid, exited_without_reaping, kill_group, wait_without_reaping};
use crate::toolchain::{self, find_cargo};
use crate::workspace::MAX_FILE_SIZE;

// 止めるときに、正式な終了を待つ時間の上限（アプリの終了時の待ち EXIT_WAIT に合わせる）
pub const STOP_WAIT: Duration = Duration::from_secs(3);
// 1 つのメッセージの大きさの上限。超えたら、壊れたサーバーとして止める
const MAX_MESSAGE: usize = 64 * 1024 * 1024;
// shutdown の要求に付ける id（画面の要求の id と重ならないように文字列にする）
const SHUTDOWN_ID: &str = "atcoder-studio-shutdown";
// 終わるのを確かめる間隔
const POLL_INTERVAL: Duration = Duration::from_millis(20);

// 画面に送る知らせ
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum LspEvent {
    // サーバーからのメッセージ（JSON の文字列。見出しは外してある）
    Message { text: String },
    // サーバーの標準エラー（開発者ツールの console に出す。D-126）
    Log { text: String },
    // サーバーが終わった（止めた場合も含む）。code は終了コード（シグナルなら None）
    Exited { code: Option<i32> },
}

// 起動したパッケージ（画面に返す）
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Started {
    // 作業領域のルートからの相対パス（区切りは /。ルート自身なら空）
    pub package: String,
    // パッケージのディレクトリの file: の URI。画面は文書の URI をこれに合わせて作る
    pub root_uri: String,
    // すでに同じパッケージで動いていた
    pub already_running: bool,
    // サーバーの名前（rust-analyzer・clangd）
    pub server: &'static str,
}

type Sink = Arc<dyn Fn(LspEvent) + Send + Sync>;

// ---- 見出しの付け外し ----

// メッセージに Content-Length の見出しを付ける
pub fn encode(body: &str) -> Vec<u8> {
    let mut bytes = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    bytes.extend_from_slice(body.as_bytes());
    bytes
}

// 塊ごとに届くバイト列から、見出しを外したメッセージを取り出す（DAP も同じ形なので、debug.rs でも使う）
#[derive(Default)]
pub struct Decoder {
    buffer: Vec<u8>,
}

impl Decoder {
    // bytes を加え、取り出せるメッセージをすべて返す。見出しが壊れていればエラー
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, String> {
        self.buffer.extend_from_slice(bytes);
        let mut messages = Vec::new();
        loop {
            let Some(end) = self.buffer.windows(4).position(|w| w == b"\r\n\r\n") else {
                if self.buffer.len() > 8 * 1024 {
                    return Err("メッセージの見出しが長すぎます".to_string());
                }
                break;
            };
            let header = std::str::from_utf8(&self.buffer[..end])
                .map_err(|_| "メッセージの見出しが UTF-8 ではありません".to_string())?;
            let mut length = None;
            for line in header.split("\r\n") {
                let Some((name, value)) = line.split_once(':') else {
                    return Err(format!("メッセージの見出しを読めません：{line}"));
                };
                // 見出しの名前は大文字・小文字を区別しない。Content-Type は使わない
                if name.trim().eq_ignore_ascii_case("content-length") {
                    length = Some(
                        value
                            .trim()
                            .parse::<usize>()
                            .map_err(|_| format!("Content-Length を読めません：{value}"))?,
                    );
                }
            }
            let length = length.ok_or_else(|| "Content-Length がありません".to_string())?;
            if length > MAX_MESSAGE {
                return Err(format!("メッセージが大きすぎます（{length} バイト）"));
            }
            let start = end + 4;
            if self.buffer.len() < start + length {
                break;
            }
            let body = String::from_utf8(self.buffer[start..start + length].to_vec())
                .map_err(|_| "メッセージが UTF-8 ではありません".to_string())?;
            self.buffer.drain(..start + length);
            messages.push(body);
        }
        Ok(messages)
    }
}

// ---- URI と設定 ----

// パスを file: の URI にする。英数字と -._~/ 以外はパーセントエンコードする
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(*byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

// rust-analyzer に渡す設定（initializationOptions と workspace/configuration の答え）。
// 決めるのは linkedProjects（projects のパッケージの Cargo.toml）だけで、ほかは既定値のまま（D-121・D-124・D-171）
fn settings(projects: &[PathBuf]) -> Value {
    let linked: Vec<String> = projects
        .iter()
        .map(|dir| dir.join("Cargo.toml").to_string_lossy().into_owned())
        .collect();
    json!({ "linkedProjects": linked })
}

// rust-analyzer を動かす場所と、扱うパッケージ（Phase 10 作業 11。D-171）。
// パッケージが「コンテストを作る」で作ったコンテストのフォルダ（IDE が書いた .cargo/config.toml がある）の中なら、
// そのフォルダで、中のパッケージ（直下のフォルダの Cargo.toml。作業領域の中だけ。CONTEST_PACKAGES_MAX まで）をまとめて扱う。
// 同じコンテストの問題を行き来しても、起動し直さない。そうでなければ、今までどおりパッケージだけ（D-124）
const CONTEST_PACKAGES_MAX: usize = 30;

pub fn analyzer_place(root: &Path, package: &Path) -> (PathBuf, Vec<PathBuf>) {
    let single = (package.to_path_buf(), vec![package.to_path_buf()]);
    let Some(contest) = package.parent() else {
        return single;
    };
    if !contest.starts_with(root) || !crate::scaffold::is_contest_dir(contest) {
        return single;
    }
    let Ok(entries) = std::fs::read_dir(contest) else {
        return single;
    };
    let mut projects: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|entry| entry.path().canonicalize().ok())
        .filter(|dir| dir.starts_with(root) && dir.parent() == Some(contest))
        .filter(|dir| dir.join("Cargo.toml").is_file())
        .collect();
    projects.sort();
    if projects.is_empty()
        || projects.len() > CONTEST_PACKAGES_MAX
        || !projects.iter().any(|dir| dir == package)
    {
        return single;
    }
    (contest.to_path_buf(), projects)
}

// 画面からの initialize の要求の、場所と設定（options。サーバーごとの initializationOptions）を上書きする（D-123）
fn override_initialize(message: &mut Value, dir: &Path, options: &Value) {
    let uri = file_uri(dir);
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !message.get("params").is_some_and(Value::is_object) {
        message["params"] = json!({});
    }
    let params = &mut message["params"];
    params["rootUri"] = json!(uri);
    params["rootPath"] = json!(dir.to_string_lossy());
    params["workspaceFolders"] = json!([{ "uri": uri, "name": name }]);
    params["initializationOptions"] = options.clone();
    if !params.get("capabilities").is_some_and(Value::is_object) {
        params["capabilities"] = json!({});
    }
    if !params["capabilities"]
        .get("textDocument")
        .is_some_and(Value::is_object)
    {
        params["capabilities"]["textDocument"] = json!({});
    }
    let text_document = &mut params["capabilities"]["textDocument"];
    // pull 診断の能力を取り除く。宣言したままだと、rust-analyzer 自身の診断（構文エラーなど）が push で来ない（D-129）
    if let Some(text_document) = text_document.as_object_mut() {
        text_document.remove("diagnostic");
    }
    // パンくずリストの記号は、入れ子の形（DocumentSymbol）で返してもらう（Phase 18 作業 2。D-216）
    text_document["documentSymbol"] = json!({ "hierarchicalDocumentSymbolSupport": true });
}

// ---- 送るメッセージの検査（D-131） ----

// 画面から送ってよい方法。名前の変更・シグネチャヘルプは 3b-2 の範囲に入れないので通さない（D-133）
const ALLOWED_METHODS: &[&str] = &[
    "initialize",
    "initialized",
    "shutdown",
    "exit",
    "textDocument/didOpen",
    "textDocument/didChange",
    "textDocument/didSave",
    "textDocument/didClose",
    "textDocument/completion",
    "textDocument/hover",
    "textDocument/definition",
    "textDocument/declaration",
    "textDocument/typeDefinition",
    "textDocument/implementation",
    "textDocument/references",
    "textDocument/formatting",
    "textDocument/inlayHint",
    // パンくずリストの記号（Phase 18 作業 2。D-216）
    "textDocument/documentSymbol",
    "completionItem/resolve",
    "$/cancelRequest",
];

// file: の URI をパスにする（file_uri の逆。%XX を戻す）。file: でない、ホスト名がある、などは None
pub(crate) fn path_of_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(decoded).ok().map(PathBuf::from)
}

// URI が作業領域（root。canonicalize 済み）の中のファイルか。あるファイルは実際のパス（リンクをたどった先）で、
// ない（消した後の didClose など）ファイルは、`.`・`..` を含まないことと、root の下にあることで確かめる
fn check_uri(uri: &str, root: &Path) -> Result<(), String> {
    let path = path_of_uri(uri).ok_or_else(|| format!("file: の URI ではありません：{uri}"))?;
    let inside = match path.canonicalize() {
        Ok(real) => real.starts_with(root),
        Err(_) => {
            path.components().all(|c| {
                matches!(
                    c,
                    std::path::Component::RootDir | std::path::Component::Normal(_)
                )
            }) && path.starts_with(root)
        }
    };
    if inside {
        Ok(())
    } else {
        Err(format!("作業領域の外のファイルです：{uri}"))
    }
}

// rust-analyzer が返した場所（file: の URI）が作業領域（root。canonicalize 済み）の中の通常のファイルなら、
// ルートからの相対パス（`/` 区切り）を返す。リンクはたどった先で確かめる。外、ない、フォルダ、
// file: でない、UTF-8 でないときは None（定義ジャンプ・参照検索でタブに開く場所を決めるため。Phase 3b-2 作業 4）
pub fn locate_in_workspace(root: &Path, uri: &str) -> Option<String> {
    let real = path_of_uri(uri)?.canonicalize().ok()?;
    if !real.is_file() {
        return None;
    }
    let parts = real
        .strip_prefix(root)
        .ok()?
        .components()
        .map(|component| component.as_os_str().to_str().map(str::to_string))
        .collect::<Option<Vec<_>>>()?;
    Some(parts.join("/"))
}

// 画面から送るメッセージを検査する。通さないものはエラー（rust-analyzer には送らない）
fn check_message(message: &Value, root: &Path) -> Result<(), String> {
    let Some(method) = message.get("method") else {
        // method のないものは、サーバーの要求への応答（id と result か error）だけを通す
        if message.get("id").is_some()
            && (message.get("result").is_some() || message.get("error").is_some())
        {
            return Ok(());
        }
        return Err("LSP のメッセージに method がありません".to_string());
    };
    let method = method
        .as_str()
        .ok_or_else(|| "LSP のメッセージの method が文字列ではありません".to_string())?;
    if !ALLOWED_METHODS.contains(&method) {
        return Err(format!("言語サーバーに送れない方法です：{method}"));
    }
    if method.starts_with("textDocument/") {
        let uri = message["params"]["textDocument"]["uri"]
            .as_str()
            .ok_or_else(|| format!("{method} に textDocument.uri がありません"))?;
        check_uri(uri, root)?;
    }
    let size: usize = match method {
        "textDocument/didOpen" => message["params"]["textDocument"]["text"]
            .as_str()
            .map_or(0, str::len),
        "textDocument/didChange" => {
            message["params"]["contentChanges"]
                .as_array()
                .map_or(0, |changes| {
                    changes
                        .iter()
                        .filter_map(|change| change["text"].as_str())
                        .map(str::len)
                        .sum()
                })
        }
        _ => 0,
    };
    if size as u64 > MAX_FILE_SIZE {
        return Err(format!(
            "{method} の本文が大きすぎます（{size} バイト。上限は {MAX_FILE_SIZE} バイト）"
        ));
    }
    Ok(())
}

// workspace/configuration の問い合わせへの答え。configuration は節の名前ごとの答え（例：rust-analyzer・python・
// basedpyright）で、節のないものには rust-analyzer の答えを、知らない節には null を返す。
// clangd は問い合わせない（configuration は null）
fn configuration_answer(request: &Value, configuration: &Value) -> Value {
    let items = request["params"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let result: Vec<Value> = items
        .iter()
        .map(|item| {
            let section = item["section"].as_str().unwrap_or("rust-analyzer");
            configuration.get(section).cloned().unwrap_or(Value::Null)
        })
        .collect();
    json!({ "jsonrpc": "2.0", "id": request["id"], "result": result })
}

// ---- サーバー ----

struct Server {
    // サーバーの名前（rust-analyzer・clangd。知らせの文に使う）
    name: &'static str,
    // initialize に入れる initializationOptions（同じものなら起動し直さない）
    options: Value,
    // workspace/configuration の答え（同じものなら起動し直さない。basedpyright の処理系など。D-225）
    configuration: Value,
    // 作業領域のルート（canonicalize 済み）。送るメッセージの URI を確かめる（D-131）
    root: PathBuf,
    dir: PathBuf,
    // 扱うパッケージ（D-171）
    projects: Vec<PathBuf>,
    // 起動を頼んだ画面のセッション（画面を読み込むたびに画面が作る文字列）
    session: String,
    child: Child,
    // 書き込みのスレッドに渡すメッセージ（見出しを付ける前）。積んだ順に書く
    outgoing: Sender<String>,
    // shutdown の答えが届いたら知らせる
    shutdown_answered: Receiver<()>,
    // 終わったことを画面に知らせる役が決まったら true。終わるのを待つスレッドか stop の、先に取った一方だけが知らせる
    reported: Arc<AtomicBool>,
    sink: Sink,
}

// 標準入力に書くスレッド。lsp_send・workspace/configuration の答え・停止の shutdown と exit を、積んだ順に書く。
// 書き込みに時間の上限はない（rust-analyzer が読まずにパイプが満杯になると、このスレッドが待つ。送る側は待たない）
fn spawn_writer(name: &'static str, mut stdin: ChildStdin, outgoing: Receiver<String>, sink: Sink) {
    thread::spawn(move || {
        for body in outgoing {
            if let Err(error) = stdin.write_all(&encode(&body)).and_then(|_| stdin.flush()) {
                sink(LspEvent::Log {
                    text: format!("{name} に送れません：{error}"),
                });
                break;
            }
        }
    });
}

fn queue(outgoing: &Sender<String>, body: String) -> Result<(), String> {
    outgoing
        .send(body)
        .map_err(|_| "言語サーバーに送れません（送信が止まっています）".to_string())
}

// 標準出力を読み、メッセージを画面に送る。workspace/configuration にはここで答え、shutdown の答えは画面に送らない
fn spawn_stdout_reader(
    stdout: impl Read + Send + 'static,
    configuration: Value,
    outgoing: Sender<String>,
    shutdown: Sender<()>,
    sink: Sink,
) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut decoder = Decoder::default();
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let n = match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let messages = match decoder.push(&buffer[..n]) {
                Ok(messages) => messages,
                Err(error) => {
                    // 壊れたサーバー。読み取りをやめる（止めるのは lsp_stop か、次の起動のとき）
                    sink(LspEvent::Log { text: error });
                    break;
                }
            };
            for text in messages {
                let parsed: Option<Value> = serde_json::from_str(&text).ok();
                if let Some(message) = &parsed {
                    if message["method"] == "workspace/configuration" && message.get("id").is_some()
                    {
                        let answer = configuration_answer(message, &configuration).to_string();
                        if let Err(error) = queue(&outgoing, answer) {
                            sink(LspEvent::Log { text: error });
                        }
                        continue;
                    }
                    if message["id"] == SHUTDOWN_ID && message.get("method").is_none() {
                        let _ = shutdown.send(());
                        continue;
                    }
                }
                sink(LspEvent::Message { text });
            }
        }
    });
}

// 標準エラーを行ごとに画面へ送る（開発者ツールの console に出す。D-126）
fn spawn_stderr_reader(stderr: impl Read + Send + 'static, sink: Sink) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => sink(LspEvent::Log {
                    text: String::from_utf8_lossy(&buffer[..n]).into_owned(),
                }),
            }
        }
    });
}

impl Server {
    // launch の命令を、その場所で、新しいプロセスグループで起動する
    fn spawn(launch: Launch, root: PathBuf, session: String, sink: Sink) -> Result<Server, String> {
        let Launch {
            name,
            mut command,
            dir,
            projects,
            options,
            configuration,
        } = launch;
        command
            .current_dir(&dir)
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| format!("{name} を起動できません：{error}"))?;
        let (outgoing, to_write) = mpsc::channel();
        spawn_writer(
            name,
            child.stdin.take().expect("piped"),
            to_write,
            sink.clone(),
        );
        let (shutdown, shutdown_answered) = mpsc::channel();
        spawn_stdout_reader(
            child.stdout.take().expect("piped"),
            configuration.clone(),
            outgoing.clone(),
            shutdown,
            sink.clone(),
        );
        spawn_stderr_reader(child.stderr.take().expect("piped"), sink.clone());
        // 自分で終わったときに知らせる。stop が先に役を取っていれば何もしない
        let reported = Arc::new(AtomicBool::new(false));
        let pid = child.id() as Pid;
        let (flag, notify) = (reported.clone(), sink.clone());
        thread::spawn(move || {
            let code = wait_without_reaping(pid);
            if !flag.swap(true, Ordering::SeqCst) {
                notify(LspEvent::Log {
                    text: format!("{name} が終わりました"),
                });
                notify(LspEvent::Exited { code });
            }
        });
        Ok(Server {
            name,
            options,
            configuration,
            root,
            dir,
            projects,
            session,
            child,
            outgoing,
            shutdown_answered,
            reported,
            sink,
        })
    }

    // メッセージを検査して積む。書き込みは待たない（書くのは書き込みのスレッド）。
    // check が false なら許可リストの検査をしない（偽のサーバーを使う単体テストだけ）
    fn send(&self, text: &str, check: bool) -> Result<(), String> {
        let mut message: Value = serde_json::from_str(text)
            .map_err(|_| "LSP のメッセージが JSON ではありません".to_string())?;
        if !message.is_object() {
            return Err("LSP のメッセージがオブジェクトではありません".to_string());
        }
        if check {
            check_message(&message, &self.root)?;
        }
        if message["method"] == "initialize" {
            override_initialize(&mut message, &self.dir, &self.options);
        }
        queue(&self.outgoing, message.to_string())
    }

    fn exited(&self) -> bool {
        exited_without_reaping(self.child.id() as Pid)
    }

    // 正式な終了を送って wait まで待ち、残っていればグループごと止めて回収する
    fn stop(mut self, wait: Duration) {
        // 先に知らせる役を取る（終わるのを待つスレッドが「終わりました」と知らせないように）。
        // すでに自分で終わって知らせてあれば、ここでは知らせない
        let report = !self.reported.swap(true, Ordering::SeqCst);
        let until = Instant::now() + wait;
        if !self.exited() {
            let shutdown = json!({ "jsonrpc": "2.0", "id": SHUTDOWN_ID, "method": "shutdown" });
            if queue(&self.outgoing, shutdown.to_string()).is_ok() {
                let left = until.saturating_duration_since(Instant::now());
                if self.shutdown_answered.recv_timeout(left).is_ok() {
                    let exit = json!({ "jsonrpc": "2.0", "method": "exit" });
                    let _ = queue(&self.outgoing, exit.to_string());
                }
            }
            while !self.exited() && Instant::now() < until {
                thread::sleep(POLL_INTERVAL);
            }
        }
        // 回収する前に、グループの残り（rust-analyzer の子など）を止める
        kill_group(self.child.id() as Pid);
        let code = self.child.wait().ok().and_then(|status| status.code());
        if report {
            (self.sink)(LspEvent::Exited { code });
        }
    }
}

// IDE 全体で 1 つだけ動かす言語サーバー（rust-analyzer・clangd・basedpyright のどれか）。
// server のロックは短い間だけ持つ（止める間は持たない）。lsp_send は同期のコマンドで、メインスレッドで動くため
#[derive(Default)]
pub struct LspControl {
    server: Mutex<Option<Server>>,
    // 起動・起動し直しを 1 つずつ行うためのロック
    starting: Mutex<()>,
}

// 言語サーバーの起動のしかた
pub struct Launch {
    name: &'static str,
    command: Command,
    // 動かす場所（パッケージか、コンテストのフォルダ）
    dir: PathBuf,
    // 扱うパッケージ（D-171・D-210）
    projects: Vec<PathBuf>,
    // initialize の initializationOptions
    options: Value,
    // workspace/configuration の答え
    configuration: Value,
}

impl Launch {
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn rust_analyzer_with(command: Command, dir: PathBuf, projects: Vec<PathBuf>) -> Launch {
        let options = settings(&projects);
        Launch {
            name: "rust-analyzer",
            command,
            dir,
            projects,
            configuration: json!({ "rust-analyzer": options.clone() }),
            options,
        }
    }
}

// rust-analyzer（rustup の代理）を探す。cargo と同じ場所を使う（D-098 と同じ探し方）
fn find_rust_analyzer() -> Result<PathBuf, String> {
    let cargo = find_cargo()
        .ok_or_else(|| "cargo が見つからないので、rust-analyzer を探せません".to_string())?;
    let path = cargo.with_file_name("rust-analyzer");
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "rust-analyzer が見つかりません（{} がありません）",
            path.display()
        ))
    }
}

// rust-analyzer を dir で、projects のパッケージを扱うように動かす（analyzer_place で決めたもの。D-171）
pub fn rust_analyzer(dir: PathBuf, projects: Vec<PathBuf>) -> Result<Launch, String> {
    let program = find_rust_analyzer()?;
    // cargo と同じく、D-104 の変数を外し、RUSTUP_AUTO_INSTALL=0 を付ける（D-107）
    let command = toolchain::cargo_command(&program, &dir);
    Ok(Launch::rust_analyzer_with(command, dir, projects))
}

// clangd（Command Line Tools のもの）を探す（D-210）。見つかった場所は覚えておく
fn find_clangd() -> Result<PathBuf, String> {
    static FOUND: OnceLock<PathBuf> = OnceLock::new();
    if let Some(path) = FOUND.get() {
        return Ok(path.clone());
    }
    let missing = "C++ の言語機能には、Xcode の Command Line Tools の clangd が要ります。ターミナルで `xcode-select --install` を実行して入れてください";
    let output = Command::new("/usr/bin/xcrun")
        .args(["--find", "clangd"])
        .output()
        .map_err(|_| missing.to_string())?;
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if !output.status.success() || !path.is_absolute() || !path.is_file() {
        return Err(missing.to_string());
    }
    Ok(FOUND.get_or_init(|| path).clone())
}

// --query-driver に渡す形にする（glob の記号の前に \ を置く）。`,` は区切りなので、含むパスは使えない
fn query_driver(compiler: &Path) -> Result<String, String> {
    let text = compiler.to_string_lossy();
    if text.contains(',') {
        return Err(format!(
            "コンパイラのパスに「,」があるので、clangd に渡せません：{text}"
        ));
    }
    let mut escaped = String::new();
    for c in text.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    Ok(escaped)
}

// clangd の引数（D-210）。標準ヘッダーの場所は compiler だけに聞き、裏での索引（パッケージに .cache/ を作る）と、
// .clangd・利用者の設定ファイルは使わない（IDE が渡すコンパイルの命令を変えられないように）
fn clangd_args(compiler: &Path) -> Result<Vec<String>, String> {
    Ok(vec![
        format!("--query-driver={}", query_driver(compiler)?),
        "--background-index=false".to_string(),
        "--enable-config=false".to_string(),
        "--log=error".to_string(),
    ])
}

// clangd を dir で、projects の C++ のパッケージを扱うように動かす。options は cpp::compilation_database のもの
pub fn clangd(
    dir: PathBuf,
    projects: Vec<PathBuf>,
    compiler: &Path,
    options: Value,
) -> Result<Launch, String> {
    let program = find_clangd()?;
    let mut command = Command::new(program);
    command.args(clangd_args(compiler)?);
    toolchain::clear_ide_env(&mut command);
    Ok(Launch {
        name: "clangd",
        command,
        dir,
        projects,
        options,
        configuration: Value::Null,
    })
}

// basedpyright（`brew install basedpyright`）を探す（D-225）。IDE は入れない。
// 探す順は、Homebrew（/opt/homebrew/bin）、PATH、/usr/local/bin（Python の処理系と同じ）
pub const BASEDPYRIGHT_MISSING: &str = "Python の言語機能には basedpyright が要ります。ターミナルで `brew install basedpyright` を実行して入れてください";

fn find_basedpyright() -> Result<PathBuf, String> {
    let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    std::iter::once(PathBuf::from("/opt/homebrew/bin"))
        .chain(path_dirs)
        .chain(std::iter::once(PathBuf::from("/usr/local/bin")))
        .map(|dir| dir.join("basedpyright-langserver"))
        .find(|path| path.is_absolute() && path.is_file())
        .ok_or_else(|| BASEDPYRIGHT_MISSING.to_string())
}

// basedpyright に渡す設定（workspace/configuration の答え。D-225）。処理系（見つからなければ渡さない）、
// 型の検査は standard、調べるのは開いているファイルだけ
fn basedpyright_configuration(interpreter: Option<&Path>) -> Value {
    let python = match interpreter {
        Some(path) => json!({ "pythonPath": path.to_string_lossy() }),
        None => json!({}),
    };
    json!({
        "python": python,
        "basedpyright": {
            "analysis": { "typeCheckingMode": "standard", "diagnosticMode": "openFilesOnly" }
        },
    })
}

// basedpyright を dir で、projects の Python のパッケージを扱うように動かす。interpreter は設定の処理系
pub fn basedpyright(
    dir: PathBuf,
    projects: Vec<PathBuf>,
    interpreter: Option<&Path>,
) -> Result<Launch, String> {
    let program = find_basedpyright()?;
    let mut command = Command::new(program);
    command.arg("--stdio");
    toolchain::clear_ide_env(&mut command);
    Ok(Launch {
        name: "basedpyright",
        command,
        dir,
        projects,
        options: Value::Null,
        configuration: basedpyright_configuration(interpreter),
    })
}

// basedpyright の中の typeshed（標準ライブラリなどの型の情報。定義ジャンプの行き先。D-225）。
// `basedpyright-langserver` のリンクをたどった先（npm のパッケージの `langserver.index.js`）の隣の
// `dist/typeshed-fallback`。なければ None
pub fn basedpyright_typeshed() -> Option<PathBuf> {
    let program = find_basedpyright().ok()?.canonicalize().ok()?;
    program
        .parent()?
        .join("dist/typeshed-fallback")
        .canonicalize()
        .ok()
}

// clangd を動かす場所と、扱う C++ のパッケージ（D-210）。パッケージの親のフォルダが作業領域の中で、
// ルートそのものでなければ（`<コンテスト>/<問題>/` の形）、そのフォルダで、直下の C++ のパッケージ
// （作業領域の中だけ。CONTEST_PACKAGES_MAX まで）をまとめて扱う。そうでなければ、パッケージだけ
pub fn clangd_place(root: &Path, package: &Path) -> (PathBuf, Vec<PathBuf>) {
    contest_place(root, package, Language::Cpp)
}

// basedpyright を動かす場所と、扱う Python のパッケージ（D-225）。決め方は clangd と同じ（同じコンテストの
// 問題を行き来しても、起動し直さない）
pub fn basedpyright_place(root: &Path, package: &Path) -> (PathBuf, Vec<PathBuf>) {
    contest_place(root, package, Language::Python)
}

// コンテストのフォルダでまとめて扱うときの、場所とパッケージ（ほかの言語のしるしもあるフォルダは数えない）
fn contest_place(root: &Path, package: &Path, language: Language) -> (PathBuf, Vec<PathBuf>) {
    let single = (package.to_path_buf(), vec![package.to_path_buf()]);
    let Some(contest) = package.parent() else {
        return single;
    };
    if contest == root || !contest.starts_with(root) {
        return single;
    }
    let Ok(entries) = std::fs::read_dir(contest) else {
        return single;
    };
    let mut projects: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|entry| entry.path().canonicalize().ok())
        .filter(|dir| dir.starts_with(root) && dir.parent() == Some(contest))
        .filter(|dir| crate::toolchain::package_language(dir) == Some(language))
        .collect();
    projects.sort();
    if projects.is_empty()
        || projects.len() > CONTEST_PACKAGES_MAX
        || !projects.iter().any(|dir| dir == package)
    {
        return single;
    }
    (contest.to_path_buf(), projects)
}

impl LspControl {
    // launch のとおりに起動する。同じサーバー・場所・パッケージの組・設定・画面のセッション（session）で
    // 動いていれば何もしない（true）。違えば、止めてから起動する。root は作業領域のルート（送るメッセージの URI を確かめる。D-131）
    pub fn start(
        &self,
        root: PathBuf,
        launch: Launch,
        session: String,
        sink: Sink,
    ) -> Result<bool, String> {
        let _starting = self
            .starting
            .lock()
            .map_err(|_| "言語サーバーの状態を読めません".to_string())?;
        let old = {
            let mut current = self
                .server
                .lock()
                .map_err(|_| "言語サーバーの状態を読めません".to_string())?;
            if let Some(server) = current.as_ref()
                && server.name == launch.name
                && server.dir == launch.dir
                && server.projects == launch.projects
                && server.options == launch.options
                && server.configuration == launch.configuration
                && server.session == session
                && !server.exited()
            {
                return Ok(true);
            }
            current.take()
        };
        // 止める間（最大 STOP_WAIT）は server のロックを持たない。その間の lsp_send は「起動していません」になる
        if let Some(server) = old {
            server.stop(STOP_WAIT);
        }
        let server = Server::spawn(launch, root, session, sink)?;
        *self
            .server
            .lock()
            .map_err(|_| "言語サーバーの状態を書けません".to_string())? = Some(server);
        Ok(false)
    }

    #[cfg(all(test, unix))]
    fn start_with(
        &self,
        command: Command,
        root: PathBuf,
        dir: PathBuf,
        session: String,
        sink: Sink,
    ) -> Result<bool, String> {
        let projects = vec![dir.clone()];
        self.start_with_projects(command, root, dir, projects, session, sink)
    }

    // パッケージの組が変わったとき（コンテストのフォルダに問題を足したなど）も、起動し直す（D-171）
    #[cfg(all(test, unix))]
    fn start_with_projects(
        &self,
        command: Command,
        root: PathBuf,
        dir: PathBuf,
        projects: Vec<PathBuf>,
        session: String,
        sink: Sink,
    ) -> Result<bool, String> {
        let launch = Launch::rust_analyzer_with(command, dir, projects);
        self.start(root, launch, session, sink)
    }

    pub fn send(&self, text: &str) -> Result<(), String> {
        self.send_checked(text, true)
    }

    // 偽のサーバーの単体テスト用：許可リストにない方法（hello・quit など）で偽のサーバーを動かす
    #[cfg(all(test, unix))]
    fn send_unchecked(&self, text: &str) -> Result<(), String> {
        self.send_checked(text, false)
    }

    fn send_checked(&self, text: &str, check: bool) -> Result<(), String> {
        let current = self
            .server
            .lock()
            .map_err(|_| "言語サーバーの状態を読めません".to_string())?;
        current
            .as_ref()
            .ok_or_else(|| "言語サーバーが起動していません".to_string())?
            .send(text, check)
    }

    // 動いていれば止める（作業領域を閉じる・アプリを終える・開発者ツールからの停止。D-124・D-126）
    // 起動し直しの途中なら、それが終わるのを待ってから止める（起動したばかりのものが残らないように）
    pub fn stop(&self) {
        let _starting = self.starting.lock();
        let server = self
            .server
            .lock()
            .ok()
            .and_then(|mut current| current.take());
        if let Some(server) = server {
            server.stop(STOP_WAIT);
        }
    }
}

// 起動の結果を、画面に返す形にする
pub fn started(root: &Path, dir: &Path, already_running: bool, server: &'static str) -> Started {
    let package = dir
        .strip_prefix(root)
        .map(|relative| {
            relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default();
    Started {
        package,
        root_uri: file_uri(dir),
        already_running,
        server,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_split_messages() {
        let mut bytes = encode(r#"{"a":"あ"}"#);
        bytes.extend(encode("{}"));
        let mut decoder = Decoder::default();
        let mut got = Vec::new();
        // 1 バイトずつ渡しても取り出せる（見出し・本文・UTF-8 の文字の途中で切れても）
        for byte in &bytes {
            got.extend(decoder.push(&[*byte]).unwrap());
        }
        assert_eq!(got, vec![r#"{"a":"あ"}"#.to_string(), "{}".to_string()]);
        // Content-Length はバイト数
        assert!(encode("あ").starts_with(b"Content-Length: 3\r\n\r\n"));
    }

    #[test]
    fn reads_headers_case_insensitively_and_rejects_broken_ones() {
        let mut decoder = Decoder::default();
        let got = decoder
            .push(b"content-length: 2\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{}")
            .unwrap();
        assert_eq!(got, vec!["{}".to_string()]);
        assert!(
            Decoder::default()
                .push(b"Content-Length: x\r\n\r\n")
                .is_err()
        );
        assert!(Decoder::default().push(b"Foo: 1\r\n\r\n").is_err());
        assert!(
            Decoder::default()
                .push(b"Content-Length: 99999999999\r\n\r\n")
                .is_err()
        );
    }

    #[test]
    fn makes_file_uris() {
        assert_eq!(
            file_uri(Path::new("/a/b c/あ")),
            "file:///a/b%20c/%E3%81%82"
        );
    }

    #[test]
    fn overrides_initialize_and_answers_configuration() {
        let dir = Path::new("/ws/problems/abc001_a");
        let mut message = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "rootUri": "file:///elsewhere", "initializationOptions": { "cargo": { "buildScripts": { "enable": false } } }, "capabilities": {} }
        });
        override_initialize(&mut message, dir, &settings(&[dir.to_path_buf()]));
        let params = &message["params"];
        assert_eq!(params["rootUri"], "file:///ws/problems/abc001_a");
        assert_eq!(params["workspaceFolders"][0]["name"], "abc001_a");
        // 画面から渡した設定は捨てる
        assert_eq!(
            params["initializationOptions"],
            json!({ "linkedProjects": ["/ws/problems/abc001_a/Cargo.toml"] })
        );
        // 入れ子の記号の能力だけを足す
        assert_eq!(
            params["capabilities"],
            json!({ "textDocument": { "documentSymbol": { "hierarchicalDocumentSymbolSupport": true } } })
        );
        // 能力がない・形が違うときも足す
        for params in [json!({}), json!({ "capabilities": { "textDocument": 1 } })] {
            let mut message =
                json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": params });
            override_initialize(&mut message, dir, &settings(&[dir.to_path_buf()]));
            assert_eq!(
                message["params"]["capabilities"]["textDocument"]["documentSymbol"]["hierarchicalDocumentSymbolSupport"],
                true
            );
        }

        let request = json!({
            "jsonrpc": "2.0", "id": 7, "method": "workspace/configuration",
            "params": { "items": [{ "section": "rust-analyzer" }, { "section": "other" }] }
        });
        let answer = configuration_answer(
            &request,
            &json!({ "rust-analyzer": settings(&[dir.to_path_buf()]) }),
        );
        assert_eq!(answer["id"], 7);
        assert_eq!(
            answer["result"][0]["linkedProjects"][0],
            "/ws/problems/abc001_a/Cargo.toml"
        );
        assert_eq!(answer["result"][1], Value::Null);
        // 節のない問い合わせには rust-analyzer の答え
        let bare = json!({ "jsonrpc": "2.0", "id": 8, "method": "workspace/configuration", "params": { "items": [{}] } });
        let answer = configuration_answer(
            &bare,
            &json!({ "rust-analyzer": settings(&[dir.to_path_buf()]) }),
        );
        assert_eq!(
            answer["result"][0]["linkedProjects"][0],
            "/ws/problems/abc001_a/Cargo.toml"
        );
        // basedpyright（D-225）：節ごとに答える。pyright の節と知らない節は null
        let request = json!({
            "jsonrpc": "2.0", "id": 9, "method": "workspace/configuration",
            "params": { "items": [{ "section": "python" }, { "section": "basedpyright" }, { "section": "pyright" }, {}] }
        });
        let configuration = basedpyright_configuration(Some(Path::new("/opt/x/python3.13")));
        let answer = configuration_answer(&request, &configuration);
        assert_eq!(answer["result"][0]["pythonPath"], "/opt/x/python3.13");
        assert_eq!(
            answer["result"][1]["analysis"]["typeCheckingMode"],
            "standard"
        );
        assert_eq!(
            answer["result"][1]["analysis"]["diagnosticMode"],
            "openFilesOnly"
        );
        assert_eq!(answer["result"][2], Value::Null);
        assert_eq!(answer["result"][3], Value::Null);
        // 処理系が見つからなければ渡さない
        assert_eq!(basedpyright_configuration(None)["python"], json!({}));
    }

    #[test]
    fn removes_the_pull_diagnostics_capability() {
        let mut message = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "capabilities": { "textDocument": { "diagnostic": {}, "hover": { "contentFormat": ["markdown"] } } } }
        });
        override_initialize(
            &mut message,
            Path::new("/ws/a"),
            &settings(&[PathBuf::from("/ws/a")]),
        );
        let text_document = &message["params"]["capabilities"]["textDocument"];
        assert!(text_document.get("diagnostic").is_none());
        // ほかの能力は残す
        assert_eq!(text_document["hover"]["contentFormat"][0], "markdown");
    }

    #[test]
    fn reads_file_uris() {
        assert_eq!(
            path_of_uri("file:///a/b%20c/%E3%81%82"),
            Some(PathBuf::from("/a/b c/あ"))
        );
        assert_eq!(
            path_of_uri(&file_uri(Path::new("/x/y z.rs"))),
            Some(PathBuf::from("/x/y z.rs"))
        );
        assert_eq!(path_of_uri("file://host/a"), None);
        assert_eq!(path_of_uri("https://example.com/a"), None);
        assert_eq!(path_of_uri("file:///a%2"), None);
        assert_eq!(path_of_uri("file:///a%ZZ"), None);
    }

    #[cfg(unix)]
    #[test]
    fn checks_messages_against_the_allow_list() {
        let root = temp_dir("allow-root");
        let outside = temp_dir("allow-outside");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(outside.join("secret.rs"), "").unwrap();
        let link = root.join("link.rs");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(outside.join("secret.rs"), &link).unwrap();
        let inside = file_uri(&root.join("src/main.rs"));
        let open = |uri: &str, text: &str| {
            json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen",
                    "params": { "textDocument": { "uri": uri, "languageId": "rust", "version": 0, "text": text } } })
        };
        // 通るもの
        assert!(check_message(&open(&inside, "fn main() {}"), &root).is_ok());
        assert!(check_message(&json!({ "jsonrpc": "2.0", "id": 3, "method": "textDocument/hover",
            "params": { "textDocument": { "uri": inside }, "position": { "line": 0, "character": 0 } } }), &root).is_ok());
        assert!(
            check_message(
                &json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 3 } }),
                &root
            )
            .is_ok()
        );
        assert!(
            check_message(
                &json!({ "jsonrpc": "2.0", "id": 9, "error": { "code": -32601, "message": "x" } }),
                &root
            )
            .is_ok()
        );
        // 消した後の didClose：ないファイルでも、作業領域の下なら通す
        let gone = file_uri(&root.join("src/gone.rs"));
        assert!(
            check_message(
                &json!({ "jsonrpc": "2.0", "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": gone } } }),
                &root
            )
            .is_ok()
        );
        // 通さないもの：許可にない方法
        for method in [
            "workspace/executeCommand",
            "workspace/didChangeConfiguration",
            "textDocument/rename",
            "textDocument/signatureHelp",
            "rust-analyzer/runFlycheck",
        ] {
            let message = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": { "textDocument": { "uri": inside } } });
            assert!(check_message(&message, &root).is_err(), "{method}");
        }
        // 通さないもの：作業領域の外、リンクで外を指すもの、.. で外に出るもの、file: でないもの、uri がないもの
        assert!(check_message(&open(&file_uri(&outside.join("secret.rs")), ""), &root).is_err());
        assert!(check_message(&open(&file_uri(&link), ""), &root).is_err());
        let escape = format!(
            "{}/../{}/nothing.rs",
            file_uri(&root),
            outside.file_name().unwrap().to_string_lossy()
        );
        assert!(check_message(&open(&escape, ""), &root).is_err());
        assert!(check_message(&open("untitled:a.rs", ""), &root).is_err());
        assert!(
            check_message(
                &json!({ "jsonrpc": "2.0", "id": 1, "method": "textDocument/hover", "params": {} }),
                &root
            )
            .is_err()
        );
        // 通さないもの：method も応答の形もないもの
        assert!(check_message(&json!({ "jsonrpc": "2.0", "id": 1 }), &root).is_err());
        // 本文の大きさ：MAX_FILE_SIZE までは通し、超えたら通さない
        let limit = "a".repeat(MAX_FILE_SIZE as usize);
        assert!(check_message(&open(&inside, &limit), &root).is_ok());
        assert!(check_message(&open(&inside, &(limit.clone() + "a")), &root).is_err());
        let change = |text: &str| {
            json!({ "jsonrpc": "2.0", "method": "textDocument/didChange",
                    "params": { "textDocument": { "uri": inside, "version": 1 }, "contentChanges": [{ "text": text }, { "text": "b" }] } })
        };
        assert!(check_message(&change("a"), &root).is_ok());
        assert!(check_message(&change(&limit), &root).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn locates_files_inside_the_workspace() {
        let root = temp_dir("locate-root").canonicalize().unwrap();
        let outside = temp_dir("locate-outside").canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src/bin")).unwrap();
        std::fs::write(root.join("src/bin/a.rs"), "").unwrap();
        std::fs::write(root.join("src/日本 語.rs"), "").unwrap();
        std::fs::write(outside.join("secret.rs"), "").unwrap();
        let link = root.join("link.rs");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(outside.join("secret.rs"), &link).unwrap();
        let locate = |path: &Path| locate_in_workspace(&root, &file_uri(path));
        assert_eq!(
            locate(&root.join("src/bin/a.rs")),
            Some("src/bin/a.rs".to_string())
        );
        // パーセントエンコードされた名前も戻る
        assert_eq!(
            locate(&root.join("src/日本 語.rs")),
            Some("src/日本 語.rs".to_string())
        );
        // 外、リンクで外を指すもの、フォルダ、ないファイル、.. で外に出るもの
        assert_eq!(locate(&outside.join("secret.rs")), None);
        assert_eq!(locate(&link), None);
        assert_eq!(locate(&root.join("src")), None);
        assert_eq!(locate(&root.join("src/none.rs")), None);
        let escape = format!(
            "{}/../{}/secret.rs",
            file_uri(&root),
            outside.file_name().unwrap().to_string_lossy()
        );
        assert_eq!(locate_in_workspace(&root, &escape), None);
        // file: でない
        assert_eq!(locate_in_workspace(&root, "untitled:a.rs"), None);
        assert_eq!(locate_in_workspace(&root, "https://example.com/a.rs"), None);
    }

    #[cfg(unix)]
    #[test]
    fn send_rejects_messages_outside_the_allow_list() {
        let control = LspControl::default();
        let (sink, events) = collector();
        let dir = temp_dir("reject");
        control
            .start_with(fake_server(false), dir.clone(), dir, "a".to_string(), sink)
            .unwrap();
        let error = control
            .send(r#"{"jsonrpc":"2.0","id":1,"method":"workspace/executeCommand","params":{"command":"x"}}"#)
            .unwrap_err();
        assert!(error.contains("workspace/executeCommand"), "{error}");
        // 拒否したメッセージは偽のサーバーに届かない（届けば echo が返る）
        thread::sleep(Duration::from_millis(200));
        assert!(!events.lock().unwrap().iter().any(
            |event| matches!(event, LspEvent::Message { text } if text.contains("executeCommand"))
        ));
        control.stop();
    }

    // 偽のサーバー（perl）。受け取ったメッセージを、method と id だけにして返す。
    // 起動したら子（sleep）の pid を標準エラーに出す。shutdown には答え、exit で終わる。
    // ignore_shutdown なら shutdown に答えず、終わらない
    #[cfg(unix)]
    fn fake_server(ignore_shutdown: bool) -> Command {
        let script = r#"
            $| = 1;
            my $child = fork();
            if ($child == 0) { exec("sleep", "100"); }
            print STDERR "child $child\n";
            binmode(STDIN);
            while (1) {
                my $len;
                while (my $line = <STDIN>) {
                    last if $line eq "\r\n";
                    $len = $1 if $line =~ /^Content-Length: (\d+)/i;
                }
                exit 1 unless defined $len;
                read(STDIN, my $body, $len);
                my ($method) = $body =~ /"method":"([^"]*)"/;
                my ($id) = $body =~ /"id":("[^"]*"|\d+)/;
                if ($method eq "shutdown") {
                    next if $ENV{IGNORE_SHUTDOWN};
                    my $out = "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":null}";
                    print "Content-Length: " . length($out) . "\r\n\r\n" . $out;
                    next;
                }
                exit 0 if $method eq "exit";
                exit 3 if $method eq "quit";
                if ($method eq "garbage") { print "Bad-Header\r\n\r\n"; next; }
                if ($method eq "ask") {
                    my $out = '{"jsonrpc":"2.0","id":99,"method":"workspace/configuration","params":{"items":[{"section":"rust-analyzer"}]}}';
                    print "Content-Length: " . length($out) . "\r\n\r\n" . $out;
                    next;
                }
                my $out = "{\"echo\":\"$method\",\"body\":" . ($body =~ /"result"/ ? $body : "null") . "}";
                print "Content-Length: " . length($out) . "\r\n\r\n" . $out;
            }
        "#;
        let mut command = Command::new("/usr/bin/perl");
        command.args(["-e", script]);
        if ignore_shutdown {
            command.env("IGNORE_SHUTDOWN", "1");
        }
        command
    }

    fn collector() -> (Sink, Arc<Mutex<Vec<LspEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = events.clone();
        let sink: Sink = Arc::new(move |event| seen.lock().unwrap().push(event));
        (sink, events)
    }

    // 条件を満たすまで、2 秒まで待つ
    #[cfg(unix)]
    fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
        (0..200).any(|_| {
            let met = condition();
            if !met {
                thread::sleep(Duration::from_millis(10));
            }
            met
        })
    }

    #[cfg(unix)]
    fn child_pid(events: &Arc<Mutex<Vec<LspEvent>>>) -> libc::pid_t {
        let mut pid = None;
        assert!(wait_until(|| {
            pid = events.lock().unwrap().iter().find_map(|event| match event {
                LspEvent::Log { text } => text.strip_prefix("child ")?.trim().parse().ok(),
                _ => None,
            });
            pid.is_some()
        }));
        pid.unwrap()
    }

    // pid のプロセスがもういないか（process.rs のテストと同じ確かめ方）
    #[cfg(unix)]
    fn gone(pid: libc::pid_t) -> bool {
        wait_until(|| {
            // SAFETY: シグナル 0 は送らず、存在だけを確かめる
            let result = unsafe { libc::kill(pid, 0) };
            result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        })
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("atcoder-studio-lsp-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn relays_messages_and_stops_gracefully() {
        let control = LspControl::default();
        let (sink, events) = collector();
        let dir = temp_dir("graceful");
        assert!(
            !control
                .start_with(
                    fake_server(false),
                    PathBuf::from("/"),
                    dir.clone(),
                    "a".to_string(),
                    sink.clone()
                )
                .unwrap()
        );
        let child = child_pid(&events);
        control
            .send_unchecked(r#"{"jsonrpc":"2.0","id":1,"method":"hello"}"#)
            .unwrap();
        // サーバーからの workspace/configuration には Rust 側で答え、画面には送らない
        control
            .send_unchecked(r#"{"jsonrpc":"2.0","method":"ask"}"#)
            .unwrap();
        assert!(wait_until(|| {
            events.lock().unwrap().iter().any(|event| matches!(event, LspEvent::Message { text } if text.contains("linkedProjects")))
        }));
        // 同じパッケージなら起動し直さない
        assert!(
            control
                .start_with(
                    fake_server(false),
                    PathBuf::from("/"),
                    dir.clone(),
                    "a".to_string(),
                    sink
                )
                .unwrap()
        );
        let started = Instant::now();
        control.stop();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "正式な終了で終わっていない"
        );
        let events = events.lock().unwrap();
        assert!(events.contains(&LspEvent::Message {
            text: r#"{"echo":"hello","body":null}"#.to_string()
        }));
        assert!(!events.iter().any(|event| matches!(event, LspEvent::Message { text } if text.contains("workspace/configuration"))));
        assert!(events.contains(&LspEvent::Exited { code: Some(0) }));
        // 終わったことは 1 回だけ知らせる
        let exits = events
            .iter()
            .filter(|event| matches!(event, LspEvent::Exited { .. }))
            .count();
        assert_eq!(exits, 1);
        assert!(gone(child), "子孫のプロセスが残っている");
        assert!(control.send("{}").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn keeps_the_order_of_messages() {
        let control = LspControl::default();
        let (sink, events) = collector();
        control
            .start_with(
                fake_server(false),
                PathBuf::from("/"),
                temp_dir("order"),
                "a".to_string(),
                sink,
            )
            .unwrap();
        for n in 0..200 {
            control
                .send_unchecked(&format!(r#"{{"jsonrpc":"2.0","method":"m{n}"}}"#))
                .unwrap();
        }
        let echoed = |events: &[LspEvent]| -> Vec<String> {
            events
                .iter()
                .filter_map(|event| match event {
                    LspEvent::Message { text } => Some(text.clone()),
                    _ => None,
                })
                .collect()
        };
        assert!(wait_until(|| echoed(&events.lock().unwrap()).len() == 200));
        let expected: Vec<String> = (0..200)
            .map(|n| format!(r#"{{"echo":"m{n}","body":null}}"#))
            .collect();
        assert_eq!(echoed(&events.lock().unwrap()), expected);
        control.stop();
    }

    #[cfg(unix)]
    #[test]
    fn stop_during_restart_leaves_nothing() {
        let control = Arc::new(LspControl::default());
        let (sink, events) = collector();
        // 前のサーバーは shutdown に答えないので、止めるのに STOP_WAIT かかる
        control
            .start_with(
                fake_server(true),
                PathBuf::from("/"),
                temp_dir("race-first"),
                "a".to_string(),
                sink.clone(),
            )
            .unwrap();
        child_pid(&events);
        let restarting = control.clone();
        let restart = thread::spawn(move || {
            restarting
                .start_with(
                    fake_server(false),
                    PathBuf::from("/"),
                    temp_dir("race-second"),
                    "a".to_string(),
                    sink,
                )
                .unwrap()
        });
        // 起動し直しの途中（前のサーバーを取り出して止めている間）に止める
        assert!(wait_until(|| control.server.lock().unwrap().is_none()));
        control.stop();
        restart.join().unwrap();
        assert!(
            control.server.lock().unwrap().is_none(),
            "新しいサーバーが残っている"
        );
        let exits = events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, LspEvent::Exited { .. }))
            .count();
        assert_eq!(exits, 2);
    }

    #[cfg(unix)]
    #[test]
    fn restarts_for_another_session() {
        let control = LspControl::default();
        let (sink, events) = collector();
        let dir = temp_dir("session");
        control
            .start_with(
                fake_server(false),
                PathBuf::from("/"),
                dir.clone(),
                "a".to_string(),
                sink.clone(),
            )
            .unwrap();
        let first = child_pid(&events);
        // 同じパッケージでも、画面のセッションが変わったら起動し直す（画面の再読み込み）
        assert!(
            !control
                .start_with(
                    fake_server(false),
                    PathBuf::from("/"),
                    dir,
                    "b".to_string(),
                    sink
                )
                .unwrap()
        );
        assert!(gone(first), "前のサーバーの子が残っている");
        control.stop();
    }

    #[cfg(unix)]
    #[test]
    fn restarts_when_the_packages_change() {
        let control = LspControl::default();
        let (sink, events) = collector();
        let dir = temp_dir("projects");
        let start = |projects: Vec<PathBuf>| {
            control
                .start_with_projects(
                    fake_server(false),
                    PathBuf::from("/"),
                    dir.clone(),
                    projects,
                    "a".to_string(),
                    sink.clone(),
                )
                .unwrap()
        };
        assert!(!start(vec![dir.join("a")]));
        let first = child_pid(&events);
        // 同じ場所・同じパッケージの組なら、そのまま
        assert!(start(vec![dir.join("a")]));
        // コンテストのフォルダに問題が増えたら、起動し直す（D-171）
        assert!(!start(vec![dir.join("a"), dir.join("b")]));
        assert!(gone(first), "前のサーバーの子が残っている");
        control.stop();
    }

    #[cfg(unix)]
    #[test]
    fn reports_server_that_exits_by_itself() {
        let control = LspControl::default();
        let (sink, events) = collector();
        control
            .start_with(
                fake_server(false),
                PathBuf::from("/"),
                temp_dir("quit"),
                "a".to_string(),
                sink,
            )
            .unwrap();
        control
            .send_unchecked(r#"{"jsonrpc":"2.0","method":"quit"}"#)
            .unwrap();
        assert!(wait_until(|| {
            events
                .lock()
                .unwrap()
                .contains(&LspEvent::Exited { code: Some(3) })
        }));
        // 回収は次の停止のとき。ここでは 2 回目を知らせない
        control.stop();
        let exits = events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, LspEvent::Exited { .. }))
            .count();
        assert_eq!(exits, 1);
    }

    #[cfg(unix)]
    #[test]
    fn stops_reading_broken_output() {
        let control = LspControl::default();
        let (sink, events) = collector();
        control
            .start_with(
                fake_server(false),
                PathBuf::from("/"),
                temp_dir("garbage"),
                "a".to_string(),
                sink,
            )
            .unwrap();
        control
            .send_unchecked(r#"{"jsonrpc":"2.0","method":"garbage"}"#)
            .unwrap();
        assert!(wait_until(|| {
            events.lock().unwrap().iter().any(
                |event| matches!(event, LspEvent::Log { text } if text.contains("見出しを読めません")),
            )
        }));
        // 読み取りをやめても、止められる
        control.stop();
        assert!(
            events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, LspEvent::Exited { .. }))
        );
    }

    #[cfg(unix)]
    #[test]
    fn kills_server_that_ignores_shutdown() {
        let control = LspControl::default();
        let (sink, events) = collector();
        let dir = temp_dir("ignore");
        control
            .start_with(
                fake_server(true),
                PathBuf::from("/"),
                dir,
                "a".to_string(),
                sink,
            )
            .unwrap();
        let child = child_pid(&events);
        let started = Instant::now();
        control.stop();
        let elapsed = started.elapsed();
        assert!(elapsed >= STOP_WAIT && elapsed < STOP_WAIT + Duration::from_secs(2));
        assert!(
            events
                .lock()
                .unwrap()
                .contains(&LspEvent::Exited { code: None })
        );
        assert!(gone(child), "子孫のプロセスが残っている");
    }

    #[cfg(unix)]
    #[test]
    fn restarts_for_another_package() {
        let control = LspControl::default();
        let (sink, events) = collector();
        control
            .start_with(
                fake_server(false),
                PathBuf::from("/"),
                temp_dir("first"),
                "a".to_string(),
                sink.clone(),
            )
            .unwrap();
        let first = child_pid(&events);
        assert!(
            !control
                .start_with(
                    fake_server(false),
                    PathBuf::from("/"),
                    temp_dir("second"),
                    "a".to_string(),
                    sink
                )
                .unwrap()
        );
        assert!(gone(first), "前のサーバーの子が残っている");
        assert!(
            events
                .lock()
                .unwrap()
                .contains(&LspEvent::Exited { code: Some(0) })
        );
        control.stop();
    }

    #[test]
    fn analyzes_a_contest_folder_together() {
        let base =
            std::env::temp_dir().join(format!("atcoder-studio-lsp-place-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("ws");
        let contest = root.join("abc480");
        for package in ["abc480_a", "abc480_b"] {
            std::fs::create_dir_all(contest.join(package).join("src")).unwrap();
            std::fs::write(contest.join(package).join("Cargo.toml"), "[package]\n").unwrap();
        }
        // Cargo.toml のないフォルダは扱わない
        std::fs::create_dir_all(contest.join("notes")).unwrap();
        let root = root.canonicalize().unwrap();
        let contest = root.join("abc480");
        let a = contest.join("abc480_a");
        // IDE が書いた .cargo/config.toml がなければ、今までどおりパッケージだけ
        assert_eq!(analyzer_place(&root, &a), (a.clone(), vec![a.clone()]));
        std::fs::create_dir_all(contest.join(".cargo")).unwrap();
        std::fs::write(
            contest.join(".cargo/config.toml"),
            "[build]\ntarget-dir = \"x\"\n",
        )
        .unwrap();
        assert_eq!(analyzer_place(&root, &a), (a.clone(), vec![a.clone()]));
        // IDE が書いたものなら、コンテストのフォルダで、中のパッケージをまとめて扱う
        std::fs::write(
            contest.join(".cargo/config.toml"),
            "# AtCoder Studio が作った。同じコンテストの問題のビルドの置き場所（target）を、このフォルダの中で共有する\n[build]\ntarget-dir = \"target\"\n",
        )
        .unwrap();
        assert_eq!(
            analyzer_place(&root, &a),
            (contest.clone(), vec![a.clone(), contest.join("abc480_b")])
        );
        // 作業領域がコンテストのフォルダそのものでも、まとめて扱う
        assert_eq!(analyzer_place(&contest, &a).0, contest);
        // パッケージを作業領域として開いたとき（親が作業領域の外）は、パッケージだけ
        assert_eq!(analyzer_place(&a, &a), (a.clone(), vec![a.clone()]));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn groups_cpp_packages_of_a_contest() {
        let base = temp_dir("clangd-place");
        let root = base.join("ws");
        let contest = root.join("abc480");
        for task in ["abc480_a", "abc480_b"] {
            std::fs::create_dir_all(contest.join(task)).unwrap();
            std::fs::write(contest.join(task).join("main.cpp"), "").unwrap();
        }
        // Rust のパッケージ・main.cpp のないフォルダは入れない
        std::fs::create_dir_all(contest.join("abc480_c")).unwrap();
        std::fs::write(contest.join("abc480_c/main.cpp"), "").unwrap();
        std::fs::write(contest.join("abc480_c/Cargo.toml"), "").unwrap();
        std::fs::create_dir_all(contest.join("notes")).unwrap();
        let single = root.join("abc481_a");
        std::fs::create_dir_all(&single).unwrap();
        std::fs::write(single.join("main.cpp"), "").unwrap();
        let a = contest.join("abc480_a");
        assert_eq!(
            clangd_place(&root, &a),
            (contest.clone(), vec![a.clone(), contest.join("abc480_b")])
        );
        // 作業領域の直下のパッケージ・作業領域として開いたパッケージは、パッケージだけ
        assert_eq!(
            clangd_place(&root, &single),
            (single.clone(), vec![single.clone()])
        );
        assert_eq!(clangd_place(&a, &a), (a.clone(), vec![a.clone()]));
        assert_eq!(clangd_place(&contest, &a), (a.clone(), vec![a.clone()]));
        let _ = std::fs::remove_dir_all(&base);
    }

    // Python も同じ決め方（D-225）。C++・Rust のしるしもあるフォルダは入れない
    #[test]
    fn groups_python_packages_of_a_contest() {
        let base = temp_dir("basedpyright-place");
        let root = base.join("ws");
        let contest = root.join("abc480");
        for task in ["abc480_a", "abc480_b", "abc480_c"] {
            std::fs::create_dir_all(contest.join(task)).unwrap();
            std::fs::write(contest.join(task).join("main.py"), "").unwrap();
        }
        std::fs::write(contest.join("abc480_c/main.cpp"), "").unwrap();
        std::fs::create_dir_all(contest.join("abc480_d")).unwrap();
        std::fs::write(contest.join("abc480_d/main.cpp"), "").unwrap();
        let a = contest.join("abc480_a");
        assert_eq!(
            basedpyright_place(&root, &a),
            (contest.clone(), vec![a.clone(), contest.join("abc480_b")])
        );
        assert_eq!(basedpyright_place(&a, &a), (a.clone(), vec![a.clone()]));
        // C++ の決め方も変わらない（Python だけのフォルダは入れない）
        assert_eq!(
            clangd_place(&root, &contest.join("abc480_d")),
            (
                contest.clone(),
                vec![contest.join("abc480_c"), contest.join("abc480_d")]
            )
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn escapes_the_query_driver() {
        assert_eq!(
            query_driver(Path::new("/opt/homebrew/bin/g++-15")).unwrap(),
            "/opt/homebrew/bin/g++-15"
        );
        assert_eq!(
            query_driver(Path::new("/x/[a]*/g++?")).unwrap(),
            r"/x/\[a\]\*/g++\?"
        );
        assert!(query_driver(Path::new("/x,y/g++")).is_err());
        let args = clangd_args(Path::new("/opt/homebrew/bin/g++-15")).unwrap();
        assert!(args.contains(&"--query-driver=/opt/homebrew/bin/g++-15".to_string()));
        // パッケージに .cache/ を作らない、.clangd を読まない（D-210）
        assert!(args.contains(&"--background-index=false".to_string()));
        assert!(args.contains(&"--enable-config=false".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn restarts_for_another_language() {
        let control = LspControl::default();
        let (sink, events) = collector();
        let dir = temp_dir("language");
        assert!(
            !control
                .start_with(
                    fake_server(false),
                    PathBuf::from("/"),
                    dir.clone(),
                    "a".to_string(),
                    sink.clone()
                )
                .unwrap()
        );
        let first = child_pid(&events);
        // 同じ場所でも、clangd に変われば起動し直す
        let launch = Launch {
            name: "clangd",
            command: fake_server(false),
            dir: dir.clone(),
            projects: vec![dir.clone()],
            options: json!({ "compilationDatabaseChanges": {} }),
            configuration: Value::Null,
        };
        assert!(
            !control
                .start(PathBuf::from("/"), launch, "a".to_string(), sink.clone())
                .unwrap()
        );
        assert!(gone(first), "前のサーバーの子が残っている");
        // initialize には clangd の設定（コンパイルの命令）が入り、画面からの設定は捨てる
        let mut message = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "initializationOptions": { "x": 1 } } });
        let options = json!({ "compilationDatabaseChanges": { "/ws/a/main.cpp": {} } });
        override_initialize(&mut message, &dir, &options);
        assert_eq!(message["params"]["initializationOptions"], options);
        control.stop();
    }

    // 手元に clangd と g++-15 があるときだけ、本当の clangd で確かめる（D-210）
    #[test]
    fn analyzes_cpp_with_local_clangd() {
        let Ok(compiler) = crate::cpp::find_compiler(&crate::cpp::CppSettings::default()) else {
            eprintln!("g++-15 がないので飛ばします");
            return;
        };
        let Ok(_) = find_clangd() else {
            eprintln!("clangd がないので飛ばします");
            return;
        };
        let root = temp_dir("clangd-real");
        let package = root.join("abc480/abc480_a");
        std::fs::create_dir_all(&package).unwrap();
        let source = "#include <bits/stdc++.h>\nusing namespace std;\nint main() {\n    vector<int> a(3);\n    cout << a.size() << endl;\n}\nstruct P {\n    int f() { return 1; }\n};\n";
        std::fs::write(package.join("main.cpp"), source).unwrap();
        // 作業 3 の compile_flags.txt があっても、IDE の命令が使われる
        std::fs::write(package.join("compile_flags.txt"), "-std=gnu++23\n").unwrap();
        let (dir, projects) = clangd_place(&root, &package);
        let settings = crate::cpp::CppSettings::default();
        let options = crate::cpp::compilation_database(&compiler, &projects, &settings);
        let launch = clangd(dir, projects, &compiler, options).unwrap();
        let control = LspControl::default();
        let (sink, events) = collector();
        assert!(
            !control
                .start(root.clone(), launch, "a".to_string(), sink)
                .unwrap()
        );
        let uri = file_uri(&package.join("main.cpp"));
        let message = |text: String| control.send(&text).unwrap();
        message(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } }).to_string());
        message(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }).to_string());
        message(json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "languageId": "cpp", "version": 1, "text": source } } }).to_string());
        message(json!({ "jsonrpc": "2.0", "id": 2, "method": "textDocument/definition", "params": { "textDocument": { "uri": uri }, "position": { "line": 3, "character": 6 } } }).to_string());
        message(json!({ "jsonrpc": "2.0", "id": 3, "method": "textDocument/documentSymbol", "params": { "textDocument": { "uri": uri } } }).to_string());
        let find = |pick: &dyn Fn(&Value) -> bool| -> Option<Value> {
            events.lock().unwrap().iter().find_map(|event| match event {
                LspEvent::Message { text } => {
                    serde_json::from_str::<Value>(text).ok().filter(|m| pick(m))
                }
                _ => None,
            })
        };
        let mut diagnostics = None;
        let mut definition = None;
        let mut symbols = None;
        for _ in 0..1500 {
            diagnostics = find(&|m| {
                m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri
            });
            definition = find(&|m| m["id"] == 2);
            symbols = find(&|m| m["id"] == 3);
            if diagnostics.is_some() && definition.is_some() && symbols.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        control.stop();
        let diagnostics = diagnostics.expect("診断が届かない");
        assert_eq!(
            diagnostics["params"]["diagnostics"],
            json!([]),
            "{diagnostics}"
        );
        // vector の定義は GCC の標準ヘッダーにあり、読み取り専用で開ける場所の中
        let target = definition.expect("定義の答えが届かない")["result"][0]["uri"]
            .as_str()
            .unwrap()
            .to_string();
        let path = path_of_uri(&target).unwrap().canonicalize().unwrap();
        assert!(path.to_string_lossy().contains("/include/c++/"), "{target}");
        let headers = crate::cpp::header_dirs(&compiler, None);
        assert!(
            headers.iter().any(|dir| path.starts_with(dir)),
            "{path:?} {headers:?}"
        );
        // 記号は入れ子の形（D-216）。main と、P の中の f
        let symbols = symbols.expect("記号の答えが届かない")["result"].clone();
        let named = |list: &Value, name: &str| -> Value {
            list.as_array()
                .unwrap()
                .iter()
                .find(|s| s["name"] == name)
                .cloned()
                .unwrap_or_else(|| panic!("{name} がない：{list}"))
        };
        let main = named(&symbols, "main");
        assert_eq!(main["selectionRange"]["start"]["line"], 2, "{symbols}");
        let p = named(&symbols, "P");
        assert_eq!(named(&p["children"], "f")["range"]["start"]["line"], 7);
        // パッケージ・コンテストのフォルダに .cache/ を作らない
        assert!(!package.join(".cache").exists());
        assert!(!root.join("abc480/.cache").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // 手元に basedpyright と CPython 3.13 があるときだけ、本当の basedpyright で確かめる（D-225）
    #[test]
    fn analyzes_python_with_local_basedpyright() {
        let Ok(interpreter) =
            crate::python::find_interpreter(&crate::python::PythonSettings::default())
        else {
            eprintln!("python3.13 がないので飛ばします");
            return;
        };
        let Ok(_) = find_basedpyright() else {
            eprintln!("basedpyright がないので飛ばします");
            return;
        };
        let root = temp_dir("basedpyright-real");
        let package = root.join("abc480/abc480_a");
        std::fs::create_dir_all(&package).unwrap();
        let source = "import sys\nfrom collections import deque\n\n\ndef main():\n    n = int(input())\n    q = deque()\n    print(n + \"a\")\n\n\nmain()\n";
        std::fs::write(package.join("main.py"), source).unwrap();
        let (dir, projects) = basedpyright_place(&root, &package);
        let launch = basedpyright(dir, projects, Some(&interpreter)).unwrap();
        let control = LspControl::default();
        let (sink, events) = collector();
        assert!(
            !control
                .start(root.clone(), launch, "a".to_string(), sink)
                .unwrap()
        );
        let uri = file_uri(&package.join("main.py"));
        let message = |text: String| control.send(&text).unwrap();
        message(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": { "workspace": { "configuration": true } } } }).to_string());
        message(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }).to_string());
        message(json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "languageId": "python", "version": 1, "text": source } } }).to_string());
        let find = |pick: &dyn Fn(&Value) -> bool| -> Option<Value> {
            events.lock().unwrap().iter().find_map(|event| match event {
                LspEvent::Message { text } => {
                    serde_json::from_str::<Value>(text).ok().filter(|m| pick(m))
                }
                _ => None,
            })
        };
        // 処理系を受け取ってから調べるので、診断が届いてから問い合わせる
        let mut diagnostics = None;
        for _ in 0..1500 {
            diagnostics = find(&|m| {
                m["method"] == "textDocument/publishDiagnostics"
                    && m["params"]["uri"] == uri
                    && !m["params"]["diagnostics"].as_array().unwrap().is_empty()
            });
            if diagnostics.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        message(json!({ "jsonrpc": "2.0", "id": 2, "method": "textDocument/definition", "params": { "textDocument": { "uri": uri }, "position": { "line": 6, "character": 9 } } }).to_string());
        message(json!({ "jsonrpc": "2.0", "id": 3, "method": "textDocument/documentSymbol", "params": { "textDocument": { "uri": uri } } }).to_string());
        message(json!({ "jsonrpc": "2.0", "id": 4, "method": "textDocument/inlayHint", "params": { "textDocument": { "uri": uri }, "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 11, "character": 0 } } } }).to_string());
        let mut definition = None;
        let mut symbols = None;
        let mut hints = None;
        for _ in 0..1500 {
            definition = find(&|m| m["id"] == 2);
            symbols = find(&|m| m["id"] == 3);
            hints = find(&|m| m["id"] == 4);
            if definition.is_some() && symbols.is_some() && hints.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        control.stop();
        // standard の検査：型の誤りは誤り（1）、使っていない import は目立たない印（4）。
        // basedpyright の既定の厳しい検査（「部分的に不明」）は出ない
        let diagnostics = diagnostics.expect("診断が届かない")["params"]["diagnostics"].clone();
        let lines: Vec<(u64, u64)> = diagnostics
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                (
                    d["range"]["start"]["line"].as_u64().unwrap(),
                    d["severity"].as_u64().unwrap(),
                )
            })
            .collect();
        assert!(lines.contains(&(7, 1)), "{diagnostics}");
        assert!(lines.contains(&(0, 4)), "{diagnostics}");
        assert!(
            lines.iter().all(|(_, severity)| *severity != 2),
            "{diagnostics}"
        );
        // deque の定義は basedpyright の typeshed にあり、読み取り専用で開ける場所の中
        let target = definition.expect("定義の答えが届かない")["result"][0]["uri"]
            .as_str()
            .unwrap()
            .to_string();
        let path = path_of_uri(&target).unwrap().canonicalize().unwrap();
        let typeshed = basedpyright_typeshed().expect("typeshed が見つからない");
        assert!(path.starts_with(&typeshed), "{path:?} {typeshed:?}");
        assert!(
            crate::sources::read_source(&target, &[], &[typeshed])
                .unwrap()
                .content
                .contains("class deque")
        );
        // 記号は入れ子の形。main の中に n・q
        let symbols = symbols.expect("記号の答えが届かない")["result"].clone();
        assert_eq!(symbols[0]["name"], "main", "{symbols}");
        let children: Vec<&str> = symbols[0]["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(children, ["n", "q"]);
        // インレイヒント（戻り値の型）
        let hints = hints.expect("インレイヒントが届かない")["result"].to_string();
        assert!(hints.contains("-> None"), "{hints}");
        // パッケージ・コンテストのフォルダにファイルを作らない
        let names = |dir: &Path| -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        assert_eq!(names(&package), ["main.py"]);
        assert_eq!(names(&root.join("abc480")), ["abc480_a"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
