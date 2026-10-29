// AtCoder への提出と判定（Phase 6 作業 2。D-147・D-148。docs/phase6/02-pre-start.md §2.4〜§2.6・§2.8）
//
// - 提出は、コンテストの開催期間中だけ IDE から送る。期間の外（開始前を含む）は送らず、
//   「コードをコピーして公式の提出ページを開く」を画面に出してもらう
// - 送るかどうかは、Rust 側が出す OS のダイアログで決まる。画面（WebView）からは「提出したい」と頼むだけ。
//   ダイアログを省けるのは、settings.json の submit.confirmDialog が false で、実行履歴で今のソースのサンプルテストが
//   すべて AC と確かめたときだけ（Phase 9 作業 2。D-162）。submit.testBeforeSubmit が true なら、通っていなければ送らない
// - 提出の前の検査（別のファイルのモジュール・ジャッジにないクレート・版の違い・bin と問題の記号）は、注意として出し、止めない
// - 判定は、公式の提出の一覧と同じ状態の JSON を、同じ間隔（最初は 5 秒後、あとはサーバーの Interval、1 秒より短くしない）で読む
// - 送る処理と、判定を追う処理は分ける（原典 §9.2）。判定を追う間も、ほかの処理や次の提出はできる
//
// 通信は session.rs の curl（Cookie は Keychain から標準入力で渡す）。ソースの写し・セッションは記録に残さない
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dom_query::Document;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::cpp;
use crate::history;
use crate::language::Language;
use crate::python::{self, Interpreter, PythonSettings};
use crate::run::{Finish, SubmittedRecord};
use crate::session::{self, Client, Keychain, NOT_LOGGED_IN, SessionState, SessionView};
use crate::storage::SubmitSettings;
use crate::toolchain::Target;

pub const BASE: &str = "https://atcoder.jp";
// 判定が変わったことを main の窓に知らせるイベント
pub const SUBMISSION_EVENT: &str = "atcoder-submission";
// 提出できるコードの大きさの上限（AtCoder の提出のページの「at most 512 KiB」）
const SOURCE_LIMIT: usize = 512 * 1024;
// 自分の提出の一覧から提出の ID を決めるとき、送った時刻よりこれだけ前の提出まで認める（手元の時計のずれ）
const CLOCK_SLACK_SECS: i64 = 120;

// ---- ジャッジのクレート（提出の前の検査） ----

// AtCoder の 2025/10 版の言語の Rust（`088-1-82-0_rustc.toml` の Cargo.toml、68 個。すべて `=` の版）。
// 出典：https://img.atcoder.jp/file/language-update/2025-10/088-1-82-0_rustc.toml （2026-10-14 に確認。
// docs/phase0/03a-atcoder-environment.md §3）。AtCoder の言語の更新のときに直す
pub const JUDGE_CRATES_NOTE: &str =
    "ジャッジのクレートは、AtCoder の 2025/10 版の言語の一覧（2026-10-14 に確認）のものです";
const JUDGE_CRATES: &[(&str, &str)] = &[
    // 202411 から
    ("rpds", "1.1.1"),
    ("rand_xorshift", "0.4.0"),
    ("rand_xoshiro", "0.7.0"),
    ("statrs", "0.18.0"),
    ("primal", "0.3.3"),
    ("thiserror", "2.0.16"),
    // 202301 から
    ("ac-library-rs", "0.2.0"),
    ("once_cell", "1.21.3"),
    ("static_assertions", "1.1.0"),
    ("varisat", "0.2.2"),
    ("memoise", "0.3.2"),
    ("argio", "0.2.0"),
    ("bitvec", "1.0.1"),
    ("counter", "0.7.0"),
    ("hashbag", "0.1.12"),
    ("pathfinding", "4.14.0"),
    ("recur-fn", "2.2.0"),
    ("indexing", "0.4.1"),
    ("amplify", "4.9.0"),
    ("amplify_derive", "4.0.1"),
    ("amplify_num", "0.5.3"),
    ("easy-ext", "1.0.2"),
    ("multimap", "0.10.1"),
    ("btreemultimap", "0.1.1"),
    ("bstr", "1.12.0"),
    ("az", "1.2.1"),
    ("glidesort", "0.1.2"),
    ("tap", "1.0.1"),
    ("omniswap", "0.1.0"),
    ("multiversion", "0.8.0"),
    // 202004 から続投
    ("num", "0.4.3"),
    ("num-bigint", "0.4.6"),
    ("num-complex", "0.4.6"),
    ("num-integer", "0.1.46"),
    ("num-iter", "0.1.45"),
    ("num-rational", "0.4.2"),
    ("num-traits", "0.2.19"),
    ("num-derive", "0.4.2"),
    ("ndarray", "0.16.1"),
    ("nalgebra", "0.34.0"),
    ("alga", "0.9.3"),
    ("libm", "0.2.15"),
    ("rand", "0.9.2"),
    ("getrandom", "0.3.3"),
    ("rand_chacha", "0.9.0"),
    ("rand_core", "0.9.3"),
    ("rand_hc", "0.4.0"),
    ("rand_pcg", "0.9.0"),
    ("rand_distr", "0.5.1"),
    ("petgraph", "0.8.2"),
    ("indexmap", "2.11.0"),
    ("regex", "1.11.2"),
    ("lazy_static", "1.5.0"),
    ("ordered-float", "5.0.0"),
    ("ascii", "1.1.0"),
    ("permutohedron", "0.2.4"),
    ("superslice", "1.0.0"),
    ("itertools", "0.14.0"),
    ("itertools-num", "0.1.3"),
    ("maplit", "1.0.2"),
    ("either", "1.15.0"),
    ("im-rc", "15.1.0"),
    ("fixedbitset", "0.5.7"),
    ("bitset-fixed", "0.1.0"),
    ("proconio", "0.5.0"),
    ("text_io", "0.1.13"),
    ("rustc-hash", "2.1.1"),
    ("smallvec", "1.15.1"),
];

// crates.io では `-` と `_` を同じ名前として扱う
fn same_crate(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x == y || (matches!(x, b'-' | b'_') && matches!(y, b'-' | b'_')))
}

fn judge_version(name: &str) -> Option<&'static str> {
    JUDGE_CRATES
        .iter()
        .find(|(judge, _)| same_crate(judge, name))
        .map(|(_, version)| *version)
}

// `cargo metadata --no-deps` の出力のうち、使うところだけ
#[derive(Deserialize)]
struct Metadata {
    packages: Vec<MetaPackage>,
    workspace_root: PathBuf,
}

#[derive(Deserialize)]
struct MetaPackage {
    manifest_path: PathBuf,
    targets: Vec<MetaTarget>,
    dependencies: Vec<MetaDependency>,
}

#[derive(Deserialize)]
struct MetaTarget {
    name: String,
    kind: Vec<String>,
    src_path: PathBuf,
}

#[derive(Deserialize)]
struct MetaDependency {
    name: String,
    // crates.io なら `registry+…`、git なら `git+…`、パスなら null
    source: Option<String>,
    // 通常の依存は null、`dev`・`build`
    kind: Option<String>,
}

// Cargo.lock の、クレートの名前ごとの版
fn locked_versions(lock: &str) -> HashMap<String, Vec<String>> {
    let unquote = |text: &str| text.trim().trim_matches('"').to_string();
    let mut versions: HashMap<String, Vec<String>> = HashMap::new();
    let mut name = None;
    for line in lock.lines().map(str::trim) {
        if line == "[[package]]" {
            name = None;
        } else if let Some(value) = line.strip_prefix("name = ") {
            name = Some(unquote(value));
        } else if let Some(value) = line.strip_prefix("version = ")
            && let Some(name) = name.take()
        {
            versions.entry(name).or_default().push(unquote(value));
        }
    }
    versions
}

// 通常の依存（dev・build を除く）についての注意
fn crate_warnings(dependencies: &[MetaDependency], lock: Option<&str>) -> Vec<String> {
    let locked = lock.map(locked_versions);
    let mut warnings = Vec::new();
    let mut missing_lock = false;
    for dependency in dependencies.iter().filter(|d| d.kind.is_none()) {
        let name = &dependency.name;
        let from_registry = dependency
            .source
            .as_deref()
            .is_some_and(|source| source.starts_with("registry+"));
        if !from_registry {
            warnings.push(format!(
                "クレート「{name}」は crates.io ではなく、パスか git から読んでいます。ジャッジでは使えません"
            ));
            continue;
        }
        let Some(judge) = judge_version(name) else {
            warnings.push(format!("クレート「{name}」は、ジャッジにありません"));
            continue;
        };
        let Some(locked) = &locked else {
            missing_lock = true;
            continue;
        };
        let versions = locked
            .iter()
            .filter(|(locked_name, _)| same_crate(locked_name, name))
            .flat_map(|(_, versions)| versions.iter().cloned())
            .collect::<Vec<_>>();
        if !versions.is_empty() && !versions.iter().any(|version| version == judge) {
            warnings.push(format!(
                "クレート「{name}」の版が、ジャッジと違います（手元は {}、ジャッジは {judge}）",
                versions.join("・")
            ));
        }
    }
    if missing_lock {
        warnings.push(
            "Cargo.lock がないので、クレートの版をジャッジと比べられませんでした".to_string(),
        );
    }
    warnings
}

// `mod x;` のように、別のファイルを読むモジュール（ジャッジには 1 ファイルしか置かれない）。
// 行ごとに見るだけで、ブロックのコメントの中は区別しない（注意を出すだけなので）
fn module_files(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            let mut rest = line.split("//").next()?.trim();
            while rest.starts_with("#[") {
                rest = rest[rest.find(']')? + 1..].trim_start();
            }
            for visibility in ["pub(crate)", "pub(super)", "pub(self)", "pub"] {
                if let Some(after) = rest.strip_prefix(visibility)
                    && after.starts_with(char::is_whitespace)
                {
                    rest = after.trim_start();
                    break;
                }
            }
            let name = rest.strip_prefix("mod ")?.trim().strip_suffix(';')?.trim();
            (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
                .then(|| name.to_string())
        })
        .collect()
}

// 1 つのパッケージに問題ごとの bin を置く形で、bin の名前の末尾（例：`a`、`abc480-a` の `a`）と問題の記号（例：`_b`）が違う
fn bin_task_warning(bins: &[String], bin: &str, task: &str) -> Option<String> {
    if bins.len() < 2 {
        return None;
    }
    let (_, letter) = task.rsplit_once('_')?;
    let tail = bin.rsplit(['-', '_']).next().unwrap_or(bin);
    if tail.is_empty() || tail.len() > 2 || !tail.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    (!tail.eq_ignore_ascii_case(letter)).then(|| {
        format!("bin「{bin}」と、選んでいる問題「{task}」の記号（{letter}）が違います。別の問題のファイルかもしれません")
    })
}

// パッケージのフォルダから決まる問題（D-155）と、送る問題が違う（画面で紐付けを外して選んだとき。D-178）。
// 画面からの値に頼らず、呼び出し元がフォルダ名から決め直した問題を渡す
pub fn folder_problem_warning(
    folder: Option<&crate::problem::ProblemId>,
    chosen: &crate::problem::ProblemId,
) -> Option<String> {
    let folder = folder?;
    (folder != chosen).then(|| {
        format!(
            "このパッケージのフォルダは問題「{}」（{}）のものです。選んでいる問題「{}」（{}）に提出します。提出の前のサンプルテストは、このパッケージのテストのケースで動きます",
            folder.task, folder.contest, chosen.task, chosen.contest
        )
    })
}

// 提出するコードと、提出の前の検査の結果
#[derive(Debug, Clone)]
pub struct Prepared {
    // 提出の言語（D-204）
    pub language: Language,
    pub source: String,
    // 作業領域のルートからのパス
    pub file: String,
    pub warnings: Vec<String>,
    // Python の処理系（提出の言語の選択肢を決める。Phase 19 作業 4。D-224）。ほかの言語は None
    pub python: Option<Interpreter>,
}

// bin のソースを読み、提出の前の検査をする。metadata は、パッケージのディレクトリ（dir）での `cargo metadata --no-deps` の出力
pub fn prepare(
    root: &Path,
    dir: &Path,
    metadata: &str,
    bin: &str,
    task: &str,
) -> Result<Prepared, String> {
    let metadata: Metadata = serde_json::from_str(metadata)
        .map_err(|_| "cargo metadata の出力を読めません".to_string())?;
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
    let bins: Vec<String> = package
        .targets
        .iter()
        .filter(|target| target.kind.iter().any(|kind| kind == "bin"))
        .map(|target| target.name.clone())
        .collect();
    let target = package
        .targets
        .iter()
        .find(|target| target.name == bin && target.kind.iter().any(|kind| kind == "bin"))
        .ok_or_else(|| format!("バイナリ「{bin}」が見つかりません"))?;
    // ソースが、リンクなどで作業領域の外を指していないことを確かめる
    let path = target
        .src_path
        .canonicalize()
        .map_err(|_| format!("バイナリ「{bin}」のソースが見つかりません"))?;
    let file = path
        .strip_prefix(root)
        .map_err(|_| "提出するファイルが作業領域の外にあります".to_string())?
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let source = read_source(&path, &file)?;
    let mut warnings = Vec::new();
    let modules = module_files(&source);
    if !modules.is_empty() {
        warnings.push(format!(
            "別のファイルを読むモジュール（{}）があります。ジャッジには、このファイルだけが置かれます",
            modules
                .iter()
                .map(|name| format!("mod {name};"))
                .collect::<Vec<_>>()
                .join("、")
        ));
    }
    let lock = std::fs::read_to_string(metadata.workspace_root.join("Cargo.lock")).ok();
    warnings.extend(crate_warnings(&package.dependencies, lock.as_deref()));
    warnings.extend(bin_task_warning(&bins, bin, task));
    Ok(Prepared {
        language: Language::Rust,
        source,
        file,
        warnings,
        python: None,
    })
}

// C++ の main.cpp を読み、提出の前の検査をする（Phase 16 作業 4。D-209）。dir はパッケージのディレクトリ。
// gcc_version は手元の g++ の版（`-dumpfullversion`）。分からなければ None（版の注意は出さない）
pub fn prepare_cpp(root: &Path, dir: &Path, gcc_version: Option<&str>) -> Result<Prepared, String> {
    // ソースが、リンクなどで作業領域の外を指していないことを確かめる
    let path = dir
        .join(cpp::SOURCE)
        .canonicalize()
        .map_err(|_| format!("{} が見つかりません", cpp::SOURCE))?;
    let file = path
        .strip_prefix(root)
        .map_err(|_| "提出するファイルが作業領域の外にあります".to_string())?
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let source = read_source(&path, &file)?;
    let mut warnings = Vec::new();
    let headers = local_headers(&source);
    if !headers.is_empty() {
        warnings.push(format!(
            "自分のヘッダーを読む #include（{}）があります。ジャッジには、このファイルだけが置かれます",
            headers
                .iter()
                .map(|name| format!("#include \"{name}\""))
                .collect::<Vec<_>>()
                .join("、")
        ));
    }
    if let Some(version) = gcc_version
        && version != cpp::JUDGE_VERSION
    {
        warnings.push(format!(
            "手元の GCC（{version}）は、ジャッジ（{}）と版が違います",
            cpp::JUDGE_VERSION
        ));
    }
    Ok(Prepared {
        language: Language::Cpp,
        source,
        file,
        warnings,
        python: None,
    })
}

