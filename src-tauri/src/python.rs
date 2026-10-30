// Python の実行（Phase 19 作業 1。D-220〜D-222。docs/phase19/01-pre-start.md）
//
// - Python のパッケージは main.py のあるフォルダ。開いたファイルが main.py か、そのフォルダの test/ の中なら対象にする
// - 処理系は settings.json の python.interpreter（cpython〔既定〕・pypy）で選ぶ。設定の絶対パス、Homebrew、PATH の順に探す。
//   ジャッジと同じ版（CPython 3.13・PyPy 3.11）でなければ、入れ方を案内して動かさない（ほかの版は使わない）
// - ビルドはない。動かす前に構文を確かめ（compile だけ。__pycache__ は作らない）、誤りなら CE にする
// - 実行はジャッジと同じ `-X int_max_str_digits=0`。手元の PYTHON… の環境変数は読ませず（-E）、__pycache__ を作らない（-B）

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::language::Language;
use crate::run::{Diagnostic, Program};
use crate::toolchain::Target;

// パッケージの中のソース
pub const SOURCE: &str = "main.py";
pub const BIN: &str = "main";

// 処理系（D-221）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Interpreter {
    #[default]
    Cpython,
    Pypy,
}

impl Interpreter {
    pub fn label(self) -> &'static str {
        match self {
            Interpreter::Cpython => "CPython",
            Interpreter::Pypy => "PyPy",
        }
    }

    // 探す名前（Homebrew・PATH の中）
    fn names(self) -> &'static [&'static str] {
        match self {
            Interpreter::Cpython => &["python3.13"],
            Interpreter::Pypy => &["pypy3.11", "pypy3"],
        }
    }

    // ジャッジの Python の版（言語一覧の CPython 3.13.7・PyPy 3.11-v7.3.20）。手元はこの「3.x」に合わせる
    fn judge_python(self) -> &'static str {
        match self {
            Interpreter::Cpython => "3.13",
            Interpreter::Pypy => "3.11",
        }
    }

    pub fn judge_note(self) -> &'static str {
        match self {
            Interpreter::Cpython => {
                "Python (CPython 3.13.7)、python3.13 -X int_max_str_digits=0 Main.py（AtCoder の 2025/10 版の言語一覧。2026/10/28 時点の確認。その後に変わっている可能性があります）"
            }
            Interpreter::Pypy => {
                "Python (PyPy 3.11-v7.3.20)、pypy3 -X int_max_str_digits=0 Main.py（AtCoder の 2025/10 版の言語一覧。2026/10/28 時点の確認。その後に変わっている可能性があります）"
            }
        }
    }

    // 提出のフォームの言語の選択肢（Phase 19 作業 4。D-224）
    pub fn submit_option(self) -> &'static str {
        match self {
            Interpreter::Cpython => "Python (CPython 3.13.7)",
            Interpreter::Pypy => "Python (PyPy 3.11-v7.3.20)",
        }
    }

    // 提出の確認のダイアログに出す、ジャッジの出典（D-224）
    pub fn submit_note(self) -> &'static str {
        match self {
            Interpreter::Cpython => {
                "ジャッジは、AtCoder の 2025/10 版の言語の一覧の Python (CPython 3.13.7) です。パッケージの一覧も同じ版のものです（2026-10-28 に確認）"
            }
            Interpreter::Pypy => {
                "ジャッジは、AtCoder の 2025/10 版の言語の一覧の Python (PyPy 3.11-v7.3.20) です。パッケージの一覧も同じ版のものです（2026-10-28 に確認）"
            }
        }
    }

    // ジャッジにあるパッケージの import の名前（言語の一覧のパッケージ名から。scikit-learn は sklearn、
    // protobuf は google、python-dateutil は dateutil、z3-solver は z3、ac-library-python は atcoder、
    // acl-cpp-python は acl_cpp。D-224）
    fn judge_packages(self) -> &'static [&'static str] {
        match self {
            Interpreter::Cpython => CPYTHON_PACKAGES,
            Interpreter::Pypy => PYPY_PACKAGES,
        }
    }

    // ジャッジの版の全体（違えば実行ログで知らせる）
    fn judge_full(self) -> &'static str {
        match self {
            Interpreter::Cpython => "3.13.7",
            Interpreter::Pypy => "7.3.20",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Interpreter::Cpython => {
                "ジャッジと同じ CPython 3.13 が要ります。ターミナルで `brew install python@3.13` を実行して入れてください。別の場所のものを使うときは、settings.json の python.cpython に python3.13 のパスを書いてください"
            }
            Interpreter::Pypy => {
                "ジャッジと同じ PyPy（Python 3.11 の版）が要ります。ターミナルで `brew install pypy3.11` を実行して入れてください。別の場所のものを使うときは、settings.json の python.pypy に pypy3 のパスを書いてください"
            }
        }
    }
}

