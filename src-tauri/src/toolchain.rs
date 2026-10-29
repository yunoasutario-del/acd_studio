// 利用者の解答をビルドするツールチェーンの呼び出し（Phase 2 作業 2）
//
// - `cargo` の場所を決める（D-098）
// - 利用者のビルドに渡す環境変数から、IDE 自身の起動で入った変数を外す（D-104）
// - 開いているファイルから、実行の対象のバイナリを決める（D-088）

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::language::Language;

// 利用者のホームのフォルダ。Unix は HOME（これまでどおり）。Windows では HOME はふつう設定されていないので、
// std::env::home_dir（USERPROFILE）を使う（Windows 移植の段階 1 C6）
#[cfg(unix)]
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(windows)]
pub fn home_dir() -> Option<PathBuf> {
    std::env::home_dir()
}

// 実行ファイルの名前。Windows では `.exe` を付ける（Unix ではそのまま。Windows 移植の段階 1 C6）
pub fn exe_name(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

// cargo を探す。PATH で見つからなければ ~/.cargo/bin/cargo（rustup）を使う（D-098）。
// それもなければ、Homebrew の置き場所（Apple Silicon の /opt/homebrew/bin、Intel の /usr/local/bin）を探す
// （Phase 10 作業 13）。Finder から起動したアプリは、シェルの PATH を引き継がないと思われる（推測・未確認）
const HOMEBREW_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];

pub fn find_cargo() -> Option<PathBuf> {
    let home = home_dir();
    let homebrew: Vec<PathBuf> = HOMEBREW_DIRS.iter().map(PathBuf::from).collect();
    find_cargo_in(
        std::env::var_os("PATH").as_deref(),
        home.as_deref(),
        &homebrew,
    )
}

// path は PATH の値、home は HOME、homebrew は最後に探すフォルダ（テストで差し替える）
fn find_cargo_in(
    path: Option<&OsStr>,
    home: Option<&Path>,
    homebrew: &[PathBuf],
) -> Option<PathBuf> {
    let from_path = path.and_then(|paths| {
        std::env::split_paths(paths)
            .map(|dir| dir.join(exe_name("cargo")))
            .find(|path| path.is_file())
    });
    from_path
        .or_else(|| {
            let path = home?.join(".cargo/bin").join(exe_name("cargo"));
            path.is_file().then_some(path)
        })
        .or_else(|| {
            homebrew
                .iter()
                .map(|dir| dir.join(exe_name("cargo")))
                .find(|path| path.is_file())
        })
}

// IDE 自身を `cargo run`（`npm run tauri dev`）で起動したときに入る環境変数のうち、
// 利用者のビルド・プログラムに引き継ぐと、IDE 開発用のツールチェーンや IDE のパッケージの情報が使われてしまうもの（D-104）。
// RUSTUP_TOOLCHAIN は利用者の rust-toolchain.toml よりも優先される。
// 一覧は docs/phase2/02-target-toolchain.md §2 で、cargo run が子に加えた変数を調べて決めた
const REMOVED_VARS: &[&str] = &[
    "RUSTUP_TOOLCHAIN",
    "RUSTUP_TOOLCHAIN_SOURCE",
    "RUST_RECURSION_COUNT",
    "CARGO",
    "CARGO_MANIFEST_DIR",
    "CARGO_MANIFEST_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH",
];

fn is_removed(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    REMOVED_VARS.contains(&name) || name.starts_with("CARGO_PKG_")
}

// 利用者のビルド（cargo）とプログラム（D-089）に渡す環境から、D-104 の変数を外す。
// RUSTFLAGS・CARGO_TARGET_DIR など、利用者が設定しうる変数は外さない（D-097）
pub fn clear_ide_env(command: &mut Command) {
    for (name, _) in std::env::vars_os() {
        if is_removed(&name) {
            command.env_remove(&name);
        }
    }
}

// 利用者のプロジェクトで cargo・rustc を動かすコマンドを作る。D-104 の変数を外したうえで、
// rustup が手元にない版を自動で取得しないように RUSTUP_AUTO_INSTALL=0 を付ける（D-107）。
// 利用者が自分で RUSTUP_AUTO_INSTALL を設定していれば、その値を使う
pub fn cargo_command(program: &Path, dir: &Path) -> Command {
    let mut command = crate::process::command(program);
    command.current_dir(dir);
    clear_ide_env(&mut command);
    if std::env::var_os("RUSTUP_AUTO_INSTALL").is_none() {
        command.env("RUSTUP_AUTO_INSTALL", "0");
    }
    command
}

