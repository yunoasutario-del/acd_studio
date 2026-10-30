// 問題・コンテストのダウンロード（Phase 7a 作業 1。D-151。docs/phase7/01-pre-start.md §3）
//
// - 作業領域の直下に `<コンテスト>/<問題 ID>/`（Cargo.toml・src/main.rs・test/sample-N、
//   test/input.txt・test/compare.json〔Phase 9 作業 1。D-161〕）を作る
// - コンテストのフォルダに `.cargo/config.toml`（target-dir = "target"）を置き、ビルドの置き場所を共有する
// - C++ を選んだときは、`<コンテスト>/<問題 ID>/` に main.cpp・compile_flags.txt・test/ を作る
//   （.cargo/config.toml は置かない。Phase 16 作業 3。D-208）。Python は main.py・test/（Phase 19。D-223）
// - すでにあるフォルダ・ファイルは、上書きも削除もしない。何を作り、何を飛ばしたかを返す
// - 問題・一覧の取得は problem.rs の取得（1 秒あける・保存して使い回す・開催中は D-149）を使う。押したときだけ
// - ファイルを書くだけで、コードは動かさない（作業領域の信頼は問わない）
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use crate::compare;
use crate::input_gen;
use crate::language::Language;
use crate::problem::{self, Problem, ProblemId, TaskEntry};

// 雛形の置き場所（アプリのフォルダの中）
pub const TEMPLATE_DIR: &str = "templates";
const DEFAULT_MAIN: &str = "fn main() {}\n";
// C++ の IDE の最小の雛形（D-208）
const DEFAULT_MAIN_CPP: &str = "#include <bits/stdc++.h>\nusing namespace std;\n\nint main() {\n    ios::sync_with_stdio(false);\n    cin.tie(nullptr);\n}\n";
// Python の IDE の最小の雛形（D-223）
const DEFAULT_MAIN_PY: &str = "def main():\n    pass\n\n\nmain()\n";
// ジャッジと同じ版と機能（D-148 の検査の一覧と同じ版。ジャッジは derive の機能を入れている。
// docs/phase0/03a-atcoder-environment.md §3。#[fastout] を使うのに要る。ユーザーの承認、2026-10-26）
const DEFAULT_DEPENDENCIES: &str = "proconio = { version = \"=0.5.0\", features = [\"derive\"] }\n";
const CARGO_CONFIG: &str = "# AtCoder Studio が作った。同じコンテストの問題のビルドの置き場所（target）を、このフォルダの中で共有する\n[build]\ntarget-dir = \"target\"\n";

// 雛形（src/main.rs の中身と、Cargo.toml の [dependencies] の中身。C++ なら main.cpp と compile_flags.txt の中身）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    // 作るパッケージの言語（D-208）。使うのは、その言語の項目だけ
    pub language: Language,
    pub main_rs: String,
    pub dependencies: String,
    pub main_cpp: String,
    pub compile_flags: String,
    pub main_py: String,
    // 問題を作るときに input! を入れておく（Phase 15 作業 4。D-198）。None なら入れない（テンプレートのまま）
    pub fill: Option<FillInput>,
}

// 問題を作るときに input! を入れる（D-198）。main_loop はテストケースが複数のとき main() に for まで書くか（D-197）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FillInput {
    pub main_loop: bool,
}

// 雛形のファイルがあるか（設定の画面の「テンプレート」に出す。Phase 13 作業 3。D-188）
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateFiles {
    pub main_rs: bool,
    pub dependencies: bool,
    pub main_cpp: bool,
    pub main_py: bool,
}

pub fn template_files(app_dir: &Path) -> TemplateFiles {
    let has = |name: &str| app_dir.join(TEMPLATE_DIR).join(name).is_file();
    TemplateFiles {
        main_rs: has("main.rs"),
        dependencies: has("dependencies.toml"),
        main_cpp: has("main.cpp"),
        main_py: has("main.py"),
    }
}

// templates/ のフォルダ。なければ空のものを作る（中のファイルは作らない。D-188）
pub fn ensure_template_dir(app_dir: &Path) -> Result<std::path::PathBuf, String> {
    let dir = app_dir.join(TEMPLATE_DIR);
    std::fs::create_dir_all(&dir).map_err(|_| "テンプレートのフォルダを作れません".to_string())?;
    Ok(dir)
}

// アプリのフォルダの templates/ の雛形を読む。Rust は main.rs・dependencies.toml、C++ は main.cpp（D-208）、Python は main.py（D-223）。
// ないものは IDE の最小の雛形を使う。使わない言語のファイルは読まない（compile_flags は呼び出し側が入れる）
pub fn load_template(app_dir: &Path, language: Language) -> Result<Template, String> {
    let read = |name: &str, fallback: &str| -> Result<String, String> {
        let path = app_dir.join(TEMPLATE_DIR).join(name);
        match std::fs::read(&path) {
            Ok(bytes) => String::from_utf8(bytes)
                .map_err(|_| format!("雛形の {name} を読めません（UTF-8 ではない）")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(fallback.to_string()),
            Err(_) => Err(format!("雛形の {name} を読めません")),
        }
    };
    Ok(match language {
        Language::Rust => Template {
            language,
            main_rs: read("main.rs", DEFAULT_MAIN)?,
            dependencies: read("dependencies.toml", DEFAULT_DEPENDENCIES)?,
            main_cpp: String::new(),
            compile_flags: String::new(),
            main_py: String::new(),
            fill: None,
        },
        Language::Cpp => Template {
            language,
            main_rs: String::new(),
            dependencies: String::new(),
            main_cpp: read("main.cpp", DEFAULT_MAIN_CPP)?,
            compile_flags: String::new(),
            main_py: String::new(),
            fill: None,
        },
        Language::Python => Template {
            language,
            main_rs: String::new(),
            dependencies: String::new(),
            main_cpp: String::new(),
            compile_flags: String::new(),
            main_py: read("main.py", DEFAULT_MAIN_PY)?,
            fill: None,
        },
    })
}

// パッケージの名前。Cargo は数字で始まる名前を受け付けないので、そのときは頭に p_ を付ける
fn package_name(task: &str) -> String {
    if task.starts_with(|c: char| c.is_ascii_digit()) {
        format!("p_{task}")
    } else {
        task.to_string()
    }
}

fn cargo_toml(name: &str, dependencies: &str) -> String {
    let mut dependencies = dependencies.trim_end().to_string();
    if !dependencies.is_empty() {
        dependencies.push('\n');
    }
    format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n{dependencies}\n# AtCoder Studio が作った。上のフォルダに Cargo のワークスペースがあってもぶつからないように、このパッケージだけのワークスペースにする\n[workspace]\n"
    )
}

