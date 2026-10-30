// C++ のビルド（Phase 16 作業 1。D-204・D-205）
//
// - C++ のパッケージは main.cpp のあるフォルダ。開いたファイルが main.cpp か、そのフォルダの test/ の中なら対象にする
// - コンパイラは GCC だけ（Apple の clang では bits/stdc++.h が使えない）。設定の cpp.compiler、Homebrew、PATH の順に探す
// - オプションはジャッジの C++23 (GCC 15.2.0) に合わせる。ライブラリなど、手元で付けないものは実行ログで知らせる
// - コンパイルのエラーは `<ファイル>:<行>:<列>: error:` の行を読む

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use regex::Regex;

use crate::language::Language;
use crate::run::Diagnostic;
use crate::toolchain::Target;

// ジャッジの GCC の版（AtCoder の 2025/10 版の言語一覧の C++23 (GCC 15.2.0)）
pub const JUDGE_VERSION: &str = "15.2.0";
pub const JUDGE_NOTE: &str = "GCC 15.2.0、-std=gnu++23 -O2（AtCoder の 2025/10 版の言語一覧の C++23 (GCC 15.2.0)。2026/10/28 時点の確認。その後に変わっている可能性があります）";
pub const NOT_LIKE_JUDGE: &str = "手元のビルドは、ジャッジと違い -march=native・-fopenmp・-pthread・-fmodules・-static と、Boost などのライブラリを付けていません";
// 提出の確認のダイアログの注意に添える（D-209）
pub const JUDGE_SUBMIT_NOTE: &str =
    "ジャッジは、AtCoder の 2025/10 版の言語の一覧の C++23 (GCC 15.2.0) です（2026-10-28 に確認）";
pub const GCC_HINT: &str = "C++ には GCC が要ります（Apple の clang では bits/stdc++.h が使えません）。ターミナルで `brew install gcc` を実行して入れてください。別の場所の GCC を使うときは、settings.json の cpp.compiler に g++ のパスを書いてください";

// パッケージの中のソースと、ビルドしたものの置き場所（パッケージの target/）
pub const SOURCE: &str = "main.cpp";
pub const BIN: &str = "main";
const OUTPUT_DIR: &str = "target";

// settings.json の cpp の欄（D-205）。compiler が空なら探す。
// ac_library は settings.json には書かず、アプリのフォルダにあれば IDE が入れる（D-206）
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CppSettings {
    pub compiler: String,
    #[serde(skip)]
    pub ac_library: Option<PathBuf>,
}

// AC Library の置き場所（アプリのフォルダの中。D-206）。この下に atcoder/ を置く
pub const AC_LIBRARY_DIR: &str = "libraries/ac-library";

// アプリのフォルダに AC Library（atcoder/）があれば、その場所（-I に渡す）
pub fn ac_library_dir(app_dir: &Path) -> Option<PathBuf> {
    let dir = app_dir.join(AC_LIBRARY_DIR);
    dir.join("atcoder").is_dir().then_some(dir)
}

// file（canonicalize 済み）が、C++ のパッケージ dir の main.cpp か、dir の test/ の中なら、その対象（D-204）
pub fn target_in_package(root: &Path, dir: &Path, file: &Path) -> Option<Target> {
    let own = file == dir.join(SOURCE) || file.starts_with(dir.join("test"));
    if !own {
        return None;
    }
    let relative = dir.strip_prefix(root).ok()?;
    Some(Target {
        package: relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
        bin: BIN.to_string(),
        language: Language::Cpp,
    })
}

// 使う g++。設定があればそれだけを使う。なければ /opt/homebrew/bin/g++-15、PATH の g++-15、/usr/local/bin/g++-15 の順
pub fn find_compiler(settings: &CppSettings) -> Result<PathBuf, String> {
    if !settings.compiler.is_empty() {
        let path = PathBuf::from(&settings.compiler);
        if path.is_absolute() && path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "settings.json の cpp.compiler（{}）が見つかりません。{GCC_HINT}",
            settings.compiler
        ));
    }
    find_compiler_in(
        std::env::var_os("PATH").as_deref(),
        &[
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ],
    )
    .ok_or_else(|| format!("g++-15 が見つかりません。{GCC_HINT}"))
}