// cargo・rustc の失敗の説明に、いつも添える文（D-107。エラーの文字列からは判断しない）
pub const TOOLCHAIN_HINT: &str = "rust-toolchain.toml で手元にない版を指定している場合、IDE はその版を取得しません。ターミナルで rustup toolchain install <版> を実行してください。";

// AtCoder のジャッジの版（D-097）。値を変えるときは、03a の確認をやり直してから、ここと D-097 を書き換える
pub const JUDGE_VERSION: &str = "1.89.0";
pub const JUDGE_NOTE: &str = "rustc 1.89.0、edition 2024（AtCoder の 2025/10 版の言語一覧。2026/06/16 時点の確認。その後に変わっている可能性があります）";

// cargo と同じ場所の rustc（rustup のプロキシ）。なければ PATH の rustc
fn rustc_beside(cargo: &Path) -> PathBuf {
    cargo
        .parent()
        .map(|dir| dir.join(exe_name("rustc")))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("rustc"))
}

// パッケージのディレクトリで解決される rustc と cargo の版（D-097）。失敗したら、その出力を返す
pub fn versions(cargo: &Path, dir: &Path) -> (Result<String, String>, Result<String, String>) {
    let version = |program: &Path| {
        let output = cargo_command(program, dir)
            .arg("--version")
            .output()
            .map_err(|error| format!("起動できません：{error}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        }
    };
    (version(&rustc_beside(cargo)), version(cargo))
}

// パッケージのディレクトリで解決される rustc の sysroot と、rustc のコミットの番号（`rustc -vV` の commit-hash）。
// デバッグで、rustc に付属する LLDB の表示スクリプトと、標準ライブラリのソースの場所を決めるのに使う（Phase 5 作業 1。D-146）
pub struct RustcPaths {
    pub sysroot: PathBuf,
    // 取れなければ None
    pub commit_hash: Option<String>,
}

pub fn rustc_paths(cargo: &Path, dir: &Path) -> Result<RustcPaths, String> {
    let rustc = rustc_beside(cargo);
    let run = |args: &[&str]| {
        let output = cargo_command(&rustc, dir)
            .args(args)
            .output()
            .map_err(|error| format!("rustc を起動できません：{error}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        }
    };
    let sysroot = PathBuf::from(run(&["--print", "sysroot"])?.trim());
    let commit_hash = run(&["-vV"]).ok().and_then(|text| {
        text.lines()
            .find_map(|line| line.strip_prefix("commit-hash: "))
            .map(|hash| hash.trim().to_string())
            .filter(|hash| !hash.is_empty() && hash.bytes().all(|b| b.is_ascii_hexdigit()))
    });
    Ok(RustcPaths {
        sysroot,
        commit_hash,
    })
}

// rustc の版の文字列（例：rustc 1.89.0 (29483883e 2025-08-04)）がジャッジと同じか
pub fn same_as_judge(rustc_version: &str) -> bool {
    rustc_version.split_whitespace().nth(1) == Some(JUDGE_VERSION)
}

// 実行の対象（D-088）。package は作業領域のルートからのパッケージのディレクトリ（ルートなら ""）
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct Target {
    pub package: String,
    pub bin: String,
    // 言語（D-204）。画面から言語のない対象が来たときは Rust とする
    #[serde(default)]
    pub language: Language,
}

// `cargo metadata` の出力のうち、使うところだけ
#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    manifest_path: PathBuf,
    targets: Vec<CargoTarget>,
}

#[derive(Deserialize)]
struct CargoTarget {
    name: String,
    kind: Vec<String>,
    src_path: PathBuf,
}

// file（canonicalize 済みの絶対パス）から、対象のバイナリを決める。
// manifest_dir は file を含む Cargo パッケージのディレクトリ。次の順で決め、どれでもなければ None。
// 1. バイナリのソースそのもの
// 2. `test/<バイナリ名>/` の中のファイル
// 3. バイナリが 1 つだけのパッケージの `test/` の中のファイル（D-106。複数なら、どのバイナリか決めない）
fn target_in_package(package: &Package, manifest_dir: &Path, file: &Path) -> Option<String> {
    let bins: Vec<&CargoTarget> = package
        .targets
        .iter()
        .filter(|target| target.kind.iter().any(|kind| kind == "bin"))
        .collect();
    // src_path はリンクや `..` を含みうるので、解決してから比べる
    if let Some(bin) = bins
        .iter()
        .find(|bin| bin.src_path.canonicalize().is_ok_and(|path| path == file))
    {
        return Some(bin.name.clone());
    }
    let test_dir = manifest_dir.join("test");
    if let Some(bin) = bins
        .iter()
        .find(|bin| file.starts_with(test_dir.join(&bin.name)))
    {
        return Some(bin.name.clone());
    }
    match bins.as_slice() {
        [only] if file.starts_with(&test_dir) => Some(only.name.clone()),
        _ => None,
    }
}

// `cargo metadata` の出力（JSON）から、file の実行の対象を決める
fn target_from_metadata(json: &str, root: &Path, file: &Path) -> Result<Option<Target>, String> {
    let metadata: Metadata =
        serde_json::from_str(json).map_err(|_| "cargo metadata の出力を読めません".to_string())?;
    // file を含むパッケージのうち、いちばん深いもの
    let package = metadata
        .packages
        .iter()
        .filter_map(|package| {
            let dir = package.manifest_path.parent()?.canonicalize().ok()?;
            file.starts_with(&dir).then_some((package, dir))
        })
        .max_by_key(|(_, dir)| dir.components().count());
    let Some((package, dir)) = package else {
        return Ok(None);
    };
    let Ok(relative) = dir.strip_prefix(root) else {
        // パッケージが作業領域の外にある
        return Ok(None);
    };
    Ok(target_in_package(package, &dir, file).map(|bin| Target {
        package: relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
        bin,
        language: Language::Rust,
    }))
}

// パッケージのバイナリの名前の一覧（D-106 の「バイナリの数」に使う）
pub fn bin_names(json: &str, dir: &Path) -> Result<Vec<String>, String> {
    target_names(json, dir, "bin")
}

// パッケージの example の名前の一覧（ランダムテストの生成器・愚直解を探すのに使う。D-143）
pub fn example_names(json: &str, dir: &Path) -> Result<Vec<String>, String> {
    target_names(json, dir, "example")
}

// パッケージの、kind が指定のもの（`bin`・`example`）の名前の一覧
fn target_names(json: &str, dir: &Path, wanted: &str) -> Result<Vec<String>, String> {
    let metadata: Metadata =
        serde_json::from_str(json).map_err(|_| "cargo metadata の出力を読めません".to_string())?;
    let package = metadata
        .packages
        .iter()
        .find(|package| {
            package
                .manifest_path
                .parent()
                .and_then(|parent| parent.canonicalize().ok())
                .is_some_and(|parent| parent == dir)
        })
        .ok_or_else(|| "Cargo パッケージが見つかりません".to_string())?;
    Ok(package
        .targets
        .iter()
        .filter(|target| target.kind.iter().any(|kind| kind == wanted))
        .map(|target| target.name.clone())
        .collect())
}

// テストケースのフォルダ（D-106）。dir はパッケージのディレクトリ（canonicalize 済み）
#[derive(Debug, PartialEq, Eq)]
pub struct TestDir {
    // 使うフォルダ。なければ None
    pub dir: Option<PathBuf>,
    // test/ の直下のファイルを使わないときの知らせ（D-106 の 3）
    pub notice: Option<String>,
}

// test/ の直下に、テストケースのファイル（sample-N・random-N・interactive-N の .in・.out、input.txt、compare.json）があるか
fn has_direct_files(test: &Path) -> bool {
    std::fs::read_dir(test).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            entry.path().is_file()
                && (name == "input.txt"
                    || name == "compare.json"
                    || ((name.starts_with("sample-")
                        || name.starts_with("random-")
                        || name.starts_with("interactive-"))
                        && (name.ends_with(".in") || name.ends_with(".out"))))
        })
    })
}