// Python の main.py を読み、提出の前の検査をする（Phase 19 作業 4。D-224）。dir はパッケージのディレクトリ。
// 処理系が見つからなくても送れる（import と版の注意は出せないので、その旨を注意に出す）
pub fn prepare_python(
    root: &Path,
    dir: &Path,
    settings: &PythonSettings,
) -> Result<Prepared, String> {
    // ソースが、リンクなどで作業領域の外を指していないことを確かめる
    let path = dir
        .join(python::SOURCE)
        .canonicalize()
        .map_err(|_| format!("{} が見つかりません", python::SOURCE))?;
    let file = path
        .strip_prefix(root)
        .map_err(|_| "提出するファイルが作業領域の外にあります".to_string())?
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let source = read_source(&path, &file)?;
    let interpreter = settings.interpreter;
    let mut warnings = Vec::new();
    match python::find_interpreter(settings) {
        Ok(found) => {
            match python::imports(&found, &path) {
                Ok(imports) => warnings.extend(python::submit_warnings(interpreter, &imports, dir)),
                Err(reason) => warnings.push(reason),
            }
            match python::version(&found) {
                Ok(version) => warnings.extend(python::version_warning(interpreter, &version)),
                Err(reason) => warnings.push(reason),
            }
        }
        Err(_) => warnings.push(format!(
            "手元に {} が見つからないので、import と版を確かめていません",
            interpreter.label()
        )),
    }
    Ok(Prepared {
        language: Language::Python,
        source,
        file,
        warnings,
        python: Some(interpreter),
    })
}

// 提出するファイルを読む（512 KiB まで、UTF-8）。file は知らせに使う名前
fn read_source(path: &Path, file: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|_| format!("「{file}」を読めません"))?;
    if bytes.len() > SOURCE_LIMIT {
        return Err(format!(
            "「{file}」は {} KiB あり、AtCoder の上限（512 KiB）を超えています",
            bytes.len().div_ceil(1024)
        ));
    }
    String::from_utf8(bytes).map_err(|_| format!("「{file}」を読めません（UTF-8 ではない）"))
}

// `#include "x.hpp"` のように、自分のヘッダーを読む行（ジャッジには 1 ファイルしか置かれない）。
// 行ごとに見るだけで、ブロックのコメントの中は区別しない（注意を出すだけなので。module_files と同じ）
fn local_headers(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix('#')?.trim_start();
            let rest = rest.strip_prefix("include")?.trim_start();
            let name = rest.strip_prefix('"')?;
            Some(name[..name.find('"')?].to_string())
        })
        .collect()
}

// ---- ページの読み取り ----

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// 「2026-10-26T20:00:00+09:00」「2026-10-17 21:00:30+0900」を UNIX 時間（秒）にする
fn parse_time(text: &str) -> Option<i64> {
    let text = text.trim();
    if !text.is_ascii() || text.len() < 22 || !matches!(text.as_bytes()[10], b'T' | b' ') {
        return None;
    }
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let offset = &text[19..];
    let sign = match offset.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits = offset[1..].replace(':', "");
    if digits.len() != 4 {
        return None;
    }
    let offset_secs =
        sign * (digits[..2].parse::<i64>().ok()? * 3600 + digits[2..].parse::<i64>().ok()? * 60);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    // 1970-01-01 からの日数（グレゴリオ暦）
    let (y, m) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * m + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset_secs)
}

// 画面に出す形（例：2026-10-26 20:00:00+09:00）
fn display_time(text: &str) -> String {
    text.replacen('T', " ", 1)
}

// コンテストの開催期間（コンテストのページの `var startTime = moment("…")`・`var endTime = moment("…")`）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Period {
    pub start: i64,
    pub end: i64,
    pub start_text: String,
    pub end_text: String,
}

impl Period {
    pub fn contains(&self, now: i64) -> bool {
        self.start <= now && now < self.end
    }
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = text.find(start)? + start.len();
    let to = text[from..].find(end)? + from;
    Some(&text[from..to])
}

pub fn parse_period(html: &str) -> Option<Period> {
    let start_text = between(html, "var startTime = moment(\"", "\")")?;
    let end_text = between(html, "var endTime = moment(\"", "\")")?;
    let (start, end) = (parse_time(start_text)?, parse_time(end_text)?);
    (start < end).then(|| Period {
        start,
        end,
        start_text: display_time(start_text),
        end_text: display_time(end_text),
    })
}

// 提出のフォーム（`/contests/{contest}/submit?taskScreenName={task}`）から読むもの
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub csrf: Option<String>,
    // 問題の選択肢（値、表示。例：`abc480_a`、`A - ADjacent`）
    pub tasks: Vec<(String, String)>,
    // その問題の言語の選択肢（値、表示。同じ値は 1 つだけ）
    pub languages: Vec<(String, String)>,
    // Cloudflare の確認（Turnstile）の部品がある
    pub turnstile: bool,
    pub user: Option<String>,
}

// フォームがなければ None（参加の登録が要るコンテストで、登録していないときなど【未確認】）
pub fn parse_form(html: &str, contest: &str, task: &str) -> Option<Form> {
    let document = Document::from(html);
    let action = format!("/contests/{contest}/submit");
    let form = document
        .select("form.form-code-submit")
        .iter()
        .find(|form| form.attr("action").as_deref() == Some(action.as_str()))?;
    let options = |selection: dom_query::Selection| -> Vec<(String, String)> {
        selection
            .iter()
            .filter_map(|option| {
                let value = option.attr("value")?.to_string();
                (!value.is_empty()).then(|| (value, squash(&option.text())))
            })
            .collect()
    };
    let tasks = options(form.select("select[name=\"data.TaskScreenName\"] option"));
    // 言語の選択肢は、問題ごとの `#select-lang-{task}` の中にある
    let mut languages = options(form.select(&format!("#select-lang-{task} select option")));
    if languages.is_empty() {
        languages = options(form.select("#select-lang select option"));
    }
    let mut unique: Vec<(String, String)> = Vec::new();
    for (value, label) in languages {
        if !unique.iter().any(|(seen, _)| *seen == value) {
            unique.push((value, label));
        }
    }
    let csrf = form
        .select("input[name=\"csrf_token\"]")
        .attr("value")
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty());
    let turnstile = html.contains("challenges.cloudflare.com/turnstile")
        || document.select(".cf-challenge, .cf-turnstile").exists();
    Some(Form {
        csrf,
        tasks,
        languages: unique,
        turnstile,
        user: session::user_screen_name(html),
    })
}

// 送るのに使うもの
#[derive(Debug, Clone, PartialEq)]
pub struct Ready {
    pub task_title: String,
    pub language_id: String,
    pub language: String,
    pub csrf: String,
}

// フォームで送れるかを確かめる。言語の選択肢は、language に使うものの最初のもの（D-148・D-209）。送れなければ理由
pub fn check_form(
    form: Option<&Form>,
    task: &str,
    language: Language,
    python: Option<Interpreter>,
) -> Result<Ready, String> {
    let form = form.ok_or_else(|| {
        "提出のフォームが見つかりません（参加の登録が要るコンテストかもしれません）".to_string()
    })?;
    if form.turnstile {
        return Err(
            "提出のページに Cloudflare の確認（Turnstile）があるので、IDE からは送りません"
                .to_string(),
        );
    }
    let csrf = form
        .csrf
        .clone()
        .ok_or_else(|| "提出のフォームを読めません（csrf_token がありません）".to_string())?;
    let (_, task_title) = form
        .tasks
        .iter()
        .find(|(value, _)| value == task)
        .ok_or_else(|| format!("提出のフォームの問題の選択肢に「{task}」がありません"))?;
    let (language_id, language) = form
        .languages
        .iter()
        .find(|(_, label)| language.is_submit_option(label, python))
        .ok_or_else(|| {
            format!(
                "提出のフォームの言語の選択肢に {} がありません",
                python.map_or(language.label(), |interpreter| interpreter.submit_option())
            )
        })?;
    Ok(Ready {
        task_title: task_title.clone(),
        language_id: language_id.clone(),
        language: language.clone(),
        csrf,
    })
}

// 自分の提出の一覧から、その問題の提出のうち、since（UNIX 時間の秒）より後のいちばん新しいものの ID
pub fn newest_submission(html: &str, contest: &str, task: &str, since: i64) -> Option<u64> {
    let document = Document::from(html);
    let link = format!("a[href=\"/contests/{contest}/tasks/{task}\"]");
    document
        .select("table tbody tr")
        .iter()
        .filter_map(|row| {
            if !row.select(&link).exists() {
                return None;
            }
            let time = parse_time(&row.select("time").text())?;
            if time < since {
                return None;
            }
            row.select(".submission-score")
                .attr("data-id")?
                .parse()
                .ok()
        })
        .max()
}

// 提出の判定（状態の JSON の Html と Score から）
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Judge {
    // 判定中（`waiting-judge`）
    pub waiting: bool,
    // ページの表記（例：AC、WA、WJ。判定中は進み具合が入ることがある【未確認】）
    pub label: String,
    // 表記の英語名（例：Accepted）
    pub title: Option<String>,
    pub score: Option<String>,
    // AtCoder のジャッジの値（手元の実行の値とは違う）
    pub time_ms: Option<u64>,
    pub memory_kib: Option<u64>,
}

// 状態の JSON（`{"Result":{"<id>":{"Html":"<td…>","Score":"…"}},"Interval":…}`）を読む。
// 戻り値の 2 つ目は、次に読むまでの間隔（ミリ秒。判定中だけ入る）。形が違えば読めないと返す（推測で埋めない）
pub fn parse_status(body: &str, id: u64) -> Result<(Judge, Option<u64>), String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| "判定の状態を読めません（JSON ではありません）".to_string())?;
    let entry = value
        .get("Result")
        .and_then(|result| result.get(id.to_string()))
        .ok_or_else(|| "判定の状態に、この提出がありません".to_string())?;
    let html = entry
        .get("Html")
        .and_then(Value::as_str)
        .ok_or_else(|| "判定の状態を読めません（Html がありません）".to_string())?;
    let score = match entry.get("Score") {
        Some(Value::String(score)) => Some(score.clone()),
        Some(Value::Number(score)) => Some(score.to_string()),
        _ => None,
    };
    let interval = value.get("Interval").and_then(Value::as_u64);
    let document =
        Document::from(format!("<table><tbody><tr>{html}</tr></tbody></table>").as_str());
    let cells: Vec<_> = document.select("td").iter().collect();
    let first = cells
        .first()
        .ok_or_else(|| "判定の状態を読めません（判定の欄がありません）".to_string())?;
    let span = first.select("span").first();
    let label = squash(&if span.exists() {
        span.text()
    } else {
        first.text()
    });
    if label.is_empty() || label.chars().count() > 40 {
        return Err("判定の状態を読めません（判定の表記がありません）".to_string());
    }
    let title = span
        .attr("title")
        .map(|title| squash(&title))
        .filter(|title| !title.is_empty());
    let number_with = |index: usize, unit: &str| {
        cells
            .get(index)
            .and_then(|cell| squash(&cell.text()).strip_suffix(unit)?.trim().parse().ok())
    };
    Ok((
        Judge {
            waiting: first.has_class("waiting-judge"),
            label,
            title,
            score,
            time_ms: number_with(1, " ms"),
            memory_kib: number_with(2, " KiB"),
        },
        interval,
    ))
}

// 受け付けられなかったときのページの知らせ（`.alert-danger` など）
fn alert_text(html: &str) -> Option<String> {
    let document = Document::from(html);
    let text = squash(&document.select(".alert-danger, .alert-warning").text());
    let text = text.trim_start_matches('×').trim().to_string();
    (!text.is_empty()).then(|| text.chars().take(200).collect())
}

fn page_title(html: &str) -> Option<String> {
    let document = Document::from(html);
    let title = squash(&document.select("title").text());
    (!title.is_empty()).then_some(title)
}

// 移動先のパス（`https://atcoder.jp/…` も `/…` も受け付ける。クエリは除く）
fn location_path(location: &str) -> &str {
    let path = location.strip_prefix(BASE).unwrap_or(location);
    path.split('?').next().unwrap_or(path)
}

// ---- 送る ----

// AtCoder とのやり取り。テストでは、手元の http の口と短い間隔を使う
pub struct Remote {
    client: Client,
    base: String,
    // 続けて読むときの間隔（D-111 と同じく 1 秒）
    spacing: Duration,
    // 送ってから、最初に判定を読むまで（公式のページと同じ 5 秒）
    first_wait: Duration,
    // 判定を読み直す間隔の下限（1 秒）
    min_interval: Duration,
    // 判定を追う時間の上限（10 分）
    cap: Duration,
    last: Mutex<Option<Instant>>,
}

impl Remote {
    pub fn app() -> Self {
        Remote {
            client: Client::app(),
            base: BASE.to_string(),
            spacing: Duration::from_secs(1),
            first_wait: Duration::from_secs(5),
            min_interval: Duration::from_secs(1),
            cap: Duration::from_secs(600),
            last: Mutex::new(None),
        }
    }

    fn pace(&self) {
        let mut last = self
            .last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(at) = *last {
            let since = at.elapsed();
            if since < self.spacing {
                std::thread::sleep(self.spacing - since);
            }
        }
        *last = Some(Instant::now());
    }

    fn get(&self, path: &str) -> Result<session::Reply, String> {
        self.pace();
        self.client.request(&format!("{}{path}", self.base), &[])
    }

    fn post(&self, path: &str, form: &[(&str, &str)]) -> Result<session::Reply, String> {
        self.pace();
        self.client.request(&format!("{}{path}", self.base), form)
    }

    fn public(&self, path: &str) -> Result<session::Reply, String> {
        self.pace();
        self.client.public(&format!("{}{path}", self.base))
    }

    fn logged_in(&self) -> Result<bool, String> {
        self.client.keychain().exists()
    }
}

pub fn submit_url(contest: &str, task: &str) -> String {
    format!("{BASE}/contests/{contest}/submit?taskScreenName={task}")
}

fn detail_url(contest: &str, id: u64) -> String {
    format!("{BASE}/contests/{contest}/submissions/{id}")
}

fn list_url(contest: &str) -> String {
    format!("{BASE}/contests/{contest}/submissions/me")
}

