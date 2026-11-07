// AtCoder のログインとセッション（Phase 6 作業 1。D-147・D-148）
//
// - ログインは、IDE が開く別の窓（保存しない WebView、IDE の権限なし）で利用者が行う。IDE はパスワードを扱わない
// - ログインのあとに Cookie の REVEL_SESSION だけを読み、macOS の Keychain に置く（/usr/bin/security。値は標準入力で渡す）
// - Cookie の値は、コマンドの引数・ファイル・記録・画面・WebView に出さない。curl にも、設定として標準入力から渡す
// - サーバーが応答で REVEL_SESSION を入れ替えたら、Keychain の値も置き換える（docs/phase6/01-pre-start-research.md §8.1）
// - ログインのページへ移されたら、ログインが切れたとみなし、Keychain の値を消す
//
// 限界：/usr/bin/security で作った項目は、同じ利用者の権限で動くほかのプログラムが確認なしに読める【推測】（D-147）
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime, Url, WebviewUrl, WebviewWindowBuilder};

// ログインの窓のラベル。capabilities は main の窓だけなので、この窓からは IDE のコマンドを呼べない
pub const LOGIN_LABEL: &str = "atcoder-login";
const LOGIN_URL: &str = "https://atcoder.jp/login?continue=https%3A%2F%2Fatcoder.jp%2Fhome";
const HOME_URL: &str = "https://atcoder.jp/home";
const SERVICE: &str = "io.github.ishigaki892.acdstudio";
const ACCOUNT: &str = "REVEL_SESSION";
// ログインの状態が変わったことを main の窓に知らせるイベント
pub const SESSION_EVENT: &str = "atcoder-session";

pub const NOT_LOGGED_IN: &str =
    "AtCoder にログインしていません。「提出結果」タブの「ログイン」からログインしてください";

// ---- Keychain ----

pub struct Keychain {
    service: String,
}

// Cookie の値として受け付ける文字（RFC 6265 の cookie-octet）。security -i と curl の設定の引用符の中にそのまま書ける
fn valid_cookie_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && value
            .bytes()
            .all(|b| b.is_ascii_graphic() && !matches!(b, b'"' | b',' | b';' | b'\\'))
}

impl Keychain {
    pub fn app() -> Self {
        Keychain {
            service: SERVICE.to_string(),
        }
    }

    // テストの間だけ使う項目（サービス名を変える）
    #[cfg(test)]
    pub fn named(service: &str) -> Self {
        Keychain {
            service: service.to_string(),
        }
    }
}

// 資格情報マネージャー（Windows。段階 3 T5。docs/windows-port-plan.md 4.1）。
// CRED_TYPE_GENERIC の項目に REVEL_SESSION の値を置く。値はエラーの文・ログに出さない
#[cfg(windows)]
impl Keychain {
    // Windows のテストのビルドでは、本物の名前（SERVICE）に対する読み書き・削除を、API を呼ぶ前に止める（9.1 の追加 24）。
    // Keychain::app() を作ること自体は止めない（Remote::app() を使うテストがあるため）
    #[cfg(test)]
    fn refuse_the_real_name(&self, operation: &str) {
        assert!(
            self.service != SERVICE,
            "テストから本物の資格情報に触れようとしました（{operation}）"
        );
    }

    #[cfg(not(test))]
    fn refuse_the_real_name(&self, _operation: &str) {}

    // 値は API に直接渡す（外部のコマンドも標準入力も使わない）。置いたあとに読み直して確かめる
    pub fn save(&self, value: &str) -> Result<(), String> {
        self.refuse_the_real_name("save");
        if !valid_cookie_value(value) {
            return Err("セッションの値に扱えない文字があります".to_string());
        }
        // 上限を超える値は、API を呼ぶ前に断る（9.1 の追加 23）
        if value.len() > crate::win::CREDENTIAL_BLOB_LIMIT {
            return Err(
                "セッションの値が長すぎるので、資格情報マネージャーに置けません（上限は 2560 バイト）"
                    .to_string(),
            );
        }
        crate::win::credential_write(&self.target(), ACCOUNT, value.as_bytes())
            .map_err(credential_error)?;
        self.confirm_saved(value)
    }

    // 置いた値。なければ None。読めない UTF-8・Cookie の値として受け付けない文字を含むものも None（Unix と同じ）
    pub fn load(&self) -> Result<Option<String>, String> {
        self.refuse_the_real_name("load");
        let blob = crate::win::credential_read(&self.target()).map_err(credential_error)?;
        Ok(blob
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .filter(|value| valid_cookie_value(value)))
    }

    // 置いてあるか（値は写さない）
    pub fn exists(&self) -> Result<bool, String> {
        self.refuse_the_real_name("exists");
        crate::win::credential_exists(&self.target()).map_err(credential_error)
    }

    // 消す。なかったら false
    pub fn delete(&self) -> Result<bool, String> {
        self.refuse_the_real_name("delete");
        crate::win::credential_delete(&self.target()).map_err(credential_error)
    }

    // 項目の名前（docs/windows-port-plan.md 4.1）。名前を作るだけで、API は呼ばない
    fn target(&self) -> String {
        format!("{}/{ACCOUNT}", self.service)
    }

    // 書いたあとに読み直して確かめる（Unix の save と同じ）
    fn confirm_saved(&self, value: &str) -> Result<(), String> {
        if self.load()?.as_deref() == Some(value) {
            Ok(())
        } else {
            Err("セッションを資格情報マネージャーに置けませんでした".to_string())
        }
    }
}

// 資格情報マネージャーのエラーの文。値は入れず、エラーの番号だけを入れる
#[cfg(windows)]
fn credential_error(code: u32) -> String {
    format!("資格情報マネージャーを使えません（エラー {code}）")
}

#[cfg(not(windows))]
impl Keychain {
    fn security() -> Command {
        Command::new("/usr/bin/security")
    }