pub fn test_dir(dir: &Path, bins: &[String], bin: &str) -> TestDir {
    let test = dir.join("test");
    let own = test.join(bin);
    let direct = has_direct_files(&test);
    if own.is_dir() {
        let notice = direct.then(|| {
            if bins.len() > 1 {
                format!("バイナリが複数あるため test/ の直下のファイルは使いません。test/{bin}/ に移してください")
            } else {
                format!("test/{bin}/ を使うので、test/ の直下のファイルは使いません")
            }
        });
        return TestDir {
            dir: Some(own),
            notice,
        };
    }
    if bins.len() == 1 {
        return TestDir {
            dir: test.is_dir().then_some(test),
            notice: None,
        };
    }
    TestDir {
        dir: None,
        notice: direct.then(|| {
            format!("バイナリが複数あるため test/ の直下のファイルは使いません。test/{bin}/ に移してください")
        }),
    }
}

// パッケージのディレクトリで cargo metadata を実行し、出力（JSON）を返す
pub fn metadata_json(cargo: &Path, dir: &Path) -> Result<String, String> {
    // 依存は解決しない（--no-deps）。ネットワークにはつながない（--offline）
    let output = cargo_command(cargo, dir)
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--offline",
        ])
        .output()
        .map_err(|_| "cargo を起動できません".to_string())?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata が失敗しました：{}\n{TOOLCHAIN_HINT}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