// 確認のダイアログに出すもの
#[derive(Debug, Clone)]
pub struct Confirmation {
    pub contest: String,
    pub task: String,
    pub task_title: String,
    pub end_text: String,
    pub file: String,
    pub size: usize,
    pub language: String,
    pub user: Option<String>,
    pub last_test: String,
    pub warnings: Vec<String>,
    // 注意に添える、ジャッジの環境の出どころ（言語ごと。D-209）
    pub judge_note: &'static str,
}

impl Confirmation {
    pub fn message(&self) -> String {
        let size = if self.size < 1024 {
            format!("{} バイト", self.size)
        } else {
            format!("{:.1} KiB", self.size as f64 / 1024.0)
        };
        let mut text = format!(
            "次の内容で AtCoder に提出します。提出は取り消せません。\n\n問題：{}「{}」\nコンテスト：{}（終了 {}）\nファイル：{}（{size}）\n言語：{}\nユーザー：{}\n最後のサンプルテスト：{}",
            self.task,
            self.task_title,
            self.contest,
            self.end_text,
            self.file,
            self.language,
            self.user.as_deref().unwrap_or("（分かりません）"),
            self.last_test,
        );
        if !self.warnings.is_empty() {
            text.push_str("\n\n注意：");
            for warning in &self.warnings {
                text.push_str(&format!("\n・{warning}"));
            }
            text.push_str(&format!("\n（{}）", self.judge_note));
        }
        text
    }
}

// 提出の状態（画面の「提出結果」の 1 行）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SubmissionState {
    // 送ったが、受け付けられなかった
    Rejected,
    // 受け付けられたが、提出の ID を決められなかった
    UnknownId,
    // 判定待ち（WJ）
    Waiting,
    // 判定中（進み具合が取れれば label に入る）
    Judging,
    // 判定が決まった
    Done,
    // 読めなかった（ネットワーク・形の違いなど）。「更新」で読み直せる
    Unreadable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionView {
    // このアプリの中での番号（「更新」に使う）
    pub key: u64,
    pub contest: String,
    pub task: String,
    pub task_title: String,
    // 送った時刻（UNIX 時間のミリ秒）
    pub time_ms: u64,
    pub language: String,
    pub file: String,
    pub submission_id: Option<u64>,
    pub state: SubmissionState,
    // 判定を読み続けている
    pub tracking: bool,
    pub judge: Option<Judge>,
    pub message: Option<String>,
    pub detail_url: Option<String>,
    pub list_url: String,
}

// 提出の結果（画面に返す）
#[derive(Debug, Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum SubmitOutcome {
    // 開催期間の外（開始前を含む）。送らない。「コードをコピーして公式の提出ページを開く」を出す
    OutOfPeriod {
        start: String,
        end: String,
        file: String,
        warnings: Vec<String>,
        submit_url: String,
    },
    // ログインしていない（切れた）。送らない
    NotLoggedIn {
        message: String,
        file: String,
        warnings: Vec<String>,
        submit_url: String,
    },
    // 送らなかった（開催期間・フォームを読めない、Turnstile があるなど）
    Refused {
        reason: String,
        file: String,
        warnings: Vec<String>,
        submit_url: String,
    },
    // ダイアログで「キャンセル」
    Cancelled,
    // 送った（受け付けられなかったときも含む。submission.state を見る）。dialog_skipped は、確認のダイアログを
    // 省いたときの、サンプルテストのケースの数（D-162）
    Sent {
        submission: Box<SubmissionView>,
        #[serde(skip_serializing_if = "Option::is_none")]
        dialog_skipped: Option<usize>,
    },
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

// 開催期間を確かめ、フォームを読み、確認のダイアログ（confirm）で「提出」なら送って、提出の ID を決める。
// on_user は、フォームから読んだユーザー名を知らせる。判定は追わない（track で追う）
#[allow(clippy::too_many_arguments)]
pub fn submit_flow(
    remote: &Remote,
    prepared: &Prepared,
    contest: &str,
    task: &str,
    key: u64,
    last_test: String,
    confirm: impl FnOnce(&Confirmation) -> bool,
    on_user: impl Fn(Option<String>),
) -> SubmitOutcome {
    let refused = |reason: String| {
        if reason == NOT_LOGGED_IN {
            SubmitOutcome::NotLoggedIn {
                message: reason,
                file: prepared.file.clone(),
                warnings: prepared.warnings.clone(),
                submit_url: submit_url(contest, task),
            }
        } else {
            SubmitOutcome::Refused {
                reason,
                file: prepared.file.clone(),
                warnings: prepared.warnings.clone(),
                submit_url: submit_url(contest, task),
            }
        }
    };
    // 1. 開催期間（ログインなしで読めるコンテストのページ。延長などに追いつくため、提出のたびに読む）
    let top = match remote.public(&format!("/contests/{contest}")) {
        Ok(reply) if reply.status == 200 => reply,
        Ok(reply) => {
            return refused(format!(
                "コンテストのページを読めません（HTTP {}）",
                reply.status
            ));
        }
        Err(error) => return refused(format!("コンテストのページを読めません：{error}")),
    };
    let Some(period) = parse_period(&top.body) else {
        return refused("コンテストのページから開催期間を読めません".to_string());
    };
    if !period.contains(now_secs()) {
        return SubmitOutcome::OutOfPeriod {
            start: period.start_text,
            end: period.end_text,
            file: prepared.file.clone(),
            warnings: prepared.warnings.clone(),
            submit_url: submit_url(contest, task),
        };
    }
    // 2. ログイン
    match remote.logged_in() {
        Ok(true) => {}
        Ok(false) => return refused(NOT_LOGGED_IN.to_string()),
        Err(error) => return refused(error),
    }
    // 3. 提出のフォーム
    let page = match remote.get(&format!("/contests/{contest}/submit?taskScreenName={task}")) {
        Ok(reply) if reply.status == 200 => reply,
        Ok(reply) => {
            return refused(format!("提出のページを読めません（HTTP {}）", reply.status));
        }
        Err(error) => return refused(error),
    };
    let form = parse_form(&page.body, contest, task);
    if let Some(form) = &form {
        on_user(form.user.clone());
    }
    let ready = match check_form(form.as_ref(), task, prepared.language, prepared.python) {
        Ok(ready) => ready,
        Err(reason) => return refused(reason),
    };
    // 4. 確認のダイアログ
    let confirmation = Confirmation {
        contest: contest.to_string(),
        task: task.to_string(),
        task_title: ready.task_title.clone(),
        end_text: period.end_text.clone(),
        file: prepared.file.clone(),
        size: prepared.source.len(),
        language: ready.language.clone(),
        user: form.as_ref().and_then(|form| form.user.clone()),
        last_test,
        warnings: prepared.warnings.clone(),
        judge_note: match prepared.language {
            Language::Rust => JUDGE_CRATES_NOTE,
            Language::Cpp => cpp::JUDGE_SUBMIT_NOTE,
            Language::Python => prepared
                .python
                .map_or("", |interpreter| interpreter.submit_note()),
        },
    };
    if !confirm(&confirmation) {
        return SubmitOutcome::Cancelled;
    }
    // ダイアログを待つ間に終わっていれば送らない
    if !period.contains(now_secs()) {
        return refused(format!(
            "開催期間が終わったので送りません（終了 {}）",
            period.end_text
        ));
    }
    // 5. 送る
    let sent_ms = now_ms();
    let mut view = SubmissionView {
        key,
        contest: contest.to_string(),
        task: task.to_string(),
        task_title: ready.task_title.clone(),
        time_ms: sent_ms,
        language: ready.language.clone(),
        file: prepared.file.clone(),
        submission_id: None,
        state: SubmissionState::Rejected,
        tracking: false,
        judge: None,
        message: None,
        detail_url: None,
        list_url: list_url(contest),
    };
    let posted = remote.post(
        &format!("/contests/{contest}/submit"),
        &[
            ("data.TaskScreenName", task),
            ("data.LanguageId", &ready.language_id),
            ("sourceCode", &prepared.source),
            ("csrf_token", &ready.csrf),
        ],
    );
    let posted = match posted {
        Ok(reply) => reply,
        Err(error) if error == NOT_LOGGED_IN => return refused(error),
        Err(error) => {
            // 届いたかどうか分からない。自動では送り直さない
            view.state = SubmissionState::Unreadable;
            view.message = Some(format!(
                "送った結果が分かりません（{error}）。公式の「自分の提出」で確かめてください"
            ));
            return SubmitOutcome::Sent {
                submission: Box::new(view),
                dialog_skipped: None,
            };
        }
    };
    let me = format!("/contests/{contest}/submissions/me");
    if posted.location.as_deref().map(location_path) != Some(me.as_str()) {
        // 受け付けられなかった。知らせのあるページ（移動先か、返ってきたページ）から理由を読む
        let reason = match posted.location.as_deref() {
            Some(location)
                if location_path(location).starts_with(&format!("/contests/{contest}/")) =>
            {
                remote
                    .get(location.strip_prefix(BASE).unwrap_or(location))
                    .ok()
                    .and_then(|reply| alert_text(&reply.body))
            }
            Some(_) => None,
            None => alert_text(&posted.body),
        };
        view.message = Some(match reason {
            Some(reason) => format!("提出が受け付けられませんでした：{reason}"),
            None => format!(
                "提出が受け付けられませんでした（HTTP {}{}）",
                posted.status,
                page_title(&posted.body)
                    .map(|title| format!("、{title}"))
                    .unwrap_or_default()
            ),
        });
        return SubmitOutcome::Sent {
            submission: Box::new(view),
            dialog_skipped: None,
        };
    }
    // 6. 提出の ID（自分の提出の一覧の、その問題のいちばん新しいもの）
    let since = (sent_ms / 1000) as i64 - CLOCK_SLACK_SECS;
    let id = remote
        .get(&me)
        .ok()
        .and_then(|reply| newest_submission(&reply.body, contest, task, since));
    match id {
        Some(id) => {
            view.submission_id = Some(id);
            view.state = SubmissionState::Waiting;
            view.tracking = true;
            view.detail_url = Some(detail_url(contest, id));
        }
        None => {
            view.state = SubmissionState::UnknownId;
            view.message = Some(
                "提出は受け付けられましたが、提出の ID を決められませんでした。公式の「自分の提出」で確かめてください"
                    .to_string(),
            );
        }
    }
    SubmitOutcome::Sent {
        submission: Box::new(view),
        dialog_skipped: None,
    }
}

// ---- 判定を追う ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    // 読めた。tracking が false なら、ここで読むのを止めた（message に理由）
    Judge {
        judge: Judge,
        tracking: bool,
        message: Option<String>,
    },
    // 読めなかった。読むのを止めた
    Unreadable(String),
}

// 次に読むまでの間隔：サーバーの Interval のとおり。ただし下限（1 秒）より短くしない
fn next_wait(interval_ms: u64, min: Duration) -> Duration {
    Duration::from_millis(interval_ms).max(min)
}

// 状態の JSON を、判定が決まるまで読む（first_wait なら、最初は 5 秒待つ）。読むたびに on で知らせる
pub fn track(
    remote: &Remote,
    contest: &str,
    id: u64,
    first_wait: bool,
    on: &mut dyn FnMut(Update),
) {
    let deadline = Instant::now() + remote.cap;
    let mut wait = if first_wait {
        remote.first_wait
    } else {
        Duration::ZERO
    };
    let path = format!("/contests/{contest}/submissions/me/status/json?reload=true&sids[]={id}");
    loop {
        std::thread::sleep(wait);
        let reply = match remote.get(&path) {
            Ok(reply) if reply.status == 200 => reply,
            Ok(reply) => {
                on(Update::Unreadable(format!(
                    "判定の状態を読めません（HTTP {}）",
                    reply.status
                )));
                return;
            }
            Err(error) => {
                on(Update::Unreadable(error));
                return;
            }
        };
        let (judge, interval) = match parse_status(&reply.body, id) {
            Ok(parsed) => parsed,
            Err(error) => {
                on(Update::Unreadable(error));
                return;
            }
        };
        if !judge.waiting {
            on(Update::Judge {
                judge,
                tracking: false,
                message: None,
            });
            return;
        }
        let Some(interval) = interval else {
            on(Update::Judge {
                judge,
                tracking: false,
                message: Some(
                    "判定中ですが、読み直す間隔が届かなかったので止めました。「更新」で読めます"
                        .to_string(),
                ),
            });
            return;
        };
        wait = next_wait(interval, remote.min_interval);
        if Instant::now() + wait > deadline {
            on(Update::Judge {
                judge,
                tracking: false,
                message: Some(format!(
                    "{} 分たっても判定が決まらないので、読むのを止めました。「更新」で読めます",
                    remote.cap.as_secs() / 60
                )),
            });
            return;
        }
        on(Update::Judge {
            judge,
            tracking: true,
            message: None,
        });
    }
}

// 読んだ結果を、画面の 1 行に当てはめる
pub fn apply(view: &mut SubmissionView, update: Update) {
    match update {
        Update::Judge {
            judge,
            tracking,
            message,
        } => {
            view.state = if !judge.waiting {
                SubmissionState::Done
            } else if judge.label.contains('/') {
                SubmissionState::Judging
            } else {
                SubmissionState::Waiting
            };
            view.judge = Some(judge);
            view.tracking = tracking;
            view.message = message;
        }
        Update::Unreadable(error) => {
            view.state = SubmissionState::Unreadable;
            view.tracking = false;
            view.message = Some(format!("判定を取得できませんでした：{error}"));
        }
    }
}

// ---- アプリの中の状態 ----

// このアプリを起動してから IDE で出した提出（保存しない）。送る処理は 1 つずつ
#[derive(Default)]
pub struct SubmitState {
    sending: AtomicBool,
    next_key: AtomicU64,
    list: Mutex<Vec<SubmissionView>>,
    // 実行履歴に残す先（提出ごと。判定が決まったら書き足す）
    places: Mutex<HashMap<u64, HistoryPlace>>,
    tracking: Mutex<HashSet<u64>>,
}

// 送る処理の間だけ持つ印
struct Sending<'a>(&'a AtomicBool);

impl Drop for Sending<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl SubmitState {
    fn begin(&self) -> Result<Sending<'_>, String> {
        if self.sending.swap(true, Ordering::SeqCst) {
            return Err("提出を送っているところです".to_string());
        }
        Ok(Sending(&self.sending))
    }

    fn lock_list(&self) -> std::sync::MutexGuard<'_, Vec<SubmissionView>> {
        self.list
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // 新しい順
    pub fn views(&self) -> Vec<SubmissionView> {
        self.lock_list().iter().rev().cloned().collect()
    }

    fn update(&self, key: u64, change: impl FnOnce(&mut SubmissionView)) -> Option<SubmissionView> {
        let mut list = self.lock_list();
        let view = list.iter_mut().find(|view| view.key == key)?;
        change(view);
        Some(view.clone())
    }

    fn place(&self, key: u64) -> Option<HistoryPlace> {
        self.places
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&key)
            .cloned()
    }

    // 追い始める。すでに追っていれば false
    fn start_tracking(&self, key: u64) -> bool {
        self.tracking
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key)
    }

    fn stop_tracking(&self, key: u64) {
        self.tracking
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&key);
    }
}