fn find_compiler_in(path_var: Option<&OsStr>, brews: &[PathBuf]) -> Option<PathBuf> {
    let name = "g++-15";
    let in_path = || {
        path_var.and_then(|paths| {
            std::env::split_paths(paths)
                .map(|dir| dir.join(name))
                .find(|candidate| candidate.is_file())
        })
    };
    let first = brews.first().map(|dir| dir.join(name));
    let rest = brews.iter().skip(1).map(|dir| dir.join(name));
    first
        .filter(|path| path.is_file())
        .or_else(in_path)
        .or_else(|| rest.into_iter().find(|path| path.is_file()))
}

// 版の表示（`--version` の 1 行目）と、版の番号（`-dumpfullversion`）。GCC でなければ Err
pub fn version(compiler: &Path) -> Result<(String, String), String> {
    let output = |arg: &str| -> Result<String, String> {
        let mut command = crate::process::command(compiler);
        command.arg(arg);
        crate::toolchain::clear_ide_env(&mut command);
        let output = command
            .output()
            .map_err(|error| format!("{} を起動できません：{error}", compiler.display()))?;
        if !output.status.success() {
            return Err(format!("{} {arg} が失敗しました", compiler.display()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    let line = output("--version")?
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    if line.to_ascii_lowercase().contains("clang") {
        return Err(format!(
            "{} は GCC ではありません（{line}）。{GCC_HINT}",
            compiler.display()
        ));
    }
    Ok((line, output("-dumpfullversion")?))
}

// macOS の SDK の場所（`xcrun --show-sdk-path`）。取れなければ None（-isysroot を付けない）
fn sdk_path() -> Option<&'static str> {
    static SDK: OnceLock<Option<String>> = OnceLock::new();
    SDK.get_or_init(|| {
        let output = Command::new("/usr/bin/xcrun")
            .arg("--show-sdk-path")
            .output()
            .ok()?;
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (output.status.success() && !path.is_empty()).then_some(path)
    })
    .as_deref()
}

// ビルドしたものの場所。release でなければデバッグ用（-g -O0）の別のファイル
pub fn output_path(dir: &Path, release: bool) -> PathBuf {
    dir.join(OUTPUT_DIR)
        .join(if release { BIN } else { "main-debug" })
}

// g++ に渡す引数（パッケージのディレクトリで動かす。ソースは main.cpp）。ジャッジに合わせる（D-205）
pub fn build_args(release: bool, output: &Path, settings: &CppSettings) -> Vec<String> {
    build_args_with(
        release,
        output,
        SOURCE,
        sdk_path(),
        settings.ac_library.as_deref(),
    )
}

// example（source はパッケージからの相対パス。`examples/gen.cpp` など）を、提出用と同じ引数でビルドする（D-212）
pub fn build_args_for(source: &str, output: &Path, settings: &CppSettings) -> Vec<String> {
    build_args_with(
        true,
        output,
        source,
        sdk_path(),
        settings.ac_library.as_deref(),
    )
}

fn build_args_with(
    release: bool,
    output: &Path,
    source: &str,
    sdk: Option<&str>,
    ac_library: Option<&Path>,
) -> Vec<String> {
    let mut args = flags_with(release, sdk, ac_library);
    args.extend([
        "-o".into(),
        output.to_string_lossy().into_owned(),
        source.into(),
    ]);
    args
}

// パッケージの examples/ の .cpp の名前（拡張子を除く。名前の順）。ランダムテストの生成器・愚直解、
// 対話テストのジャッジを選ぶのに使う（D-212）
pub fn example_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir.join("examples"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().is_file())
                .filter_map(|entry| {
                    let name = entry.file_name().into_string().ok()?;
                    name.strip_suffix(".cpp")
                        .filter(|stem| !stem.is_empty())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

// ビルドした example の場所（パッケージの target/examples/<名前>）
pub fn example_output_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(OUTPUT_DIR).join("examples").join(name)
}

// ビルドのオプション（出力の場所とソースの前まで）
fn flags_with(release: bool, sdk: Option<&str>, ac_library: Option<&Path>) -> Vec<String> {
    let mut args: Vec<String> = vec!["-std=gnu++23".into()];
    if release {
        args.push("-O2".into());
    } else {
        // デバッグ用は、範囲外の読み書きなどを abort にする（D-214。デバッグで止まる）
        args.extend(["-g".into(), "-O0".into(), "-D_GLIBCXX_ASSERTIONS".into()]);
    }
    args.extend(
        [
            "-Wall",
            "-Wextra",
            "-DATCODER",
            "-DONLINE_JUDGE",
            "-fconstexpr-depth=1024",
            "-fconstexpr-loop-limit=524288",
            "-fconstexpr-ops-limit=2097152",
            "-ftrivial-auto-var-init=zero",
            "-fdiagnostics-color=never",
        ]
        .map(String::from),
    );
    if let Some(sdk) = sdk {
        args.extend(["-isysroot".into(), sdk.to_string()]);
    }
    if let Some(dir) = ac_library {
        args.push(format!("-I{}", dir.to_string_lossy()));
    }
    args
}

// ---- clangd（Phase 17 作業 5。D-210） ----

// clang にない、GCC だけのオプション。clangd に渡すと「Unknown argument」の診断になるので除く
const GCC_ONLY: &[&str] = &["-fconstexpr-loop-limit=", "-fconstexpr-ops-limit="];
// clangd に命令を渡すファイルの数の上限（コンテストのフォルダ全体で）
const MAX_ANALYZED_FILES: usize = 200;

// clangd に渡す、1 つのファイルのコンパイルの命令。コンパイラ（--query-driver で標準ヘッダーの場所を聞く）と、
// 提出用のビルドと同じオプション（GCC だけのものを除く）に、`-c <ファイル>` を付ける
fn analysis_command_with(
    compiler: &Path,
    file: &Path,
    sdk: Option<&str>,
    ac_library: Option<&Path>,
) -> Vec<String> {
    let mut command = vec![compiler.to_string_lossy().into_owned()];
    command.extend(
        flags_with(true, sdk, ac_library)
            .into_iter()
            .filter(|flag| !GCC_ONLY.iter().any(|prefix| flag.starts_with(prefix))),
    );
    command.extend(["-c".into(), file.to_string_lossy().into_owned()]);
    command
}

// clangd の initializationOptions.compilationDatabaseChanges（ファイルのパス → 場所と命令）。
// packages（canonicalize 済みのパッケージのディレクトリ）の main.cpp と examples/*.cpp を載せる。
// ほかのファイル（パッケージの .hpp など）は、clangd が近くのファイルの命令から推し量る
pub fn compilation_database(
    compiler: &Path,
    packages: &[PathBuf],
    settings: &CppSettings,
) -> serde_json::Value {
    compilation_database_with(
        compiler,
        packages,
        sdk_path(),
        settings.ac_library.as_deref(),
    )
}

fn compilation_database_with(
    compiler: &Path,
    packages: &[PathBuf],
    sdk: Option<&str>,
    ac_library: Option<&Path>,
) -> serde_json::Value {
    let mut changes = serde_json::Map::new();
    for package in packages {
        let mut files = vec![package.join(SOURCE)];
        let mut examples: Vec<PathBuf> = std::fs::read_dir(package.join("examples"))
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.extension() == Some(OsStr::new("cpp")))
                    .collect()
            })
            .unwrap_or_default();
        examples.sort();
        files.extend(examples);
        for file in files.into_iter().filter(|file| file.is_file()) {
            if changes.len() >= MAX_ANALYZED_FILES {
                break;
            }
            let command = analysis_command_with(compiler, &file, sdk, ac_library);
            changes.insert(
                file.to_string_lossy().into_owned(),
                serde_json::json!({
                    "workingDirectory": package.to_string_lossy(),
                    "compilationCommand": command,
                }),
            );
        }
    }
    serde_json::json!({ "compilationDatabaseChanges": changes })
}

// `g++ -E -v` の標準エラーから、`#include <...>` の検索の場所を取り出す
fn parse_search_dirs(stderr: &str) -> Vec<PathBuf> {
    stderr
        .lines()
        .skip_while(|line| !line.starts_with("#include <...> search starts here:"))
        .skip(1)
        .take_while(|line| !line.starts_with("End of search list."))
        .map(|line| line.trim().trim_end_matches(" (framework directory)"))
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

// 定義ジャンプの行き先として、読み取り専用で開いてよい C++ のヘッダーの場所（D-210・D-133）。
// GCC の検索の場所（`g++ -E -v`）、SDK の usr/include、AC Library。canonicalize 済み、ないものは載せない
pub fn header_dirs(compiler: &Path, ac_library: Option<&Path>) -> Vec<PathBuf> {
    let mut command = crate::process::command(compiler);
    command.args(["-E", "-v", "-x", "c++", "/dev/null"]);
    crate::toolchain::clear_ide_env(&mut command);
    let mut dirs = command
        .output()
        .map(|output| parse_search_dirs(&String::from_utf8_lossy(&output.stderr)))
        .unwrap_or_default();
    if let Some(sdk) = sdk_path() {
        dirs.push(Path::new(sdk).join("usr/include"));
    }
    dirs.extend(ac_library.map(Path::to_path_buf));
    let mut real: Vec<PathBuf> = Vec::new();
    for dir in dirs.iter().filter_map(|dir| dir.canonicalize().ok()) {
        if !real.contains(&dir) {
            real.push(dir);
        }
    }
    real
}

// C++ のコンテストのフォルダに置く .clang-format（clangd の整形の書き方。エディタの字下げの 4 に合わせる。D-208・D-210）
pub const CLANG_FORMAT: &str =
    "# AtCoder Studio が作った。clangd の整形の書き方\nBasedOnStyle: LLVM\nIndentWidth: 4\n";

// パッケージの compile_flags.txt の中身（clangd が読む。Phase 17 の言語機能で使う。D-208・D-210）。
// ビルドと同じ言語の版・定義と、AC Library の場所。1 行に 1 つ
pub fn compile_flags(ac_library: Option<&Path>) -> String {
    let mut flags: Vec<String> = [
        "-std=gnu++23",
        "-Wall",
        "-Wextra",
        "-DATCODER",
        "-DONLINE_JUDGE",
    ]
    .map(String::from)
    .to_vec();
    if let Some(dir) = ac_library {
        flags.push(format!("-I{}", dir.to_string_lossy()));
    }
    let mut text = flags.join("\n");
    text.push('\n');
    text
}

// g++ の標準エラーから、エラー・警告を取り出す。`note:` と、前の行の「In function …」などは、
// 近くのエラー・警告の本文（rendered）に入れる
pub fn parse_diagnostics(stderr: &str) -> Vec<Diagnostic> {
    static HEADER: OnceLock<Regex> = OnceLock::new();
    let header = HEADER.get_or_init(|| {
        Regex::new(r"^(.+?):(\d+):(?:\d+:)? (fatal error|error|warning|note): (.*)$").unwrap()
    });
    let mut items: Vec<Diagnostic> = Vec::new();
    // まだどのエラーにも付けていない前置きの行（「main.cpp: In function 'int main()':」など）
    let mut pending: Vec<&str> = Vec::new();
    for line in stderr.lines() {
        let Some(found) = header.captures(line) else {
            match items.last_mut() {
                // 本文の続き（ソースの行・^ の印）
                Some(last) if line.starts_with(' ') => {
                    last.rendered.push('\n');
                    last.rendered.push_str(line);
                }
                _ => pending.push(line),
            }
            continue;
        };
        let level = &found[3];
        if level == "note"
            && pending.is_empty()
            && let Some(last) = items.last_mut()
        {
            last.rendered.push('\n');
            last.rendered.push_str(line);
            continue;
        }
        let mut rendered = pending.join("\n");
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(line);
        pending.clear();
        items.push(Diagnostic {
            level: match level {
                "fatal error" | "error" => "error",
                "warning" => "warning",
                _ => "note",
            }
            .to_string(),
            message: found[4].to_string(),
            rendered,
            file: Some(found[1].to_string()),
            line: found[2].parse().ok(),
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let dir = std::env::temp_dir()
                .join(format!("atcoder-studio-cpp-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir.canonicalize().unwrap())
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn finds_the_target_from_main_or_tests() {
        let root = TempDir::new("target");
        let dir = root.0.join("abc480/abc480_a");
        std::fs::create_dir_all(dir.join("test")).unwrap();
        std::fs::write(dir.join("main.cpp"), "").unwrap();
        let expected = Some(Target {
            package: "abc480/abc480_a".to_string(),
            bin: "main".to_string(),
            language: Language::Cpp,
        });
        assert_eq!(
            target_in_package(&root.0, &dir, &dir.join("main.cpp")),
            expected
        );
        assert_eq!(
            target_in_package(&root.0, &dir, &dir.join("test/sample-1.in")),
            expected
        );
        // ほかの .cpp（生成器など）からは決めない
        assert_eq!(
            target_in_package(&root.0, &dir, &dir.join("examples/gen.cpp")),
            None
        );
        // 作業領域のルートがパッケージ
        assert_eq!(
            target_in_package(&dir, &dir, &dir.join("main.cpp")).map(|t| t.package),
            Some(String::new())
        );
    }

    #[test]
    fn finds_gcc_in_homebrew_then_path_then_usr_local() {
        let base = TempDir::new("find");
        let (brew, path_dir, local) = (
            base.0.join("brew"),
            base.0.join("path"),
            base.0.join("local"),
        );
        for dir in [&brew, &path_dir, &local] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let brews = vec![brew.clone(), local.clone()];
        let path_var = std::ffi::OsString::from(path_dir.as_os_str());
        assert_eq!(find_compiler_in(Some(&path_var), &brews), None);
        std::fs::write(local.join("g++-15"), "").unwrap();
        assert_eq!(
            find_compiler_in(Some(&path_var), &brews),
            Some(local.join("g++-15"))
        );
        std::fs::write(path_dir.join("g++-15"), "").unwrap();
        assert_eq!(
            find_compiler_in(Some(&path_var), &brews),
            Some(path_dir.join("g++-15"))
        );
        std::fs::write(brew.join("g++-15"), "").unwrap();
        assert_eq!(
            find_compiler_in(Some(&path_var), &brews),
            Some(brew.join("g++-15"))
        );
    }

    #[test]
    fn refuses_a_missing_compiler_setting() {
        for compiler in ["/nonexistent/g++-15", "g++-15"] {
            let error = find_compiler(&CppSettings {
                compiler: compiler.to_string(),
                ac_library: None,
            })
            .unwrap_err();
            assert!(error.contains(compiler), "{error}");
            assert!(error.contains("brew install gcc"), "{error}");
        }
    }

    #[test]
    fn builds_with_the_judge_options() {
        let output = Path::new("/p/target/main");
        let args = build_args_with(
            true,
            output,
            SOURCE,
            Some("/sdk"),
            Some(Path::new("/app/acl")),
        );
        for expected in [
            "-std=gnu++23",
            "-O2",
            "-DATCODER",
            "-DONLINE_JUDGE",
            "-ftrivial-auto-var-init=zero",
        ] {
            assert!(args.iter().any(|arg| arg == expected), "{expected}");
        }
        assert!(args.windows(2).any(|pair| pair == ["-isysroot", "/sdk"]));
        assert!(args.iter().any(|arg| arg == "-I/app/acl"));
        assert_eq!(args[args.len() - 3..], ["-o", "/p/target/main", "main.cpp"]);
        let debug = build_args_with(false, output, SOURCE, None, None);
        assert!(debug.iter().any(|arg| arg == "-g") && debug.iter().any(|arg| arg == "-O0"));
        // 範囲外の読み書きの検査は、デバッグ用のビルドだけ（D-214）
        assert!(debug.iter().any(|arg| arg == "-D_GLIBCXX_ASSERTIONS"));
        assert!(!args.iter().any(|arg| arg == "-D_GLIBCXX_ASSERTIONS"));
        assert!(
            !debug
                .iter()
                .any(|arg| arg == "-O2" || arg == "-isysroot" || arg.starts_with("-I"))
        );
    }

    #[test]
    fn writes_compile_flags_for_clangd() {
        assert_eq!(
            compile_flags(Some(Path::new("/app/acl"))),
            "-std=gnu++23\n-Wall\n-Wextra\n-DATCODER\n-DONLINE_JUDGE\n-I/app/acl\n"
        );
        assert!(!compile_flags(None).contains("-I"));
    }

    #[test]
    fn reads_gcc_diagnostics() {
        let stderr = r"main.cpp: In function 'int main()':
main.cpp:4:5: error: 'x' was not declared in this scope
    4 |     x = 1;
      |     ^
main.cpp:5:9: warning: unused variable 'y' [-Wunused-variable]
    5 |     int y;
      |         ^
main.cpp:6:1: note: some note
In file included from main.cpp:1:
other.hpp:2:1: fatal error: missing.h: No such file or directory
compilation terminated.
";
        let items = parse_diagnostics(stderr);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].level, "error");
        assert_eq!(items[0].message, "'x' was not declared in this scope");
        assert_eq!(
            (items[0].file.as_deref(), items[0].line),
            (Some("main.cpp"), Some(4))
        );
        assert!(
            items[0]
                .rendered
                .starts_with("main.cpp: In function 'int main()':\n")
        );
        assert!(items[0].rendered.ends_with("      |     ^"));
        assert_eq!(items[1].level, "warning");
        // note は前の警告の本文に入る
        assert!(items[1].rendered.ends_with("main.cpp:6:1: note: some note"));
        assert_eq!(items[2].level, "error");
        assert_eq!(
            (items[2].file.as_deref(), items[2].line),
            (Some("other.hpp"), Some(2))
        );
        assert!(
            items[2]
                .rendered
                .starts_with("In file included from main.cpp:1:\n")
        );
    }

    #[test]
    fn makes_commands_for_clangd() {
        let dir = TempDir::new("clangd");
        let package = dir.0.join("abc480_a");
        std::fs::create_dir_all(package.join("examples")).unwrap();
        std::fs::write(package.join(SOURCE), "").unwrap();
        std::fs::write(package.join("examples/gen.cpp"), "").unwrap();
        std::fs::write(package.join("examples/notes.txt"), "").unwrap();
        // main.cpp のないパッケージ（消した）は載せない
        let empty = dir.0.join("abc480_b");
        std::fs::create_dir_all(&empty).unwrap();
        let compiler = Path::new("/opt/homebrew/bin/g++-15");
        let database = compilation_database_with(
            compiler,
            &[package.clone(), empty],
            Some("/sdk"),
            Some(Path::new("/app/acl")),
        );
        let changes = database["compilationDatabaseChanges"].as_object().unwrap();
        let main = package.join(SOURCE).to_string_lossy().into_owned();
        let generator = package
            .join("examples/gen.cpp")
            .to_string_lossy()
            .into_owned();
        // 載せるのは main.cpp と examples/*.cpp だけ
        let mut keys: Vec<String> = changes.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, vec![generator.clone(), main.clone()]);
        assert_eq!(
            changes[&main]["workingDirectory"],
            package.to_string_lossy().as_ref()
        );
        let command: Vec<String> =
            serde_json::from_value(changes[&main]["compilationCommand"].clone()).unwrap();
        // コンパイラが先頭（--query-driver の対象）。ビルドと同じオプションで、GCC だけのものは除く
        assert_eq!(command[0], "/opt/homebrew/bin/g++-15");
        for flag in [
            "-std=gnu++23",
            "-O2",
            "-DATCODER",
            "-DONLINE_JUDGE",
            "-fconstexpr-depth=1024",
            "-isysroot",
            "/sdk",
            "-I/app/acl",
        ] {
            assert!(command.contains(&flag.to_string()), "{flag}: {command:?}");
        }
        assert!(
            !command
                .iter()
                .any(|flag| flag.starts_with("-fconstexpr-loop-limit"))
        );
        assert!(
            !command
                .iter()
                .any(|flag| flag.starts_with("-fconstexpr-ops-limit"))
        );
        assert!(!command.contains(&"-o".to_string()));
        assert_eq!(command[command.len() - 2..], ["-c".to_string(), main]);
        let command: Vec<String> =
            serde_json::from_value(changes[&generator]["compilationCommand"].clone()).unwrap();
        assert_eq!(command.last().unwrap(), &generator);
    }

    #[test]
    fn lists_examples_and_their_outputs() {
        let dir = TempDir::new("examples");
        assert!(example_names(&dir.0).is_empty());
        std::fs::create_dir_all(dir.0.join("examples/sub.cpp")).unwrap();
        for name in ["naive.cpp", "gen.cpp", "notes.txt", ".cpp", "judge.rs"] {
            std::fs::write(dir.0.join("examples").join(name), "").unwrap();
        }
        // .cpp のファイルだけ（フォルダ・名前のないものは入れない）、名前の順
        assert_eq!(example_names(&dir.0), vec!["gen", "naive"]);
        assert_eq!(
            example_output_path(&dir.0, "gen"),
            dir.0.join("target/examples/gen")
        );
        let args = build_args_for("examples/gen.cpp", Path::new("/o"), &CppSettings::default());
        assert!(args.contains(&"-O2".to_string()));
        assert_eq!(args[args.len() - 3..], ["-o", "/o", "examples/gen.cpp"]);
    }

    #[test]
    fn reads_the_include_search_list() {
        let stderr = "Using built-in specs.\n#include \"...\" search starts here:\n#include <...> search starts here:\n /gcc/include/c++/15\n /gcc/include/c++/15/aarch64-apple-darwin24\n /sdk/usr/include\n /sdk/System/Library/Frameworks (framework directory)\nEnd of search list.\n /not/this\n";
        assert_eq!(
            parse_search_dirs(stderr),
            [
                "/gcc/include/c++/15",
                "/gcc/include/c++/15/aarch64-apple-darwin24",
                "/sdk/usr/include",
                "/sdk/System/Library/Frameworks"
            ]
            .map(PathBuf::from)
            .to_vec()
        );
        assert!(parse_search_dirs("").is_empty());
    }

    // 手元に g++-15 があるときだけ、本当にビルドして動かす
    #[test]
    fn builds_and_runs_with_local_gcc() {
        let Ok(compiler) = find_compiler(&CppSettings::default()) else {
            eprintln!("g++-15 がないので飛ばします");
            return;
        };
        let (line, number) = version(&compiler).unwrap();
        assert!(!line.is_empty() && number.starts_with("15."));
        let dir = TempDir::new("build");
        std::fs::write(
            dir.0.join(SOURCE),
            "#include <bits/stdc++.h>\nusing namespace std;\nint main(){long long a,b;cin>>a>>b;\n#ifdef ATCODER\ncout<<a+b<<endl;\n#endif\n}\n",
        )
        .unwrap();
        // AC Library の代わりに、atcoder/ の下に小さなヘッダーを置いた場所を -I に渡す
        let library = TempDir::new("library");
        std::fs::create_dir_all(library.0.join("atcoder")).unwrap();
        std::fs::write(
            library.0.join("atcoder/add"),
            "#pragma once\nnamespace atcoder { inline long long add(long long a, long long b) { return a + b; } }\n",
        )
        .unwrap();
        assert_eq!(ac_library_dir(&library.0.join("none")), None);
        let settings = CppSettings {
            compiler: String::new(),
            ac_library: Some(library.0.clone()),
        };
        std::fs::write(
            dir.0.join(SOURCE),
            "#include <bits/stdc++.h>\n#include <atcoder/add>\nusing namespace std;\nint main(){long long a,b;cin>>a>>b;\n#ifdef ATCODER\ncout<<atcoder::add(a,b)<<endl;\n#endif\n}\n",
        )
        .unwrap();
        let output = output_path(&dir.0, true);
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        let status = Command::new(&compiler)
            .current_dir(&dir.0)
            .args(build_args(true, &output, &settings))
            .status()
            .unwrap();
        assert!(status.success());
        let mut child = Command::new(&output)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child.stdin.take().unwrap().write_all(b"2 40\n").unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "42\n");
        // エラーのあるソース
        std::fs::write(dir.0.join(SOURCE), "int main() { x = 1; }\n").unwrap();
        let failed = Command::new(&compiler)
            .current_dir(&dir.0)
            .args(build_args(true, &output, &settings))
            .output()
            .unwrap();
        assert!(!failed.status.success());
        let items = parse_diagnostics(&String::from_utf8_lossy(&failed.stderr));
        assert!(
            items
                .iter()
                .any(|item| item.level == "error" && item.line == Some(1)),
            "{items:?}"
        );
    }
}