// settings.json の python の欄（D-221）。パスが空なら探す
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PythonSettings {
    pub interpreter: Interpreter,
    pub cpython: String,
    pub pypy: String,
}

impl PythonSettings {
    fn configured(&self) -> &str {
        match self.interpreter {
            Interpreter::Cpython => &self.cpython,
            Interpreter::Pypy => &self.pypy,
        }
    }
}

// file（canonicalize 済み）が、Python のパッケージ dir の main.py か、dir の test/ の中なら、その対象（D-220）
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
        language: Language::Python,
    })
}

// 使う処理系。設定があればそれだけを使う。なければ /opt/homebrew/bin、PATH、/usr/local/bin の順
pub fn find_interpreter(settings: &PythonSettings) -> Result<PathBuf, String> {
    let interpreter = settings.interpreter;
    let configured = settings.configured();
    if !configured.is_empty() {
        let path = PathBuf::from(configured);
        if path.is_absolute() && path.is_file() {
            return Ok(path);
        }
        let key = match interpreter {
            Interpreter::Cpython => "python.cpython",
            Interpreter::Pypy => "python.pypy",
        };
        return Err(format!(
            "settings.json の {key}（{configured}）が見つかりません。{}",
            interpreter.hint()
        ));
    }
    find_in(
        interpreter.names(),
        std::env::var_os("PATH").as_deref(),
        &[
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ],
    )
    .ok_or_else(|| {
        format!(
            "{} が見つかりません。{}",
            interpreter.names().join("・"),
            interpreter.hint()
        )
    })
}

fn find_in(names: &[&str], path_var: Option<&OsStr>, brews: &[PathBuf]) -> Option<PathBuf> {
    names.iter().find_map(|name| {
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
    })
}

// 処理系の版：表示（`--version` の出力を 1 行に）と、Python の版（例：3.13.7）・PyPy の版（例：7.3.20）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub line: String,
    pub python: String,
    pub pypy: Option<String>,
}

// `--version` の出力を読む。CPython は「Python 3.13.7」、PyPy は「Python 3.11.13 (…)\n[PyPy 7.3.20 with …]」
pub fn parse_version(text: &str) -> Option<Version> {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let python = line
        .strip_prefix("Python ")?
        .split([' ', '+'])
        .next()?
        .to_string();
    let pypy = line
        .split("[PyPy ")
        .nth(1)
        .and_then(|rest| rest.split([' ', ']']).next())
        .map(str::to_string);
    Some(Version { line, python, pypy })
}

pub fn version(interpreter: &Path) -> Result<Version, String> {
    let mut command = crate::process::command(interpreter);
    command.arg("--version");
    crate::toolchain::clear_ide_env(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("{} を起動できません：{error}", interpreter.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} --version が失敗しました",
            interpreter.display()
        ));
    }
    // 古い Python は版を標準エラーに出す
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_version(&text).ok_or_else(|| {
        format!(
            "{} の版を読めません（{}）",
            interpreter.display(),
            text.trim()
        )
    })
}