// 実行履歴に残す先（アプリのフォルダと作業領域のルート）
#[derive(Clone)]
pub struct HistoryPlace {
    pub dir: PathBuf,
    pub workspace: String,
}

// 最後のサンプルテストの結果（実行履歴から）。例：「全 4 件中 AC 4 件（10/26 20:12）」
pub fn last_test_summary(
    place: Option<&HistoryPlace>,
    target: &Target,
    source_path: Option<&Path>,
) -> String {
    let none = "このファイルではまだ行っていません".to_string();
    let Some(place) = place else {
        return none;
    };
    let Ok(entries) = history::entries_for(&place.dir, &place.workspace) else {
        return none;
    };
    let Some(entry) = entries.iter().find(|entry| {
        entry.package == target.package
            && entry.bin == target.bin
            && matches!(entry.kind, history::Kind::Test | history::Kind::PartialTest)
    }) else {
        return none;
    };
    let Finish::Tested {
        total,
        accepted,
        failed,
        unjudged,
        interrupted,
        ..
    } = entry.finish
    else {
        return none;
    };
    let mut text = format!("全 {total} 件中 AC {accepted} 件");
    if failed > 0 {
        text.push_str(&format!("、AC 以外 {failed} 件"));
    }
    if unjudged > 0 {
        text.push_str(&format!("、目視確認 {unjudged} 件"));
    }
    if entry.kind == history::Kind::PartialTest {
        text.push_str("、一部のケースだけ");
    }
    if interrupted {
        text.push_str("、中断");
    }
    text.push_str(&format!("（{}）", local_time(entry.time_ms)));
    let modified_after = source_path
        .and_then(|path| std::fs::metadata(path).ok()?.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .is_some_and(|modified| modified.as_millis() as u64 > entry.time_ms);
    if modified_after {
        text.push_str("。そのあとでファイルが変わっています");
    }
    text
}

// Windows：例：10/26 20:12（手元の時刻）。Unix の localtime_r の代わりに Windows の API で変える
#[cfg(windows)]
fn local_time(ms: u64) -> String {
    match crate::win::local_month_day_time(ms) {
        Some((month, day, hour, minute)) => format!("{month:02}/{day:02} {hour:02}:{minute:02}"),
        None => String::new(),
    }
}

// 例：10/26 20:12（手元の時刻）
#[cfg(unix)]
fn local_time(ms: u64) -> String {
    let secs = (ms / 1000) as libc::time_t;
    // SAFETY: localtime_r は、渡した tm に書くだけ（スレッドごとの領域を使わない）
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return String::new();
    }
    format!(
        "{:02}/{:02} {:02}:{:02}",
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

fn emit<R: Runtime>(app: &AppHandle<R>, view: &SubmissionView) {
    let _ = app.emit_to("main", SUBMISSION_EVENT, view);
}

// 判定を追う処理を、別のスレッドで動かす。決まったら実行履歴に判定を書き足す
fn spawn_tracker<R: Runtime>(
    app: AppHandle<R>,
    key: u64,
    contest: String,
    id: u64,
    first_wait: bool,
) {
    if !app.state::<SubmitState>().start_tracking(key) {
        return;
    }
    std::thread::spawn(move || {
        let remote = Remote::app();
        let state = app.state::<SubmitState>();
        let place = state.place(key);
        track(&remote, &contest, id, first_wait, &mut |update| {
            let Some(view) = state.update(key, |view| apply(view, update)) else {
                return;
            };
            if view.state == SubmissionState::Done
                && let (Some(place), Some(judge)) = (&place, &view.judge)
            {
                let _ = history::set_submission_verdict(
                    &place.dir,
                    &place.workspace,
                    id,
                    &judge.label,
                    judge.score.as_deref(),
                );
            }
            emit(&app, &view);
        });
        state.stop_tracking(key);
    });
}

// 提出の前のサンプルテストの確かめ（Phase 9 作業 2。D-162）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SampleGate {
    // 最後のサンプルテスト（全ケース）が、今のソースで、すべて AC
    Passed { total: usize },
    // 通っていない（まだ・一部だけ・AC 以外・中断・ビルドできない・そのあとでソースが変わった）
    Failed { reason: String },
    // 確かめられない（ケースがない・目視確認のケースがある）。ダイアログを出し、止めはしない
    Unconfirmed,
}

// entries は実行履歴（新しい順）。source_modified_ms は提出するソースの更新日時
pub fn sample_gate(
    entries: &[history::EntryView],
    target: &Target,
    source_modified_ms: Option<u64>,
    has_cases: bool,
) -> SampleGate {
    let failed = |reason: &str| SampleGate::Failed {
        reason: reason.to_string(),
    };
    if !has_cases {
        return SampleGate::Unconfirmed;
    }
    let Some(entry) = entries.iter().find(|entry| {
        entry.package == target.package
            && entry.bin == target.bin
            && matches!(entry.kind, history::Kind::Test | history::Kind::PartialTest)
    }) else {
        return failed("このファイルではまだサンプルテストを行っていません");
    };
    if entry.kind == history::Kind::PartialTest {
        return failed("最後のサンプルテストが、一部のケースだけでした");
    }
    let Finish::Tested {
        total,
        accepted,
        failed: wrong,
        unjudged,
        interrupted,
        ..
    } = entry.finish
    else {
        return failed("最後のサンプルテストが、ビルドの失敗などで終わっています");
    };
    if interrupted {
        return failed("最後のサンプルテストを止めました");
    }
    if wrong > 0 || total == 0 {
        return SampleGate::Failed {
            reason: format!("全 {total} 件中 AC {accepted} 件"),
        };
    }
    if unjudged > 0 {
        return SampleGate::Unconfirmed;
    }
    if source_modified_ms.is_none_or(|modified| modified > entry.time_ms) {
        return failed("サンプルテストのあとで、ファイルが変わっています");
    }
    SampleGate::Passed { total }
}

// 確認のダイアログを省くか（D-162）。省くなら、サンプルテストのケースの数。
// 提出の前の検査の注意（別の問題のファイルかもしれない・別ファイルの mod・ジャッジにないクレートなど）があるときは、
// 省かない（Phase 10 作業 2。D-165）
fn dialog_skip(gate: &SampleGate, settings: &SubmitSettings, warnings: &[String]) -> Option<usize> {
    match gate {
        SampleGate::Passed { total } if !settings.confirm_dialog && warnings.is_empty() => {
            Some(*total)
        }
        _ => None,
    }
}

// 提出の前の確かめに使うもの（commands.rs が settings.json とテストのフォルダから決める。画面からは受け取らない）
pub struct SubmitChecks {
    pub settings: SubmitSettings,
    pub has_cases: bool,
}

// 「提出」を押したとき（commands.rs から、メインスレッドの外で呼ぶ）
#[allow(clippy::too_many_arguments)]
pub fn submit<R: Runtime>(
    app: &AppHandle<R>,
    prepared: Prepared,
    source_path: Option<PathBuf>,
    target: &Target,
    contest: &str,
    task: &str,
    place: Option<HistoryPlace>,
    checks: SubmitChecks,
) -> Result<SubmitOutcome, String> {
    let state = app.state::<SubmitState>();
    let _sending = state.begin()?;
    let key = state.next_key.fetch_add(1, Ordering::SeqCst) + 1;
    let remote = Remote::app();
    let last_test = last_test_summary(place.as_ref(), target, source_path.as_deref());
    // 提出の前のサンプルテスト（D-162）。通っていなければ、ネットワークに出る前に止める
    let entries = place
        .as_ref()
        .and_then(|place| history::entries_for(&place.dir, &place.workspace).ok())
        .unwrap_or_default();
    let source_modified_ms = source_path
        .as_deref()
        .and_then(|path| std::fs::metadata(path).ok()?.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|modified| modified.as_millis() as u64);
    let gate = sample_gate(&entries, target, source_modified_ms, checks.has_cases);
    if let SampleGate::Failed { reason } = &gate
        && checks.settings.test_before_submit
    {
        return Ok(SubmitOutcome::Refused {
            reason: format!("サンプルテストが通っていないので、提出しません（{reason}）"),
            file: prepared.file.clone(),
            warnings: prepared.warnings.clone(),
            submit_url: submit_url(contest, task),
        });
    }
    let skip_dialog = dialog_skip(&gate, &checks.settings, &prepared.warnings);
    let sessions = app.state::<SessionState>();
    let outcome = submit_flow(
        &remote,
        &prepared,
        contest,
        task,
        key,
        last_test,
        |confirmation| {
            if skip_dialog.is_some() {
                return true;
            }
            app.dialog()
                .message(confirmation.message())
                .title("AtCoder に提出しますか")
                .kind(MessageDialogKind::Warning)
                .buttons(MessageDialogButtons::OkCancelCustom(
                    "提出する".to_string(),
                    "キャンセル".to_string(),
                ))
                .blocking_show()
        },
        |user| {
            if user.is_some() && user != sessions.user() {
                sessions.set_user(user.clone());
                session::notify(
                    app,
                    SessionView {
                        logged_in: true,
                        user_name: user,
                        error: None,
                    },
                );
            }
        },
    );
    let outcome = match outcome {
        SubmitOutcome::Sent { submission, .. } => SubmitOutcome::Sent {
            submission,
            dialog_skipped: skip_dialog,
        },
        other => other,
    };
    match &outcome {
        SubmitOutcome::NotLoggedIn { .. } => {
            sessions.set_user(None);
            if let Ok(view) = sessions.view(&Keychain::app()) {
                session::notify(app, view);
            }
        }
        SubmitOutcome::Sent { submission, .. } => {
            if let Some(place) = &place {
                let entry = history::Entry {
                    time_ms: submission.time_ms,
                    workspace: place.workspace.clone(),
                    package: target.package.clone(),
                    bin: target.bin.clone(),
                    kind: history::Kind::Submit,
                    finish: Finish::Submitted(Box::new(SubmittedRecord {
                        contest: contest.to_string(),
                        task: task.to_string(),
                        submission_id: submission.submission_id,
                        verdict: None,
                        score: None,
                        error: matches!(
                            submission.state,
                            SubmissionState::Rejected | SubmissionState::Unreadable
                        )
                        .then(|| submission.message.clone())
                        .flatten(),
                    })),
                };
                let _ = history::record(&place.dir, entry);
            }
            state.lock_list().push(submission.as_ref().clone());
            if let Some(place) = place {
                state
                    .places
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(submission.key, place);
            }
            emit(app, submission);
            if let Some(id) = submission.submission_id {
                spawn_tracker(app.clone(), submission.key, contest.to_string(), id, true);
            }
        }
        _ => {}
    }
    Ok(outcome)
}

// 「更新」：判定を読み直す。まだ判定中なら、また追う（上限は読み直したときから数える）
pub fn refresh<R: Runtime>(app: &AppHandle<R>, key: u64) -> Result<SubmissionView, String> {
    let state = app.state::<SubmitState>();
    let view = state
        .views()
        .into_iter()
        .find(|view| view.key == key)
        .ok_or_else(|| {
            "その提出は、このアプリを起動してから出したものではありません".to_string()
        })?;
    let id = view
        .submission_id
        .ok_or_else(|| "提出の ID が分からないので、読み直せません".to_string())?;
    let view = state
        .update(key, |view| {
            view.tracking = true;
            view.message = None;
        })
        .unwrap_or(view);
    emit(app, &view);
    spawn_tracker(app.clone(), key, view.contest.clone(), id, false);
    Ok(view)
}

// 公式の提出のページを、既定のブラウザで開く（期間の外・送れなかったときの「コピーして開く」）
pub fn open_submit_page(contest: &str, task: &str) -> Result<(), String> {
    crate::problem::ProblemId::new(contest, task)?;
    open_in_browser(&submit_url(contest, task))
}

// 提出の詳細（id があるとき）か、自分の提出の一覧を、既定のブラウザで開く（「提出結果」・「履歴」の「詳細」「自分の提出」）。
// URL は、ID を確かめてから IDE が組み立てる（画面から URL は受け取らない）
pub fn open_submission_page(contest: &str, id: Option<u64>) -> Result<(), String> {
    crate::problem::ProblemId::new(contest, "x")?;
    open_in_browser(&match id {
        Some(id) => detail_url(contest, id),
        None => list_url(contest),
    })
}

// ---- コンテストの提出状況（Phase 18 作業 4。D-218。docs/phase18/05-contest-plan.md） ----

// 読む「自分の提出」のページの上限（1 ページ 20 件なら 200 件【推測】）
const MAX_STATUS_PAGES: u32 = 10;

// 「自分の提出」の表の 1 行
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MineRow {
    pub id: u64,
    pub task: String,
    // 判定の欄の文字（例：AC、WA、WJ、3/15 WA）
    pub verdict: String,
}

// 「自分の提出」の 1 ページ（`/contests/<コンテスト>/submissions/me?page=<page>`）の行と、次のページへのリンクがあるか。
// 問題のリンク・提出の ID が読めない行は飛ばす
pub fn parse_mine_page(html: &str, contest: &str, page: u32) -> (Vec<MineRow>, bool) {
    let document = Document::from(html);
    let tasks = format!("/contests/{contest}/tasks/");
    let details = format!("/contests/{contest}/submissions/");
    let rows = document
        .select("table tbody tr")
        .iter()
        .filter_map(|row| {
            let task = row.select("a").iter().find_map(|link| {
                let href = link.attr("href")?;
                let task = href.strip_prefix(tasks.as_str())?;
                (!task.is_empty()
                    && task
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
                .then(|| task.to_string())
            })?;
            let id = row
                .select(".submission-score")
                .attr("data-id")
                .and_then(|id| id.parse().ok())
                .or_else(|| {
                    row.select("a").iter().find_map(|link| {
                        link.attr("href")?
                            .strip_prefix(details.as_str())?
                            .parse()
                            .ok()
                    })
                })?;
            let verdict = squash(&row.select("span.label").first().text());
            Some(MineRow {
                id,
                task,
                verdict: if verdict.is_empty() {
                    "?".to_string()
                } else {
                    verdict
                },
            })
        })
        .collect();
    // ページ送りのリンク（`…/submissions/me?page=<次>`。ほかのクエリが付いていてもよい）
    let me = format!("/contests/{contest}/submissions/me");
    let next = format!("page={}", page + 1);
    let has_next = document.select("a").iter().any(|link| {
        link.attr("href").is_some_and(|href| {
            location_path(&href) == me
                && href
                    .split(['?', '&'])
                    .skip(1)
                    .any(|part| part == next.as_str())
        })
    });
    (rows, has_next)
}

// 問題ごとの提出状況
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    // 問題 ID（例：abc480_a）
    pub task: String,
    // 例：「A」。問題の一覧にない問題は空
    pub label: String,
    pub title: String,
    // AC の提出がある
    pub accepted: bool,
    // いちばん新しい提出の判定と ID（提出がなければ None）
    pub last: Option<String>,
    pub last_id: Option<u64>,
    // 読んだ提出の数
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContestStatus {
    pub contest: String,
    pub tasks: Vec<TaskStatus>,
    // 読んだ時刻（UNIX 時間のミリ秒）
    pub read_at_ms: u64,
    // 上限のページまで読んでも続きがあった（新しいものだけを数えた）
    pub truncated: bool,
    pub pages: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum StatusOutcome {
    Read { status: ContestStatus },
    // ログインしていない（Keychain にない、ログインのページへ移された）
    NotLoggedIn,
    Failed { message: String },
}

// 問題の一覧と、読んだ行から、問題ごとの状況を作る。一覧にない問題の提出は、後ろに足す
pub fn summarize(tasks: &[crate::problem::TaskEntry], rows: &[MineRow]) -> Vec<TaskStatus> {
    let mut list: Vec<TaskStatus> = tasks
        .iter()
        .map(|entry| TaskStatus {
            task: entry.task.clone(),
            label: entry.label.clone(),
            title: entry.title.clone(),
            accepted: false,
            last: None,
            last_id: None,
            count: 0,
        })
        .collect();
    for row in rows {
        let index = match list.iter().position(|status| status.task == row.task) {
            Some(index) => index,
            None => {
                list.push(TaskStatus {
                    task: row.task.clone(),
                    label: String::new(),
                    title: row.task.clone(),
                    accepted: false,
                    last: None,
                    last_id: None,
                    count: 0,
                });
                list.len() - 1
            }
        };
        let status = &mut list[index];
        status.count += 1;
        status.accepted |= row.verdict == "AC";
        if status.last_id.is_none_or(|last| row.id > last) {
            status.last = Some(row.verdict.clone());
            status.last_id = Some(row.id);
        }
    }
    list
}

// 「自分の提出」のページを 1 ページ目から読む。次のページへのリンクがない・新しい行がないところで止める。
// 上限まで読んでも続きがあれば truncated。同じ提出が 2 つのページに出ても（読む間に提出が増えたとき）1 つと数える
fn read_mine(remote: &Remote, contest: &str) -> Result<(Vec<MineRow>, bool, u32), String> {
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    for page in 1..=MAX_STATUS_PAGES {
        let reply = remote.get(&format!("/contests/{contest}/submissions/me?page={page}"))?;
        if reply.status != 200 {
            let reason = alert_text(&reply.body)
                .or_else(|| page_title(&reply.body))
                .map(|text| format!("、{text}"))
                .unwrap_or_default();
            return Err(format!(
                "「自分の提出」のページを読めません（HTTP {}{reason}）。コンテストが始まる前か、参加していないのかもしれません",
                reply.status
            ));
        }
        let (found, has_next) = parse_mine_page(&reply.body, contest, page);
        let before = rows.len();
        for row in found {
            if seen.insert(row.id) {
                rows.push(row);
            }
        }
        if !has_next || rows.len() == before {
            return Ok((rows, false, page));
        }
    }
    Ok((rows, true, MAX_STATUS_PAGES))
}

// コンテストの提出状況（サイドバーの「コンテスト」）。問題の一覧は覚えてあるもの（なければ読む）
pub fn contest_status(remote: &Remote, app_dir: &Path, contest: &str) -> StatusOutcome {
    if let Err(message) = crate::problem::ProblemId::new(contest, "x") {
        return StatusOutcome::Failed { message };
    }
    match remote.logged_in() {
        Ok(true) => {}
        Ok(false) => return StatusOutcome::NotLoggedIn,
        Err(message) => return StatusOutcome::Failed { message },
    }
    let tasks = match crate::problem::load_task_list(app_dir, contest, false) {
        Ok(tasks) => tasks,
        Err(message) => return StatusOutcome::Failed { message },
    };
    match read_mine(remote, contest) {
        Ok((rows, truncated, pages)) => StatusOutcome::Read {
            status: ContestStatus {
                contest: contest.to_string(),
                tasks: summarize(&tasks, &rows),
                read_at_ms: now_ms(),
                truncated,
                pages,
            },
        },
        Err(message) if message == NOT_LOGGED_IN => StatusOutcome::NotLoggedIn,
        Err(message) => StatusOutcome::Failed { message },
    }
}

fn open_in_browser(url: &str) -> Result<(), String> {
    let status = std::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .map_err(|error| format!("ブラウザを開けません：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("ブラウザを開けません".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tests::{TestKeychain, decode};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    const SUBMIT_PAGE: &str = include_str!("../testdata/phase6/submit-ended.html");
    const MINE_PAGE: &str = include_str!("../testdata/phase6/submissions-me.html");
    const STATUS_AC: &str = include_str!("../testdata/phase6/status-ac.json");
    // 判定中の状態の JSON は、まだ手に入っていない（P3 で確かめる）。公式の contest.js の読み方（td に waiting-judge）に合わせて作ったもの
    const STATUS_JUDGING: &str = r#"{"Result":{"123":{"Html":"<td class='text-center waiting-judge' data-id='123'><span class='label label-default' data-toggle='tooltip' data-placement='top' title=\"Judging\">3/20</span></td>","Score":"0"}},"Interval":5}"#;

    // 期間中のフォーム：終わったコンテストのページから、Turnstile の部品を外したもの
    fn open_form(contest: &str) -> String {
        SUBMIT_PAGE
            .replace("abc480", contest)
            .replace(
                "challenges.cloudflare.com/turnstile",
                "example.invalid/removed",
            )
            .replace("cf-challenge", "removed")
    }

    // UNIX 時間（秒）を「2026-10-26T20:00:00+09:00」（sep が T）・「2026-10-26 20:00:00+0900」（sep が空白）にする
    fn jst(unix: i64, sep: char) -> String {
        let local = unix + 9 * 3600;
        let (days, secs) = (local.div_euclid(86_400), local.rem_euclid(86_400));
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        let offset = if sep == 'T' { "+09:00" } else { "+0900" };
        format!(
            "{year:04}-{month:02}-{day:02}{sep}{:02}:{:02}:{:02}{offset}",
            secs / 3600,
            secs % 3600 / 60,
            secs % 60
        )
    }

    fn top_page(start: i64, end: i64) -> String {
        format!(
            "<script>var startTime = moment(\"{}\");\nvar endTime = moment(\"{}\");</script>",
            jst(start, 'T'),
            jst(end, 'T')
        )
    }

    fn mine_page(contest: &str, rows: &[(&str, u64, i64)]) -> String {
        let rows: String = rows
            .iter()
            .map(|(task, id, time)| {
                format!(
                    "<tr><td><time class='fixtime fixtime-second'>{}</time></td><td><a href=\"/contests/{contest}/tasks/{task}\">A</a></td><td class=\"text-right submission-score\" data-id=\"{id}\">0</td></tr>",
                    jst(*time, ' ')
                )
            })
            .collect();
        format!("<table><thead><tr><th>t</th></tr></thead><tbody>{rows}</tbody></table>")
    }

    // 受け取った要求（方法、パスとクエリ、見出し、本文）
    #[derive(Debug, Clone)]
    struct Request {
        method: String,
        target: String,
        head: String,
        body: String,
    }

    type Route = dyn Fn(&Request, usize) -> String + Send + Sync;

    // 手元の http の口。route は、要求と、同じ方法・パスの何回目か（0 から）から、応答の全体を返す
    fn fake(route: Arc<Route>) -> (String, Arc<Mutex<Vec<Request>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let seen = Arc::new(Mutex::new(Vec::<Request>::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let target = parts.next().unwrap_or("").to_string();
                let (mut head, mut length, mut expect) = (String::new(), 0, false);
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let lower = line.to_ascii_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                    expect |= lower.starts_with("expect: 100-continue");
                    if line == "\r\n" {
                        break;
                    }
                    head.push_str(&line);
                }
                if expect {
                    reader
                        .get_mut()
                        .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
                        .unwrap();
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let request = Request {
                    method,
                    target,
                    head,
                    body: String::from_utf8(body).unwrap(),
                };
                let nth = {
                    let mut seen = log.lock().unwrap();
                    let nth = seen
                        .iter()
                        .filter(|r| r.method == request.method && r.target == request.target)
                        .count();
                    seen.push(request.clone());
                    nth
                };
                let response = route(&request, nth);
                let _ = reader.get_mut().write_all(response.as_bytes());
            }
        });
        (base, seen)
    }

    fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn redirect(location: &str, cookie: Option<&str>) -> String {
        let cookie = cookie
            .map(|value| format!("Set-Cookie: REVEL_SESSION={value}; Path=/; HttpOnly\r\n"))
            .unwrap_or_default();
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\n{cookie}Content-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }

    fn not_found() -> String {
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
    }

    fn remote(keychain: &TestKeychain, base: &str) -> Remote {
        Remote {
            client: keychain.client(),
            base: base.to_string(),
            spacing: Duration::ZERO,
            first_wait: Duration::ZERO,
            min_interval: Duration::from_millis(1),
            cap: Duration::from_secs(5),
            last: Mutex::new(None),
        }
    }

    fn prepared() -> Prepared {
        Prepared {
            language: Language::Rust,
            source: "fn main() {\n    println!(\"a\\\\b \\\" あ\");\n}\n".to_string(),
            file: "src/bin/a.rs".to_string(),
            warnings: vec!["注意の例".to_string()],
            python: None,
        }
    }

    fn form_fields(body: &str) -> Vec<(String, String)> {
        body.split('&')
            .map(|pair| {
                let (key, value) = pair.split_once('=').unwrap();
                (decode(key), decode(value))
            })
            .collect()
    }

    // 期間中のコンテスト x の、ふつうの応答
    fn contest_routes(post: fn(usize) -> String) -> Arc<Route> {
        Arc::new(move |request: &Request, nth: usize| {
            let now = now_secs();
            match (request.method.as_str(), request.target.as_str()) {
                ("GET", "/contests/x") => ok(&top_page(now - 3600, now + 3600)),
                ("GET", "/contests/x/submit?taskScreenName=x_a") => ok(&open_form("x")),
                ("POST", "/contests/x/submit") => post(nth),
                ("GET", "/contests/x/submissions/me") => ok(&mine_page(
                    "x",
                    &[
                        ("x_a", 100, now - 4000),
                        ("x_b", 124, now),
                        ("x_a", 123, now),
                    ],
                )),
                ("GET", "/contests/x/submissions/me/status/json?reload=true&sids[]=123") => {
                    ok(if nth == 0 {
                        STATUS_JUDGING
                    } else {
                        STATUS_AC_123
                    })
                }
                _ => not_found(),
            }
        })
    }

    const STATUS_AC_123: &str = r#"{"Result":{"123":{"Html":"<td class='text-center'><span class='label label-success' data-toggle='tooltip' data-placement='top' title=\"Accepted\">AC</span></td><td class='text-right'>30 ms</td><td class='text-right'>4000 KiB</td>","Score":"400"}}}"#;

    #[test]
    fn lists_the_68_judge_crates() {
        assert_eq!(JUDGE_CRATES.len(), 68);
        let mut names: Vec<&str> = JUDGE_CRATES.iter().map(|(name, _)| *name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 68);
        assert_eq!(judge_version("proconio"), Some("0.5.0"));
        assert_eq!(judge_version("ac_library_rs"), Some("0.2.0"));
        assert_eq!(judge_version("serde"), None);
    }

    #[test]
    fn finds_modules_in_other_files() {
        let source = "mod a;\npub mod b;\n  #[path = \"x.rs\"] mod c; // c\npub(crate) mod d;\nmod e {\n}\n// mod f;\nlet mod_x = 1;\nmod g { fn h() {} }\n";
        assert_eq!(module_files(source), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn warns_when_bin_and_task_letters_differ() {
        let two = ["a".to_string(), "b".to_string()];
        assert!(bin_task_warning(&two, "a", "abc480_b").is_some());
        assert_eq!(bin_task_warning(&two, "a", "abc480_a"), None);
        assert_eq!(bin_task_warning(&two, "A", "abc480_a"), None);
        assert_eq!(bin_task_warning(&["a".to_string()], "a", "abc480_b"), None);
        let compete = ["abc480-a".to_string(), "abc480-b".to_string()];
        assert!(bin_task_warning(&compete, "abc480-a", "abc480_b").is_some());
        assert_eq!(bin_task_warning(&compete, "abc480-b", "abc480_b"), None);
        // 記号の形でない名前は比べない
        let named = ["main".to_string(), "tool".to_string()];
        assert_eq!(bin_task_warning(&named, "main", "abc480_b"), None);
    }

    #[test]
    fn warns_when_folder_problem_differs() {
        let id = |contest: &str, task: &str| crate::problem::ProblemId::new(contest, task).unwrap();
        let folder = id("abc480", "abc480_a");
        assert_eq!(
            folder_problem_warning(Some(&folder), &id("abc480", "abc480_a")),
            None
        );
        assert_eq!(
            folder_problem_warning(None, &id("abc480", "abc480_b")),
            None
        );
        let other = folder_problem_warning(Some(&folder), &id("abc480", "abc480_b")).unwrap();
        assert!(other.contains("abc480_a") && other.contains("abc480_b"));
        // 問題 ID が同じでもコンテストが違えば知らせる（ABC と ARC の同時開催の問題など）
        assert!(
            folder_problem_warning(Some(&id("arc058", "arc058_a")), &id("abc042", "arc058_a"))
                .is_some()
        );
    }

    fn dependency(name: &str, source: Option<&str>, kind: Option<&str>) -> MetaDependency {
        MetaDependency {
            name: name.to_string(),
            source: source.map(str::to_string),
            kind: kind.map(str::to_string),
        }
    }

    #[test]
    fn compares_crates_with_the_judge() {
        let registry = Some("registry+https://github.com/rust-lang/crates.io-index");
        let dependencies = [
            dependency("proconio", registry, None),
            dependency("ac-library-rs", registry, None),
            dependency("serde", registry, None),
            dependency("mylib", None, None),
            dependency("gitlib", Some("git+https://example.com/x"), None),
            dependency("criterion", registry, Some("dev")),
        ];
        let lock = "version = 4\n\n[[package]]\nname = \"proconio\"\nversion = \"0.4.5\"\nsource = \"registry+…\"\n\n[[package]]\nname = \"ac-library-rs\"\nversion = \"0.2.0\"\n";
        let warnings = crate_warnings(&dependencies, Some(lock));
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        assert!(
            warnings[0].contains("proconio")
                && warnings[0].contains("0.4.5")
                && warnings[0].contains("0.5.0")
        );
        assert!(warnings[1].contains("serde") && warnings[1].contains("ジャッジにありません"));
        assert!(warnings[2].contains("mylib") && warnings[2].contains("パスか git"));
        assert!(warnings[3].contains("gitlib"));
        // Cargo.lock がなければ、版は比べられないと出す
        let warnings = crate_warnings(&dependencies[..2], None);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Cargo.lock"));
    }

    #[test]
    fn parses_times_and_the_contest_period() {
        assert_eq!(parse_time("2026-10-26T20:00:00+09:00"), Some(1_793_012_400));
        assert_eq!(parse_time("2026-10-26 20:00:00+0900"), Some(1_793_012_400));
        assert_eq!(parse_time("2026-10-26T11:00:00+00:00"), Some(1_793_012_400));
        assert_eq!(parse_time("1970-01-01T09:00:00+09:00"), Some(0));
        for bad in [
            "",
            "2026-10-26",
            "2026-13-01T00:00:00+09:00",
            "2026-10-26T20:00:00Z",
            "２０２６-10-26T20:00:00+09:00",
        ] {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
        let period = parse_period(SUBMIT_PAGE).unwrap();
        assert_eq!(
            period.start,
            parse_time("2026-10-17T21:00:00+09:00").unwrap()
        );
        assert_eq!(period.end - period.start, 100 * 60);
        assert_eq!(period.end_text, "2026-10-17 22:40:00+09:00");
        assert!(period.contains(period.start) && !period.contains(period.end));
        assert_eq!(parse_period("<html></html>"), None);
        assert_eq!(jst(1_793_012_400, 'T'), "2026-10-26T20:00:00+09:00");
    }

    #[test]
    fn prepares_a_cpp_submission() {
        let base =
            std::env::temp_dir().join(format!("atcoder-studio-submit-cpp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("abc480/abc480_a")).unwrap();
        let root = base.canonicalize().unwrap();
        let dir = root.join("abc480/abc480_a");
        let source = "#include <bits/stdc++.h>\n  # include \"lib/x.hpp\"\n// #include \"y.hpp\" も行ごとに見る\n#include<atcoder/all>\nint main() {}\n";
        std::fs::write(dir.join("main.cpp"), source).unwrap();
        let prepared = prepare_cpp(&root, &dir, Some("15.2.0")).unwrap();
        assert_eq!(prepared.language, Language::Cpp);
        assert_eq!(prepared.file, "abc480/abc480_a/main.cpp");
        assert_eq!(prepared.source, source);
        assert_eq!(
            prepared.warnings,
            [
                "自分のヘッダーを読む #include（#include \"lib/x.hpp\"）があります。ジャッジには、このファイルだけが置かれます"
            ]
        );
        // 手元の GCC の版が違うと知らせる
        std::fs::write(dir.join("main.cpp"), "int main() {}\n").unwrap();
        assert_eq!(
            prepare_cpp(&root, &dir, Some("14.2.0")).unwrap().warnings,
            ["手元の GCC（14.2.0）は、ジャッジ（15.2.0）と版が違います"]
        );
        assert!(prepare_cpp(&root, &dir, None).unwrap().warnings.is_empty());
        // 大きすぎる・作業領域の外を指すリンク・ない
        std::fs::write(dir.join("main.cpp"), vec![b'a'; SOURCE_LIMIT + 1]).unwrap();
        assert!(
            prepare_cpp(&root, &dir, None)
                .unwrap_err()
                .contains("512 KiB")
        );
        let outside = std::env::temp_dir().join(format!(
            "atcoder-studio-submit-cpp-outside-{}",
            std::process::id()
        ));
        std::fs::write(&outside, "int main() {}\n").unwrap();
        std::fs::remove_file(dir.join("main.cpp")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("main.cpp")).unwrap();
        assert!(
            prepare_cpp(&root, &dir, None)
                .unwrap_err()
                .contains("作業領域の外")
        );
        std::fs::remove_file(dir.join("main.cpp")).unwrap();
        assert!(
            prepare_cpp(&root, &dir, None)
                .unwrap_err()
                .contains("main.cpp が見つかりません")
        );
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn chooses_the_cpp_option_and_note() {
        // 言語の選択肢に C++ を足したフォーム。C++ は C++23 (GCC 15.2.0) だけを選ぶ
        let page = open_form("abc480").replace(
            "<option value=\"6088\" data-ace-mode=\"rust\">Rust (rustc 1.89.0)",
            "<option value=\"6017\" data-ace-mode=\"c_cpp\">C++23 (Clang 21.1.0)</option><option value=\"6018\" data-ace-mode=\"c_cpp\">C++23 (GCC 15.2.0)</option><option value=\"6088\" data-ace-mode=\"rust\">Rust (rustc 1.89.0)",
        );
        let form = parse_form(&page, "abc480", "abc480_a").unwrap();
        let ready = check_form(Some(&form), "abc480_a", Language::Cpp, None).unwrap();
        assert_eq!(
            (ready.language_id.as_str(), ready.language.as_str()),
            ("6018", "C++23 (GCC 15.2.0)")
        );
        assert_eq!(
            check_form(Some(&form), "abc480_a", Language::Rust, None)
                .unwrap()
                .language_id,
            "6088"
        );
        // C++ の選択肢がなければ送らない
        let open = parse_form(&open_form("abc480"), "abc480", "abc480_a").unwrap();
        assert_eq!(
            check_form(Some(&open), "abc480_a", Language::Cpp, None).unwrap_err(),
            "提出のフォームの言語の選択肢に C++ がありません"
        );
        // 確認のダイアログの注意には、C++ のジャッジの出どころを添える
        let confirmation = Confirmation {
            contest: "abc480".into(),
            task: "abc480_a".into(),
            task_title: "A - 例".into(),
            end_text: "2026-10-26 21:00:00+09:00".into(),
            file: "abc480/abc480_a/main.cpp".into(),
            size: 10,
            language: "C++23 (GCC 15.2.0)".into(),
            user: None,
            last_test: "なし".into(),
            warnings: vec!["注意".into()],
            judge_note: cpp::JUDGE_SUBMIT_NOTE,
        };
        let message = confirmation.message();
        assert!(message.contains("言語：C++23 (GCC 15.2.0)"));
        assert!(message.contains(cpp::JUDGE_SUBMIT_NOTE));
        assert!(!message.contains(JUDGE_CRATES_NOTE));
    }

    // Python は設定の処理系の選択肢を選ぶ（Phase 19 作業 4。D-224）。選択肢の値は見本のもので、本物のページの値ではない
    #[test]
    fn chooses_the_python_option() {
        let page = open_form("abc480").replace(
            "<option value=\"6088\" data-ace-mode=\"rust\">Rust (rustc 1.89.0)",
            "<option value=\"9001\" data-ace-mode=\"python\">Python (CPython 3.13.7)</option><option value=\"9002\" data-ace-mode=\"python\">Python (PyPy 3.11-v7.3.20)</option><option value=\"9003\" data-ace-mode=\"python\">Python (Codon 0.19.3)</option><option value=\"6088\" data-ace-mode=\"rust\">Rust (rustc 1.89.0)",
        );
        let form = parse_form(&page, "abc480", "abc480_a").unwrap();
        let choose = |python| {
            let ready = check_form(Some(&form), "abc480_a", Language::Python, python).unwrap();
            (ready.language_id, ready.language)
        };
        assert_eq!(
            choose(Some(Interpreter::Cpython)),
            ("9001".to_string(), "Python (CPython 3.13.7)".to_string())
        );
        assert_eq!(
            choose(Some(Interpreter::Pypy)),
            ("9002".to_string(), "Python (PyPy 3.11-v7.3.20)".to_string())
        );
        // 処理系が分からない・選択肢がなければ送らない
        assert!(check_form(Some(&form), "abc480_a", Language::Python, None).is_err());
        let open = parse_form(&open_form("abc480"), "abc480", "abc480_a").unwrap();
        assert_eq!(
            check_form(
                Some(&open),
                "abc480_a",
                Language::Python,
                Some(Interpreter::Pypy)
            )
            .unwrap_err(),
            "提出のフォームの言語の選択肢に Python (PyPy 3.11-v7.3.20) がありません"
        );
    }

    #[test]
    fn prepares_a_python_submission() {
        let base = std::env::temp_dir().join(format!(
            "atcoder-studio-submit-python-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("abc480/abc480_a")).unwrap();
        let root = base.canonicalize().unwrap();
        let dir = root.join("abc480/abc480_a");
        let source = "import sys\nimport lib\nimport requests\nprint(1)\n";
        std::fs::write(dir.join("main.py"), source).unwrap();
        std::fs::write(dir.join("lib.py"), "").unwrap();
        // 見つからない処理系：送れるが、確かめていないことを注意に出す
        let missing = PythonSettings {
            interpreter: Interpreter::Pypy,
            pypy: root.join("no-pypy").display().to_string(),
            ..PythonSettings::default()
        };
        let prepared = prepare_python(&root, &dir, &missing).unwrap();
        assert_eq!(prepared.language, Language::Python);
        assert_eq!(prepared.python, Some(Interpreter::Pypy));
        assert_eq!(prepared.file, "abc480/abc480_a/main.py");
        assert_eq!(prepared.source, source);
        assert_eq!(
            prepared.warnings,
            ["手元に PyPy が見つからないので、import と版を確かめていません"]
        );
        // 手元の CPython 3.13 があれば、import の注意が出る
        if python::find_interpreter(&PythonSettings::default()).is_ok() {
            let prepared = prepare_python(&root, &dir, &PythonSettings::default()).unwrap();
            assert_eq!(prepared.python, Some(Interpreter::Cpython));
            assert_eq!(
                prepared.warnings[..2],
                [
                    "自分のモジュールを読む import（lib）があります。ジャッジには、このファイルだけが置かれます",
                    "ジャッジの CPython の言語の一覧にないパッケージ（requests）を import しています",
                ]
            );
        } else {
            eprintln!("python3.13 がないので、import の検査は飛ばします");
        }
        std::fs::remove_file(dir.join("main.py")).unwrap();
        assert!(
            prepare_python(&root, &dir, &PythonSettings::default())
                .unwrap_err()
                .contains("main.py が見つかりません")
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn reads_the_submit_form() {
        let form = parse_form(SUBMIT_PAGE, "abc480", "abc480_a").unwrap();
        // 入力欄の値の文字参照（&#43;）は戻して読む
        assert_eq!(form.csrf.as_deref(), Some("test+csrf/token="));
        assert_eq!(form.tasks.len(), 7);
        assert_eq!(
            form.tasks[0],
            ("abc480_a".to_string(), "A - ADjacent".to_string())
        );
        assert_eq!(
            form.languages
                .iter()
                .filter(|(_, label)| Language::Rust.is_submit_option(label, None))
                .collect::<Vec<_>>(),
            vec![&("6088".to_string(), "Rust (rustc 1.89.0)".to_string())]
        );
        // 同じ値の選択肢は 1 つだけ（ページには言語の選択肢が 2 か所ある）
        assert_eq!(
            form.languages
                .iter()
                .filter(|(value, _)| value == "6088")
                .count(),
            1
        );
        assert!(form.turnstile);
        assert_eq!(form.user.as_deref(), Some("testuser"));
        // 終わったコンテストのフォームには Turnstile があるので、送らない
        assert!(
            check_form(Some(&form), "abc480_a", Language::Rust, None)
                .unwrap_err()
                .contains("Turnstile")
        );
        // Turnstile がなければ送れる
        let open = parse_form(&open_form("abc480"), "abc480", "abc480_b").unwrap();
        assert!(!open.turnstile);
        assert_eq!(
            check_form(Some(&open), "abc480_b", Language::Rust, None).unwrap(),
            Ready {
                task_title: "B - Invalid Vote".to_string(),
                language_id: "6088".to_string(),
                language: "Rust (rustc 1.89.0)".to_string(),
                csrf: "test+csrf/token=".to_string(),
            }
        );
        // 問題の選択肢にない問題
        assert!(
            check_form(Some(&open), "abc480_z", Language::Rust, None)
                .unwrap_err()
                .contains("abc480_z")
        );
        // Rust の選択肢がない
        let no_rust = parse_form(
            &open_form("abc480").replace("Rust (rustc 1.89.0)", "Rast"),
            "abc480",
            "abc480_a",
        )
        .unwrap();
        assert!(
            check_form(Some(&no_rust), "abc480_a", Language::Rust, None)
                .unwrap_err()
                .contains("Rust")
        );
        // csrf_token がない
        let no_csrf = parse_form(
            &open_form("abc480").replace("name=\"csrf_token\"", "name=\"other\""),
            "abc480",
            "abc480_a",
        )
        .unwrap();
        assert!(
            check_form(Some(&no_csrf), "abc480_a", Language::Rust, None)
                .unwrap_err()
                .contains("csrf_token")
        );
        // フォームがない・ほかのコンテストのフォーム
        assert_eq!(
            parse_form("<html><body>Sign In</body></html>", "abc480", "abc480_a"),
            None
        );
        assert_eq!(parse_form(SUBMIT_PAGE, "abc481", "abc481_a"), None);
        assert!(
            check_form(None, "abc480_a", Language::Rust, None)
                .unwrap_err()
                .contains("フォームが見つかりません")
        );
    }

    #[test]
    fn finds_the_newest_submission_for_the_task() {
        let since = parse_time("2026-10-17 21:00:00+0900").unwrap();
        assert_eq!(
            newest_submission(MINE_PAGE, "abc480", "abc480_d", 0),
            Some(10000003)
        );
        assert_eq!(
            newest_submission(MINE_PAGE, "abc480", "abc480_c", 0),
            Some(10000002)
        );
        assert_eq!(
            newest_submission(MINE_PAGE, "abc480", "abc480_c", since),
            Some(10000002)
        );
        // 送った時刻より前の提出は使わない
        let after = parse_time("2026-10-17 21:00:25+0900").unwrap();
        assert_eq!(
            newest_submission(MINE_PAGE, "abc480", "abc480_c", after),
            None
        );
        assert_eq!(newest_submission(MINE_PAGE, "abc480", "abc480_a", 0), None);
        assert_eq!(newest_submission(MINE_PAGE, "abc481", "abc480_d", 0), None);
    }

    #[test]
    fn reads_the_judge_status() {
        let (judge, interval) = parse_status(STATUS_AC, 10000003).unwrap();
        assert_eq!(
            judge,
            Judge {
                waiting: false,
                label: "AC".to_string(),
                title: Some("Accepted".to_string()),
                score: Some("400".to_string()),
                time_ms: Some(30),
                memory_kib: Some(4000),
            }
        );
        assert_eq!(interval, None);
        let (judge, interval) = parse_status(STATUS_JUDGING, 123).unwrap();
        assert!(judge.waiting);
        assert_eq!(judge.label, "3/20");
        assert_eq!((judge.time_ms, judge.memory_kib), (None, None));
        assert_eq!(interval, Some(5));
        // 形が違えば読めないと返す
        assert!(parse_status(STATUS_AC, 1).is_err());
        assert!(parse_status("<html>Request blocked</html>", 1).is_err());
        assert!(parse_status(r#"{"Result":{"1":{"Html":"","Score":"0"}}}"#, 1).is_err());
        assert!(parse_status(r#"{"Result":{"1":{"Score":"0"}}}"#, 1).is_err());
    }

    #[test]
    fn never_reads_the_status_faster_than_once_a_second() {
        let second = Duration::from_secs(1);
        assert_eq!(next_wait(5, second), second);
        assert_eq!(next_wait(0, second), second);
        assert_eq!(next_wait(2500, second), Duration::from_millis(2500));
        assert_eq!(Remote::app().min_interval, second);
        assert_eq!(Remote::app().first_wait, Duration::from_secs(5));
        assert_eq!(Remote::app().cap, Duration::from_secs(600));
    }

    #[test]
    fn shows_states_from_updates() {
        let mut view = SubmissionView {
            key: 1,
            contest: "x".into(),
            task: "x_a".into(),
            task_title: "A".into(),
            time_ms: 0,
            language: "Rust".into(),
            file: "a.rs".into(),
            submission_id: Some(1),
            state: SubmissionState::Waiting,
            tracking: true,
            judge: None,
            message: None,
            detail_url: None,
            list_url: String::new(),
        };
        let judge = |waiting: bool, label: &str| Judge {
            waiting,
            label: label.to_string(),
            title: None,
            score: None,
            time_ms: None,
            memory_kib: None,
        };
        apply(
            &mut view,
            Update::Judge {
                judge: judge(true, "WJ"),
                tracking: true,
                message: None,
            },
        );
        assert_eq!(view.state, SubmissionState::Waiting);
        apply(
            &mut view,
            Update::Judge {
                judge: judge(true, "3/20 WA"),
                tracking: true,
                message: None,
            },
        );
        assert_eq!(view.state, SubmissionState::Judging);
        apply(&mut view, Update::Unreadable("切れた".into()));
        assert_eq!(view.state, SubmissionState::Unreadable);
        assert!(!view.tracking);
        assert!(
            view.message
                .as_deref()
                .unwrap()
                .contains("取得できませんでした")
        );
        apply(
            &mut view,
            Update::Judge {
                judge: judge(false, "WA"),
                tracking: false,
                message: None,
            },
        );
        assert_eq!(view.state, SubmissionState::Done);
        assert_eq!(view.message, None);
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["state"], "done");
        assert_eq!(json["judge"]["label"], "WA");
        assert_eq!(json["submissionId"], 1);
    }

    #[test]
    fn confirmation_lists_what_will_be_sent() {
        let confirmation = Confirmation {
            contest: "awc0186".into(),
            task: "awc0186_a".into(),
            task_title: "A - 例".into(),
            end_text: "2026-10-26 21:00:00+09:00".into(),
            file: "src/bin/a.rs".into(),
            size: 2048,
            language: "Rust (rustc 1.89.0)".into(),
            user: Some("testuser".into()),
            last_test: "全 3 件中 AC 3 件（10/26 20:10）".into(),
            warnings: vec!["クレート「serde」は、ジャッジにありません".into()],
            judge_note: JUDGE_CRATES_NOTE,
        };
        let message = confirmation.message();
        for part in [
            "取り消せません",
            "awc0186_a「A - 例」",
            "終了 2026-10-26 21:00:00+09:00",
            "src/bin/a.rs（2.0 KiB）",
            "Rust (rustc 1.89.0)",
            "testuser",
            "全 3 件中 AC 3 件",
            "・クレート「serde」",
            "2025/10",
        ] {
            assert!(message.contains(part), "{part}\n{message}");
        }
    }

    #[test]
    fn submits_in_the_contest_period_and_tracks_the_judge() {
        let keychain = TestKeychain::new("submit-flow");
        keychain.0.save("old-session").unwrap();
        let (base, seen) = fake(contest_routes(|_| {
            redirect("/contests/x/submissions/me", Some("rotated-session"))
        }));
        let remote = remote(&keychain, &base);
        let mut asked = None;
        let users = Mutex::new(Vec::new());
        let outcome = submit_flow(
            &remote,
            &prepared(),
            "x",
            "x_a",
            7,
            "まだ".to_string(),
            |confirmation| {
                asked = Some(confirmation.clone());
                true
            },
            |user| users.lock().unwrap().push(user),
        );
        let SubmitOutcome::Sent { submission, .. } = outcome else {
            panic!("{outcome:?}");
        };
        assert_eq!(submission.key, 7);
        assert_eq!(submission.submission_id, Some(123));
        assert_eq!(submission.state, SubmissionState::Waiting);
        assert!(submission.tracking);
        assert_eq!(submission.task_title, "A - ADjacent");
        assert_eq!(
            submission.detail_url.as_deref(),
            Some("https://atcoder.jp/contests/x/submissions/123")
        );
        let asked = asked.unwrap();
        assert_eq!(asked.language, "Rust (rustc 1.89.0)");
        assert_eq!(asked.user.as_deref(), Some("testuser"));
        assert_eq!(asked.size, prepared().source.len());
        assert_eq!(*users.lock().unwrap(), vec![Some("testuser".to_string())]);
        // 送った中身。ソースは、改行・引用符・日本語もそのまま届く
        let requests = seen.lock().unwrap().clone();
        let post = requests.iter().find(|r| r.method == "POST").unwrap();
        assert_eq!(
            form_fields(&post.body),
            vec![
                ("data.TaskScreenName".to_string(), "x_a".to_string()),
                ("data.LanguageId".to_string(), "6088".to_string()),
                ("sourceCode".to_string(), prepared().source),
                ("csrf_token".to_string(), "test+csrf/token=".to_string()),
            ]
        );
        assert!(post.head.contains("Cookie: REVEL_SESSION=old-session"));
        // コンテストのページはログインなしで読む
        assert!(!requests[0].head.to_ascii_lowercase().contains("cookie"));
        assert_eq!(requests[0].target, "/contests/x");
        // サーバーが入れ替えた Cookie に置き換わり、そのあとの要求で使う
        assert_eq!(
            keychain.0.load().unwrap().as_deref(),
            Some("rotated-session")
        );
        let me = requests
            .iter()
            .find(|r| r.target == "/contests/x/submissions/me")
            .unwrap();
        assert!(me.head.contains("Cookie: REVEL_SESSION=rotated-session"));
        // 判定を追う：判定中 → 確定
        let mut updates = Vec::new();
        track(&remote, "x", 123, true, &mut |update| updates.push(update));
        assert_eq!(updates.len(), 2, "{updates:?}");
        let mut view = submission.clone();
        apply(&mut view, updates[0].clone());
        assert_eq!(view.state, SubmissionState::Judging);
        assert!(view.tracking);
        apply(&mut view, updates[1].clone());
        assert_eq!(view.state, SubmissionState::Done);
        assert!(!view.tracking);
        let judge = view.judge.unwrap();
        assert_eq!(
            (judge.label.as_str(), judge.score.as_deref()),
            ("AC", Some("400"))
        );
        assert_eq!((judge.time_ms, judge.memory_kib), (Some(30), Some(4000)));
    }

    #[test]
    fn sends_nothing_when_cancelled_or_out_of_period() {
        let keychain = TestKeychain::new("submit-cancel");
        keychain.0.save("session").unwrap();
        let (base, seen) = fake(contest_routes(|_| {
            redirect("/contests/x/submissions/me", None)
        }));
        let remote = remote(&keychain, &base);
        let outcome = submit_flow(
            &remote,
            &prepared(),
            "x",
            "x_a",
            1,
            String::new(),
            |_| false,
            |_| {},
        );
        assert!(matches!(outcome, SubmitOutcome::Cancelled), "{outcome:?}");
        assert!(seen.lock().unwrap().iter().all(|r| r.method != "POST"));

        // 終わったコンテスト：フォームも読まない
        let (base, seen) = fake(Arc::new(|request: &Request, _| {
            let now = now_secs();
            match request.target.as_str() {
                "/contests/x" => ok(&top_page(now - 7200, now - 3600)),
                _ => not_found(),
            }
        }));
        let remote = Remote { base, ..remote };
        let outcome = submit_flow(
            &remote,
            &prepared(),
            "x",
            "x_a",
            1,
            String::new(),
            |_| true,
            |_| {},
        );
        let SubmitOutcome::OutOfPeriod {
            end,
            file,
            warnings,
            submit_url,
            ..
        } = outcome
        else {
            panic!("{outcome:?}");
        };
        assert!(end.ends_with("+09:00"));
        assert_eq!(file, "src/bin/a.rs");
        assert_eq!(warnings, vec!["注意の例".to_string()]);
        assert_eq!(
            submit_url,
            "https://atcoder.jp/contests/x/submit?taskScreenName=x_a"
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn refuses_without_login_or_with_turnstile() {
        // ログインしていない（Keychain に値がない）：フォームを読まない
        let keychain = TestKeychain::new("submit-nologin");
        let (base, seen) = fake(contest_routes(|_| {
            redirect("/contests/x/submissions/me", None)
        }));
        let outcome = submit_flow(
            &remote(&keychain, &base),
            &prepared(),
            "x",
            "x_a",
            1,
            String::new(),
            |_| true,
            |_| {},
        );
        assert!(
            matches!(outcome, SubmitOutcome::NotLoggedIn { .. }),
            "{outcome:?}"
        );
        assert_eq!(seen.lock().unwrap().len(), 1);

        // フォームを読んだら /login へ移された（切れた）：Keychain の値を消して送らない
        keychain.0.save("expired").unwrap();
        let (base, _) = fake(Arc::new(|request: &Request, _| {
            let now = now_secs();
            match request.target.as_str() {
                "/contests/x" => ok(&top_page(now - 60, now + 60)),
                _ => redirect("/login?continue=x", None),
            }
        }));
        let outcome = submit_flow(
            &remote(&keychain, &base),
            &prepared(),
            "x",
            "x_a",
            1,
            String::new(),
            |_| true,
            |_| {},
        );
        assert!(
            matches!(outcome, SubmitOutcome::NotLoggedIn { .. }),
            "{outcome:?}"
        );
        assert!(!keychain.0.exists().unwrap());

        // 期間中でも、フォームに Turnstile があれば送らない
        keychain.0.save("session").unwrap();
        let (base, seen) = fake(Arc::new(|request: &Request, _| {
            let now = now_secs();
            match request.target.as_str() {
                "/contests/abc480" => ok(&top_page(now - 60, now + 60)),
                _ => ok(SUBMIT_PAGE),
            }
        }));
        let outcome = submit_flow(
            &remote(&keychain, &base),
            &prepared(),
            "abc480",
            "abc480_a",
            1,
            String::new(),
            |_| true,
            |_| {},
        );
        let SubmitOutcome::Refused {
            reason, submit_url, ..
        } = outcome
        else {
            panic!("{outcome:?}");
        };
        assert!(reason.contains("Turnstile"));
        assert!(submit_url.ends_with("taskScreenName=abc480_a"));
        assert!(seen.lock().unwrap().iter().all(|r| r.method != "POST"));
    }

    #[test]
    fn reports_a_rejected_submission() {
        let keychain = TestKeychain::new("submit-rejected");
        keychain.0.save("session").unwrap();
        // 提出の一覧ではなく、提出のページへ戻された：そのページの知らせを理由にする
        let (base, seen) = fake(Arc::new(|request: &Request, nth: usize| {
            let now = now_secs();
            match (request.method.as_str(), request.target.as_str()) {
                ("GET", "/contests/x") => ok(&top_page(now - 60, now + 60)),
                ("GET", "/contests/x/submit?taskScreenName=x_a") if nth == 0 => ok(&open_form("x")),
                ("GET", "/contests/x/submit?taskScreenName=x_a") => ok(
                    "<div class=\"alert alert-danger alert-dismissible\"><button>×</button>同じコードを短い間隔で提出することはできません</div>",
                ),
                ("POST", "/contests/x/submit") => {
                    redirect("/contests/x/submit?taskScreenName=x_a", None)
                }
                _ => not_found(),
            }
        }));
        let outcome = submit_flow(
            &remote(&keychain, &base),
            &prepared(),
            "x",
            "x_a",
            1,
            String::new(),
            |_| true,
            |_| {},
        );
        let SubmitOutcome::Sent { submission, .. } = outcome else {
            panic!("{outcome:?}");
        };
        assert_eq!(submission.state, SubmissionState::Rejected);
        assert_eq!(submission.submission_id, None);
        assert_eq!(
            submission.message.as_deref(),
            Some("提出が受け付けられませんでした：同じコードを短い間隔で提出することはできません")
        );
        // 自動では送り直さない
        assert_eq!(
            seen.lock()
                .unwrap()
                .iter()
                .filter(|r| r.method == "POST")
                .count(),
            1
        );
    }

    #[test]
    fn stops_tracking_when_the_status_cannot_be_read() {
        let keychain = TestKeychain::new("submit-unreadable");
        keychain.0.save("session").unwrap();
        let (base, _) = fake(Arc::new(|_: &Request, _| {
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 15\r\nConnection: close\r\n\r\nRequest blocked".to_string()
        }));
        let mut updates = Vec::new();
        track(&remote(&keychain, &base), "x", 123, false, &mut |update| {
            updates.push(update)
        });
        assert_eq!(
            updates,
            vec![Update::Unreadable(
                "判定の状態を読めません（HTTP 403）".to_string()
            )]
        );

        // 判定中のまま上限の時間を過ぎたら止める
        let (base, seen) = fake(Arc::new(|_: &Request, _| ok(STATUS_JUDGING)));
        let remote = Remote {
            cap: Duration::from_millis(30),
            min_interval: Duration::from_millis(10),
            ..remote(&keychain, &base)
        };
        let mut updates = Vec::new();
        track(&remote, "x", 123, false, &mut |update| updates.push(update));
        let Some(Update::Judge {
            tracking: false,
            message: Some(message),
            ..
        }) = updates.last()
        else {
            panic!("{updates:?}");
        };
        assert!(message.contains("「更新」"));
        assert!(seen.lock().unwrap().len() <= 4);
    }

    #[test]
    fn prepares_the_bin_source_with_warnings() {
        let root =
            std::env::temp_dir().join(format!("atcoder-studio-submit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pkg/src/bin")).unwrap();
        std::fs::create_dir_all(root.join("mylib/src")).unwrap();
        std::fs::write(
            root.join("mylib/Cargo.toml"),
            "[package]\nname = \"mylib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(root.join("mylib/src/lib.rs"), "").unwrap();
        std::fs::write(
            root.join("pkg/Cargo.toml"),
            "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nproconio = \"0.4\"\nmylib = { path = \"../mylib\" }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("pkg/Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"proconio\"\nversion = \"0.4.5\"\n",
        )
        .unwrap();
        std::fs::write(root.join("pkg/src/bin/a.rs"), "mod helper;\nfn main() {}\n").unwrap();
        std::fs::write(root.join("pkg/src/bin/b.rs"), "fn main() {}\n").unwrap();
        let root = root.canonicalize().unwrap();
        let dir = root.join("pkg");
        let cargo = crate::toolchain::find_cargo().unwrap();
        let metadata = crate::toolchain::metadata_json(&cargo, &dir).unwrap();
        let prepared = prepare(&root, &dir, &metadata, "a", "abc480_b").unwrap();
        assert_eq!(prepared.file, "pkg/src/bin/a.rs");
        assert_eq!(prepared.source, "mod helper;\nfn main() {}\n");
        let warnings = prepared.warnings.join("\n");
        assert_eq!(prepared.warnings.len(), 4, "{warnings}");
        assert!(warnings.contains("mod helper;"));
        assert!(warnings.contains("proconio") && warnings.contains("0.4.5"));
        assert!(warnings.contains("mylib"));
        assert!(warnings.contains("bin「a」"));
        // 上限を超えるコードは送らない
        std::fs::write(dir.join("src/bin/b.rs"), "/".repeat(SOURCE_LIMIT + 1)).unwrap();
        assert!(
            prepare(&root, &dir, &metadata, "b", "abc480_b")
                .unwrap_err()
                .contains("512 KiB")
        );
        assert!(prepare(&root, &dir, &metadata, "c", "abc480_c").is_err());
        // ソースが作業領域の外にある（作業領域を mylib にしたとき）
        assert!(
            prepare(&root.join("mylib"), &dir, &metadata, "a", "abc480_a")
                .unwrap_err()
                .contains("作業領域の外")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sample_gate_needs_a_full_passing_test_on_the_current_source() {
        let target = Target {
            package: "abc480_a".to_string(),
            bin: "abc480_a".to_string(),
            language: crate::language::Language::Rust,
        };
        let tested =
            |time_ms: u64, kind: history::Kind, total, accepted, failed, unjudged, interrupted| {
                history::EntryView {
                    time_ms,
                    package: "abc480_a".to_string(),
                    bin: "abc480_a".to_string(),
                    kind,
                    finish: Finish::Tested {
                        total,
                        accepted,
                        failed,
                        unjudged,
                        interrupted,
                        max_elapsed_ms: None,
                    },
                }
            };
        let pass = tested(1000, history::Kind::Test, 3, 3, 0, 0, false);
        let gate = |entries: &[history::EntryView], modified: Option<u64>| {
            sample_gate(entries, &target, modified, true)
        };
        // 全ケース AC、そのあとソースが変わっていない
        assert_eq!(
            gate(std::slice::from_ref(&pass), Some(1000)),
            SampleGate::Passed { total: 3 }
        );
        // ケースがない（対話の問題など）は、確かめられない（ダイアログを出す）
        assert_eq!(
            sample_gate(std::slice::from_ref(&pass), &target, Some(900), false),
            SampleGate::Unconfirmed
        );
        // 目視確認のケースがある
        assert_eq!(
            gate(
                &[tested(1000, history::Kind::Test, 3, 2, 0, 1, false)],
                Some(900)
            ),
            SampleGate::Unconfirmed
        );
        // 通っていないもの
        let failed = |gate: SampleGate| matches!(gate, SampleGate::Failed { .. });
        assert!(failed(gate(&[], Some(900))));
        assert!(failed(gate(std::slice::from_ref(&pass), Some(1001))));
        assert!(failed(gate(std::slice::from_ref(&pass), None)));
        assert!(failed(gate(
            &[tested(1000, history::Kind::Test, 3, 2, 1, 0, false)],
            Some(900)
        )));
        assert!(failed(gate(
            &[tested(1000, history::Kind::Test, 3, 3, 0, 0, true)],
            Some(900)
        )));
        assert!(failed(gate(
            &[tested(1000, history::Kind::Test, 0, 0, 0, 0, false)],
            Some(900)
        )));
        // 新しい順。最後が一部のケースだけなら、前の全ケースの AC は使わない
        let partial = tested(2000, history::Kind::PartialTest, 1, 1, 0, 0, false);
        assert!(failed(gate(&[partial, pass.clone()], Some(900))));
        // ビルドに失敗した
        let compile = history::EntryView {
            finish: Finish::CompileError,
            ..pass.clone()
        };
        assert!(failed(gate(&[compile], Some(900))));
        // ほかのバイナリ・パッケージの結果は使わない
        let other = history::EntryView {
            bin: "other".to_string(),
            ..pass.clone()
        };
        assert!(failed(gate(&[other], Some(900))));
        // 実行・提出の記録は飛ばして、最後のサンプルテストを見る
        let run = history::EntryView {
            kind: history::Kind::Run,
            finish: Finish::CompileError,
            time_ms: 3000,
            ..pass.clone()
        };
        assert_eq!(
            gate(&[run, pass], Some(1000)),
            SampleGate::Passed { total: 3 }
        );
    }

    #[test]
    fn skips_the_dialog_only_when_passed_without_warnings() {
        let off = SubmitSettings {
            test_before_submit: true,
            confirm_dialog: false,
        };
        let passed = SampleGate::Passed { total: 2 };
        assert_eq!(dialog_skip(&passed, &off, &[]), Some(2));
        // 注意があるとき、設定がダイアログを出すとき、通っていないときは省かない
        assert_eq!(
            dialog_skip(
                &passed,
                &off,
                &["別の問題のファイルかもしれません".to_string()]
            ),
            None
        );
        assert_eq!(dialog_skip(&passed, &SubmitSettings::default(), &[]), None);
        assert_eq!(dialog_skip(&SampleGate::Unconfirmed, &off, &[]), None);
        let failed = SampleGate::Failed {
            reason: "x".to_string(),
        };
        assert_eq!(dialog_skip(&failed, &off, &[]), None);
    }

    // ---- コンテストの提出状況（Phase 18 作業 4。D-218） ----

    // 「自分の提出」の 1 ページ（行は（ID、問題、判定）。next があれば、そのページへのリンクを置く）
    fn status_page(rows: &[(u64, &str, &str)], next: Option<u32>) -> String {
        let rows: String = rows
            .iter()
            .map(|(id, task, verdict)| {
                format!(
                    "<tr><td><a href=\"/contests/x/tasks/{task}\">A - あ</a></td><td class=\"text-right submission-score\" data-id=\"{id}\">0</td><td class='text-center'><span class='label label-success' title=\"t\">{verdict}</span></td><td><a href=\"/contests/x/submissions/{id}\">Detail</a></td></tr>"
                )
            })
            .collect();
        let pager = next
            .map(|page| format!("<ul class=\"pagination\"><li><a href='/contests/x/submissions/me?page={page}'>{page}</a></li></ul>"))
            .unwrap_or_default();
        format!("<table><tbody>{rows}</tbody></table>{pager}")
    }

    fn task_entries(tasks: &[(&str, &str)]) -> Vec<crate::problem::TaskEntry> {
        tasks
            .iter()
            .map(|(task, label)| crate::problem::TaskEntry {
                task: task.to_string(),
                label: label.to_string(),
                title: format!("{label} の問題"),
            })
            .collect()
    }

    // 問題の一覧を覚えてあるアプリのフォルダ（読みに行かない）
    fn status_app_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "atcoder-studio-status-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("task-lists")).unwrap();
        std::fs::write(
            dir.join("task-lists/x.json"),
            serde_json::to_vec(&task_entries(&[("x_a", "A"), ("x_b", "B"), ("x_c", "C")])).unwrap(),
        )
        .unwrap();
        dir
    }

    #[test]
    fn reads_rows_and_the_next_page_link() {
        // 本物のページを短くした見本：問題・提出の ID・判定
        let (rows, has_next) = parse_mine_page(MINE_PAGE, "abc480", 1);
        assert_eq!(
            rows,
            vec![
                MineRow {
                    id: 10000003,
                    task: "abc480_d".to_string(),
                    verdict: "AC".to_string()
                },
                MineRow {
                    id: 10000002,
                    task: "abc480_c".to_string(),
                    verdict: "AC".to_string()
                },
                MineRow {
                    id: 10000001,
                    task: "abc480_c".to_string(),
                    verdict: "WA".to_string()
                },
            ]
        );
        // 並べ替えのリンク（orderBy）は次のページではない
        assert!(!has_next);
        // 判定中の行（waiting-judge。得点の欄がない）は、詳細のリンクの ID と、判定の欄の文字
        let judging = "<table><tbody><tr><td><a href=\"/contests/x/tasks/x_a\">A</a></td><td class=\"text-center waiting-judge\" data-id=\"7\" colspan=\"3\"><span class='label label-default'>3/15 WA</span></td><td><a href=\"/contests/x/submissions/7\">Detail</a></td></tr>\n<tr><td><a href=\"/contests/y/tasks/y_a\">A</a></td><td class=\"submission-score\" data-id=\"8\">0</td></tr></tbody></table>";
        let (rows, _) = parse_mine_page(judging, "x", 1);
        assert_eq!(
            rows,
            vec![MineRow {
                id: 7,
                task: "x_a".to_string(),
                verdict: "3/15 WA".to_string()
            }]
        );
        // ページ送り：次のページへのリンク（ほかのクエリが付いていてもよい）
        let pager = "<ul class=\"pagination\"><li><a href='/contests/x/submissions/me?f.Task=&amp;page=3'>3</a></li><li><a href='/contests/y/submissions/me?page=2'>2</a></li></ul>";
        assert!(!parse_mine_page(pager, "x", 1).1);
        assert!(parse_mine_page(pager, "x", 2).1);
    }

    #[test]
    fn summarizes_submissions_by_task() {
        let rows = [
            MineRow {
                id: 5,
                task: "x_b".to_string(),
                verdict: "WA".to_string(),
            },
            MineRow {
                id: 3,
                task: "x_b".to_string(),
                verdict: "AC".to_string(),
            },
            MineRow {
                id: 4,
                task: "x_a".to_string(),
                verdict: "TLE".to_string(),
            },
            MineRow {
                id: 9,
                task: "x_z".to_string(),
                verdict: "WJ".to_string(),
            },
        ];
        let tasks = task_entries(&[("x_a", "A"), ("x_b", "B"), ("x_c", "C")]);
        let list = summarize(&tasks, &rows);
        let brief: Vec<_> = list
            .iter()
            .map(|t| {
                (
                    t.task.as_str(),
                    t.label.as_str(),
                    t.accepted,
                    t.last.as_deref(),
                    t.last_id,
                    t.count,
                )
            })
            .collect();
        assert_eq!(
            brief,
            vec![
                ("x_a", "A", false, Some("TLE"), Some(4), 1),
                // AC のあとに WA を出しても AC のまま。最後の判定は新しいほう
                ("x_b", "B", true, Some("WA"), Some(5), 2),
                ("x_c", "C", false, None, None, 0),
                // 一覧にない問題は後ろに足す
                ("x_z", "", false, Some("WJ"), Some(9), 1),
            ]
        );
    }

    #[test]
    fn reads_every_page_of_the_contest() {
        let dir = status_app_dir("pages");
        let keychain = TestKeychain::new("status-pages");
        keychain.0.save("session").unwrap();
        // 2 ページ目の先頭は、読む間に増えた提出で 1 ページ目と重なる
        let (base, seen) = fake(Arc::new(|request: &Request, _| {
            match request.target.as_str() {
                "/contests/x/submissions/me?page=1" => {
                    ok(&status_page(&[(6, "x_a", "WA"), (5, "x_b", "AC")], Some(2)))
                }
                "/contests/x/submissions/me?page=2" => {
                    ok(&status_page(&[(5, "x_b", "AC"), (2, "x_a", "AC")], None))
                }
                _ => not_found(),
            }
        }));
        let StatusOutcome::Read { status } = contest_status(&remote(&keychain, &base), &dir, "x")
        else {
            panic!("読めるはず");
        };
        assert_eq!((status.pages, status.truncated), (2, false));
        let a = &status.tasks[0];
        assert_eq!(
            (a.accepted, a.last.as_deref(), a.count),
            (true, Some("WA"), 2)
        );
        assert_eq!(status.tasks[1].count, 1);
        assert_eq!(status.tasks[2].count, 0);
        // セッションを付けて、コンテストの「自分の提出」だけを読む
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(
            seen.iter()
                .all(|r| r.method == "GET"
                    && r.target.starts_with("/contests/x/submissions/me?page="))
        );
        assert!(
            seen.iter()
                .all(|r| r.head.contains("Cookie: REVEL_SESSION=session"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stops_at_the_page_limit() {
        let dir = status_app_dir("limit");
        let keychain = TestKeychain::new("status-limit");
        keychain.0.save("session").unwrap();
        // どのページにも次のページがある
        let (base, seen) = fake(Arc::new(|request: &Request, _| {
            let page: u64 = request.target.rsplit('=').next().unwrap().parse().unwrap();
            ok(&status_page(
                &[(1000 - page, "x_c", "WA")],
                Some(page as u32 + 1),
            ))
        }));
        let StatusOutcome::Read { status } = contest_status(&remote(&keychain, &base), &dir, "x")
        else {
            panic!("読めるはず");
        };
        assert_eq!((status.pages, status.truncated), (MAX_STATUS_PAGES, true));
        assert_eq!(status.tasks[2].count, MAX_STATUS_PAGES);
        assert_eq!(seen.lock().unwrap().len(), MAX_STATUS_PAGES as usize);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reports_login_and_failures() {
        let dir = status_app_dir("fail");
        // Keychain にない：読みに行かない
        let keychain = TestKeychain::new("status-nologin");
        let (base, seen) = fake(Arc::new(|_: &Request, _| {
            redirect("/login?continue=x", None)
        }));
        assert_eq!(
            contest_status(&remote(&keychain, &base), &dir, "x"),
            StatusOutcome::NotLoggedIn
        );
        assert!(seen.lock().unwrap().is_empty());
        // ログインのページへ移された
        keychain.0.save("expired").unwrap();
        assert_eq!(
            contest_status(&remote(&keychain, &base), &dir, "x"),
            StatusOutcome::NotLoggedIn
        );
        // 読めない（404）
        let keychain = TestKeychain::new("status-404");
        keychain.0.save("session").unwrap();
        let (base, _) = fake(Arc::new(|_: &Request, _| not_found()));
        let StatusOutcome::Failed { message } =
            contest_status(&remote(&keychain, &base), &dir, "x")
        else {
            panic!("失敗するはず");
        };
        assert!(message.contains("HTTP 404"), "{message}");
        // コンテスト ID の形が違えば、読みに行かない
        let (base, seen) = fake(Arc::new(|_: &Request, _| not_found()));
        assert!(matches!(
            contest_status(&remote(&keychain, &base), &dir, "../x"),
            StatusOutcome::Failed { .. }
        ));
        assert!(seen.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
