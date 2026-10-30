// 作業領域を信頼するかの確認（Phase 3b-2 作業 2。D-128）。
// 確認の画面に出す「見つかった設定ファイル」の調査と、画面の文言を作る。
//
// 調査は目安で、保護ではない。依存クレートのコード・ビルドスクリプト・手続きマクロ、環境変数、
// `include` で読まれる設定などは調べない（調べられない）。調べる数と深さに上限があり、
// 上限に達したら incomplete にして、画面に「すべては調べていません」と出す。

use std::fs;
use std::path::{Path, PathBuf};

// 調べる深さ（作業領域のルートが 0）と、見るエントリの数の上限
const MAX_DEPTH: usize = 8;
const MAX_ENTRIES: usize = 20_000;
// 画面に出す設定ファイルの数。超えた分は「ほか N 件」にまとめる
const MAX_SHOWN: usize = 20;

// cargo・rustup が、実行するプログラムを指定したり選んだりできるファイル（07 §4）。
// 作業領域の上のフォルダでは、これだけを見る
const ABOVE: &[&str] = &[
    ".cargo/config",
    ".cargo/config.toml",
    "rust-toolchain",
    "rust-toolchain.toml",
];

#[derive(Debug, Default, PartialEq)]
pub struct Inspection {
    // 見つかったファイルの表示用のパス（作業領域の中は相対パス、上のフォルダは絶対パス。ホームは ~）
    pub found: Vec<String>,
    // 上限に達して、すべては調べていない
    pub incomplete: bool,
}

fn is_config_name(name: &str) -> bool {
    matches!(name, "build.rs" | "rust-toolchain" | "rust-toolchain.toml")
}

struct Walk {
    entries: usize,
    inspection: Inspection,
}

impl Walk {
    // dir の中を調べる。リンクはたどらない（見つかったリンク自身は記録する）。
    // target と、.cargo 以外の隠しフォルダは飛ばす
    fn walk(&mut self, root: &Path, dir: &Path, depth: usize) {
        let Ok(read) = fs::read_dir(dir) else {
            // 読めないフォルダがあった
            self.inspection.incomplete = true;
            return;
        };
        let in_cargo = dir.file_name().is_some_and(|name| name == ".cargo");
        for entry in read.flatten() {
            self.entries += 1;
            if self.entries > MAX_ENTRIES {
                self.inspection.incomplete = true;
                return;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if kind.is_dir() {
                if name == "target" || (name.starts_with('.') && name != ".cargo") {
                    continue;
                }
                if depth >= MAX_DEPTH {
                    self.inspection.incomplete = true;
                    continue;
                }
                self.walk(root, &path, depth + 1);
            } else if is_config_name(&name)
                || (in_cargo && matches!(name.as_str(), "config" | "config.toml"))
            {
                // ファイルと、リンク（先は調べない）
                let shown = path.strip_prefix(root).unwrap_or(&path);
                self.inspection.found.push(display_inside(shown));
            }
        }
    }
}

// 作業領域の中のファイルの表示（作業領域のルートからの相対パス）
#[cfg(unix)]
fn display_inside(shown: &Path) -> String {
    shown.to_string_lossy().into_owned()
}

#[cfg(unix)]
fn display_above(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) => format!("~/{}", rest.to_string_lossy()),
        None => path.to_string_lossy().into_owned(),
    }
}

// Windows（移植の段階 1 C7）：確認の画面に出す形をそろえる。作業領域の中・ホームの下の相対の部分は、ファイル一覧などと同じ `/` 区切り。
// 作業領域のルートは canonicalize した `\\?\C:\…` の形で、ホーム（home_dir）には `\\?\` がないので、外してから比べ、表示にも出さない
#[cfg(windows)]
fn without_verbatim(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        // `\\?\UNC\server\share` は `\\server\share` に戻す
        Some(rest) => match rest.strip_prefix(r"UNC\") {
            Some(unc) => PathBuf::from(format!(r"\\{unc}")),
            None => PathBuf::from(rest),
        },
        None => path.to_path_buf(),
    }
}

#[cfg(windows)]
fn display_inside(shown: &Path) -> String {
    without_verbatim(shown).to_string_lossy().replace('\\', "/")
}