// 版がジャッジの処理系と合うか（D-221）。合わなければ、動かさない理由。
// 合っていても細かい版が違えば、2 つ目に実行ログに出す知らせ
pub fn check_version(
    interpreter: Interpreter,
    version: &Version,
) -> Result<Option<String>, String> {
    let same_minor = version.python == interpreter.judge_python()
        || version
            .python
            .starts_with(&format!("{}.", interpreter.judge_python()));
    let kind_ok = match interpreter {
        Interpreter::Cpython => version.pypy.is_none(),
        Interpreter::Pypy => version.pypy.is_some(),
    };
    if !same_minor || !kind_ok {
        return Err(format!(
            "手元の処理系（{}）は、ジャッジの {}（Python {}）と違うので動かしません。{}",
            version.line,
            interpreter.label(),
            interpreter.judge_python(),
            interpreter.hint()
        ));
    }
    let local = match interpreter {
        Interpreter::Cpython => version.python.as_str(),
        Interpreter::Pypy => version.pypy.as_deref().unwrap_or_default(),
    };
    Ok((local != interpreter.judge_full()).then(|| {
        format!(
            "ローカルの {} の版（{local}）が、ジャッジ（{}）と異なります。",
            interpreter.label(),
            interpreter.judge_full()
        )
    }))
}

// ジャッジのパッケージ（AtCoder の 2025/10 版の言語の一覧。2026-10-28 に確認）の import の名前
const CPYTHON_PACKAGES: &[&str] = &[
    "Cython",
    "absl",
    "acl_cpp",
    "atcoder",
    "bitarray",
    "cppyy",
    "cppyy_backend",
    "cython",
    "dateutil",
    "filelock",
    "fsspec",
    "gmpy2",
    "google",
    "immutabledict",
    "jinja2",
    "joblib",
    "lightgbm",
    "llvmlite",
    "markupsafe",
    "more_itertools",
    "mpmath",
    "networkx",
    "numba",
    "numpy",
    "ortools",
    "pandas",
    "polars",
    "pulp",
    "pyrsistent",
    "pytz",
    "pyximport",
    "rustworkx",
    "scipy",
    "shapely",
    "six",
    "sklearn",
    "sortedcontainers",
    "sympy",
    "threadpoolctl",
    "torch",
    "typing_extensions",
    "tzdata",
    "z3",
];
const PYPY_PACKAGES: &[&str] = &[
    "acl_cpp",
    "atcoder",
    "bitarray",
    "cppyy",
    "more_itertools",
    "mpmath",
    "networkx",
    "numpy",
    "pandas",
    "pulp",
    "scipy",
    "shapely",
    "sklearn",
    "sortedcontainers",
    "sympy",
    "z3",
];

// ソースが import するモジュールの一番上の名前を、処理系の ast で読む（文字列・コメントの中は数えない）。
// 1 行に「名前<TAB>標準ライブラリなら 1」。相対の import（from . import x）は名前を「.」にする
const IMPORTS: &str = "import ast, sys\ntree = ast.parse(open(sys.argv[1], 'rb').read())\nnames = set()\nfor node in ast.walk(tree):\n    if isinstance(node, ast.Import):\n        names.update(a.name.split('.')[0] for a in node.names)\n    elif isinstance(node, ast.ImportFrom):\n        names.add('.' if node.level else node.module.split('.')[0])\nstd = set(sys.stdlib_module_names) | set(sys.builtin_module_names)\nfor n in sorted(names):\n    print(n, int(n in std), sep='\\t')\n";

// import する名前と、標準ライブラリか
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub name: String,
    pub standard: bool,
}

// main.py の import を読む（提出の前の検査。D-224）。パッケージのフォルダで動かし、-I で、そのフォルダの
// モジュール（ast.py など）を読ませない
pub fn imports(interpreter: &Path, source: &Path) -> Result<Vec<Import>, String> {
    let mut command = crate::process::command(interpreter);
    if let Some(dir) = source.parent() {
        command.current_dir(dir);
    }
    command
        .arg("-I")
        .arg("-B")
        .arg("-c")
        .arg(IMPORTS)
        .arg(source);
    crate::toolchain::clear_ide_env(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("{} を起動できません：{error}", interpreter.display()))?;
    if !output.status.success() {
        return Err("import を読めません（構文の誤りがあるかもしれません）".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (name, standard) = line.split_once('\t')?;
            Some(Import {
                name: name.to_string(),
                standard: standard.trim() == "1",
            })
        })
        .collect())
}