// file を含むいちばん近い Cargo.toml のあるディレクトリ（root の中だけを探す）
pub(crate) fn nearest_manifest_dir(root: &Path, file: &Path) -> Option<PathBuf> {
    file.ancestors()
        .skip(1)
        .take_while(|dir| dir.starts_with(root))
        .find(|dir| dir.join("Cargo.toml").is_file())
        .map(Path::to_path_buf)
}

// 開いているファイル（canonicalize 済み）から、実行の対象を決める（D-088 の 1・2）。
// file からいちばん近い Cargo.toml・main.cpp・main.py のあるディレクトリ（root の中だけ）で、Cargo.toml なら Rust、
// main.cpp なら C++、main.py だけなら Python の対象にする（D-204・D-220）。
// 決まらなければ None（フロントエンドが直前の対象を使い続ける。D-088 の 3・4）
pub fn resolve_target(root: &Path, file: &Path) -> Result<Option<Target>, String> {
    match nearest_package(root, file) {
        None => Ok(None),
        Some((dir, Language::Rust)) => {
            let cargo = find_cargo().ok_or_else(|| "cargo が見つかりません".to_string())?;
            target_from_metadata(&metadata_json(&cargo, dir)?, root, file)
        }
        Some((dir, Language::Cpp)) => Ok(crate::cpp::target_in_package(root, dir, file)),
        Some((dir, Language::Python)) => Ok(crate::python::target_in_package(root, dir, file)),
    }
}

// file のいちばん近くのパッケージと、その言語（作業領域の中だけ。同じフォルダに 2 つ以上あれば Rust、C++、Python の順。
// D-204・D-220）
pub fn nearest_package<'a>(root: &Path, file: &'a Path) -> Option<(&'a Path, Language)> {
    file.ancestors()
        .skip(1)
        .take_while(|dir| dir.starts_with(root))
        .find_map(|dir| package_language(dir).map(|language| (dir, language)))
}

