// OS の既定のアプリで URL・フォルダ・テキストファイルを開く（Windows 移植の段階 3 T2・T3。docs/windows-port-plan.md 7 章）
//
// - Unix：/usr/bin/open（problem.rs・submit.rs・commands.rs から移した。起動するもの・引数・終わるのを待つ形・エラーの文は同じ）
// - Windows の URL・フォルダ：ShellExecuteW。プログラムも起動できる関数なので、URL は小文字の https:// で始まるものだけ、
//   フォルダは is_dir のものだけを渡す。渡す前の確かめと戻り値の判定は、ShellExecuteW を呼ばずにテストできるように分ける
// - Windows のテキストファイル：VS Code があれば VS Code、なければメモ帳（9.1 の 4・追加 28）。起動したら待たない。
//   環境変数と PATH の値は引数で受け取り、絶対パスのものだけを使う（作業フォルダのプログラムを起動しないように）
use std::ffi::OsStr;
#[cfg(windows)]
use std::ffi::OsString;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;
use std::process::Command;
#[cfg(any(not(windows), test))]
use std::process::ExitStatus;
#[cfg(windows)]
use std::process::Stdio;

const BROWSER_FAILURE: &str = "ブラウザを開けません";
pub const TEMPLATES_FAILURE: &str = "テンプレートのフォルダを開けません";

// テキストファイルを開けないときの文
fn text_failure(name: &str) -> String {
    format!("{name} を開けません")
}

// 既定のブラウザで開く
#[cfg(not(windows))]
pub fn url(url: &str) -> Result<(), String> {
    finish(open_command(OsStr::new(url)).status(), BROWSER_FAILURE)
}

#[cfg(windows)]
pub fn url(url: &str) -> Result<(), String> {
    url_with(url, crate::win::shell_open)
}

// フォルダを開く（Finder・エクスプローラー）。failure は失敗したときの文
#[cfg(not(windows))]
pub fn folder(path: &Path, failure: &str) -> Result<(), String> {
    finish(open_command(path.as_os_str()).status(), failure)
}

#[cfg(windows)]
pub fn folder(path: &Path, failure: &str) -> Result<(), String> {
    folder_with(path, failure, crate::win::shell_open)
}

// テキストファイルを開く（settings.json・スニペット）。name は失敗したときの文に出す名前
#[cfg(not(windows))]
pub fn text_file(path: &Path, name: &str) -> Result<(), String> {
    wait_for(text_command(path.as_os_str()), &text_failure(name))
}

#[cfg(windows)]
pub fn text_file(path: &Path, name: &str) -> Result<(), String> {
    open_text_with(
        path,
        &text_failure(name),
        &EditorEnv::current(),
        spawn_detached,
    )
}

// ---- Unix ----

// 動かさずに、起動するものと引数を確かめられるように分ける
#[cfg(any(not(windows), test))]
fn open_command(target: &OsStr) -> Command {
    let mut command = Command::new("/usr/bin/open");
    command.arg(target);
    command
}