// 処理系の標準ライブラリ・site-packages の場所（定義ジャンプの行き先を読み取り専用で開く。D-225）。
// -I で、今のフォルダ・利用者の site-packages・PYTHON… の環境変数を入れない。読めなければ空
pub fn library_dirs(interpreter: &Path) -> Vec<PathBuf> {
    let mut command = crate::process::command(interpreter);
    command.args([
        "-I",
        "-B",
        "-c",
        "import sys\nfor p in sys.path:\n    print(p)\n",
    ]);
    crate::toolchain::clear_ide_env(&mut command);
    let Ok(output) = command.output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_dir())
        .collect()
}

// 提出の前の検査の注意（D-224。止めずに確認のダイアログに出す）。dir はパッケージのディレクトリ
pub fn submit_warnings(interpreter: Interpreter, imports: &[Import], dir: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    let is_local = |name: &str| {
        name == "." || dir.join(format!("{name}.py")).is_file() || dir.join(name).is_dir()
    };
    let local: Vec<&str> = imports
        .iter()
        .filter(|import| !import.standard && is_local(&import.name))
        .map(|import| import.name.as_str())
        .collect();
    if !local.is_empty() {
        warnings.push(format!(
            "自分のモジュールを読む import（{}）があります。ジャッジには、このファイルだけが置かれます",
            local
                .iter()
                .map(|name| if *name == "." {
                    "from . import".to_string()
                } else {
                    (*name).to_string()
                })
                .collect::<Vec<_>>()
                .join("、")
        ));
    }
    let missing: Vec<&str> = imports
        .iter()
        .filter(|import| {
            !import.standard
                && !is_local(&import.name)
                && !interpreter.judge_packages().contains(&import.name.as_str())
        })
        .map(|import| import.name.as_str())
        .collect();
    if !missing.is_empty() {
        warnings.push(format!(
            "ジャッジの {} の言語の一覧にないパッケージ（{}）を import しています",
            interpreter.label(),
            missing.join("、")
        ));
    }
    warnings
}

// 手元の処理系の版がジャッジと違うときの注意（D-224）
pub fn version_warning(interpreter: Interpreter, version: &Version) -> Option<String> {
    let local = match interpreter {
        Interpreter::Cpython => version.python.as_str(),
        Interpreter::Pypy => version.pypy.as_deref().unwrap_or(version.line.as_str()),
    };
    (local != interpreter.judge_full()).then(|| {
        format!(
            "手元の {}（{local}）は、ジャッジ（{}）と版が違います",
            interpreter.label(),
            interpreter.judge_full()
        )
    })
}

// 動かす命令（パッケージのディレクトリで、script は相対パス）。-E で PYTHON… の環境変数を読ませず、
// -B で __pycache__ を作らない。-X int_max_str_digits=0 はジャッジと同じ
pub fn program(interpreter: &Path, script: &str) -> Program {
    Program {
        path: interpreter.to_path_buf(),
        args: ["-E", "-B", "-X", "int_max_str_digits=0", script]
            .iter()
            .map(OsString::from)
            .collect(),
    }
}

// 構文を確かめる小さな命令（compile だけで、動かさず、__pycache__ も作らない）。
// 誤りなら「行<TAB>列<TAB>メッセージ」を 1 行出して 1 で終わる
const CHECK: &str = "import sys\np = sys.argv[1]\ntry:\n    compile(open(p, 'rb').read(), p, 'exec')\nexcept (SyntaxError, ValueError) as e:\n    print(getattr(e, 'lineno', None) or 0, getattr(e, 'offset', None) or 0, getattr(e, 'msg', None) or str(e), sep='\\t')\n    sys.exit(1)\n";

pub fn check_command(interpreter: &Path, dir: &Path, script: &str) -> Command {
    let mut command = crate::process::command(interpreter);
    command
        .current_dir(dir)
        .args(["-E", "-B", "-c", CHECK, script]);
    crate::toolchain::clear_ide_env(&mut command);
    command
}