    // 値は引数に置かず、security -i（対話の形）の標準入力でコマンドとして渡す。置いたあとに読み直して確かめる
    pub fn save(&self, value: &str) -> Result<(), String> {
        if !valid_cookie_value(value) {
            return Err("セッションの値に扱えない文字があります".to_string());
        }
        let mut child = Self::security()
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("Keychain を使えません：{error}"))?;
        let script = format!(
            "add-generic-password -U -s {} -a {ACCOUNT} -w \"{value}\"\n",
            self.service
        );
        child
            .stdin
            .take()
            .ok_or_else(|| "Keychain を使えません".to_string())?
            .write_all(script.as_bytes())
            .map_err(|error| format!("Keychain を使えません：{error}"))?;
        child
            .wait()
            .map_err(|error| format!("Keychain を使えません：{error}"))?;
        if self.load()?.as_deref() == Some(value) {
            Ok(())
        } else {
            Err("セッションを Keychain に置けませんでした".to_string())
        }
    }

    // 置いた値。なければ None
    pub fn load(&self) -> Result<Option<String>, String> {
        let output = Self::security()
            .args([
                "find-generic-password",
                "-s",
                &self.service,
                "-a",
                ACCOUNT,
                "-w",
            ])
            .stderr(Stdio::null())
            .output()
            .map_err(|error| format!("Keychain を使えません：{error}"))?;
        if !output.status.success() {
            return Ok(None);
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(valid_cookie_value(&value).then_some(value))
    }

    // 置いてあるか（値は読まない）
    pub fn exists(&self) -> Result<bool, String> {
        Self::security()
            .args(["find-generic-password", "-s", &self.service, "-a", ACCOUNT])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .map_err(|error| format!("Keychain を使えません：{error}"))
    }

    // 消す。なかったら false
    pub fn delete(&self) -> Result<bool, String> {
        Self::security()
            .args([
                "delete-generic-password",
                "-s",
                &self.service,
                "-a",
                ACCOUNT,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .map_err(|error| format!("Keychain を使えません：{error}"))
    }
}

// ---- セッション付きの curl ----

pub struct Reply {
    pub status: u16,
    // 提出の成否（/submissions/me へ移ったか）に使う
    pub location: Option<String>,
    pub body: String,
}

// curl の設定の引用符の中に書けるようにする（curl の設定では \\ \" \t \n \r \v が使える）
fn quote_config(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\u{0B}' => quoted.push_str("\\v"),
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
}

// curl が失敗したときの文（Phase 7b 作業 2）。よくある終了コードには、日本語の理由と次の手段を付ける。
// curl の文（英語）は、詳しい情報としてかっこの中に残す（2026-10-25 に、6 と 7 の文を手元で確かめた）
pub(crate) fn curl_failure(prefix: &str, code: Option<i32>, stderr: &str) -> String {
    let reason = match code {
        Some(6) => Some(
            "AtCoder のサーバーの名前を引けません。ネットワークにつながっているか確かめてください",
        ),
        Some(7) => Some(
            "AtCoder のサーバーにつながりません。ネットワークにつながっているか確かめてください",
        ),
        Some(28) => Some(
            "時間内に応答がありませんでした。ネットワークの状態を確かめて、もう一度試してください",
        ),
        Some(35) => Some(
            "安全な接続（TLS）を始められませんでした。ネットワークの状態を確かめて、もう一度試してください",
        ),
        _ => None,
    };
    let detail = stderr.trim();
    match reason {
        Some(reason) => format!("{prefix}：{reason}（{detail}）"),
        None => format!("{prefix}：{detail}"),
    }
}

// 応答の見出しと本文を分ける。100 Continue などの 1xx の見出しは飛ばす
fn split_response(text: &str) -> (u16, Vec<String>, String) {
    let mut rest = text;
    loop {
        let (head, body) = match rest.split_once("\r\n\r\n") {
            Some(parts) => parts,
            None => (rest, ""),
        };
        let mut lines = head.lines();
        let status = lines
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse::<u16>().ok())
            .unwrap_or(0);
        if (100..200).contains(&status) && !body.is_empty() {
            rest = body;
            continue;
        }
        return (
            status,
            lines.map(str::to_string).collect(),
            body.to_string(),
        );
    }
}

fn header<'a>(headers: &'a [String], name: &str) -> impl Iterator<Item = &'a str> {
    let prefix = format!("{}:", name.to_ascii_lowercase());
    headers
        .iter()
        .filter(move |line| line.to_ascii_lowercase().starts_with(&prefix))
        .map(|line| line[line.find(':').map_or(0, |i| i + 1)..].trim())
}

// Set-Cookie の REVEL_SESSION の新しい値（空や削除は None）
fn rotated_session(headers: &[String]) -> Option<String> {
    header(headers, "set-cookie").find_map(|cookie| {
        let value = cookie
            .strip_prefix(&format!("{ACCOUNT}="))?
            .split(';')
            .next()?
            .trim();
        valid_cookie_value(value).then(|| value.to_string())
    })
}

pub struct Client {
    keychain: Keychain,
    // テストのときだけ http を許す
    proto: &'static str,
}

impl Client {
    pub fn app() -> Self {
        Client {
            keychain: Keychain::app(),
            proto: "=https",
        }
    }

    // テストのときだけ：手元の http の口に送る。Keychain の項目もテスト用のもの
    #[cfg(test)]
    pub fn for_test(service: &str) -> Self {
        Client {
            keychain: Keychain::named(service),
            proto: "=http",
        }
    }

    pub fn keychain(&self) -> &Keychain {
        &self.keychain
    }

    // セッションを付けて url を読む（form があれば POST）。リダイレクトはたどらない。
    // ログインしていない・ログインのページへ移されたら Err(NOT_LOGGED_IN)（Keychain の値も消す）
    pub fn request(&self, url: &str, form: &[(&str, &str)]) -> Result<Reply, String> {
        let session = self
            .keychain
            .load()?
            .ok_or_else(|| NOT_LOGGED_IN.to_string())?;
        let (status, headers, body) = self.send(url, Some(&session), form)?;
        if let Some(value) = rotated_session(&headers)
            && value != session
        {
            self.keychain.save(&value)?;
        }
        let location = header(&headers, "location").next().map(str::to_string);
        if location.as_deref().is_some_and(is_login_location) {
            let _ = self.keychain.delete();
            return Err(NOT_LOGGED_IN.to_string());
        }
        Ok(Reply {
            status,
            location,
            body,
        })
    }