// 1 問のパッケージを作った結果。パスは作業領域のルートからの相対パス
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageResult {
    pub task: String,
    // パッケージのフォルダ（例：abc480/abc480_a）
    pub path: String,
    // 新しく作ったファイル
    pub created: Vec<String>,
    // すでにあったので、そのままにしたファイル（サンプルは同じ内容のもの）
    pub existed: Vec<String>,
    // すでにあって中身が違うので、書かなかったサンプル
    pub differed: Vec<String>,
    // 作れなかった理由（取れなかった・書けなかった）。作れたら None
    pub error: Option<String>,
    // input! を入れたか（D-198。入れる設定で main.rs を新しく書いたときだけ）
    pub input: Option<String>,
    // 作ったあとに開くファイル（例：abc480/abc480_a/src/main.rs、C++ なら abc480/abc480_a/main.cpp）
    pub main: String,
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|path| {
            path.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

// フォルダを作り（すでにあればそのまま）、リンクなどで作業領域の外を指していないことを確かめる
fn ensure_dir_inside(root: &Path, dir: &Path) -> Result<(), String> {
    if dir.exists() && !dir.is_dir() {
        return Err(format!(
            "「{}」はフォルダではありません",
            relative(root, dir)
        ));
    }
    std::fs::create_dir_all(dir).map_err(|_| format!("「{}」を作れません", relative(root, dir)))?;
    if !dir.canonicalize().is_ok_and(|real| real.starts_with(root)) {
        return Err(format!(
            "「{}」が作業領域の外にあります",
            relative(root, dir)
        ));
    }
    Ok(())
}

// ないときだけ書く。書いたら true。create_new なので、すでにあるファイル（リンクを含む）は開かず、
// 確かめてから書くまでの間に作られても上書きしない
fn write_new(path: &Path, content: &str) -> Result<bool, String> {
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(_) => return Err("書けません".to_string()),
    };
    file.write_all(content.as_bytes())
        .map_err(|_| "書けません".to_string())?;
    Ok(true)
}

// 問題の時間の制限（「2 sec」「2.5 sec」「500 ms」）のミリ秒。読めなければ None
fn time_limit_ms(text: &str) -> Option<u64> {
    let mut parts = text.split_whitespace();
    let number: f64 = parts.next()?.parse().ok()?;
    let scale = match parts.next()? {
        "sec" | "s" | "秒" => 1000.0,
        "ms" => 1.0,
        _ => return None,
    };
    let ms = (number * scale).round();
    (ms.is_finite() && ms >= 1.0).then_some(ms as u64)
}

// 問題のメモリの制限（「1024 MiB」。古い問題の「256 MB」も数をそのまま使う。「1 GiB」）の MiB。読めなければ None
fn memory_limit_mib(text: &str) -> Option<u64> {
    let mut parts = text.split_whitespace();
    let number: f64 = parts.next()?.parse().ok()?;
    let scale = match parts.next()? {
        "MiB" | "MB" => 1.0,
        "GiB" | "GB" => 1024.0,
        _ => return None,
    };
    let mib = (number * scale).round();
    (mib.is_finite() && mib >= 1.0).then_some(mib as u64)
}

// test/compare.json の中身（D-161）。時間の制限は問題の制限＋500 ms、メモリは問題の値（読めなければ IDE の既定）。
// 答えが複数ありうる問題なら none（目視確認。D-168）、そうでなく問題文の「出力」に誤差の書き方があれば float。
// キーの順を決めたいので、文字で組み立てる
pub fn compare_json(problem: &Problem) -> String {
    let time = time_limit_ms(&problem.time_limit)
        .map(|ms| (ms + 500).min(compare::MAX_TIME_LIMIT_MS))
        .unwrap_or(compare::DEFAULT_TIME_LIMIT_MS);
    let memory = memory_limit_mib(&problem.memory_limit)
        .map(|mib| mib.min(compare::MAX_MEMORY_LIMIT_MIB))
        .unwrap_or(compare::DEFAULT_MEMORY_LIMIT_MIB);
    let mut lines = Vec::new();
    let tolerance = if problem::multiple_answers(&problem.statement_html) {
        lines.push("  \"mode\": \"none\"".to_string());
        None
    } else {
        Some(problem::output_tolerance(&problem.statement_html))
    };
    match tolerance {
        None => {}
        Some(Some(tolerance)) => {
            lines.push("  \"mode\": \"float\"".to_string());
            if tolerance.abs {
                lines.push(format!("  \"abs\": 1e-{}", tolerance.exponent));
            }
            if tolerance.rel {
                lines.push(format!("  \"rel\": 1e-{}", tolerance.exponent));
            }
        }
        Some(None) => lines.push("  \"mode\": \"whitespace\"".to_string()),
    }
    lines.push(format!("  \"timeLimitMs\": {time}"));
    lines.push(format!("  \"memoryLimitMiB\": {memory}"));
    lines.push(format!(
        "  \"randomCount\": {}",
        compare::DEFAULT_RANDOM_COUNT
    ));
    format!("{{\n{}\n}}\n", lines.join(",\n"))
}

// 「コンテストを作る」で作ったコンテストのフォルダか（IDE が書いた .cargo/config.toml がある。
// rust-analyzer をフォルダごとに動かすのに使う。Phase 10 作業 11。D-171）。中身の最初の行で見分ける
pub fn is_contest_dir(dir: &Path) -> bool {
    let first = CARGO_CONFIG.lines().next().unwrap_or_default();
    std::fs::read_to_string(dir.join(".cargo").join("config.toml"))
        .is_ok_and(|text| text.lines().next() == Some(first))
}

// コンテストのフォルダ（作業領域の直下）と、その .cargo/config.toml（Rust のとき）・.clang-format（C++ のとき。
// clangd の整形の書き方。Phase 17 作業 5。D-208・D-210）。どちらも、ないときだけ書く。Python は何も置かない
// （Phase 19 作業 3。D-223）。作ったファイルを返す
fn prepare_contest_dir(
    root: &Path,
    contest: &str,
    language: Language,
) -> Result<(PathBuf, Vec<String>), String> {
    let dir = root.join(contest);
    ensure_dir_inside(root, &dir)?;
    if language == Language::Python {
        return Ok((dir, Vec::new()));
    }
    if language == Language::Cpp {
        let style = dir.join(".clang-format");
        let mut created = Vec::new();
        if write_new(&style, crate::cpp::CLANG_FORMAT)
            .map_err(|error| format!("「{}」を{error}", relative(root, &style)))?
        {
            created.push(relative(root, &style));
        }
        return Ok((dir, created));
    }
    let cargo_dir = dir.join(".cargo");
    ensure_dir_inside(root, &cargo_dir)?;
    let config = cargo_dir.join("config.toml");
    let mut created = Vec::new();
    if write_new(&config, CARGO_CONFIG)
        .map_err(|error| format!("「{}」を{error}", relative(root, &config)))?
    {
        created.push(relative(root, &config));
    }
    Ok((dir, created))
}

// 1 問のパッケージを、コンテストのフォルダ（contest_dir）の中に作る。すでにあるものは変えない
pub fn write_package(
    root: &Path,
    contest_dir: &Path,
    task: &str,
    problem: &Problem,
    template: &Template,
) -> PackageResult {
    let dir = contest_dir.join(task);
    let main_path = match template.language {
        Language::Rust => dir.join("src").join("main.rs"),
        Language::Cpp => dir.join(crate::cpp::SOURCE),
        Language::Python => dir.join(crate::python::SOURCE),
    };
    let mut result = PackageResult {
        task: task.to_string(),
        path: relative(root, &dir),
        main: relative(root, &main_path),
        ..PackageResult::default()
    };
    // ほかの言語のパッケージのフォルダには、ファイルを足さない（D-208）
    if let Some(other) = [Language::Rust, Language::Cpp, Language::Python]
        .into_iter()
        .find(|other| *other != template.language && other.is_package(&dir))
    {
        result.error = Some(format!(
            "「{}」は {} のパッケージなので、{} のファイルは作りません",
            result.path,
            other.label(),
            template.language.label()
        ));
        return result;
    }
    // 入れる設定なら、テンプレートの solve()（と main()）の input! を作ったものにする（D-198）。
    // C++ は main.cpp の solve()（なければ main()）に、目印で囲んで入れる（Phase 17 作業 8。D-213）。
    // 作れないときは、テンプレートのまま書いて理由を知らせる
    let source = match template.language {
        Language::Rust => &template.main_rs,
        Language::Cpp => &template.main_cpp,
        Language::Python => &template.main_py,
    };
    let (main_text, input_note) = match template.fill {
        None => (source.clone(), None),
        Some(fill) => match input_gen::generate_for(
            &problem.statement_html,
            &problem.samples,
            template.language,
        )
        .and_then(|generated| {
            let text = match template.language {
                Language::Rust => input_gen::fill_template(source, &generated, fill.main_loop),
                Language::Cpp => crate::input_cpp::fill(
                    source,
                    &generated.solve,
                    generated.main.as_deref(),
                    fill.main_loop,
                )
                .map(|filled| filled.text),
                // Python は行ごとに読む書き方で、main.py の solve()（なければ main()）に入れる（Phase 20 作業 7。D-227）
                Language::Python => crate::input_py::fill(
                    source,
                    &generated.solve,
                    generated.main.as_deref(),
                    fill.main_loop,
                )
                .map(|filled| filled.text),
            };
            text.map(|text| (text, input_gen::fill_note(&generated)))
        }) {
            Ok((text, note)) => (text, Some(note)),
            Err(reason) => (
                source.clone(),
                Some(format!(
                    "{}を作れませんでした（{reason}）",
                    input_gen::input_name(template.language)
                )),
            ),
        },
    };
    let outcome = (|| -> Result<(), String> {
        ensure_dir_inside(root, &dir)?;
        let files = match template.language {
            Language::Rust => {
                ensure_dir_inside(root, &dir.join("src"))?;
                vec![
                    (
                        dir.join("Cargo.toml"),
                        cargo_toml(&package_name(task), &template.dependencies),
                    ),
                    (main_path.clone(), main_text),
                ]
            }
            Language::Cpp => vec![
                (
                    dir.join("compile_flags.txt"),
                    template.compile_flags.clone(),
                ),
                (main_path.clone(), main_text),
            ],
            Language::Python => vec![(main_path.clone(), main_text)],
        };
        for (path, content) in files {
            let name = relative(root, &path);
            if write_new(&path, &content).map_err(|error| format!("「{name}」を{error}"))? {
                // すでにある main.rs は変えないので、新しく書いたときだけ知らせる
                if path == main_path {
                    result.input = input_note.clone();
                }
                result.created.push(name);
            } else {
                result.existed.push(name);
            }
        }
        // サンプルは、今の「サンプルを保存」と同じ書き方（上書きしない。D-112）。bin が 1 つなので test/（D-106）
        let tests = dir.join("test");
        ensure_dir_inside(root, &tests)?;
        let saved = problem::save_samples(&tests, &problem.samples)?;
        let prefix = relative(root, &tests);
        result
            .created
            .extend(saved.written.iter().map(|name| format!("{prefix}/{name}")));
        result.existed.extend(
            saved
                .unchanged
                .iter()
                .map(|name| format!("{prefix}/{name}")),
        );
        result
            .differed
            .extend(saved.skipped.iter().map(|name| format!("{prefix}/{name}")));
        // 「実行」の標準入力（入力例 1。サンプルがなければ空）と、比べ方・制限（D-161）。すでにあれば変えない
        let input = problem
            .samples
            .first()
            .map(|sample| sample.input.clone())
            .unwrap_or_default();
        for (path, content) in [
            (tests.join("input.txt"), input),
            (tests.join("compare.json"), compare_json(problem)),
        ] {
            let name = relative(root, &path);
            if write_new(&path, &content).map_err(|error| format!("「{name}」を{error}"))? {
                result.created.push(name);
            } else {
                result.existed.push(name);
            }
        }
        Ok(())
    })();
    result.error = outcome.err();
    result
}

// ---- 1 問・コンテスト ----

// 1 問だけ作る。load は問題を返す（アプリでは problem::load_problem。保存してあれば読まない）
pub fn create_problem(
    root: &Path,
    id: &ProblemId,
    template: &Template,
    load: &dyn Fn(&ProblemId) -> Result<Problem, String>,
) -> Result<ProblemCreated, String> {
    let problem = load(id)?;
    let (contest_dir, config_created) = prepare_contest_dir(root, &id.contest, template.language)?;
    let package = write_package(root, &contest_dir, &id.task, &problem, template);
    Ok(ProblemCreated {
        config_created,
        package,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemCreated {
    // 作った .cargo/config.toml（すでにあれば空）
    pub config_created: Vec<String>,
    pub package: PackageResult,
}

// コンテストを作っている間の知らせ
#[derive(Debug, Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum ScaffoldEvent {
    // 問題の一覧を読んだ
    Listed {
        total: usize,
    },
    // index 番目（0 から）の問題を取り始めた
    Started {
        index: usize,
        total: usize,
        task: String,
    },
    // 1 問の結果
    Finished {
        result: PackageResult,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContestCreated {
    pub contest: String,
    pub config_created: Vec<String>,
    // 問題ごとの結果（一覧の順。止めたら、そこまで）
    pub packages: Vec<PackageResult>,
    // 一覧の問題の数
    pub total: usize,
    // 途中で止めた
    pub stopped: bool,
}

// コンテストの問題をすべて作る。list は問題の一覧、load は問題を返す。1 問ずつ順に取り、cancel が立ったら止める。
// 取れなかった・書けなかった問題は、理由を付けて飛ばす（自動では読み直さない）
pub fn create_contest(
    root: &Path,
    contest: &str,
    template: &Template,
    list: &dyn Fn(&str) -> Result<Vec<TaskEntry>, String>,
    load: &dyn Fn(&ProblemId) -> Result<Problem, String>,
    cancel: &AtomicBool,
    on_event: &dyn Fn(ScaffoldEvent),
) -> Result<ContestCreated, String> {
    // ID の形を確かめる（問題 ID の部分は仮のもの）
    ProblemId::new(contest, "x")?;
    let tasks = list(contest)
        .map_err(|error| format!("{error}（開始前のコンテストは、問題の一覧を読めません）"))?;
    on_event(ScaffoldEvent::Listed { total: tasks.len() });
    let (contest_dir, config_created) = prepare_contest_dir(root, contest, template.language)?;
    let mut packages = Vec::new();
    let mut stopped = false;
    for (index, entry) in tasks.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            stopped = true;
            break;
        }
        on_event(ScaffoldEvent::Started {
            index,
            total: tasks.len(),
            task: entry.task.clone(),
        });
        let result = match ProblemId::new(contest, &entry.task).and_then(|id| load(&id)) {
            Ok(problem) => write_package(root, &contest_dir, &entry.task, &problem, template),
            Err(error) => PackageResult {
                task: entry.task.clone(),
                path: relative(root, &contest_dir.join(&entry.task)),
                error: Some(error),
                ..PackageResult::default()
            },
        };
        on_event(ScaffoldEvent::Finished {
            result: result.clone(),
        });
        packages.push(result);
    }
    Ok(ContestCreated {
        contest: contest.to_string(),
        config_created,
        packages,
        total: tasks.len(),
        stopped,
    })
}

// ---- アプリの中の状態 ----

// 作る処理は 1 つずつ。止める印を持つ
#[derive(Default)]
pub struct ScaffoldControl {
    running: AtomicBool,
    cancel: std::sync::Mutex<Option<Arc<AtomicBool>>>,
}

// 作る処理の間だけ持つ印
pub struct ScaffoldGuard<'a>(&'a ScaffoldControl);

impl Drop for ScaffoldGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut cancel) = self.0.cancel.lock() {
            *cancel = None;
        }
        self.0.running.store(false, Ordering::SeqCst);
    }
}