// dir がパッケージなら、その言語（しるしが 2 つ以上あれば Rust、C++、Python の順）
pub fn package_language(dir: &Path) -> Option<Language> {
    [Language::Rust, Language::Cpp, Language::Python]
        .into_iter()
        .find(|language| language.is_package(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_cpp_packages() {
        let base =
            std::env::temp_dir().join(format!("atcoder-studio-resolve-cpp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("abc480/abc480_a");
        std::fs::create_dir_all(dir.join("test")).unwrap();
        std::fs::write(dir.join("main.cpp"), "").unwrap();
        std::fs::write(dir.join("test/sample-1.in"), "").unwrap();
        let root = base.canonicalize().unwrap();
        let dir = root.join("abc480/abc480_a");
        let target = resolve_target(&root, &dir.join("main.cpp"))
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                target.package.as_str(),
                target.bin.as_str(),
                target.language
            ),
            ("abc480/abc480_a", "main", Language::Cpp)
        );
        assert_eq!(
            resolve_target(&root, &dir.join("test/sample-1.in")).unwrap(),
            Some(target)
        );
        // パッケージの外のファイルからは決めない
        std::fs::write(root.join("abc480/notes.md"), "").unwrap();
        assert_eq!(
            resolve_target(&root, &root.join("abc480/notes.md")).unwrap(),
            None
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn finds_cargo_in_path_then_rustup_then_homebrew() {
        let base =
            std::env::temp_dir().join(format!("atcoder-studio-find-cargo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (path_dir, home, brew) = (base.join("path"), base.join("home"), base.join("brew"));
        for dir in [&path_dir, &home.join(".cargo/bin"), &brew] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let brews = vec![base.join("none"), brew.clone()];
        let path_var = std::ffi::OsString::from(path_dir.as_os_str());
        let cargo = exe_name("cargo");
        assert_eq!(find_cargo_in(Some(&path_var), Some(&home), &brews), None);
        // Homebrew だけにある
        std::fs::write(brew.join(&cargo), "").unwrap();
        assert_eq!(
            find_cargo_in(Some(&path_var), Some(&home), &brews),
            Some(brew.join(&cargo))
        );
        // rustup（~/.cargo/bin）が Homebrew より先
        std::fs::write(home.join(".cargo/bin").join(&cargo), "").unwrap();
        assert_eq!(
            find_cargo_in(Some(&path_var), Some(&home), &brews),
            Some(home.join(".cargo/bin").join(&cargo))
        );
        // PATH がいちばん先
        std::fs::write(path_dir.join(&cargo), "").unwrap();
        assert_eq!(
            find_cargo_in(Some(&path_var), Some(&home), &brews),
            Some(path_dir.join(&cargo))
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    fn metadata(dir: &Path) -> String {
        serde_json::json!({
            "packages": [{
                "manifest_path": dir.join("abc/Cargo.toml"),
                "targets": [
                    { "name": "a", "kind": ["bin"], "src_path": dir.join("abc/src/bin/a.rs") },
                    { "name": "abc", "kind": ["bin"], "src_path": dir.join("abc/src/main.rs") },
                    { "name": "abc", "kind": ["lib"], "src_path": dir.join("abc/src/lib.rs") }
                ]
            }]
        })
        .to_string()
    }

    fn workspace(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "atcoder-studio-target-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("abc/src/bin")).unwrap();
        for file in ["abc/src/bin/a.rs", "abc/src/main.rs", "abc/src/lib.rs"] {
            std::fs::write(dir.join(file), "").unwrap();
        }
        dir.canonicalize().unwrap()
    }

    #[test]
    fn finds_bins_and_test_dirs() {
        let root = workspace("bins");
        let json = metadata(&root);
        let target = |file: &str| target_from_metadata(&json, &root, &root.join(file)).unwrap();
        let expected = |bin: &str| {
            Some(Target {
                package: "abc".to_string(),
                bin: bin.to_string(),
                language: Language::Rust,
            })
        };
        assert_eq!(target("abc/src/bin/a.rs"), expected("a"));
        assert_eq!(target("abc/src/main.rs"), expected("abc"));
        assert_eq!(target("abc/test/a/input.txt"), expected("a"));
        assert_eq!(target("abc/test/a/sample-1.in"), expected("a"));
        assert_eq!(target("abc/test/abc/compare.json"), expected("abc"));
        // ライブラリ、モジュール、知らないバイナリの test、パッケージの外は決まらない
        assert_eq!(target("abc/src/lib.rs"), None);
        assert_eq!(target("abc/src/util.rs"), None);
        assert_eq!(target("abc/test/b/input.txt"), None);
        assert_eq!(target("other/main.rs"), None);
        // バイナリが複数なら、test/ の直下のファイルからは決めない（D-106）
        assert_eq!(target("abc/test/sample-1.in"), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    // バイナリが 1 つのパッケージ（問題ごとのパッケージ）では、test/ の直下も使う（D-106）
    #[test]
    fn uses_test_dir_directly_for_single_bin() {
        let root = workspace("single");
        std::fs::create_dir_all(root.join("p/src")).unwrap();
        std::fs::write(root.join("p/src/solution.rs"), "").unwrap();
        let json = serde_json::json!({
            "packages": [{
                "manifest_path": root.join("p/Cargo.toml"),
                "targets": [
                    { "name": "solution", "kind": ["bin"], "src_path": root.join("p/src/solution.rs") }
                ]
            }]
        })
        .to_string();
        let target = |file: &str| target_from_metadata(&json, &root, &root.join(file)).unwrap();
        let expected = Some(Target {
            package: "p".to_string(),
            bin: "solution".to_string(),
            language: Language::Rust,
        });
        assert_eq!(target("p/src/solution.rs"), expected);
        assert_eq!(target("p/test/sample-1.in"), expected);
        assert_eq!(target("p/test/input.txt"), expected);
        assert_eq!(target("p/test/solution/sample-1.in"), expected);
        assert_eq!(target("p/README.md"), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ライブラリはバイナリとして数えない（lib と bin 1 つなら test/ の直下も使う）
    #[test]
    fn does_not_count_lib_as_bin() {
        let root = workspace("lib");
        std::fs::create_dir_all(root.join("q/src")).unwrap();
        std::fs::write(root.join("q/src/main.rs"), "").unwrap();
        let json = serde_json::json!({
            "packages": [{
                "manifest_path": root.join("q/Cargo.toml"),
                "targets": [
                    { "name": "q", "kind": ["lib"], "src_path": root.join("q/src/lib.rs") },
                    { "name": "q", "kind": ["bin"], "src_path": root.join("q/src/main.rs") }
                ]
            }]
        })
        .to_string();
        assert_eq!(
            target_from_metadata(&json, &root, &root.join("q/test/sample-1.in")).unwrap(),
            Some(Target {
                package: "q".to_string(),
                bin: "q".to_string(),
                language: Language::Rust,
            })
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn removes_only_ide_launch_variables() {
        for name in [
            "RUSTUP_TOOLCHAIN",
            "RUSTUP_TOOLCHAIN_SOURCE",
            "CARGO",
            "CARGO_MANIFEST_DIR",
            "CARGO_PKG_NAME",
            "DYLD_FALLBACK_LIBRARY_PATH",
        ] {
            assert!(is_removed(OsStr::new(name)), "{name}");
        }
        for name in [
            "RUSTFLAGS",
            "CARGO_TARGET_DIR",
            "CARGO_HOME",
            "RUSTUP_HOME",
            "PATH",
        ] {
            assert!(!is_removed(OsStr::new(name)), "{name}");
        }
    }

    #[test]
    fn chooses_test_dir_by_rule() {
        let root = workspace("testdir");
        let dir = root.join("p");
        let one = vec!["a".to_string()];
        let two = vec!["a".to_string(), "b".to_string()];
        std::fs::create_dir_all(dir.join("test")).unwrap();
        // バイナリが 1 つで test/<名前>/ がない：test/ を使う
        assert_eq!(test_dir(&dir, &one, "a").dir, Some(dir.join("test")));
        // バイナリが複数で test/<名前>/ がない：使うフォルダはない
        assert_eq!(
            test_dir(&dir, &two, "a"),
            TestDir {
                dir: None,
                notice: None
            }
        );
        std::fs::write(dir.join("test/sample-1.in"), "").unwrap();
        let multiple = test_dir(&dir, &two, "a");
        assert_eq!(multiple.dir, None);
        assert!(multiple.notice.unwrap().contains("test/a/ に移して"));
        // test/<名前>/ があれば、それを優先する
        std::fs::create_dir_all(dir.join("test/a")).unwrap();
        let own = test_dir(&dir, &one, "a");
        assert_eq!(own.dir, Some(dir.join("test/a")));
        assert!(own.notice.unwrap().contains("直下のファイルは使いません"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn notices_random_and_interactive_files_directly_under_test() {
        // バイナリが複数で test/<名前>/ がないとき、test/ の直下の random-N・interactive-N も知らせる（D-106 の 3。D-143・D-144）
        let root = workspace("testdir-extra");
        let dir = root.join("p");
        let two = vec!["a".to_string(), "b".to_string()];
        let test = dir.join("test");
        let files = [
            "random-1.in",
            "random-1.out",
            "interactive-1.in",
            "interactive-1.out",
        ];
        for file in files {
            let _ = std::fs::remove_dir_all(&test);
            std::fs::create_dir_all(&test).unwrap();
            std::fs::write(test.join(file), "").unwrap();
            let found = test_dir(&dir, &two, "a");
            assert_eq!(found.dir, None);
            assert!(found.notice.unwrap().contains("test/a/ に移して"), "{file}");
        }
        // 似た名前では知らせない
        let _ = std::fs::remove_dir_all(&test);
        std::fs::create_dir_all(&test).unwrap();
        std::fs::write(test.join("randomized-1.in"), "").unwrap();
        assert_eq!(test_dir(&dir, &two, "a").notice, None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn compares_rustc_version_with_judge() {
        assert!(same_as_judge("rustc 1.89.0 (29483883e 2025-08-04)"));
        assert!(!same_as_judge("rustc 1.99.0 (b940084d7 2026-09-28)"));
    }

    #[test]
    fn finds_nearest_manifest_inside_root() {
        let root = workspace("manifest");
        std::fs::write(root.join("abc/Cargo.toml"), "").unwrap();
        assert_eq!(
            nearest_manifest_dir(&root, &root.join("abc/src/bin/a.rs")),
            Some(root.join("abc"))
        );
        assert_eq!(nearest_manifest_dir(&root, &root.join("x.rs")), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
