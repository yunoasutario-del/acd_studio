// OS の既定のアプリで URL・フォルダを開く（Windows 移植の段階 3 T2。docs/windows-port-plan.md 7 章）
//
// - Unix：/usr/bin/open（problem.rs・submit.rs・commands.rs から移した。起動するもの・引数・エラーの文は同じ）
// - Windows：ShellExecuteW。プログラムも起動できる関数なので、URL は小文字の https:// で始まるものだけ、
//   フォルダは is_dir のものだけを渡す。渡す前の確かめと戻り値の判定は、ShellExecuteW を呼ばずにテストできるように分ける
use std::ffi::OsStr;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;
#[cfg(any(not(windows), test))]
use std::process::{Command, ExitStatus};

const BROWSER_FAILURE: &str = "ブラウザを開けません";
pub const TEMPLATES_FAILURE: &str = "テンプレートのフォルダを開けません";

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
}