#[cfg(windows)]
fn display_above(path: &Path, home: Option<&Path>) -> String {
    let path = without_verbatim(path);
    let home = home.map(without_verbatim);
    match home.as_deref().and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) => format!("~/{}", rest.to_string_lossy().replace('\\', "/")),
        None => path.to_string_lossy().into_owned(),
    }
}

// root（canonicalize 済み）の中と、その上のフォルダの設定ファイルを調べる
pub fn inspect(root: &Path, home: Option<&Path>) -> Inspection {
    let mut walk = Walk {
        entries: 0,
        inspection: Inspection::default(),
    };
    walk.walk(root, root, 0);
    let mut inspection = walk.inspection;
    inspection.found.sort();
    // cargo・rustup は上のフォルダの設定も読む
    for ancestor in root.ancestors().skip(1) {
        for name in ABOVE {
            let path: PathBuf = ancestor.join(name);
            if fs::symlink_metadata(&path).is_ok() {
                inspection.found.push(display_above(&path, home));
            }
        }
    }
    inspection
}

// 確認の画面の本文
pub fn confirmation_message(name: &str, inspection: &Inspection) -> String {
    let mut list = String::new();
    for shown in inspection.found.iter().take(MAX_SHOWN) {
        list.push_str(&format!("・{shown}\n"));
    }
    if inspection.found.len() > MAX_SHOWN {
        list.push_str(&format!(
            "・ほか {} 件\n",
            inspection.found.len() - MAX_SHOWN
        ));
    }
    if list.is_empty() {
        list.push_str("（見つかりませんでした）\n");
    }
    if inspection.incomplete {
        list.push_str("調べる数・深さの上限に達した（または読めないフォルダがあった）ので、すべては調べていません。\n");
    }
    format!(
        "「{name}」のコードの実行を許可しますか。\n\
         \n\
         信頼すると、rust-analyzer（コードの解析）・実行・サンプルテストが使えるようになります。\
         これらは、このフォルダの build.rs、依存クレートのビルドスクリプトと手続きマクロ、設定ファイルで指定されたプログラムを、\
         あなたの権限で実行します。ファイルを開いただけで、保存の前にも実行されます。\n\
         \n\
         見つかった設定ファイル：\n{list}\n\
         この確認は保護ではありません。信頼したあとは、悪意のあるコードの実行を防げません。\
         設定ファイルの調査には限界があり、ここに表示されないもの（依存クレートのコード、隠しフォルダなど）からも実行されることがあります。\n\
         \n\
         信頼しない場合も、ファイルの表示と編集はできます。あとで信頼するときは、フォルダを開き直してください。"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = fs::canonicalize(std::env::temp_dir())
                .unwrap()
                .join(format!(
                    "atcoder-studio-trust-{}-{}",
                    name,
                    std::process::id()
                ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn file(&self, relative: &str) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn finds_config_files_inside_the_workspace() {
        let dir = TempDir::new("inside");
        let root = dir.0.join("ws");
        for relative in [
            "build.rs",
            "a/rust-toolchain.toml",
            "a/src/main.rs",
            "b/.cargo/config.toml",
            "b/.cargo/config",
            "b/.cargo/env",
            // config は .cargo の中だけ
            "c/config.toml",
            // 飛ばすフォルダ
            "target/debug/build.rs",
            ".git/rust-toolchain",
            ".hidden/build.rs",
        ] {
            dir.file(&format!("ws/{relative}"));
        }
        let found = inspect(&root, None).found;
        // 上のフォルダ（temp_dir の上）に設定ファイルがあれば加わるので、中のものだけを見る
        let inside: Vec<_> = found
            .iter()
            .filter(|path| !path.starts_with('/'))
            .cloned()
            .collect();
        assert_eq!(
            inside,
            vec![
                "a/rust-toolchain.toml",
                "b/.cargo/config",
                "b/.cargo/config.toml",
                "build.rs"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_links_but_reports_them() {
        let dir = TempDir::new("links");
        let root = dir.0.join("ws");
        dir.file("outside/build.rs");
        fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(dir.0.join("outside"), root.join("link")).unwrap();
        std::os::unix::fs::symlink(dir.0.join("outside/build.rs"), root.join("build.rs")).unwrap();
        let found = inspect(&root, None).found;
        let inside: Vec<_> = found
            .iter()
            .filter(|path| !path.starts_with('/'))
            .cloned()
            .collect();
        // リンクの先のフォルダの中（link/build.rs）は調べない。リンクのファイル自身は記録する
        assert_eq!(inside, vec!["build.rs"]);
    }

    #[test]
    fn finds_config_files_above_the_workspace() {
        let dir = TempDir::new("above");
        dir.file(".cargo/config.toml");
        dir.file("rust-toolchain");
        dir.file("sub/.cargo/config");
        fs::create_dir_all(dir.0.join("sub/ws")).unwrap();
        let root = dir.0.join("sub/ws");
        let found = inspect(&root, Some(&dir.0)).found;
        let above: Vec<_> = found
            .iter()
            .filter(|path| path.starts_with('~'))
            .cloned()
            .collect();
        assert_eq!(
            above,
            vec![
                "~/sub/.cargo/config",
                "~/.cargo/config.toml",
                "~/rust-toolchain"
            ]
        );
        // ホームの外なら絶対パスで表示する
        let found = inspect(&root, None).found;
        let shown = dir.0.join("rust-toolchain").to_string_lossy().into_owned();
        // Windows では canonicalize が付ける `\\?\` を表示に出さない
        #[cfg(windows)]
        let shown = shown.strip_prefix(r"\\?\").unwrap().to_string();
        assert!(found.contains(&shown), "{found:?}");
    }

    // Windows：作業領域のルートは canonicalize した `\\?\C:\…`、ホームは home_dir の `\\?\` のない形（実際の起動と同じ組み合わせ）でも、
    // ホームの下は `~/…`、中は `/` 区切りで表示する
    #[cfg(windows)]
    #[test]
    fn shows_windows_paths_like_unix_paths() {
        let dir = TempDir::new("windows");
        dir.file("rust-toolchain");
        dir.file("sub/ws/b/.cargo/config.toml");
        let root = dir.0.join("sub/ws");
        assert!(root.to_string_lossy().starts_with(r"\\?\"), "{root:?}");
        let home = without_verbatim(&dir.0);
        assert!(!home.to_string_lossy().starts_with(r"\\?\"), "{home:?}");
        let found = inspect(&root, Some(&home)).found;
        assert!(found.contains(&"b/.cargo/config.toml".to_string()), "{found:?}");
        assert!(found.contains(&"~/rust-toolchain".to_string()), "{found:?}");
        assert!(found.iter().all(|path| !path.contains(r"\\?\")), "{found:?}");
    }

    #[test]
    fn stops_at_the_depth_and_entry_limits() {
        let dir = TempDir::new("limits");
        let root = dir.0.join("ws");
        let deep = (0..=MAX_DEPTH + 1)
            .map(|i| format!("d{i}"))
            .collect::<Vec<_>>()
            .join("/");
        dir.file(&format!("ws/{deep}/build.rs"));
        let inspection = inspect(&root, None);
        assert!(inspection.incomplete);
        assert!(
            !inspection
                .found
                .iter()
                .any(|path| path.ends_with("build.rs") && !path.starts_with('/'))
        );
        // 上限の中なら incomplete ではない
        let shallow = TempDir::new("shallow");
        shallow.file("ws/a/b/build.rs");
        assert!(!inspect(&shallow.0.join("ws"), None).incomplete);
    }

    #[test]
    fn builds_the_confirmation_message() {
        let none = confirmation_message("ws", &Inspection::default());
        assert!(none.contains("「ws」"));
        assert!(none.contains("（見つかりませんでした）"));
        assert!(none.contains("この確認は保護ではありません"));
        assert!(!none.contains("すべては調べていません"));
        let many = Inspection {
            found: (0..25).map(|i| format!("p{i}/build.rs")).collect(),
            incomplete: true,
        };
        let text = confirmation_message("ws", &many);
        assert!(text.contains("・p19/build.rs"));
        assert!(!text.contains("・p20/build.rs"));
        assert!(text.contains("・ほか 5 件"));
        assert!(text.contains("すべては調べていません"));
    }
}