    // セッションを付けずに url を読む（ログインなしで読めるページ。コンテストのトップなど）
    pub fn public(&self, url: &str) -> Result<Reply, String> {
        let (status, headers, body) = self.send(url, None, &[])?;
        Ok(Reply {
            status,
            location: header(&headers, "location").next().map(str::to_string),
            body,
        })
    }

    // curl で 1 回送る
    fn send(
        &self,
        url: &str,
        session: Option<&str>,
        form: &[(&str, &str)],
    ) -> Result<(u16, Vec<String>, String), String> {
        run_curl(curl_command()?, self.proto, url, session, form)
    }
}

// 起動する curl
#[cfg(not(windows))]
fn curl_command() -> Result<Command, String> {
    Ok(Command::new("/usr/bin/curl"))
}

// Windows：%SystemRoot%\System32\curl.exe（PATH は探さない）。CREATE_NO_WINDOW を付ける（process::command）
#[cfg(windows)]
fn curl_command() -> Result<Command, String> {
    Ok(crate::process::command(windows_curl()?))
}

// Windows（段階 2 S11。段階 3 T6 で problem.rs から移した）：Windows に入っている curl.exe の場所。
// PATH は探さない（同じ名前の別のプログラムを使わないように）
#[cfg(windows)]
pub(crate) fn windows_curl() -> Result<std::path::PathBuf, String> {
    let root = std::env::var_os("SystemRoot")
        .ok_or_else(|| "curl.exe が見つかりません（SystemRoot が分かりません）".to_string())?;
    let path = std::path::PathBuf::from(root).join(r"System32\curl.exe");
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("curl.exe が見つかりません（{}）", path.display()))
    }
}

