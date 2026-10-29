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

    // curl で 1 回送る。設定（Cookie・URL・フォーム）は標準入力から渡す
    fn send(
        &self,
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
        let mut child = Command::new("/usr/bin/curl")
            .args([
                "--silent",
                "--show-error",
                "--include",
                // 状態の JSON の URL の `sids[]` を、curl の範囲の書き方として読ませない
                "--globoff",
                "--proto",
                self.proto,
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
    pub(crate) struct TestKeychain(pub(crate) Keychain);

    impl TestKeychain {
        pub(crate) fn new(name: &str) -> Self {
            TestKeychain(Keychain::named(&format!(
                "{SERVICE}.test-{name}-{}",
                std::process::id()
            )))
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

    #[test]
    #[cfg_attr(windows, ignore = "Windows：段階 3（Keychain を資格情報マネージャーに）と段階 5（curl.exe）で直す。docs/windows-port-plan.md 7 章")]
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
    #[cfg_attr(windows, ignore = "Windows：段階 3（Keychain を資格情報マネージャーに）と段階 5（curl.exe）で直す。docs/windows-port-plan.md 7 章")]
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
    #[cfg_attr(windows, ignore = "Windows：段階 3（Keychain を資格情報マネージャーに）と段階 5（curl.exe）で直す。docs/windows-port-plan.md 7 章")]
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
    #[cfg_attr(windows, ignore = "Windows：段階 3（Keychain を資格情報マネージャーに）と段階 5（curl.exe）で直す。docs/windows-port-plan.md 7 章")]
    fn keeps_the_saved_session_until_the_window_is_logged_in() {
        let keychain = TestKeychain::new("confirm");
        keychain.0.save("saved-session").unwrap();
        let client = keychain.client();
        // ログインしていない窓の Cookie：今の値は変えない
        let (url, server) = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Length: 30\r\nConnection: close\r\n\r\nvar userScreenName = \"\";\n\n\n\n\n\n",
        );
        assert!(confirm_session(&client, &url, "anonymous-session").is_err());
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
        assert!(confirm_session(&client, &url, "anonymous-session").is_err());
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