#[cfg(any(not(windows), test))]
fn finish(status: std::io::Result<ExitStatus>, failure: &str) -> Result<(), String> {
    let status = status.map_err(|error| format!("{failure}：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(failure.to_string())
    }
}

// 既定のテキストエディタで開く（open -t）
#[cfg(any(not(windows), test))]
fn text_command(path: &OsStr) -> Command {
    let mut command = Command::new("/usr/bin/open");
    command.arg("-t").arg(path);
    command
}

// 起動して、終わるのを待つ
#[cfg(any(not(windows), test))]
fn wait_for(mut command: Command, failure: &str) -> Result<(), String> {
    finish(command.status(), failure)
}

// ---- Windows ----

// ShellExecuteW に渡してよい URL か。IDE が組み立てる URL は小文字の https:// で始まるので、それだけを通す。
// 空白や制御文字を含むものは断る（NUL で切れて、確かめていない文字列が渡らないように）
#[cfg(windows)]
fn checked_url(url: &str) -> Result<&str, String> {
    const PREFIX: &str = "https://";
    let is_https = url.starts_with(PREFIX);
    let has_rest = url.len() > PREFIX.len();
    let is_clean = !url.chars().any(|c| c.is_whitespace() || c.is_control());
    if is_https && has_rest && is_clean {
        Ok(url)
    } else {
        Err(format!("{BROWSER_FAILURE}：https の URL ではありません"))
    }
}

// ShellExecuteW に渡してよいフォルダか。フォルダでなければ断る（プログラムを起動しないように）。`\\?\` は外して渡す
#[cfg(windows)]
fn checked_folder(path: &Path, failure: &str) -> Result<PathBuf, String> {
    if !path.is_dir() {
        return Err(format!(
            "{failure}：フォルダではありません（{}）",
            path.display()
        ));
    }
    Ok(crate::trust::without_verbatim(path))
}

// ShellExecuteW の戻り値：32 より大きければ成功、32 以下は失敗
#[cfg(windows)]
fn check_shell_result(code: isize, failure: &str) -> Result<(), String> {
    if code > 32 {
        Ok(())
    } else {
        Err(format!("{failure}：ShellExecuteW のエラー {code}"))
    }
}

#[cfg(windows)]
fn url_with(url: &str, execute: impl FnOnce(&OsStr) -> isize) -> Result<(), String> {
    let url = checked_url(url)?;
    check_shell_result(execute(OsStr::new(url)), BROWSER_FAILURE)
}

#[cfg(windows)]
fn folder_with(
    path: &Path,
    failure: &str,
    execute: impl FnOnce(&OsStr) -> isize,
) -> Result<(), String> {
    let path = checked_folder(path, failure)?;
    check_shell_result(execute(path.as_os_str()), failure)
}

// エディタを探すのに使う環境の値。テストで本物の環境変数を変えないように、引数で渡す
#[cfg(windows)]
#[derive(Default)]
struct EditorEnv {
    local_app_data: Option<OsString>,
    program_files: Option<OsString>,
    path: Option<OsString>,
    windir: Option<OsString>,
}

#[cfg(windows)]
impl EditorEnv {
    fn current() -> Self {
        EditorEnv {
            local_app_data: std::env::var_os("LOCALAPPDATA"),
            program_files: std::env::var_os("ProgramFiles"),
            path: std::env::var_os("PATH"),
            windir: std::env::var_os("WINDIR"),
        }
    }
}

// 環境の値が絶対パスのときだけ使う（空の値・相対パスは、作業フォルダのプログラムを指しうるので無視する）
#[cfg(windows)]
fn absolute(value: &Option<OsString>) -> Option<&Path> {
    value
        .as_deref()
        .map(Path::new)
        .filter(|path| path.is_absolute())
}

// VS Code の Code.exe：ユーザーごとの場所 → マシン全体の場所 → PATH の code.cmd の 2 つ上のフォルダ（9.1 の追加 28）
#[cfg(windows)]
fn find_vscode(env: &EditorEnv) -> Option<PathBuf> {
    const CODE: &str = r"Microsoft VS Code\Code.exe";
    let user = absolute(&env.local_app_data).map(|dir| dir.join("Programs").join(CODE));
    let machine = absolute(&env.program_files).map(|dir| dir.join(CODE));
    [user, machine]
        .into_iter()
        .flatten()
        .find(|path| path.is_file())
        .or_else(|| vscode_from_path(env))
}

// PATH の項目のうち、絶対パスで code.cmd があるフォルダ（<VS Code>\bin）から Code.exe を求める。
// code.cmd は cmd.exe を通すので、場所を知るためだけに使い、起動しない
#[cfg(windows)]
fn vscode_from_path(env: &EditorEnv) -> Option<PathBuf> {
    std::env::split_paths(env.path.as_ref()?)
        .filter(|dir| dir.is_absolute())
        .find_map(|dir| {
            if !dir.join("code.cmd").is_file() {
                return None;
            }
            let code = dir.parent()?.join("Code.exe");
            code.is_file().then_some(code)
        })
}

#[cfg(windows)]
fn find_notepad(env: &EditorEnv) -> Option<PathBuf> {
    let notepad = absolute(&env.windir)?.join(r"System32\notepad.exe");
    notepad.is_file().then_some(notepad)
}

// エディタを起動する Command。process::command で窓を出さない。process_group は使わない（Job に入れないので、IDE を
// 閉じてもエディタは残る）。標準入出力は null
#[cfg(windows)]
fn editor_command(program: &Path, file: &Path) -> Command {
    let mut command = crate::process::command(program);
    command
        .arg(file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

// ELECTRON_RUN_AS_NODE が残っていると、Code.exe がエディタではなく Node として動くかもしれない（推測）ので取り除く
#[cfg(windows)]
fn vscode_command(program: &Path, file: &Path) -> Command {
    let mut command = editor_command(program, file);
    command.env_remove("ELECTRON_RUN_AS_NODE");
    command
}

// 起動して、終わるのを待たない（待つと、エディタを閉じるまで IDE の主スレッドが止まる）
#[cfg(windows)]
fn spawn_detached(mut command: Command) -> std::io::Result<()> {
    command.spawn().map(drop)
}

// VS Code を試し、なければ・起動できなければメモ帳を試す。どちらもだめなら失敗の文を返す。起動の部分は spawn で差し替えられる
#[cfg(windows)]
fn open_text_with(
    path: &Path,
    failure: &str,
    env: &EditorEnv,
    mut spawn: impl FnMut(Command) -> std::io::Result<()>,
) -> Result<(), String> {
    let file = crate::trust::without_verbatim(path);
    if let Some(code) = find_vscode(env)
        && spawn(vscode_command(&code, &file)).is_ok()
    {
        return Ok(());
    }
    let Some(notepad) = find_notepad(env) else {
        return Err(format!("{failure}：メモ帳が見つかりません"));
    };
    spawn(editor_command(&notepad, &file)).map_err(|error| format!("{failure}：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(windows)]
    use std::os::windows::process::ExitStatusExt;

    // Unix の経路：open を動かさず、起動するものと引数だけを見る（Command は作るだけ）
    #[test]
    fn unix_command_is_usr_bin_open_with_only_the_target() {
        for target in [
            "https://atcoder.jp/contests/abc001/tasks/abc001_1",
            "/tmp/templates",
        ] {
            let command = open_command(OsStr::new(target));
            assert_eq!(command.get_program(), "/usr/bin/open");
            let args: Vec<OsString> = command.get_args().map(OsStr::to_os_string).collect();
            assert_eq!(args, [OsString::from(target)]);
        }
    }

    #[test]
    fn unix_failure_texts_stay_the_same() {
        assert_eq!(BROWSER_FAILURE, "ブラウザを開けません");
        assert_eq!(TEMPLATES_FAILURE, "テンプレートのフォルダを開けません");
        for failure in [BROWSER_FAILURE, TEMPLATES_FAILURE] {
            assert_eq!(finish(Ok(ExitStatus::from_raw(0)), failure), Ok(()));
            assert_eq!(
                finish(Ok(ExitStatus::from_raw(1)), failure),
                Err(failure.to_string())
            );
            let error = std::io::Error::other("理由");
            assert_eq!(finish(Err(error), failure), Err(format!("{failure}：理由")));
        }
    }

    #[test]
    fn unix_text_command_is_open_t_and_the_path() {
        let command = text_command(OsStr::new("/tmp/settings.json"));
        assert_eq!(command.get_program(), "/usr/bin/open");
        let args: Vec<OsString> = command.get_args().map(OsStr::to_os_string).collect();
        assert_eq!(
            args,
            [OsString::from("-t"), OsString::from("/tmp/settings.json")]
        );
    }

    #[test]
    fn unix_text_failure_texts_stay_the_same() {
        for (name, failure) in [
            ("settings.json", "settings.json を開けません"),
            ("snippets-rust.json", "snippets-rust.json を開けません"),
        ] {
            assert_eq!(text_failure(name), failure);
            assert_eq!(
                finish(Ok(ExitStatus::from_raw(1)), &text_failure(name)),
                Err(failure.to_string())
            );
            let error = std::io::Error::other("理由");
            assert_eq!(
                finish(Err(error), &text_failure(name)),
                Err(format!("{failure}：理由"))
            );
        }
    }

    // 待つ形の確かめに使う子。Windows は test_child、Unix は /bin/sh（open は動かさない）
    #[cfg(windows)]
    fn sleep_then_touch(marker: &Path) -> Command {
        let mut command = Command::new(crate::process::test_child_path());
        command
            .arg("sleep:300")
            .arg(format!("touch:{}", marker.display()));
        command
    }

    #[cfg(unix)]
    fn sleep_then_touch(marker: &Path) -> Command {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(format!("sleep 0.3; touch '{}'", marker.display()));
        command
    }

    #[cfg(windows)]
    fn exit_with_3() -> Command {
        let mut command = Command::new(crate::process::test_child_path());
        command.arg("exit:3");
        command
    }

    #[cfg(unix)]
    fn exit_with_3() -> Command {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg("exit 3");
        command
    }

    #[test]
    fn unix_text_open_waits_and_reports_failures() {
        let marker =
            std::env::temp_dir().join(format!("atcoder-studio-open-wait-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        // 終わるのを待つので、戻ったときには印ができている
        assert_eq!(
            wait_for(sleep_then_touch(&marker), "x を開けません"),
            Ok(())
        );
        assert!(marker.exists(), "終わるのを待っていない");
        let _ = std::fs::remove_file(&marker);
        // 0 以外で終わった
        assert_eq!(
            wait_for(exit_with_3(), "x を開けません"),
            Err("x を開けません".to_string())
        );
        // 起動できない
        let missing = std::env::temp_dir().join("atcoder-studio-open-no-such-program");
        let message = wait_for(Command::new(missing), "x を開けません").unwrap_err();
        assert!(message.starts_with("x を開けません："), "{message}");
    }
}

// Windows：ShellExecuteW は呼ばない（ブラウザやエクスプローラーを開かない）。execute に渡す閉じた関数で、何が渡るかを見る
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::cell::Cell;
    use std::ffi::OsString;

    const GOOD_URL: &str = "https://atcoder.jp/contests/abc001/tasks/abc001_1";

    // 一時フォルダ。終わったら消す
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("atcoder-studio-open-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn refuses_anything_but_https_before_calling_shell_execute() {
        for bad in [
            "",
            "http://atcoder.jp/",
            // 大文字・小文字：IDE が組み立てる URL は小文字だけなので、小文字の https:// だけを通す
            "HTTPS://atcoder.jp/",
            "Https://atcoder.jp/",
            "hTTps://atcoder.jp/",
            // https:// で始まらないもの
            "https:",
            "https:/atcoder.jp/",
            " https://atcoder.jp/",
            "atcoder.jp",
            "calc.exe",
            r"C:\Windows\System32\calc.exe",
            r"\\server\share\x.exe",
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "ms-settings:",
            // https:// のあとが空・空白や制御文字を含むもの（NUL で切れて、確かめていない文字列が渡らないように）
            "https://",
            "https://atcoder.jp/a b",
            "https://atcoder.jp/\nb",
            "https://atcoder.jp/\0.exe",
        ] {
            let called = Cell::new(false);
            let result = url_with(bad, |_| {
                called.set(true);
                33
            });
            assert!(!called.get(), "{bad:?} が ShellExecuteW に渡った");
            let message = result.expect_err(bad);
            assert!(message.starts_with("ブラウザを開けません："), "{message}");
        }
    }

    #[test]
    fn passes_https_urls_as_they_are() {
        let seen = std::cell::RefCell::new(None);
        let result = url_with(GOOD_URL, |target| {
            *seen.borrow_mut() = Some(target.to_os_string());
            33
        });
        assert_eq!(result, Ok(()));
        assert_eq!(seen.into_inner(), Some(OsString::from(GOOD_URL)));
    }

    #[test]
    fn treats_shell_execute_results_of_32_or_less_as_failures() {
        for code in [33, 34, 42, 1000] {
            assert_eq!(check_shell_result(code, "開けません"), Ok(()), "{code}");
        }
        // 0：メモリ不足など、2：ファイルがない、31：関連付けがない、32：境目。負の値も成功にしない
        for code in [32, 31, 2, 1, 0, -1] {
            assert_eq!(
                check_shell_result(code, "開けません"),
                Err(format!("開けません：ShellExecuteW のエラー {code}")),
                "{code}"
            );
        }
        // 戻り値が url_with・folder_with の結果になる
        assert_eq!(
            url_with(GOOD_URL, |_| 2),
            Err("ブラウザを開けません：ShellExecuteW のエラー 2".to_string())
        );
        let dir = TempDir::new("result");
        assert_eq!(
            folder_with(&dir.0, TEMPLATES_FAILURE, |_| 31),
            Err("テンプレートのフォルダを開けません：ShellExecuteW のエラー 31".to_string())
        );
    }

    #[test]
    fn opens_only_folders() {
        let dir = TempDir::new("folders");
        let file = dir.0.join("x.exe");
        std::fs::write(&file, "").unwrap();
        for bad in [file, dir.0.join("none")] {
            let called = Cell::new(false);
            let result = folder_with(&bad, TEMPLATES_FAILURE, |_| {
                called.set(true);
                33
            });
            assert!(!called.get(), "{bad:?} が ShellExecuteW に渡った");
            let message = result.unwrap_err();
            assert!(
                message.starts_with("テンプレートのフォルダを開けません："),
                "{message}"
            );
        }
        let seen = std::cell::RefCell::new(None);
        let result = folder_with(&dir.0, TEMPLATES_FAILURE, |target| {
            *seen.borrow_mut() = Some(target.to_os_string());
            33
        });
        assert_eq!(result, Ok(()));
        assert_eq!(seen.into_inner(), Some(dir.0.as_os_str().to_os_string()));
    }

    #[test]
    fn removes_the_verbatim_prefix_before_opening_a_folder() {
        let dir = TempDir::new("verbatim");
        // canonicalize は `\\?\C:\…` の形を返す
        let verbatim = std::fs::canonicalize(&dir.0).unwrap();
        let text = verbatim.to_string_lossy().into_owned();
        let plain = text.strip_prefix(r"\\?\").expect(&text).to_string();
        let seen = std::cell::RefCell::new(None);
        let result = folder_with(&verbatim, TEMPLATES_FAILURE, |target| {
            *seen.borrow_mut() = Some(target.to_os_string());
            33
        });
        assert_eq!(result, Ok(()));
        assert_eq!(seen.into_inner(), Some(OsString::from(plain)));
        // `\\?\UNC\server\share` は `\\server\share` に戻す
        assert_eq!(
            crate::trust::without_verbatim(Path::new(r"\\?\UNC\server\share\x")),
            Path::new(r"\\server\share\x")
        );
    }

    // ---- テキストファイル（T3）。本物の環境変数は変えない。VS Code もメモ帳も起動しない ----

    fn fake_file(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    fn user_code(root: &Path) -> PathBuf {
        root.join(r"Programs\Microsoft VS Code\Code.exe")
    }

    fn machine_code(root: &Path) -> PathBuf {
        root.join(r"Microsoft VS Code\Code.exe")
    }

    fn with_local_app_data(value: impl Into<OsString>) -> EditorEnv {
        EditorEnv {
            local_app_data: Some(value.into()),
            ..EditorEnv::default()
        }
    }

    fn with_program_files(value: impl Into<OsString>) -> EditorEnv {
        EditorEnv {
            program_files: Some(value.into()),
            ..EditorEnv::default()
        }
    }

    fn with_path(value: impl Into<OsString>) -> EditorEnv {
        EditorEnv {
            path: Some(value.into()),
            ..EditorEnv::default()
        }
    }

    fn with_windir(value: impl Into<OsString>) -> EditorEnv {
        EditorEnv {
            windir: Some(value.into()),
            ..EditorEnv::default()
        }
    }

    // 作業フォルダ（cargo test では src-tauri）の下の、相対パスで指せるフォルダ。終わったら消す
    struct RelativeDir(PathBuf);

    impl RelativeDir {
        fn new(name: &str) -> Self {
            let path =
                PathBuf::from("target").join(format!("open-test-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            RelativeDir(path)
        }
    }

    impl Drop for RelativeDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn finds_vscode_in_the_decided_order() {
        let dir = TempDir::new("order");
        let local = dir.0.join("local");
        let files = dir.0.join("files");
        let install = dir.0.join("install");
        for path in [
            user_code(&local),
            machine_code(&files),
            install.join(r"bin\code.cmd"),
            install.join("Code.exe"),
        ] {
            fake_file(&path);
        }
        let mut env = EditorEnv {
            local_app_data: Some(local.clone().into()),
            program_files: Some(files.clone().into()),
            path: Some(install.join("bin").into()),
            windir: None,
        };
        assert_eq!(find_vscode(&env), Some(user_code(&local)));
        env.local_app_data = None;
        assert_eq!(find_vscode(&env), Some(machine_code(&files)));
        env.program_files = None;
        assert_eq!(find_vscode(&env), Some(install.join("Code.exe")));
        env.path = None;
        assert_eq!(find_vscode(&env), None);
    }

    #[test]
    fn ignores_empty_and_relative_values() {
        // 相対パスのまま使うと、作業フォルダの下の偽のプログラムを指すように作る
        let relative = RelativeDir::new("relative");
        let root = &relative.0;
        let bin = root.join("bin");
        for path in [
            user_code(root),
            machine_code(root),
            bin.join("code.cmd"),
            root.join("Code.exe"),
            root.join(r"System32\notepad.exe"),
        ] {
            fake_file(&path);
            assert!(path.is_relative() && path.is_file(), "{path:?}");
        }
        assert_eq!(find_vscode(&with_local_app_data(root)), None);
        assert_eq!(find_vscode(&with_program_files(root)), None);
        assert_eq!(find_vscode(&with_path(&bin)), None);
        assert_eq!(find_notepad(&with_windir(root)), None);
        // PATH の空の項目・相対の項目は飛ばし、絶対パスの項目は使う
        let dir = TempDir::new("relative-path");
        fake_file(&dir.0.join(r"bin\code.cmd"));
        fake_file(&dir.0.join("Code.exe"));
        let mut mixed = OsString::from(";.;");
        mixed.push(&bin);
        mixed.push(";");
        mixed.push(dir.0.join("bin"));
        assert_eq!(find_vscode(&with_path(mixed)), Some(dir.0.join("Code.exe")));
        // 空の値
        assert_eq!(find_vscode(&with_local_app_data("")), None);
        assert_eq!(find_vscode(&with_program_files("")), None);
        assert_eq!(find_vscode(&with_path("")), None);
        assert_eq!(find_vscode(&with_path(";;")), None);
        assert_eq!(find_notepad(&with_windir("")), None);
        // ドライブだけ・ルートだけの形も、絶対パスではない
        for value in ["C:foo", r"\foo", "foo", "."] {
            assert_eq!(absolute(&Some(OsString::from(value))), None, "{value}");
        }
        // 値がないとき
        assert_eq!(find_vscode(&EditorEnv::default()), None);
        assert_eq!(find_notepad(&EditorEnv::default()), None);
    }

    #[test]
    fn uses_only_files_and_never_code_cmd_itself() {
        let dir = TempDir::new("files");
        // Code.exe・notepad.exe という名前のフォルダは使わない
        let local = dir.0.join("local");
        std::fs::create_dir_all(user_code(&local)).unwrap();
        assert_eq!(find_vscode(&with_local_app_data(&local)), None);
        let files = dir.0.join("files");
        std::fs::create_dir_all(machine_code(&files)).unwrap();
        assert_eq!(find_vscode(&with_program_files(&files)), None);
        let windir = dir.0.join("windows");
        std::fs::create_dir_all(windir.join(r"System32\notepad.exe")).unwrap();
        assert_eq!(find_notepad(&with_windir(&windir)), None);
        // code.cmd があっても、Code.exe がなければ使わない（code.cmd そのものを返さない）
        let only_cmd = dir.0.join("only-cmd");
        fake_file(&only_cmd.join(r"bin\code.cmd"));
        assert_eq!(find_vscode(&with_path(only_cmd.join("bin"))), None);
        // Code.exe がフォルダでも使わない
        std::fs::create_dir_all(only_cmd.join("Code.exe")).unwrap();
        assert_eq!(find_vscode(&with_path(only_cmd.join("bin"))), None);
        // code.cmd がなければ、隣の Code.exe があっても PATH からは使わない
        let only_code = dir.0.join("only-code");
        fake_file(&only_code.join("Code.exe"));
        std::fs::create_dir_all(only_code.join("bin")).unwrap();
        assert_eq!(find_vscode(&with_path(only_code.join("bin"))), None);
        // code.cmd という名前のフォルダは使わない
        std::fs::create_dir_all(only_code.join(r"bin\code.cmd")).unwrap();
        assert_eq!(find_vscode(&with_path(only_code.join("bin"))), None);
        // 使えない項目を飛ばして、次の項目を使う
        let good = dir.0.join("good");
        fake_file(&good.join(r"bin\code.cmd"));
        fake_file(&good.join("Code.exe"));
        let joined = std::env::join_paths([only_code.join("bin"), good.join("bin")]).unwrap();
        assert_eq!(find_vscode(&with_path(joined)), Some(good.join("Code.exe")));
        // 見つけたものは本物のファイル
        let windir = dir.0.join("windows2");
        fake_file(&windir.join(r"System32\notepad.exe"));
        assert_eq!(
            find_notepad(&with_windir(&windir)),
            Some(windir.join(r"System32\notepad.exe"))
        );
    }

    // spawn に渡された Command の中身
    #[derive(Debug, PartialEq)]
    struct Launch {
        program: OsString,
        args: Vec<OsString>,
        removes_electron: bool,
    }

    fn launch_of(command: &Command) -> Launch {
        Launch {
            program: command.get_program().to_os_string(),
            args: command.get_args().map(OsStr::to_os_string).collect(),
            removes_electron: command.get_envs().any(|(key, value)| {
                key.eq_ignore_ascii_case("ELECTRON_RUN_AS_NODE") && value.is_none()
            }),
        }
    }

    // VS Code とメモ帳がある環境（どちらも空の偽のファイルで、起動しない）
    fn editors(dir: &TempDir) -> (EditorEnv, PathBuf, PathBuf) {
        let local = dir.0.join("local");
        let windir = dir.0.join("windows");
        let code = user_code(&local);
        let notepad = windir.join(r"System32\notepad.exe");
        fake_file(&code);
        fake_file(&notepad);
        let env = EditorEnv {
            local_app_data: Some(local.into()),
            windir: Some(windir.into()),
            ..EditorEnv::default()
        };
        (env, code, notepad)
    }

    const SETTINGS: &str = r"C:\app dir\settings.json";

    #[test]
    fn opens_with_vscode_and_removes_electron_run_as_node() {
        let dir = TempDir::new("vscode");
        let (env, code, _) = editors(&dir);
        let mut seen = Vec::new();
        let result = open_text_with(
            Path::new(SETTINGS),
            "settings.json を開けません",
            &env,
            |command| {
                seen.push(launch_of(&command));
                Ok(())
            },
        );
        assert_eq!(result, Ok(()));
        assert_eq!(
            seen,
            [Launch {
                program: code.into(),
                args: vec![SETTINGS.into()],
                removes_electron: true,
            }]
        );
    }

    #[test]
    fn opens_with_notepad_when_vscode_is_missing() {
        let dir = TempDir::new("notepad");
        let (mut env, _, notepad) = editors(&dir);
        env.local_app_data = None;
        let mut seen = Vec::new();
        let result = open_text_with(Path::new(SETTINGS), "x", &env, |command| {
            seen.push(launch_of(&command));
            Ok(())
        });
        assert_eq!(result, Ok(()));
        assert_eq!(
            seen,
            [Launch {
                program: notepad.into(),
                args: vec![SETTINGS.into()],
                removes_electron: false,
            }]
        );
    }

    #[test]
    fn falls_back_to_notepad_and_reports_the_same_form_of_failure() {
        let dir = TempDir::new("fallback");
        let (env, code, notepad) = editors(&dir);
        // VS Code を起動できなければ、メモ帳を試す
        let mut seen = Vec::new();
        let result = open_text_with(Path::new(SETTINGS), "x", &env, |command| {
            let launch = launch_of(&command);
            let failed = launch.program == code.as_os_str();
            seen.push(launch.program);
            if failed {
                Err(std::io::Error::other("VS Code の理由"))
            } else {
                Ok(())
            }
        });
        assert_eq!(result, Ok(()));
        assert_eq!(
            seen,
            [
                code.clone().into_os_string(),
                notepad.clone().into_os_string()
            ]
        );
        // メモ帳も起動できなければ、今と同じ形の文（名前 を開けません：理由）
        for name in ["settings.json", "snippets-rust.json"] {
            let result = open_text_with(Path::new(SETTINGS), &text_failure(name), &env, |_| {
                Err(std::io::Error::other("理由"))
            });
            assert_eq!(result, Err(format!("{name} を開けません：理由")));
        }
        // どちらも見つからない
        let mut called = false;
        let result = open_text_with(
            Path::new(SETTINGS),
            &text_failure("settings.json"),
            &EditorEnv::default(),
            |_| {
                called = true;
                Ok(())
            },
        );
        assert!(!called);
        assert_eq!(
            result,
            Err("settings.json を開けません：メモ帳が見つかりません".to_string())
        );
    }

    #[test]
    fn removes_the_verbatim_prefix_before_opening_a_text_file() {
        let dir = TempDir::new("text-verbatim");
        let (env, _, _) = editors(&dir);
        let mut seen = Vec::new();
        let result = open_text_with(
            Path::new(r"\\?\C:\app\settings.json"),
            "x",
            &env,
            |command| {
                seen.push(launch_of(&command).args);
                Ok(())
            },
        );
        assert_eq!(result, Ok(()));
        assert_eq!(seen, [vec![OsString::from(r"C:\app\settings.json")]]);
    }

    // 本物の起動（test_child。VS Code もメモ帳も起動しない）：待たないこと
    #[test]
    fn spawn_detached_does_not_wait() {
        let dir = TempDir::new("detached");
        let marker = dir.0.join("done");
        let mut command = Command::new(crate::process::test_child_path());
        command
            .arg("sleep:3000")
            .arg(format!("touch:{}", marker.display()))
            // テストが終わったあとに子が出す文が、ログに混ざらないように
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        assert!(spawn_detached(command).is_ok());
        // 待っていれば、戻ったときには印ができている
        assert!(!marker.exists(), "終わるのを待った");
    }

    // 本物の起動（test_child）：process_group を使わず（使うと止まったまま動かない）、標準入出力が null。
    // cargo test 自身の標準入力が null のときは、標準入力を null にしなくても null に見える
    #[test]
    fn editor_command_starts_at_once_with_null_stdio() {
        let dir = TempDir::new("stdio");
        let report = dir.0.join("kinds.txt");
        // editor_command はファイルのパスを 1 つ渡す形なので、そこに test_child の手順を置く
        let step = PathBuf::from(format!("stdio-kinds:{}", report.display()));
        let command = editor_command(&crate::process::test_child_path(), &step);
        assert!(spawn_detached(command).is_ok());
        let started = std::time::Instant::now();
        let kinds = loop {
            let text = std::fs::read_to_string(&report).unwrap_or_default();
            if !text.is_empty() {
                break text;
            }
            assert!(
                started.elapsed() < std::time::Duration::from_secs(20),
                "子が動いていない（止まったまま）"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert_eq!(kinds, "null null null");
    }
}