// curl を 1 回動かす。設定（Cookie・URL・フォーム）は標準入力から渡す。引数には入れない
fn run_curl(
    mut curl: Command,
    proto: &str,
    url: &str,
    session: Option<&str>,
    form: &[(&str, &str)],
) -> Result<(u16, Vec<String>, String), String> {
    let mut config = String::new();
    if let Some(session) = session {
        config.push_str(&format!(
            "cookie = {}\n",
            quote_config(&format!("{ACCOUNT}={session}"))
        ));
    }
    config.push_str(&format!("url = {}\n", quote_config(url)));
    for (key, value) in form {
        config.push_str(&format!(
            "data-urlencode = {}\n",
            quote_config(&format!("{key}={value}"))
        ));
    }
    let mut child = curl
        .args([
            "--silent",
            "--show-error",
            "--include",
            // 状態の JSON の URL の `sids[]` を、curl の範囲の書き方として読ませない
            "--globoff",
            "--proto",
            proto,
            "--max-time",
            "30",
            "--max-filesize",
            "8000000",
            "--config",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("curl を起動できません：{error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "curl に渡せません".to_string())?
        .write_all(config.as_bytes())
        .map_err(|error| format!("curl に渡せません：{error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("curl が終わりません：{error}"))?;
    if !output.status.success() {
        return Err(curl_failure(
            "AtCoder につながりません",
            output.status.code(),
            &String::from_utf8_lossy(&output.stderr),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(split_response(&text))
}

// ログインのページへの移動か（/login、https://atcoder.jp/login?...）
fn is_login_location(location: &str) -> bool {
    let path = location
        .strip_prefix("https://atcoder.jp")
        .unwrap_or(location);
    path == "/login" || path.starts_with("/login?")
}

// ページの `var userScreenName = "…";`。ログインしていなければ空なので None
pub fn user_screen_name(html: &str) -> Option<String> {
    let start = html.find("var userScreenName = \"")? + "var userScreenName = \"".len();
    let end = html[start..].find('"')? + start;
    let name = &html[start..end];
    (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .then(|| name.to_string())
}

// ---- ログインの状態 ----

#[derive(Default)]
pub struct SessionState {
    // ログインを確かめたユーザー名（保存しない。アプリを起動し直すと、次に確かめるまで分からない）
    user: Mutex<Option<String>>,
    // ログインの窓で、確かめる処理を 1 つだけ動かす
    confirming: AtomicBool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub logged_in: bool,
    pub user_name: Option<String>,
    // ログインの窓で失敗したときの理由
    pub error: Option<String>,
}

impl SessionState {
    pub fn user(&self) -> Option<String> {
        self.user.lock().ok().and_then(|user| user.clone())
    }

    pub fn set_user(&self, name: Option<String>) {
        if let Ok(mut user) = self.user.lock() {
            *user = name;
        }
    }

    pub fn view(&self, keychain: &Keychain) -> Result<SessionView, String> {
        let logged_in = keychain.exists()?;
        if !logged_in {
            self.set_user(None);
        }
        Ok(SessionView {
            logged_in,
            user_name: self.user().filter(|_| logged_in),
            error: None,
        })
    }
}

pub fn logout(state: &SessionState, keychain: &Keychain) -> Result<SessionView, String> {
    keychain.delete()?;
    state.set_user(None);
    state.view(keychain)
}

// ---- ログインの窓 ----

// 窓の中で移れる先。Turnstile は challenges.cloudflare.com の埋め込みとして読まれ、
// wry の移動の判定は埋め込みにも効くので、その 2 つと about:blank・about:srcdoc だけを許す
fn allowed_in_login(url: &Url) -> bool {
    match url.scheme() {
        "https" => matches!(
            url.host_str(),
            Some("atcoder.jp") | Some("challenges.cloudflare.com")
        ),
        "about" => matches!(url.path(), "blank" | "srcdoc"),
        _ => false,
    }
}

pub fn notify<R: Runtime>(app: &AppHandle<R>, view: SessionView) {
    let _ = app.emit_to("main", SESSION_EVENT, view);
}

pub fn open_login_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(LOGIN_LABEL) {
        let _ = window.set_focus();
        return Ok(());
    }
    let url: Url = LOGIN_URL
        .parse()
        .map_err(|_| "ログインのページを開けません".to_string())?;
    WebviewWindowBuilder::new(app, LOGIN_LABEL, WebviewUrl::External(url))
        .title("AtCoder にログイン")
        .inner_size(960.0, 780.0)
        // 閲覧データを残さない（閉じれば Cookie も消える。REVEL_SESSION は Keychain に置く）
        .incognito(true)
        .on_navigation(allowed_in_login)
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_page_load(|window, payload| {
            if payload.event() != PageLoadEvent::Finished {
                return;
            }
            let url = payload.url();
            if url.host_str() != Some("atcoder.jp") || url.path().starts_with("/login") {
                return;
            }
            let app = window.app_handle().clone();
            if app
                .state::<SessionState>()
                .confirming
                .swap(true, Ordering::SeqCst)
            {
                return;
            }
            // cookies() はイベントループに頼んで答えを待つので、メインスレッド（この中）から呼ぶと止まる。別のスレッドで読む
            std::thread::spawn(move || {
                let result = take_session(&window);
                let state = app.state::<SessionState>();
                state.confirming.store(false, Ordering::SeqCst);
                match result {
                    Ok(name) => {
                        state.set_user(Some(name.clone()));
                        notify(
                            &app,
                            SessionView {
                                logged_in: true,
                                user_name: Some(name),
                                error: None,
                            },
                        );
                        let _ = window.close();
                    }
                    // ログインせずにほかのページへ移ったときなど。窓は開いたままにする
                    Err(error) => notify(
                        &app,
                        SessionView {
                            logged_in: false,
                            user_name: None,
                            error: Some(error),
                        },
                    ),
                }
            });
        })
        .build()
        .map_err(|error| format!("ログインの窓を開けません：{error}"))?;
    Ok(())
}

// 窓の Cookie から REVEL_SESSION を Keychain に置き、/home を読んでログインしているかを確かめる。ユーザー名を返す
fn take_session<R: Runtime>(window: &tauri::WebviewWindow<R>) -> Result<String, String> {
    let cookies = window
        .cookies()
        .map_err(|error| format!("ログインの窓の Cookie を読めません：{error}"))?;
    let session = cookies
        .iter()
        .find(|cookie| {
            cookie.name() == ACCOUNT
                && cookie
                    .domain()
                    .is_some_and(|domain| domain.trim_start_matches('.') == "atcoder.jp")
        })
        .ok_or_else(|| "ログインのセッションが見つかりません".to_string())?;
    confirm_session(&Client::app(), HOME_URL, session.value())
}

// 窓の Cookie の値で /home を読み、ログインしていると分かってから Keychain に書く（Phase 10 作業 8。D-170）。
// ログインしていなければ（ログインせずに別のページへ移ったなど）、Keychain の今の値は変えない。
// 値は、ほかの通信と同じく、標準入力の設定で curl に渡す
fn confirm_session(client: &Client, url: &str, value: &str) -> Result<String, String> {
    if !valid_cookie_value(value) {
        return Err("ログインのセッションを読めません".to_string());
    }
    let (status, headers, body) = client.send(url, Some(value), &[])?;
    match user_screen_name(&body) {
        Some(name) if status == 200 => {
            let session = rotated_session(&headers).unwrap_or_else(|| value.to_string());
            client.keychain().save(&session)?;
            Ok(name)
        }
        _ => Err("ログインしていません。窓でログインしてください".to_string()),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read};
    use std::net::TcpListener;

    // テストの間だけ使う Keychain の項目。終わったら（失敗しても）消す
    pub(crate) struct TestKeychain(
        pub(crate) Keychain,
        // Windows：資格情報を使うテストを 1 つずつ動かす順番（9.1 の追加 32）。Drop で項目を消したあとに返す
        #[cfg(windows)]
        #[allow(dead_code)]
        Turn,
    );

    // Windows のテストの順番（段階 3 T5）。別の名前の書き込みと重なると、消した項目が戻ることがあるので、
    // 同じテストのプログラムの中では、資格情報を使うテストを 1 つずつ動かす（別のプログラムとの重なりは防げない）。
    // 同じスレッドは何回でも取れる（1 つのテストで TestKeychain を 2 つ作るものがあるため）。全部を返したら空く
    #[cfg(windows)]
    struct TurnState {
        holder: Option<std::thread::ThreadId>,
        count: usize,
    }

    #[cfg(windows)]
    struct TurnLock {
        state: Mutex<TurnState>,
        free: std::sync::Condvar,
        limit: std::time::Duration,
    }

    #[cfg(windows)]
    static TURN: TurnLock = TurnLock::new(std::time::Duration::from_secs(120));

    #[cfg(windows)]
    struct Turn(&'static TurnLock);

    #[cfg(windows)]
    impl TurnLock {
        const fn new(limit: std::time::Duration) -> Self {
            TurnLock {
                state: Mutex::new(TurnState {
                    holder: None,
                    count: 0,
                }),
                free: std::sync::Condvar::new(),
                limit,
            }
        }

        // 空くまで待つ。同じスレッドが持っていれば待たない。長く待ったら、分かる文で panic する（本来は届かない上限）
        fn take(&'static self) -> Turn {
            let me = std::thread::current().id();
            let started = std::time::Instant::now();
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                match state.holder {
                    None => {
                        state.holder = Some(me);
                        state.count = 1;
                        return Turn(self);
                    }
                    Some(holder) if holder == me => {
                        state.count += 1;
                        return Turn(self);
                    }
                    Some(_) => {
                        let Some(left) = self.limit.checked_sub(started.elapsed()) else {
                            // 毒にしないように、Mutex を返してから panic する
                            drop(state);
                            panic!(
                                "資格情報のテストの順番を {:?} 待っても取れません",
                                self.limit
                            );
                        };
                        state = self
                            .free
                            .wait_timeout(state, left)
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .0;
                    }
                }
            }
        }
    }

    // 順番を返す。全部を返したら空く。別のスレッドで返すのは誤りなので、順番を返してから、分かる文で panic する。
    // panic の最中なら panic しない（panic の最中の panic で、プログラムごと止まるのを避ける）
    #[cfg(windows)]
    impl Drop for Turn {
        fn drop(&mut self) {
            let wrong_thread = {
                let mut state = self
                    .0
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let wrong_thread = state.holder != Some(std::thread::current().id());
                state.count = state.count.saturating_sub(1);
                if state.count == 0 {
                    state.holder = None;
                    self.0.free.notify_all();
                }
                wrong_thread
            };
            if wrong_thread && !std::thread::panicking() {
                panic!("資格情報のテストの順番を、別のスレッドで返そうとしました");
            }
        }
    }

    impl TestKeychain {
        fn item(name: &str) -> Keychain {
            Keychain::named(&format!("{SERVICE}.test-{name}-{}", std::process::id()))
        }

        #[cfg(windows)]
        pub(crate) fn new(name: &str) -> Self {
            TestKeychain(Self::item(name), TURN.take())
        }

        #[cfg(not(windows))]
        pub(crate) fn new(name: &str) -> Self {
            TestKeychain(Self::item(name))
        }

        pub(crate) fn client(&self) -> Client {
            Client::for_test(&self.0.service)
        }
    }

    impl Drop for TestKeychain {
        fn drop(&mut self) {
            let _ = self.0.delete();
        }
    }

    #[test]
    fn accepts_only_cookie_octets() {
        assert!(valid_cookie_value("abc-123_x%2B.~"));
        for bad in ["", "a b", "a;b", "a\"b", "a\\b", "a,b", "a\nb", "あ"] {
            assert!(!valid_cookie_value(bad), "{bad:?}");
        }
    }

    // ---- テストの順番（段階 3 T5。9.1 の追加 32）。ここのテストは API を呼ばない（turn_is_held… の TestKeychain は消すだけ） ----

    #[cfg(windows)]
    fn local_lock(limit: std::time::Duration) -> &'static TurnLock {
        Box::leak(Box::new(TurnLock::new(limit)))
    }

    #[cfg(windows)]
    fn panic_text(panic: Box<dyn std::any::Any + Send>) -> String {
        match panic.downcast_ref::<&str>() {
            Some(text) => text.to_string(),
            None => panic.downcast_ref::<String>().cloned().unwrap_or_default(),
        }
    }

    #[cfg(windows)]
    #[test]
    fn turn_can_be_taken_twice_on_the_same_thread() {
        let lock = local_lock(std::time::Duration::from_secs(1));
        let _first = lock.take();
        let _second = lock.take();
        assert_eq!(lock.state.lock().unwrap().count, 2);
    }

    #[cfg(windows)]
    #[test]
    fn turn_makes_other_threads_wait_until_the_last_release() {
        use std::sync::atomic::AtomicBool;
        let lock = local_lock(std::time::Duration::from_secs(10));
        let first = lock.take();
        let second = lock.take();
        let taken: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
        let other = std::thread::spawn(move || {
            let _turn = lock.take();
            taken.store(true, Ordering::SeqCst);
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!taken.load(Ordering::SeqCst), "持っている間に取れた");
        drop(first);
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!taken.load(Ordering::SeqCst), "1 つ返しただけで取れた");
        drop(second);
        other.join().unwrap();
        assert!(taken.load(Ordering::SeqCst));
    }

    #[cfg(windows)]
    #[test]
    fn turn_gives_up_after_the_limit_with_a_clear_text() {
        let lock = local_lock(std::time::Duration::from_millis(50));
        let _held = lock.take();
        let waiting = std::thread::spawn(move || drop(lock.take())).join();
        let text = panic_text(waiting.expect_err("待ちの上限で止まらなかった"));
        assert!(text.contains("順番を"), "{text}");
    }

    // 別のスレッドで返すと、順番を返してから、分かる文で panic する
    #[cfg(windows)]
    #[test]
    fn turn_panics_when_released_on_another_thread_and_still_frees_it() {
        let lock = local_lock(std::time::Duration::from_secs(1));
        let turn = lock.take();
        let released = std::thread::spawn(move || drop(turn)).join();
        let text = panic_text(released.expect_err("別のスレッドで返しても止まらなかった"));
        assert!(text.contains("別のスレッド"), "{text}");
        // 空いている：ほかのスレッドが取れる
        std::thread::spawn(move || drop(lock.take()))
            .join()
            .unwrap();
    }

    // 別のスレッドで返すのが panic の最中なら、もう panic しない（プログラムごと止まらない）。順番は返す
    #[cfg(windows)]
    #[test]
    fn turn_does_not_panic_again_while_panicking_on_another_thread() {
        let lock = local_lock(std::time::Duration::from_secs(1));
        let turn = lock.take();
        let result = std::thread::spawn(move || {
            let _turn = turn;
            panic!("もとの panic");
        })
        .join();
        let panic = result.expect_err("panic のはず");
        assert_eq!(panic.downcast_ref::<&str>(), Some(&"もとの panic"));
        std::thread::spawn(move || drop(lock.take()))
            .join()
            .unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn turn_is_freed_when_the_holder_panics() {
        let lock = local_lock(std::time::Duration::from_secs(1));
        let result = std::panic::catch_unwind(|| {
            let _turn = lock.take();
            panic!("テストの panic");
        });
        assert!(result.is_err());
        std::thread::spawn(move || drop(lock.take()))
            .join()
            .unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn turn_works_after_the_mutex_is_poisoned() {
        let lock = local_lock(std::time::Duration::from_secs(1));
        let _ = std::panic::catch_unwind(|| {
            let _state = lock.state.lock().unwrap();
            panic!("毒にする");
        });
        assert!(lock.state.is_poisoned());
        drop(lock.take());
        std::thread::spawn(move || drop(lock.take()))
            .join()
            .unwrap();
    }

    // reports_login_and_failures の形：1 つのテストで TestKeychain を 2 つ作っても止まらず、全部を捨てるまで持つ
    #[cfg(windows)]
    #[test]
    fn turn_is_held_while_test_keychains_live() {
        let me = std::thread::current().id();
        let held = || TURN.state.lock().unwrap().holder == Some(me);
        let first = TestKeychain::new("turn-first");
        assert!(held());
        let second = TestKeychain::new("turn-second");
        assert!(held());
        drop(first);
        assert!(held(), "1 つ捨てただけで返した");
        drop(second);
        assert!(!held(), "全部捨てても返していない");
    }

    // 段階 3 T5（9.1 の追加 24）：テストのビルドでは、本物の名前の読み書き・削除は API を呼ぶ前に止まる
    #[cfg(windows)]
    #[test]
    fn refuses_the_real_name_in_test_builds() {
        fn assert_stops(operation: &str, call: impl FnOnce()) {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(call));
            let Err(panic) = result else {
                panic!("{operation} が止まらなかった");
            };
            let text = panic.downcast_ref::<String>().cloned().unwrap_or_default();
            assert!(text.contains("本物の資格情報"), "{operation}：{text}");
        }
        for real in [Keychain::named(SERVICE), Keychain::app()] {
            assert_stops("load", || drop(real.load()));
            assert_stops("exists", || drop(real.exists()));
            assert_stops("save", || drop(real.save("fake-value")));
            assert_stops("delete", || drop(real.delete()));
        }
        // 作るだけなら止まらない（submit.rs の never_reads_the_status_faster_than_once_a_second が Remote::app() を使う）
        let _ = Client::app();
    }

    // 段階 3 T5：テストのビルドでは、win.rs の層でも .test- のない名前には API を呼ばない。
    // 書く関数は、この確かめが効かなくなったときに項目が残らないように、ここでは呼ばない（同じ関数で止めている）
    #[cfg(windows)]
    #[test]
    fn win_layer_refuses_names_without_test_mark() {
        const NAME: &str = "atcoder-studio-guard-check/none";
        for call in [
            (|| {
                let _ = crate::win::credential_read(NAME);
            }) as fn(),
            || {
                let _ = crate::win::credential_exists(NAME);
            },
            || {
                let _ = crate::win::credential_delete(NAME);
            },
        ] {
            assert!(std::panic::catch_unwind(call).is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn credential_names_follow_the_plan() {
        // 名前を作るだけで、API は呼ばない
        assert_eq!(
            Keychain::app().target(),
            "io.github.ishigaki892.acdstudio/REVEL_SESSION"
        );
        // UserName は REVEL_SESSION（AtCoder の利用者名ではない）。マシンごとに保存する
        let keychain = TestKeychain::new("names");
        keychain.0.save("fake-value").unwrap();
        assert_eq!(
            crate::win::credential_user_and_persist(&keychain.0.target()),
            Some(("REVEL_SESSION".to_string(), 2))
        );
        assert_eq!(
            crate::win::credential_read(&keychain.0.target()).unwrap(),
            Some(b"fake-value".to_vec())
        );
    }

    #[cfg(windows)]
    #[test]
    fn missing_credentials_are_none_false_false() {
        let keychain = TestKeychain::new("missing");
        assert_eq!(keychain.0.load(), Ok(None));
        assert_eq!(keychain.0.exists(), Ok(false));
        assert_eq!(keychain.0.delete(), Ok(false));
    }

    #[cfg(windows)]
    #[test]
    fn other_credential_errors_are_err_with_the_number_only() {
        // 名前が長すぎる（上限は 32767 文字）：ERROR_NOT_FOUND ではないエラーになる
        let keychain = TestKeychain::new(&"x".repeat(40000));
        for (operation, result) in [
            ("load", keychain.0.load().map(|_| ())),
            ("exists", keychain.0.exists().map(|_| ())),
            ("delete", keychain.0.delete().map(|_| ())),
            ("save", keychain.0.save("fake-value")),
        ] {
            let message = result.expect_err(operation);
            assert!(
                message.starts_with("資格情報マネージャーを使えません（エラー "),
                "{operation}：{message}"
            );
            assert!(!message.contains("fake-value"), "{operation}：{message}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn unreadable_values_load_as_none() {
        let keychain = TestKeychain::new("unreadable");
        let target = keychain.0.target();
        let values: [&[u8]; 4] = [&[0xff, 0xfe, 0x00], b"a b", b"a;b", b""];
        for value in values {
            crate::win::credential_write(&target, ACCOUNT, value).unwrap();
            assert_eq!(keychain.0.load(), Ok(None), "{value:?}");
            assert_eq!(keychain.0.exists(), Ok(true), "{value:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn writes_up_to_2560_bytes_and_refuses_more_before_calling_the_api() {
        let at_limit = "a".repeat(2560);
        let over_limit = "b".repeat(2561);
        let keychain = TestKeychain::new("limit");
        // 2561 バイト：API を呼ばないので、項目はできない
        let refused = keychain.0.save(&over_limit);
        assert_eq!(
            refused,
            Err("セッションの値が長すぎるので、資格情報マネージャーに置けません（上限は 2560 バイト）".to_string())
        );
        assert_eq!(keychain.0.exists(), Ok(false));
        // 2560 バイト：書けて、同じものを読める（長いので assert! で比べる）
        assert_eq!(keychain.0.save(&at_limit), Ok(()));
        assert!(keychain.0.load().unwrap().as_deref() == Some(at_limit.as_str()));
        // 断ったときは、今の値を変えない
        assert!(keychain.0.save(&over_limit).is_err());
        assert!(keychain.0.load().unwrap().as_deref() == Some(at_limit.as_str()));
    }

    // 不正な値は API を呼ぶ前に断る（書いたあとの読み直しでも Err になるので、文と、項目がないことで見分ける）
    #[cfg(windows)]
    #[test]
    fn refuses_invalid_values_before_calling_the_api() {
        let keychain = TestKeychain::new("invalid");
        assert_eq!(
            keychain.0.save("bad value"),
            Err("セッションの値に扱えない文字があります".to_string())
        );
        assert_eq!(keychain.0.exists(), Ok(false));
    }

    #[cfg(windows)]
    #[test]
    fn windows_texts_have_no_value_and_no_keychain() {
        const MARK: &str = "secret-mark";
        let keychain = TestKeychain::new("texts");
        // 書いたものと読めたものが違う
        crate::win::credential_write(&keychain.0.target(), ACCOUNT, b"other").unwrap();
        let texts = [
            credential_error(5),
            keychain.0.save(&format!("{MARK} bad")).unwrap_err(),
            keychain.0.save(&MARK.repeat(300)).unwrap_err(),
            keychain.0.confirm_saved(MARK).unwrap_err(),
        ];
        assert_eq!(texts[0], "資格情報マネージャーを使えません（エラー 5）");
        for text in texts {
            assert!(!text.contains(MARK), "{text}");
            assert!(!text.contains("Keychain"), "{text}");
        }
    }

    #[test]
    fn saves_loads_and_deletes_in_keychain() {
        let keychain = TestKeychain::new("roundtrip");
        assert_eq!(keychain.0.load().unwrap(), None);
        assert!(!keychain.0.exists().unwrap());
        keychain.0.save("first-value%2B").unwrap();
        assert!(keychain.0.exists().unwrap());
        keychain.0.save("second-value").unwrap();
        assert_eq!(keychain.0.load().unwrap().as_deref(), Some("second-value"));
        assert!(keychain.0.save("bad\"value").is_err());
        assert!(keychain.0.delete().unwrap());
        assert!(!keychain.0.delete().unwrap());
        assert_eq!(keychain.0.load().unwrap(), None);
    }

    #[test]
    fn quotes_curl_config_values() {
        assert_eq!(quote_config("a\"b\\c\nd\te"), "\"a\\\"b\\\\c\\nd\\te\"");
    }

    #[test]
    fn explains_common_curl_failures() {
        let text = curl_failure(
            "AtCoder につながりません",
            Some(6),
            "curl: (6) Could not resolve host: atcoder.jp\n",
        );
        assert!(
            text.starts_with(
                "AtCoder につながりません：AtCoder のサーバーの名前を引けません。ネットワーク"
            ),
            "{text}"
        );
        assert!(
            text.ends_with("（curl: (6) Could not resolve host: atcoder.jp）"),
            "{text}"
        );
        assert!(curl_failure("x", Some(7), "e").contains("つながりません。ネットワーク"));
        assert!(curl_failure("x", Some(28), "e").contains("時間内に応答"));
        // 知らない終了コードは、curl の文だけ
        assert_eq!(
            curl_failure("x", Some(3), " curl: (3) bad \n"),
            "x：curl: (3) bad"
        );
        assert_eq!(curl_failure("x", None, "e"), "x：e");
    }

    #[test]
    fn splits_responses_and_reads_headers() {
        let text = "HTTP/1.1 100 Continue\r\n\r\nHTTP/2 302 \r\nlocation: /contests/x/submissions/me\r\nset-cookie: REVEL_SESSION=new-value; Path=/; HttpOnly\r\n\r\nbody";
        let (status, headers, body) = split_response(text);
        assert_eq!(status, 302);
        assert_eq!(body, "body");
        assert_eq!(
            header(&headers, "Location").next(),
            Some("/contests/x/submissions/me")
        );
        assert_eq!(rotated_session(&headers).as_deref(), Some("new-value"));
        assert_eq!(
            rotated_session(&["set-cookie: REVEL_SESSION=; Max-Age=0".to_string()]),
            None
        );
    }

    #[test]
    fn recognizes_login_redirects() {
        assert!(is_login_location(
            "/login?continue=https%3A%2F%2Fatcoder.jp%2Fhome"
        ));
        assert!(is_login_location("https://atcoder.jp/login"));
        assert!(!is_login_location("/contests/abc480/submissions/me"));
        assert!(!is_login_location("/loginx"));
    }

    #[test]
    fn reads_user_screen_name() {
        assert_eq!(
            user_screen_name("var userScreenName = \"ishigaki122\";").as_deref(),
            Some("ishigaki122")
        );
        assert_eq!(user_screen_name("var userScreenName = \"\";"), None);
        assert_eq!(user_screen_name("<html></html>"), None);
        assert_eq!(user_screen_name("var userScreenName = \"<script>\";"), None);
    }

    #[test]
    fn allows_only_atcoder_and_turnstile_in_login_window() {
        for ok in [
            "https://atcoder.jp/login",
            "https://atcoder.jp/home",
            "https://challenges.cloudflare.com/cdn-cgi/challenge-platform/x",
            "about:blank",
            "about:srcdoc",
        ] {
            assert!(allowed_in_login(&ok.parse().unwrap()), "{ok}");
        }
        for ng in [
            "http://atcoder.jp/login",
            "https://atcoder.jp.example.com/",
            "https://example.com/",
            "https://img.atcoder.jp/",
            "tauri://localhost/",
            "file:///etc/hosts",
        ] {
            assert!(!allowed_in_login(&ng.parse().unwrap()), "{ng}");
        }
    }

    // 手元の http の口に curl で送り、Cookie と送った本文（URL エンコード）と、Cookie の入れ替えを確かめる
    pub(crate) fn serve_once(response: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://127.0.0.1:{}/contests/x/submit",
            listener.local_addr().unwrap().port()
        );
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            reader.get_mut().write_all(response.as_bytes()).unwrap();
            format!("{head}\n{}", String::from_utf8(body).unwrap())
        });
        (url, handle)
    }

    #[test]
    fn posts_with_session_and_rotates_cookie() {
        let keychain = TestKeychain::new("post");
        keychain.0.save("old-session").unwrap();
        let client = keychain.client();
        let (url, server) = serve_once(
            "HTTP/1.1 302 Found\r\nLocation: /contests/x/submissions/me\r\nSet-Cookie: REVEL_SESSION=new-session; Path=/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        let source = "fn main() {\n\tprintln!(\"a\\\\b \\\" あ\");\r\n}\n";
        let reply = client
            .request(
                &url,
                &[("data.TaskScreenName", "x_a"), ("sourceCode", source)],
            )
            .unwrap();
        let request = server.join().unwrap();
        assert_eq!(reply.status, 302);
        assert_eq!(
            reply.location.as_deref(),
            Some("/contests/x/submissions/me")
        );
        assert!(
            request.contains("Cookie: REVEL_SESSION=old-session"),
            "{request}"
        );
        let body = request.rsplit('\n').next().unwrap();
        let fields: Vec<(String, String)> = body
            .split('&')
            .map(|pair| {
                let (k, v) = pair.split_once('=').unwrap();
                (decode(k), decode(v))
            })
            .collect();
        assert_eq!(
            fields,
            vec![
                ("data.TaskScreenName".into(), "x_a".into()),
                ("sourceCode".into(), source.into())
            ]
        );
        // サーバーが入れ替えた値に置き換わる
        assert_eq!(keychain.0.load().unwrap().as_deref(), Some("new-session"));
    }

    #[test]
    fn treats_login_redirect_as_logged_out() {
        let keychain = TestKeychain::new("expired");
        keychain.0.save("expired-session").unwrap();
        let client = keychain.client();
        let (url, server) = serve_once(
            "HTTP/1.1 302 Found\r\nLocation: /login?continue=x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(
            client.request(&url, &[]).err().as_deref(),
            Some(NOT_LOGGED_IN)
        );
        server.join().unwrap();
        assert!(!keychain.0.exists().unwrap());
        // 値がなければ送らない
        assert_eq!(
            client.request(&url, &[]).err().as_deref(),
            Some(NOT_LOGGED_IN)
        );
    }

    #[test]
    fn keeps_the_saved_session_until_the_window_is_logged_in() {
        let keychain = TestKeychain::new("confirm");
        keychain.0.save("saved-session").unwrap();
        let client = keychain.client();
        // ログインしていない窓の Cookie：今の値は変えない
        let (url, server) = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Length: 30\r\nConnection: close\r\n\r\nvar userScreenName = \"\";\n\n\n\n\n\n",
        );
        assert_eq!(
            confirm_session(&client, &url, "anonymous-session")
                .err()
                .as_deref(),
            Some("ログインしていません。窓でログインしてください")
        );
        let request = server.join().unwrap();
        assert!(
            request.contains("Cookie: REVEL_SESSION=anonymous-session"),
            "{request}"
        );
        assert_eq!(keychain.0.load().unwrap().as_deref(), Some("saved-session"));
        // ログインのページへ移されたときも変えない
        let (url, server) = serve_once(
            "HTTP/1.1 302 Found\r\nLocation: /login\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(
            confirm_session(&client, &url, "anonymous-session")
                .err()
                .as_deref(),
            Some("ログインしていません。窓でログインしてください")
        );
        server.join().unwrap();
        assert_eq!(keychain.0.load().unwrap().as_deref(), Some("saved-session"));
        // ログインしている：窓の値（入れ替えられたら新しい値）を書く
        let (url, server) = serve_once(
            "HTTP/1.1 200 OK\r\nSet-Cookie: REVEL_SESSION=rotated; Path=/\r\nContent-Length: 29\r\nConnection: close\r\n\r\nvar userScreenName = \"alice\";",
        );
        assert_eq!(
            confirm_session(&client, &url, "window-session").as_deref(),
            Ok("alice")
        );
        server.join().unwrap();
        assert_eq!(keychain.0.load().unwrap().as_deref(), Some("rotated"));
        // 読めない値は送らない
        assert!(confirm_session(&client, &url, "bad value;").is_err());
    }

    // 段階 3 T6：curl は決まった場所から起動する（Windows では PATH を探さない）
    #[test]
    fn starts_curl_from_the_fixed_place() {
        let curl = curl_command().unwrap();
        #[cfg(windows)]
        let expected = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join(r"System32\curl.exe");
        #[cfg(not(windows))]
        let expected = std::path::PathBuf::from("/usr/bin/curl");
        assert_eq!(std::path::Path::new(curl.get_program()), expected);
    }

    // 段階 3 T6：Cookie は引数に入れず、標準入力の設定だけで渡す。テスト用の子のプログラムを curl の代わりに起動し、
    // 受け取った引数と標準入力を記録させて見る（資格情報にも通信にも触れない）
    #[cfg(windows)]
    #[test]
    fn passes_the_cookie_only_through_standard_input() {
        const MARK: &str = "secret-mark-cookie";
        let path =
            std::env::temp_dir().join(format!("atcoder-studio-curl-record-{}", std::process::id()));
        let mut child = Command::new(crate::process::test_child_path());
        child.arg(format!("record:{}", path.display()));
        let result = run_curl(
            child,
            "=http",
            "http://127.0.0.1:9/x",
            Some(MARK),
            &[("key", "form-value")],
        );
        let recorded = std::fs::read_to_string(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_ok(), "{result:?}");
        let recorded = recorded.unwrap();
        let (args, stdin) = recorded.split_once("\n\n").unwrap();
        // 引数は、この形だけ（値を含まない）
        assert_eq!(
            args.lines().collect::<Vec<_>>(),
            [
                "--silent",
                "--show-error",
                "--include",
                "--globoff",
                "--proto",
                "=http",
                "--max-time",
                "30",
                "--max-filesize",
                "8000000",
                "--config",
                "-",
            ]
        );
        assert_eq!(
            stdin,
            format!(
                "cookie = \"REVEL_SESSION={MARK}\"\nurl = \"http://127.0.0.1:9/x\"\ndata-urlencode = \"key=form-value\"\n"
            )
        );
    }

    // 段階 3 T6：curl が失敗したときの文に、Cookie の値が出ない（本物の curl を手元の口とつながらない形で動かす）。
    // curl を起動できないときは、先の確かめで落ちるので、接続を待つ join で止まらない
    #[test]
    fn curl_failures_do_not_show_the_cookie_value() {
        const MARK: &str = "secret-mark-cookie";
        let keychain = TestKeychain::new("curl-errors");
        keychain.0.save(MARK).unwrap();
        let client = keychain.client();
        // つながらない：手元の口を開いて、すぐ閉じる
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let refused = format!(
            "http://127.0.0.1:{}/",
            listener.local_addr().unwrap().port()
        );
        drop(listener);
        // 応答を返さずに切る
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let silent = format!(
            "http://127.0.0.1:{}/",
            listener.local_addr().unwrap().port()
        );
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            let _ = stream.read(&mut buffer);
        });
        let texts = [
            client.request(&refused, &[]).err().unwrap(),
            client.request(&silent, &[]).err().unwrap(),
            // 読めない URL
            client.request("http://[/", &[]).err().unwrap(),
            // 許していないプロトコル（テストの Client は http だけ）
            client.request("https://127.0.0.1:9/", &[]).err().unwrap(),
            // ログインの確かめの経路（値を保存の前に送る）
            confirm_session(&client, &refused, MARK).unwrap_err(),
        ];
        for text in &texts {
            // 値が入っていても出力に出さないように、文そのものは出さない
            assert!(!text.contains(MARK), "エラーの文に値が出ました");
            assert!(text.contains("curl: ("), "{text}");
        }
        server.join().unwrap();
    }

    pub(crate) fn decode(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'%' => {
                    out.push(u8::from_str_radix(&text[i + 1..i + 3], 16).unwrap());
                    i += 3;
                }
                b'+' => {
                    out.push(b' ');
                    i += 1;
                }
                b => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        String::from_utf8(out).unwrap()
    }
}