// 構文の確かめの出力から、エラー一覧に出すもの（file はパッケージからの相対パス）
pub fn parse_check(stdout: &str, script: &str) -> Vec<Diagnostic> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let row: u64 = parts.next()?.trim().parse().ok()?;
            let column: u64 = parts.next()?.trim().parse().ok()?;
            let message = parts.next()?.trim().to_string();
            let at = if row > 0 {
                format!("{script}:{row}:{column}")
            } else {
                script.to_string()
            };
            Some(Diagnostic {
                level: "error".to_string(),
                rendered: format!("{at}: SyntaxError: {message}"),
                message: format!("SyntaxError: {message}"),
                file: Some(script.to_string()),
                line: (row > 0).then_some(row),
            })
        })
        .collect()
}

// examples/ の Python のソースの名前（拡張子を除く。ランダムテスト・対話テスト。D-220）
pub fn example_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir.join("examples"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().is_file())
                .filter_map(|entry| {
                    let name = entry.file_name().into_string().ok()?;
                    name.strip_suffix(".py")
                        .filter(|stem| !stem.is_empty())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "atcoder-studio-python-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn reads_versions() {
        let cpython = parse_version("Python 3.13.7\n").unwrap();
        assert_eq!((cpython.python.as_str(), cpython.pypy), ("3.13.7", None));
        let pypy = parse_version(
            "Python 3.11.13 (413c9b7f57f5, Jul 04 2025, 15:52:42)\n[PyPy 7.3.20 with GCC 10.2.1 20210130 (Red Hat 10.2.1-11)]\n",
        )
        .unwrap();
        assert_eq!(pypy.python, "3.11.13");
        assert_eq!(pypy.pypy.as_deref(), Some("7.3.20"));
        assert!(parse_version("pyenv: command not found").is_none());
        assert_eq!(parse_version("Python 3.14.0+\n").unwrap().python, "3.14.0");
    }

    #[test]
    fn checks_versions_against_the_judge() {
        let v = |text: &str| parse_version(text).unwrap();
        assert_eq!(
            check_version(Interpreter::Cpython, &v("Python 3.13.7")),
            Ok(None)
        );
        assert!(
            check_version(Interpreter::Cpython, &v("Python 3.13.9"))
                .unwrap()
                .unwrap()
                .contains("3.13.9")
        );
        // ほかの版・ほかの処理系は動かさない
        assert!(check_version(Interpreter::Cpython, &v("Python 3.14.3")).is_err());
        assert!(check_version(Interpreter::Cpython, &v("Python 3.1.0")).is_err());
        let pypy = v("Python 3.11.13 (x)\n[PyPy 7.3.20 with GCC]");
        assert!(check_version(Interpreter::Cpython, &pypy).is_err());
        assert_eq!(check_version(Interpreter::Pypy, &pypy), Ok(None));
        assert!(check_version(Interpreter::Pypy, &v("Python 3.11.9")).is_err());
        assert!(
            check_version(
                Interpreter::Pypy,
                &v("Python 3.10.14 (x)\n[PyPy 7.3.17 with GCC]")
            )
            .is_err()
        );
    }

    #[test]
    fn finds_interpreters() {
        let dir = temp_dir("find");
        let (brew, path_dir, local) = (dir.join("brew"), dir.join("path"), dir.join("local"));
        for d in [&brew, &path_dir, &local] {
            std::fs::create_dir_all(d).unwrap();
        }
        let path_var = OsString::from(path_dir.as_os_str());
        let brews = [brew.clone(), local.clone()];
        assert_eq!(find_in(&["python3.13"], Some(&path_var), &brews), None);
        std::fs::write(local.join("python3.13"), "").unwrap();
        assert_eq!(
            find_in(&["python3.13"], Some(&path_var), &brews),
            Some(local.join("python3.13"))
        );
        std::fs::write(path_dir.join("python3.13"), "").unwrap();
        assert_eq!(
            find_in(&["python3.13"], Some(&path_var), &brews),
            Some(path_dir.join("python3.13"))
        );
        std::fs::write(brew.join("python3.13"), "").unwrap();
        assert_eq!(
            find_in(&["python3.13"], Some(&path_var), &brews),
            Some(brew.join("python3.13"))
        );
        // PyPy は pypy3.11 を先に
        std::fs::write(path_dir.join("pypy3"), "").unwrap();
        assert_eq!(
            find_in(&["pypy3.11", "pypy3"], Some(&path_var), &brews),
            Some(path_dir.join("pypy3"))
        );
        std::fs::write(local.join("pypy3.11"), "").unwrap();
        assert_eq!(
            find_in(&["pypy3.11", "pypy3"], Some(&path_var), &brews),
            Some(local.join("pypy3.11"))
        );
        // 設定のパスがなければ、案内つきで断る
        let settings = PythonSettings {
            interpreter: Interpreter::Pypy,
            pypy: "/nonexistent/pypy3".to_string(),
            ..PythonSettings::default()
        };
        let error = find_interpreter(&settings).unwrap_err();
        assert!(error.contains("python.pypy") && error.contains("brew install pypy3.11"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_targets_and_examples() {
        let root = temp_dir("target");
        let dir = root.join("abc480/abc480_a");
        std::fs::create_dir_all(dir.join("test")).unwrap();
        std::fs::create_dir_all(dir.join("examples")).unwrap();
        for name in [
            "main.py",
            "test/input.txt",
            "examples/gen.py",
            "examples/naive.py",
            "examples/.py",
            "examples/note.txt",
            "other.py",
        ] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        let target = target_in_package(&root, &dir, &dir.join("main.py")).unwrap();
        assert_eq!(
            (
                target.package.as_str(),
                target.bin.as_str(),
                target.language
            ),
            ("abc480/abc480_a", "main", Language::Python)
        );
        assert!(target_in_package(&root, &dir, &dir.join("test/input.txt")).is_some());
        assert!(target_in_package(&root, &dir, &dir.join("other.py")).is_none());
        assert_eq!(example_names(&dir), vec!["gen", "naive"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn builds_the_run_command() {
        let program = program(Path::new("/opt/homebrew/bin/python3.13"), "main.py");
        assert_eq!(program.path, PathBuf::from("/opt/homebrew/bin/python3.13"));
        assert_eq!(
            program.args,
            ["-E", "-B", "-X", "int_max_str_digits=0", "main.py"]
                .map(OsString::from)
                .to_vec()
        );
    }

    #[test]
    fn reads_syntax_errors() {
        let items = parse_check("3\t5\tinvalid syntax\n", "main.py");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].file.as_deref(), Some("main.py"));
        assert_eq!(items[0].line, Some(3));
        assert_eq!(items[0].message, "SyntaxError: invalid syntax");
        assert_eq!(
            items[0].rendered,
            "main.py:3:5: SyntaxError: invalid syntax"
        );
        // 行が分からないもの（ソースに NUL があるなど）
        let items = parse_check(
            "0\t0\tsource code string cannot contain null bytes\n",
            "main.py",
        );
        assert_eq!(items[0].line, None);
        assert!(parse_check("", "main.py").is_empty());
    }

    // 手元に CPython 3.13 があるときだけ、本当に構文を確かめる
    #[test]
    fn checks_syntax_with_local_python() {
        let Ok(interpreter) = find_interpreter(&PythonSettings::default()) else {
            eprintln!("python3.13 がないので飛ばします");
            return;
        };
        let dir = temp_dir("syntax");
        std::fs::write(dir.join("main.py"), "print(1)\nif True\n    print(2)\n").unwrap();
        let output = check_command(&interpreter, &dir, "main.py")
            .output()
            .unwrap();
        assert!(!output.status.success());
        let items = parse_check(&String::from_utf8_lossy(&output.stdout), "main.py");
        assert_eq!(items[0].line, Some(2), "{items:?}");
        std::fs::write(dir.join("main.py"), "print(1)\n").unwrap();
        assert!(
            check_command(&interpreter, &dir, "main.py")
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(!dir.join("__pycache__").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 提出の前の検査（Phase 19 作業 4。D-224）
    #[test]
    fn warns_before_submit() {
        let dir = temp_dir("warnings");
        std::fs::write(dir.join("lib.py"), "").unwrap();
        std::fs::create_dir_all(dir.join("pkg")).unwrap();
        let import = |name: &str, standard: bool| Import {
            name: name.to_string(),
            standard,
        };
        let imports = [
            import(".", false),
            import("collections", true),
            import("lib", false),
            import("numba", false),
            import("numpy", false),
            import("pkg", false),
            import("requests", false),
            import("sys", true),
        ];
        assert_eq!(
            submit_warnings(Interpreter::Cpython, &imports, &dir),
            [
                "自分のモジュールを読む import（from . import、lib、pkg）があります。ジャッジには、このファイルだけが置かれます",
                "ジャッジの CPython の言語の一覧にないパッケージ（requests）を import しています",
            ]
        );
        // PyPy のジャッジには numba がない
        assert_eq!(
            submit_warnings(Interpreter::Pypy, &imports, &dir)[1],
            "ジャッジの PyPy の言語の一覧にないパッケージ（numba、requests）を import しています"
        );
        // 標準ライブラリとジャッジのパッケージだけなら、注意はない
        let plain = [
            import("sys", true),
            import("sortedcontainers", false),
            import("atcoder", false),
        ];
        assert!(submit_warnings(Interpreter::Pypy, &plain, &dir).is_empty());
        // 版
        let cpython = parse_version("Python 3.13.7\n").unwrap();
        assert_eq!(version_warning(Interpreter::Cpython, &cpython), None);
        let newer = parse_version("Python 3.13.18\n").unwrap();
        assert_eq!(
            version_warning(Interpreter::Cpython, &newer).as_deref(),
            Some("手元の CPython（3.13.18）は、ジャッジ（3.13.7）と版が違います")
        );
        let pypy =
            parse_version("Python 3.11.17 (abc, Jan 1 2026)\n[PyPy 8.0.1 with GCC]\n").unwrap();
        assert_eq!(
            version_warning(Interpreter::Pypy, &pypy).as_deref(),
            Some("手元の PyPy（8.0.1）は、ジャッジ（7.3.20）と版が違います")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 手元の処理系で import を読む（入っているものだけ）。文字列・コメントの中は数えない
    #[test]
    fn reads_imports_with_local_python() {
        let dir = temp_dir("imports");
        std::fs::write(
            dir.join("main.py"),
            "import sys, os.path as p\nfrom collections import deque\nimport numpy as np\nfrom .lib import y\ns = \"import fake\"\n# import fake2\ndef f():\n    from numba import njit\n",
        )
        .unwrap();
        // 同じフォルダの ast.py は読ませない（-I）
        std::fs::write(dir.join("ast.py"), "raise SystemExit(3)\n").unwrap();
        let mut tried = 0;
        for interpreter in [Interpreter::Cpython, Interpreter::Pypy] {
            let settings = PythonSettings {
                interpreter,
                ..PythonSettings::default()
            };
            let Ok(found) = find_interpreter(&settings) else {
                eprintln!("{} がないので飛ばします", interpreter.label());
                continue;
            };
            tried += 1;
            let names: Vec<(String, bool)> = imports(&found, &dir.join("main.py"))
                .unwrap()
                .into_iter()
                .map(|import| (import.name, import.standard))
                .collect();
            let expected: Vec<(String, bool)> = [
                (".", false),
                ("collections", true),
                ("numba", false),
                ("numpy", false),
                ("os", true),
                ("sys", true),
            ]
            .iter()
            .map(|(name, standard)| (name.to_string(), *standard))
            .collect();
            assert_eq!(names, expected, "{}", interpreter.label());
        }
        // 構文の誤りでは読めない
        if tried > 0 {
            std::fs::write(dir.join("main.py"), "if True\n").unwrap();
            let found = find_interpreter(&PythonSettings::default())
                .or_else(|_| {
                    find_interpreter(&PythonSettings {
                        interpreter: Interpreter::Pypy,
                        ..PythonSettings::default()
                    })
                })
                .unwrap();
            assert!(imports(&found, &dir.join("main.py")).is_err());
        }
        assert!(!dir.join("__pycache__").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