impl ScaffoldControl {
    pub fn begin(&self) -> Result<(ScaffoldGuard<'_>, Arc<AtomicBool>), String> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err("問題を作っているところです".to_string());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        if let Ok(mut current) = self.cancel.lock() {
            *current = Some(cancel.clone());
        }
        Ok((ScaffoldGuard(self), cancel))
    }

    // 作っている途中か（フォルダの名前の変更・削除を断るのに使う。D-157）
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn stop(&self) {
        if let Ok(current) = self.cancel.lock()
            && let Some(cancel) = current.as_ref()
        {
            cancel.store(true, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_and_creates_the_template_folder() {
        let app =
            std::env::temp_dir().join(format!("atcoder-studio-templates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&app);
        std::fs::create_dir_all(&app).unwrap();
        let none = TemplateFiles {
            main_cpp: false,
            main_rs: false,
            dependencies: false,
            main_py: false,
        };
        assert_eq!(template_files(&app), none);
        // ないときは空のフォルダを作るだけ（中のファイルは作らない）
        let dir = ensure_template_dir(&app).unwrap();
        assert!(dir.is_dir());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        assert_eq!(template_files(&app), none);
        std::fs::write(dir.join("main.rs"), "fn main() {}").unwrap();
        assert_eq!(
            template_files(&app),
            TemplateFiles {
                main_rs: true,
                dependencies: false,
                main_cpp: false,
                main_py: false,
            }
        );
        // もう一度呼んでも、中身は変えない
        ensure_template_dir(&app).unwrap();
        assert!(dir.join("main.rs").is_file());
        let _ = std::fs::remove_dir_all(&app);
    }
    use crate::problem::Sample;
    use std::sync::Mutex;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "atcoder-studio-scaffold-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path.canonicalize().unwrap())
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn problem(samples: &[(&str, &str)]) -> Problem {
        Problem {
            title: "A - 例".to_string(),
            time_limit: "2 sec".to_string(),
            memory_limit: "1024 MiB".to_string(),
            statement_html: String::new(),
            samples: samples
                .iter()
                .map(|(input, output)| Sample {
                    input: input.to_string(),
                    output: output.to_string(),
                })
                .collect(),
        }
    }

    fn default_template() -> Template {
        Template {
            language: Language::Rust,
            main_rs: DEFAULT_MAIN.to_string(),
            dependencies: DEFAULT_DEPENDENCIES.to_string(),
            main_cpp: String::new(),
            compile_flags: String::new(),
            main_py: String::new(),
            fill: None,
        }
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn names_packages_cargo_accepts() {
        assert_eq!(package_name("abc480_a"), "abc480_a");
        assert_eq!(package_name("typical90-a"), "typical90-a");
        assert_eq!(package_name("1_a"), "p_1_a");
        let toml = cargo_toml("abc480_a", "proconio = \"=0.5.0\"\n\n");
        assert!(toml.starts_with(
            "[package]\nname = \"abc480_a\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
        ));
        assert!(toml.contains("[dependencies]\nproconio = \"=0.5.0\"\n\n#"));
        assert!(toml.trim_end().ends_with("[workspace]"));
        assert!(cargo_toml("x", "").contains("[dependencies]\n\n#"));
    }

    #[test]
    fn reads_the_template_or_uses_the_default() {
        let app = TempDir::new("template");
        assert_eq!(
            load_template(&app.0, Language::Rust).unwrap(),
            default_template()
        );
        std::fs::create_dir_all(app.0.join(TEMPLATE_DIR)).unwrap();
        std::fs::write(
            app.0.join(TEMPLATE_DIR).join("main.rs"),
            "use proconio::input;\nfn main() {}\n",
        )
        .unwrap();
        // 片方だけ置いたときは、もう片方は IDE の雛形
        let template = load_template(&app.0, Language::Rust).unwrap();
        assert_eq!(template.main_rs, "use proconio::input;\nfn main() {}\n");
        assert_eq!(template.dependencies, DEFAULT_DEPENDENCIES);
        // ジャッジと同じく、proconio の derive の機能（#[fastout]）を入れる
        assert_eq!(
            DEFAULT_DEPENDENCIES,
            "proconio = { version = \"=0.5.0\", features = [\"derive\"] }\n"
        );
        std::fs::write(app.0.join(TEMPLATE_DIR).join("dependencies.toml"), "").unwrap();
        assert_eq!(
            load_template(&app.0, Language::Rust).unwrap().dependencies,
            ""
        );
        std::fs::write(app.0.join(TEMPLATE_DIR).join("main.rs"), [0xff, 0xfe]).unwrap();
        assert!(
            load_template(&app.0, Language::Rust)
                .unwrap_err()
                .contains("UTF-8")
        );
    }

    #[test]
    fn fills_the_input_when_the_setting_is_on() {
        let root = TempDir::new("fill");
        let template = |fill: Option<FillInput>| {
            Template {
            language: Language::Rust,
            main_rs: "fn solve() {\n    input! {\n        n: usize,\n    }\n}\n\nfn main() {\n    solve();\n}\n"
                .to_string(),
            dependencies: String::new(),
            main_cpp: String::new(),
            compile_flags: String::new(),
            main_py: String::new(),
            fill,
        }
        };
        let mut sample = problem(&[("2\n3 4\n", "7\n")]);
        sample.statement_html = "<section><h3>制約</h3><ul><li><var>1 \\leq N, A_i \\leq 10</var></li></ul></section>\
             <section><h3>入力</h3><pre><var>N</var>\n<var>A_1</var> <var>\\ldots</var> <var>A_N</var></pre></section>"
            .to_string();
        let load = |_: &ProblemId| Ok(sample.clone());
        // 入：solve() の input! が作ったものになり、知らせる
        let id = ProblemId::new("abc480", "abc480_a").unwrap();
        let on = Some(FillInput { main_loop: false });
        let created = create_problem(&root.0, &id, &template(on), &load).unwrap();
        assert_eq!(
            created.package.input.as_deref(),
            Some("input! を入れました")
        );
        assert_eq!(
            read(&root.0.join("abc480/abc480_a/src/main.rs")),
            "fn solve() {\n    input! {\n        n: usize,\n        a: [usize; n],\n    }\n}\n\nfn main() {\n    solve();\n}\n"
        );
        // すでにある main.rs は変えず、知らせない
        std::fs::write(
            root.0.join("abc480/abc480_a/src/main.rs"),
            "// 自分で書いた\n",
        )
        .unwrap();
        let again = create_problem(&root.0, &id, &template(on), &load).unwrap();
        assert_eq!(again.package.input, None);
        assert_eq!(
            read(&root.0.join("abc480/abc480_a/src/main.rs")),
            "// 自分で書いた\n"
        );
        // 切：テンプレートのまま
        let id = ProblemId::new("abc480", "abc480_b").unwrap();
        let off = create_problem(&root.0, &id, &template(None), &load).unwrap();
        assert_eq!(off.package.input, None);
        assert_eq!(
            read(&root.0.join("abc480/abc480_b/src/main.rs")),
            template(None).main_rs
        );
        // 作れない問題（形式がない）：テンプレートのまま書き、理由を知らせる
        let plain = problem(&[]);
        let id = ProblemId::new("abc480", "abc480_c").unwrap();
        let failed = create_problem(&root.0, &id, &template(on), &|_| Ok(plain.clone())).unwrap();
        assert_eq!(
            failed.package.input.as_deref(),
            Some("input! を作れませんでした（問題文に「入力」の形式が見つかりません）")
        );
        assert_eq!(
            read(&root.0.join("abc480/abc480_c/src/main.rs")),
            template(None).main_rs
        );
    }

    // C++ でも入れる（Phase 17 作業 8。D-213）。作った main.cpp を g++-15 でビルドして、入力例を読めることも確かめる
    #[test]
    fn fills_the_cpp_input_when_the_setting_is_on() {
        let root = TempDir::new("fill-cpp");
        let app = TempDir::new("fill-cpp-template");
        let mut template = load_template(&app.0, Language::Cpp).unwrap();
        template.fill = Some(FillInput { main_loop: false });
        let mut sample = problem(&[("2\n3 4\n", "7\n")]);
        sample.statement_html = "<section><h3>制約</h3><ul><li><var>1 \\leq N, A_i \\leq 10</var></li></ul></section>\
             <section><h3>入力</h3><pre><var>N</var>\n<var>A_1</var> <var>\\ldots</var> <var>A_N</var></pre></section>"
            .to_string();
        let load = |_: &ProblemId| Ok(sample.clone());
        let id = ProblemId::new("abc480", "abc480_a").unwrap();
        let created = create_problem(&root.0, &id, &template, &load).unwrap();
        assert_eq!(
            created.package.input.as_deref(),
            Some("入力のコードを入れました")
        );
        let main = root.0.join("abc480/abc480_a/main.cpp");
        assert_eq!(
            read(&main),
            "#include <bits/stdc++.h>\nusing namespace std;\n\nint main() {\n    ios::sync_with_stdio(false);\n    cin.tie(nullptr);\n    // 入力（AtCoder Studio）\n    long long N;\n    cin >> N;\n    vector<long long> A(N);\n    for (auto& a : A) cin >> a;\n    // 入力ここまで\n}\n"
        );
        // 作れない問題：テンプレートのまま書き、理由を知らせる
        let plain = problem(&[]);
        let id = ProblemId::new("abc480", "abc480_b").unwrap();
        let failed = create_problem(&root.0, &id, &template, &|_| Ok(plain.clone())).unwrap();
        assert_eq!(
            failed.package.input.as_deref(),
            Some("入力のコードを作れませんでした（問題文に「入力」の形式が見つかりません）")
        );
        assert_eq!(
            read(&root.0.join("abc480/abc480_b/main.cpp")),
            DEFAULT_MAIN_CPP
        );

        // 作ったコードで入力例を読み、合計を出してみる（手元に g++-15 があるときだけ）
        let Ok(compiler) = crate::cpp::find_compiler(&crate::cpp::CppSettings::default()) else {
            eprintln!("g++-15 がないので、ビルドは飛ばします");
            return;
        };
        let source = read(&main).replace(
            "    // 入力ここまで\n",
            "    // 入力ここまで\n    cout << accumulate(A.begin(), A.end(), 0LL) << endl;\n",
        );
        std::fs::write(&main, source).unwrap();
        let dir = root.0.join("abc480/abc480_a");
        let output = crate::cpp::output_path(&dir, true);
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        let status = std::process::Command::new(&compiler)
            .current_dir(&dir)
            .args(crate::cpp::build_args(
                true,
                &output,
                &crate::cpp::CppSettings::default(),
            ))
            .status()
            .unwrap();
        assert!(status.success());
        let mut child = std::process::Command::new(&output)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child.stdin.take().unwrap().write_all(b"2\n3 4\n").unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "7\n");
    }

    #[test]
    fn creates_a_cpp_package() {
        let root = TempDir::new("cpp-package");
        let app = TempDir::new("cpp-template");
        let mut template = load_template(&app.0, Language::Cpp).unwrap();
        assert_eq!(template.main_cpp, DEFAULT_MAIN_CPP);
        template.compile_flags = "-std=gnu++23\n".to_string();
        let id = ProblemId::new("abc480", "abc480_a").unwrap();
        let sample = problem(&[("1 2\n", "3\n")]);
        let load = |_: &ProblemId| Ok(sample.clone());
        let created = create_problem(&root.0, &id, &template, &load).unwrap();
        // .cargo/config.toml は作らず、コンテストのフォルダに .clang-format を置く（D-210）
        assert_eq!(created.config_created, vec!["abc480/.clang-format"]);
        assert!(!root.0.join("abc480/.cargo").exists());
        assert_eq!(
            read(&root.0.join("abc480/.clang-format")),
            crate::cpp::CLANG_FORMAT
        );
        // 利用者が書き換えた .clang-format は、次の問題を作るときも変えない
        std::fs::write(
            root.0.join("abc480/.clang-format"),
            "BasedOnStyle: Google\n",
        )
        .unwrap();
        let next = ProblemId::new("abc480", "abc480_c").unwrap();
        let again = create_problem(&root.0, &next, &template, &load).unwrap();
        assert!(again.config_created.is_empty());
        assert_eq!(
            read(&root.0.join("abc480/.clang-format")),
            "BasedOnStyle: Google\n"
        );
        assert_eq!(created.package.error, None);
        assert_eq!(created.package.main, "abc480/abc480_a/main.cpp");
        assert_eq!(
            created.package.created,
            vec![
                "abc480/abc480_a/compile_flags.txt",
                "abc480/abc480_a/main.cpp",
                "abc480/abc480_a/test/sample-1.in",
                "abc480/abc480_a/test/sample-1.out",
                "abc480/abc480_a/test/input.txt",
                "abc480/abc480_a/test/compare.json",
            ]
        );
        let dir = root.0.join("abc480/abc480_a");
        assert_eq!(read(&dir.join("main.cpp")), DEFAULT_MAIN_CPP);
        assert_eq!(read(&dir.join("compile_flags.txt")), "-std=gnu++23\n");
        // 作ったフォルダは C++ の対象になる
        assert!(Language::Cpp.is_package(&dir));
        // 自分の雛形があれば使う
        std::fs::create_dir_all(app.0.join(TEMPLATE_DIR)).unwrap();
        std::fs::write(app.0.join(TEMPLATE_DIR).join("main.cpp"), "// 雛形\n").unwrap();
        assert_eq!(
            load_template(&app.0, Language::Cpp).unwrap().main_cpp,
            "// 雛形\n"
        );
        assert!(template_files(&app.0).main_cpp);
        // Rust のパッケージのフォルダには C++ のファイルを足さない（逆も）
        let rust = Template {
            language: Language::Rust,
            main_rs: DEFAULT_MAIN.to_string(),
            dependencies: String::new(),
            main_cpp: String::new(),
            compile_flags: String::new(),
            main_py: String::new(),
            fill: None,
        };
        let refused = create_problem(&root.0, &id, &rust, &load).unwrap();
        assert_eq!(
            refused.package.error.as_deref(),
            Some("「abc480/abc480_a」は C++ のパッケージなので、Rust のファイルは作りません")
        );
        assert!(!dir.join("Cargo.toml").exists());
        let id_b = ProblemId::new("abc480", "abc480_b").unwrap();
        create_problem(&root.0, &id_b, &rust, &load).unwrap();
        let refused = create_problem(&root.0, &id_b, &template, &load).unwrap();
        assert!(
            refused
                .package
                .error
                .unwrap()
                .contains("Rust のパッケージなので、C++ のファイルは作りません")
        );
        assert!(!root.0.join("abc480/abc480_b/main.cpp").exists());
    }

    // Python は main.py と test/ だけ（Phase 19 作業 3。D-223）。入力のコードを入れておく設定なら、main() に入れる
    // （Phase 20 作業 7。D-227）
    #[test]
    fn creates_a_python_package() {
        let root = TempDir::new("python-package");
        let app = TempDir::new("python-template");
        let mut template = load_template(&app.0, Language::Python).unwrap();
        assert_eq!(template.main_py, DEFAULT_MAIN_PY);
        template.fill = Some(FillInput { main_loop: false });
        let mut sample = problem(&[("1 2\n", "3\n")]);
        sample.statement_html =
            "<section><h3>入力</h3><pre><var>A</var> <var>B</var></pre></section>".to_string();
        let load = |_: &ProblemId| Ok(sample.clone());
        let id = ProblemId::new("abc480", "abc480_a").unwrap();
        let created = create_problem(&root.0, &id, &template, &load).unwrap();
        // .cargo/config.toml も .clang-format も作らない
        assert!(created.config_created.is_empty());
        assert!(!root.0.join("abc480/.cargo").exists());
        assert!(!root.0.join("abc480/.clang-format").exists());
        assert_eq!(created.package.error, None);
        assert_eq!(
            created.package.input.as_deref(),
            Some("入力のコードを入れました")
        );
        assert_eq!(created.package.main, "abc480/abc480_a/main.py");
        assert_eq!(
            created.package.created,
            vec![
                "abc480/abc480_a/main.py",
                "abc480/abc480_a/test/sample-1.in",
                "abc480/abc480_a/test/sample-1.out",
                "abc480/abc480_a/test/input.txt",
                "abc480/abc480_a/test/compare.json",
            ]
        );
        let dir = root.0.join("abc480/abc480_a");
        assert_eq!(
            read(&dir.join("main.py")),
            "def main():\n    # 入力（AtCoder Studio）\n    A, B = map(int, input().split())\n    # 入力ここまで\n\n\nmain()\n"
        );
        // 作ったフォルダは Python の対象になる
        assert!(Language::Python.is_package(&dir));
        // 自分の雛形があれば使う
        assert!(!template_files(&app.0).main_py);
        std::fs::create_dir_all(app.0.join(TEMPLATE_DIR)).unwrap();
        std::fs::write(app.0.join(TEMPLATE_DIR).join("main.py"), "# 雛形\n").unwrap();
        assert_eq!(
            load_template(&app.0, Language::Python).unwrap().main_py,
            "# 雛形\n"
        );
        assert!(template_files(&app.0).main_py);
        // C++ のパッケージのフォルダには Python のファイルを足さない（逆も）
        let cpp = load_template(&app.0, Language::Cpp).unwrap();
        let refused = create_problem(&root.0, &id, &cpp, &load).unwrap();
        assert_eq!(
            refused.package.error.as_deref(),
            Some("「abc480/abc480_a」は Python のパッケージなので、C++ のファイルは作りません")
        );
        assert!(!dir.join("main.cpp").exists());
        let id_b = ProblemId::new("abc480", "abc480_b").unwrap();
        create_problem(&root.0, &id_b, &cpp, &load).unwrap();
        let refused = create_problem(&root.0, &id_b, &template, &load).unwrap();
        assert!(
            refused
                .package
                .error
                .unwrap()
                .contains("C++ のパッケージなので、Python のファイルは作りません")
        );
        assert!(!root.0.join("abc480/abc480_b/main.py").exists());
    }

    #[test]
    fn creates_a_package_and_never_overwrites() {
        let root = TempDir::new("package");
        let template = Template {
            language: Language::Rust,
            main_rs: "// 雛形\nfn main() {}\n".to_string(),
            dependencies: "proconio = \"=0.5.0\"\n".to_string(),
            main_cpp: String::new(),
            compile_flags: String::new(),
            main_py: String::new(),
            fill: None,
        };
        let id = ProblemId::new("abc480", "abc480_a").unwrap();
        let sample = problem(&[("1 2\n", "3\n"), ("5 5\n", "10\n")]);
        let load = |_: &ProblemId| Ok(sample.clone());
        let first = create_problem(&root.0, &id, &template, &load).unwrap();
        assert_eq!(first.config_created, vec!["abc480/.cargo/config.toml"]);
        assert_eq!(first.package.path, "abc480/abc480_a");
        assert_eq!(first.package.error, None);
        assert_eq!(
            first.package.created,
            vec![
                "abc480/abc480_a/Cargo.toml",
                "abc480/abc480_a/src/main.rs",
                "abc480/abc480_a/test/sample-1.in",
                "abc480/abc480_a/test/sample-1.out",
                "abc480/abc480_a/test/sample-2.in",
                "abc480/abc480_a/test/sample-2.out",
                "abc480/abc480_a/test/input.txt",
                "abc480/abc480_a/test/compare.json",
            ]
        );
        let dir = root.0.join("abc480/abc480_a");
        // input.txt は入力例 1、compare.json は問題の制限から（D-161）。どちらも IDE がそのまま読める
        assert_eq!(read(&dir.join("test/input.txt")), "1 2\n");
        let settings = compare::read_settings(&dir.join("test")).unwrap();
        assert_eq!(settings.mode, compare::Mode::Whitespace);
        assert_eq!(
            (settings.time_limit_ms, settings.memory_limit_mib),
            (2500, 1024)
        );
        assert_eq!(read(&dir.join("src/main.rs")), "// 雛形\nfn main() {}\n");
        assert!(read(&dir.join("Cargo.toml")).contains("name = \"abc480_a\""));
        assert_eq!(read(&dir.join("test/sample-2.out")), "10\n");
        assert_eq!(
            read(&root.0.join("abc480/.cargo/config.toml")),
            CARGO_CONFIG
        );
        // 書き換えた main.rs・違う中身のサンプルはそのまま。消したサンプルだけ作り直す
        std::fs::write(dir.join("src/main.rs"), "// 書き換えた\n").unwrap();
        std::fs::write(dir.join("test/sample-1.out"), "4\n").unwrap();
        std::fs::remove_file(dir.join("test/sample-2.in")).unwrap();
        std::fs::write(root.0.join("abc480/.cargo/config.toml"), "# 変えた\n").unwrap();
        // 書き換えた input.txt はそのまま、消した compare.json だけ作り直す
        std::fs::write(dir.join("test/input.txt"), "9 9\n").unwrap();
        std::fs::remove_file(dir.join("test/compare.json")).unwrap();
        let second = create_problem(&root.0, &id, &template, &load).unwrap();
        assert!(second.config_created.is_empty());
        assert_eq!(
            second.package.created,
            vec![
                "abc480/abc480_a/test/sample-2.in",
                "abc480/abc480_a/test/compare.json"
            ]
        );
        assert_eq!(read(&dir.join("test/input.txt")), "9 9\n");
        assert_eq!(
            second.package.differed,
            vec!["abc480/abc480_a/test/sample-1.out"]
        );
        assert!(
            second
                .package
                .existed
                .contains(&"abc480/abc480_a/src/main.rs".to_string())
        );
        assert_eq!(read(&dir.join("src/main.rs")), "// 書き換えた\n");
        assert_eq!(read(&dir.join("test/sample-1.out")), "4\n");
        assert_eq!(
            read(&root.0.join("abc480/.cargo/config.toml")),
            "# 変えた\n"
        );
    }

    #[test]
    fn writes_compare_json_from_the_problem() {
        let mut problem = problem(&[]);
        let settings = |problem: &Problem| {
            compare::parse_settings(&compare_json(problem)).expect("IDE が読める compare.json")
        };
        // 時間は制限＋500 ms、メモリは問題の値
        for (time, memory, expected) in [
            ("2 sec", "1024 MiB", (2500, 1024)),
            ("2.5 sec", "256 MB", (3000, 256)),
            ("500 ms", "1 GiB", (1000, 1024)),
            ("100 sec", "999999 MiB", (60_000, 65_536)),
            (
                "不明",
                "",
                (
                    compare::DEFAULT_TIME_LIMIT_MS,
                    compare::DEFAULT_MEMORY_LIMIT_MIB,
                ),
            ),
        ] {
            problem.time_limit = time.to_string();
            problem.memory_limit = memory.to_string();
            let settings = settings(&problem);
            assert_eq!(
                (settings.time_limit_ms, settings.memory_limit_mib),
                expected,
                "{time} {memory}"
            );
            assert_eq!(settings.random_count, compare::DEFAULT_RANDOM_COUNT);
            assert_eq!(settings.mode, compare::Mode::Whitespace);
        }
        // 誤差（abc334_f の書き方。− は U+2212）
        let output = |text: &str| {
            format!(
                r#"<div class="part"><section><h3>入力</h3><p>誤差 <var>10^{{-9}}</var></p></section></div><div class="part"><section><h3>出力</h3><p>{text}</p></section></div>"#
            )
        };
        problem.statement_html = output(
            "出力は、真の値との絶対誤差または相対誤差が <var>10^{−6}</var> 以下のとき正解と判定される。",
        );
        assert_eq!(
            settings(&problem).mode,
            compare::Mode::Float {
                abs: Some(1e-6),
                rel: Some(1e-6)
            }
        );
        assert!(compare_json(&problem).contains("\"abs\": 1e-6"));
        problem.statement_html =
            output("絶対誤差が <var>10^{-4}</var> 以下ならば正解とみなされる。");
        assert_eq!(
            settings(&problem).mode,
            compare::Mode::Float {
                abs: Some(1e-4),
                rel: None
            }
        );
        problem.statement_html = output("相対誤差が 10^-9 以下");
        assert_eq!(
            settings(&problem).mode,
            compare::Mode::Float {
                abs: None,
                rel: Some(1e-9)
            }
        );
        // 答えが複数ありうる問題は、目視確認（none）。誤差より先に見る（D-168）
        for text in [
            "答えが複数ある場合、どれを出力しても正解とみなされる。",
            "そのような正整数が複数存在する場合には、いずれか <var>1</var> つを出力してください。",
            "条件を満たす <var>X</var> が複数ある場合、どれを出力しても正答となる。誤差 <var>10^{-6}</var>",
        ] {
            problem.statement_html = output(text);
            assert_eq!(settings(&problem).mode, compare::Mode::None, "{text}");
        }
        // 「2 種類のいずれかです」のような書き方は、答えが複数とはしない
        problem.statement_html = output("クエリは次の <var>2</var> 種類のいずれかです。");
        assert_eq!(settings(&problem).mode, compare::Mode::Whitespace);
        // 「出力」の区画に誤差がない（ほかの区画の誤差は見ない）・桁が読めない
        for text in [
            "答えを出力せよ。",
            "誤差は許されない。",
            "誤差が <var>10^{k}</var> 以下",
        ] {
            problem.statement_html = output(text);
            assert_eq!(settings(&problem).mode, compare::Mode::Whitespace, "{text}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn refuses_folders_that_point_outside() {
        let root = TempDir::new("outside");
        let outside = TempDir::new("outside-target");
        std::os::unix::fs::symlink(&outside.0, root.0.join("abc480")).unwrap();
        let id = ProblemId::new("abc480", "abc480_a").unwrap();
        let load = |_: &ProblemId| Ok(problem(&[]));
        let error = create_problem(&root.0, &id, &default_template(), &load).unwrap_err();
        assert!(error.contains("作業領域の外"), "{error}");
        assert_eq!(std::fs::read_dir(&outside.0).unwrap().count(), 0);
        // フォルダの名前のところにファイルがある
        std::fs::remove_file(root.0.join("abc480")).unwrap();
        std::fs::write(root.0.join("abc480"), "").unwrap();
        assert!(create_problem(&root.0, &id, &default_template(), &load).is_err());
        // 取れなければ、何も作らない
        std::fs::remove_file(root.0.join("abc480")).unwrap();
        let fail = |_: &ProblemId| Err("取れません".to_string());
        assert_eq!(
            create_problem(&root.0, &id, &default_template(), &fail),
            Err("取れません".to_string())
        );
        assert!(!root.0.join("abc480").exists());
    }

    fn tasks(names: &[&str]) -> Vec<TaskEntry> {
        names
            .iter()
            .map(|task| TaskEntry {
                task: task.to_string(),
                label: task.rsplit('_').next().unwrap().to_uppercase(),
                title: "例".to_string(),
            })
            .collect()
    }

    #[test]
    fn creates_every_task_and_skips_the_ones_it_cannot_read() {
        let root = TempDir::new("contest");
        let list = |_: &str| Ok(tasks(&["abc480_a", "abc480_b", "abc480_c"]));
        let load = |id: &ProblemId| {
            if id.task == "abc480_b" {
                Err("問題ページを取得できません：curl: (28) timed out".to_string())
            } else {
                Ok(problem(&[("1\n", "1\n")]))
            }
        };
        let events = Mutex::new(Vec::new());
        let created = create_contest(
            &root.0,
            "abc480",
            &default_template(),
            &list,
            &load,
            &AtomicBool::new(false),
            &|event| events.lock().unwrap().push(event),
        )
        .unwrap();
        assert_eq!(created.total, 3);
        assert!(!created.stopped);
        let errors: Vec<_> = created.packages.iter().map(|p| p.error.is_some()).collect();
        assert_eq!(errors, vec![false, true, false]);
        assert!(
            created.packages[1]
                .error
                .as_deref()
                .unwrap()
                .contains("timed out")
        );
        assert!(root.0.join("abc480/abc480_a/src/main.rs").is_file());
        assert!(!root.0.join("abc480/abc480_b").exists());
        assert!(root.0.join("abc480/abc480_c/test/sample-1.in").is_file());
        // 知らせ：一覧 → (始めた → 結果) × 3
        let kinds: Vec<&str> = events
            .lock()
            .unwrap()
            .iter()
            .map(|event| match event {
                ScaffoldEvent::Listed { .. } => "listed",
                ScaffoldEvent::Started { .. } => "started",
                ScaffoldEvent::Finished { .. } => "finished",
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "listed", "started", "finished", "started", "finished", "started", "finished"
            ]
        );
    }

    #[test]
    fn stops_between_tasks_and_reports_list_failures() {
        let root = TempDir::new("stop");
        let cancel = AtomicBool::new(false);
        let list = |_: &str| Ok(tasks(&["abc480_a", "abc480_b", "abc480_c"]));
        // 1 問目を取ったところで止める
        let load = |_: &ProblemId| {
            cancel.store(true, Ordering::SeqCst);
            Ok(problem(&[]))
        };
        let created = create_contest(
            &root.0,
            "abc480",
            &default_template(),
            &list,
            &load,
            &cancel,
            &|_| {},
        )
        .unwrap();
        assert!(created.stopped);
        assert_eq!(created.packages.len(), 1);
        assert!(root.0.join("abc480/abc480_a/Cargo.toml").is_file());
        assert!(!root.0.join("abc480/abc480_b").exists());
        // 一覧を読めなければ、何も作らずに理由を返す
        let missing = |_: &str| Err("ページが見つかりません（404）".to_string());
        let error = create_contest(
            &root.0,
            "abc999",
            &default_template(),
            &missing,
            &load,
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap_err();
        assert!(error.contains("404") && error.contains("開始前"), "{error}");
        assert!(!root.0.join("abc999").exists());
        // ID の形が違えば、読まない
        let never = |_: &str| -> Result<Vec<TaskEntry>, String> { panic!("読まない") };
        assert!(
            create_contest(
                &root.0,
                "../x",
                &default_template(),
                &never,
                &load,
                &AtomicBool::new(false),
                &|_| {}
            )
            .is_err()
        );
    }

    #[test]
    fn runs_one_scaffold_at_a_time() {
        let control = ScaffoldControl::default();
        let (guard, cancel) = control.begin().unwrap();
        assert!(control.begin().is_err());
        control.stop();
        assert!(cancel.load(Ordering::SeqCst));
        drop(guard);
        let (_guard, cancel) = control.begin().unwrap();
        assert!(!cancel.load(Ordering::SeqCst));
    }

    // 作ったパッケージを、本物の cargo で読む。上のフォルダのワークスペースとぶつからず、
    // ビルドの置き場所がコンテストのフォルダの target になり、IDE が実行の対象を決められる（AC7a-02・AC7a-09 の一部）
    #[test]
    #[cfg_attr(windows, ignore = "Windows：段階 2（cargo の出力のパスと、canonicalize が付ける接頭辞の違い）で直す。docs/windows-port-plan.md 7 章")]
    fn cargo_reads_created_packages_under_a_parent_workspace() {
        let root = TempDir::new("cargo");
        std::fs::write(
            root.0.join("Cargo.toml"),
            "[workspace]\nmembers = [\"tools\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.0.join("tools/src")).unwrap();
        std::fs::write(
            root.0.join("tools/Cargo.toml"),
            "[package]\nname = \"tools\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(root.0.join("tools/src/main.rs"), "fn main() {}\n").unwrap();
        // 依存なしの雛形（ネットワークにつながずに読めるように）
        let template = Template {
            language: Language::Rust,
            main_rs: DEFAULT_MAIN.to_string(),
            dependencies: String::new(),
            main_cpp: String::new(),
            compile_flags: String::new(),
            main_py: String::new(),
            fill: None,
        };
        let list = |_: &str| Ok(tasks(&["abc999_a", "abc999_b"]));
        let load = |_: &ProblemId| Ok(problem(&[("1\n", "1\n")]));
        create_contest(
            &root.0,
            "abc999",
            &template,
            &list,
            &load,
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap();
        let dir = root.0.join("abc999/abc999_a");
        let cargo = crate::toolchain::find_cargo().unwrap();
        let metadata: serde_json::Value =
            serde_json::from_str(&crate::toolchain::metadata_json(&cargo, &dir).unwrap()).unwrap();
        assert_eq!(metadata["workspace_root"].as_str(), dir.to_str());
        assert_eq!(
            metadata["target_directory"].as_str(),
            root.0.join("abc999/target").to_str()
        );
        let target = crate::toolchain::resolve_target(
            &root.0,
            &dir.join("src/main.rs").canonicalize().unwrap(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(target.package, "abc999/abc999_a");
        assert_eq!(target.bin, "abc999_a");
    }
}
