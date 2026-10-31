// 問題ページの取得・読み取り・保存（Phase 3a 作業 1。D-110〜D-112）
//
// - 公開されている問題ページ（ログインなし）を、macOS の curl コマンドで取得する（P3a-02。依存を増やさない）。
//   開催中のコンテストの問題は、ログインしていないと 404 になるので、そのときだけ、ログインしていればセッションを付けて読み直す（D-149）
// - 取得は利用者の操作のときだけ。続けて取得するときは 1 秒以上あける（D-111）
// - 取得したものはアプリのフォルダに保存し、2 回目からは保存したものを使う（D-111・D-112）
// - 問題名・制限・問題文（日本語）・サンプルを、ページの作り（docs/phase3a/01-pre-start-research.md §2）から読み取る。
//   読み取れなければ理由を返し、画面は公式ページを開く手段を示す（D-110）
//
// 問題文の HTML の無害化は作業 2（D-113）で行う。ここでは取り出すだけ

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dom_query::Document;
use serde::{Deserialize, Serialize};

use crate::session::{Client, NOT_LOGGED_IN};
use crate::storage;

// 続けて取得するときの間隔（D-111）
const MIN_INTERVAL: Duration = Duration::from_secs(1);
// 1 回の取得の時間の上限と、大きさの上限（問題ページは数十 KB ほど。推測）
const FETCH_TIMEOUT_SECS: &str = "20";
const MAX_PAGE_BYTES: &str = "5000000";

// 問題の ID（例：コンテスト abc430、問題 abc430_a）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProblemId {
    pub contest: String,
    pub task: String,
}

impl ProblemId {
    // URL やファイル名に使うので、英小文字・数字・`_`・`-` だけを許す
    pub fn new(contest: &str, task: &str) -> Result<Self, String> {
        let valid = |id: &str| {
            !id.is_empty()
                && id.len() <= 64
                && id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        };
        if !valid(contest) || !valid(task) {
            return Err("コンテスト・問題の ID が正しくありません".to_string());
        }
        Ok(ProblemId {
            contest: contest.to_string(),
            task: task.to_string(),
        })
    }

    // 公式の問題ページ（取得できないときに、利用者が開く先でもある）
    pub fn url(&self) -> String {
        format!(
            "https://atcoder.jp/contests/{}/tasks/{}",
            self.contest, self.task
        )
    }
}

// 読み取った問題
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub title: String,
    // 例：「2 sec」「1024 MiB」（ページの表記のまま）
    pub time_limit: String,
    pub memory_limit: String,
    // 問題文（日本語）の HTML。無害化の前（作業 2 で無害化する）
    pub statement_html: String,
    // サンプル（入力・期待出力）
    pub samples: Vec<Sample>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub input: String,
    pub output: String,
}

// 前回の取得の時刻（アプリ全体で 1 つ）
static LAST_FETCH: Mutex<Option<Instant>> = Mutex::new(None);

// 前回の取得から MIN_INTERVAL たつまで待つ
fn wait_for_interval() {
    let mut last = LAST_FETCH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(at) = *last {
        let since = at.elapsed();
        if since < MIN_INTERVAL {
            std::thread::sleep(MIN_INTERVAL - since);
        }
    }
    *last = Some(Instant::now());
}

// ログインなしで読んだときの失敗
#[derive(Debug, PartialEq, Eq)]
enum FetchError {
    // 404（開催中のコンテストの問題は、ログインしていないとこうなる。2026-10-24 に ABC481 の開催中に確かめた）
    NotFound,
    Other(String),
}

// curl の --fail の失敗のうち、404 か。macOS の curl は、このとき終了コード 56 で
// 「curl: (56) The requested URL returned error: 404」を出す（2026-10-24 に確かめた）ので、終了コードではなく文で見る
fn is_not_found(stderr: &str) -> bool {
    stderr.contains("returned error: 404")
}

const NOT_FOUND_LOGGED_OUT: &str = "ページが見つかりません（404）。開催中のコンテストの問題は、AtCoder にログインしていないと読めません。「提出結果」タブの「ログイン」からログインしてください";

// 公開されているページを取得する。ログインなしで 404 なら、ログインしているときだけセッションを付けてもう 1 回読む（D-149）
fn fetch(url: &str) -> Result<String, String> {
    with_session_fallback(url, fetch_public(url), &Client::app())
}

fn with_session_fallback(
    url: &str,
    first: Result<String, FetchError>,
    client: &Client,
) -> Result<String, String> {
    match first {
        Ok(body) => Ok(body),
        Err(FetchError::Other(error)) => Err(error),
        Err(FetchError::NotFound) => {
            if !client.keychain().exists()? {
                return Err(NOT_FOUND_LOGGED_OUT.to_string());
            }
            wait_for_interval();
            let reply = client.request(url, &[]).map_err(|error| {
                if error == NOT_LOGGED_IN {
                    // ログインが切れていた（Keychain の値は消えている）
                    NOT_FOUND_LOGGED_OUT.to_string()
                } else {
                    format!("ページを取得できません：{error}")
                }
            })?;
            if reply.status == 200 {
                Ok(reply.body)
            } else {
                Err(format!(
                    "ページを取得できません（ログインして読んでも HTTP {}）",
                    reply.status
                ))
            }
        }
    }
}

// 公開されているページを、ログインなしで curl で取得する。https だけを許し、リダイレクトの先も https に限る
fn fetch_public(url: &str) -> Result<String, FetchError> {
    wait_for_interval();
    let output = Command::new("/usr/bin/curl")
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-time",
            FETCH_TIMEOUT_SECS,
            "--max-filesize",
            MAX_PAGE_BYTES,
            url,
        ])
        .output()
        .map_err(|error| FetchError::Other(format!("curl を起動できません：{error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if is_not_found(&stderr) {
            return Err(FetchError::NotFound);
        }
        // 問題の一覧のページにも使うので、「問題ページ」とは書かない
        return Err(FetchError::Other(crate::session::curl_failure(
            "AtCoder のページを取得できません",
            output.status.code(),
            &stderr,
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| FetchError::Other("問題ページを読めません（UTF-8 ではない）".to_string()))
}

// 空白をまとめて前後を切る
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// 問題ページの HTML から問題を読み取る（ページの作りは 01 §2）
pub fn parse_problem(html: &str) -> Result<Problem, String> {
    let document = Document::from(html);

    // 問題名：span.h2 の、リンク（「解説」）を除いた文字
    let heading = document.select("span.h2");
    if !heading.exists() {
        return Err("問題名が見つかりません（ページの作りが変わった可能性があります）".to_string());
    }
    let link_text = heading.select("a").text().to_string();
    let title = squash(heading.text().as_ref().replacen(&link_text, "", 1).as_str());

    // 制限：「実行時間制限: 2 sec / メモリ制限: 1024 MiB」の 1 行。
    // 英語の画面では「Time Limit: 2 sec / Memory Limit: 1024 MiB」になる
    let limits = document
        .select("p")
        .iter()
        .map(|p| squash(p.text().as_ref()))
        .find(|text| text.starts_with("実行時間制限:") || text.starts_with("Time Limit:"))
        .ok_or_else(|| {
            "実行時間制限が見つかりません（ページの作りが変わった可能性があります）".to_string()
        })?;
    let (time_part, memory_part) = limits
        .split_once(" / ")
        .ok_or_else(|| "制限の行を読めません".to_string())?;
    let value = |part: &str| {
        part.split_once(':')
            .map(|(_, value)| value.trim().to_string())
            .unwrap_or_default()
    };
    let time_limit = value(time_part);
    let memory_limit = value(memory_part);

    // 問題文（日本語）
    let statement = document.select("#task-statement .lang-ja");
    if !statement.exists() {
        return Err(
            "問題文（日本語）が見つかりません（ページの作りが変わった可能性があります）"
                .to_string(),
        );
    }
    let statement_html = statement.inner_html().to_string();

    // サンプル：見出し（h3）が「入力例 N」「出力例 N」の区画の、最初の pre。番号で入力と出力を組にする。
    // 取得したページの pre には id がない（ブラウザで開いたあとの pre-sample0… の id は、ページのスクリプトが付けたものと推測。
    // 2026-10-25 に、保存した問題で確かめた）ので、id では探さない
    let mut inputs: BTreeMap<u32, String> = BTreeMap::new();
    let mut outputs: BTreeMap<u32, String> = BTreeMap::new();
    for section in statement.select("section").iter() {
        let heading = squash(section.select("h3").first().text().as_ref());
        let (found, rest) = if let Some(rest) = heading.strip_prefix("入力例") {
            (&mut inputs, rest)
        } else if let Some(rest) = heading.strip_prefix("出力例") {
            (&mut outputs, rest)
        } else {
            continue;
        };
        let Ok(number) = rest.trim().parse::<u32>() else {
            continue;
        };
        let pre = section.select("pre").first();
        if !pre.exists() {
            return Err(format!("「{heading}」の中身が見つかりません"));
        }
        found
            .entry(number)
            .or_insert_with(|| pre.text().to_string());
    }
    if !inputs.keys().eq(outputs.keys()) {
        return Err("サンプルの入力と出力の数が合いません".to_string());
    }
    let with_newline = |text: &str| {
        let mut text = text.replace("\r\n", "\n");
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text
    };
    let samples = inputs
        .values()
        .zip(outputs.values())
        .map(|(input, output)| Sample {
            input: with_newline(input),
            output: with_newline(output),
        })
        .collect();

    Ok(Problem {
        title,
        time_limit,
        memory_limit,
        statement_html,
        samples,
    })
}

// 出力の誤差の範囲（Phase 9 作業 1。D-161）。abs・rel は、問題文が絶対誤差・相対誤差のどちらを書いているか
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tolerance {
    pub abs: bool,
    pub rel: bool,
    // 10^{-k} の k
    pub exponent: u32,
}

// 問題文（日本語）の「出力」の区画に「誤差」と 10^{-k} があれば、その範囲。読めなければ None。
// 「絶対誤差」「相対誤差」のどちらも書いていなければ、両方とする【推測】
pub fn output_tolerance(statement_html: &str) -> Option<Tolerance> {
    let document = Document::from(statement_html);
    let section = document
        .select("section")
        .iter()
        .find(|section| squash(section.select("h3").first().text().as_ref()) == "出力")?;
    let text = squash(section.text().as_ref());
    if !text.contains("誤差") {
        return None;
    }
    let exponent = power_of_ten(&text)?;
    let (abs, rel) = (text.contains("絶対誤差"), text.contains("相対誤差"));
    let (abs, rel) = if abs || rel { (abs, rel) } else { (true, true) };
    Some(Tolerance { abs, rel, exponent })
}

// 答えが複数ありうる問題か（Phase 10 作業 5。D-168）。問題文（日本語）に「どれを出力しても」「いずれか 1 つを出力」
// 「出力しても正解（正答）」などの書き方があれば true。空白を除いてから探す【推測】書き方が違う問題は読めない
pub fn multiple_answers(statement_html: &str) -> bool {
    const PHRASES: [&str; 6] = [
        "どれを出力しても",
        "いずれを出力しても",
        "いずれか1つを出力",
        "いずれか一つを出力",
        "出力しても正解",
        "出力しても正答",
    ];
    let text: String = Document::from(statement_html)
        .select("body")
        .text()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    PHRASES.iter().any(|phrase| text.contains(phrase))
}

// 「10^{-6}」「10^{−6}」（U+2212）「10^-6」の 6。最初に読めたもの
fn power_of_ten(text: &str) -> Option<u32> {
    let mut rest = text;
    while let Some(index) = rest.find("10^") {
        rest = &rest[index + 3..];
        let after = rest.strip_prefix('{').unwrap_or(rest).trim_start();
        let Some(after) = after
            .strip_prefix('-')
            .or_else(|| after.strip_prefix('−'))
            .map(str::trim_start)
        else {
            continue;
        };
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(exponent) = digits.parse::<u32>()
            && (1..=18).contains(&exponent)
        {
            return Some(exponent);
        }
    }
    None
}

// 保存先：アプリのフォルダの problems/<コンテスト>/<問題>.json（D-112）
fn cache_path(app_dir: &Path, id: &ProblemId) -> PathBuf {
    app_dir
        .join("problems")
        .join(&id.contest)
        .join(format!("{}.json", id.task))
}

// 問題を返す。保存してあればそれを使い、refresh が true（「再取得」）か保存がなければ取得する（D-111）
pub fn load_problem(app_dir: &Path, id: &ProblemId, refresh: bool) -> Result<Problem, String> {
    let path = cache_path(app_dir, id);
    if !refresh
        && let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(problem) = serde_json::from_str::<Problem>(&text)
    {
        return Ok(problem);
    }
    // 画面の言語を日本語にして取得する（指定しないと英語の画面になり、制限の行が「Time Limit:」になる。推測）
    let problem = parse_problem(&fetch(&format!("{}?lang=ja", id.url()))?)?;
    let json =
        serde_json::to_vec_pretty(&problem).map_err(|_| "問題を保存できません".to_string())?;
    storage::write_atomic(&path, &json)?;
    Ok(problem)
}

// 保存してある問題だけを返す（取得しない。Phase 15 の input! を作るときに使う。D-194）
pub fn load_saved(app_dir: &Path, id: &ProblemId) -> Result<Problem, String> {
    let text = std::fs::read_to_string(cache_path(app_dir, id)).map_err(|_| {
        "問題がまだ読み込まれていません。問題文を表示してから作ってください".to_string()
    })?;
    serde_json::from_str(&text)
        .map_err(|_| "保存してある問題を読めません。「再取得」を押してください".to_string())
}

// サンプルを保存した結果（D-112）
#[derive(Debug, Default, PartialEq, Serialize)]
pub struct SavedSamples {
    // 新しく書いたファイル
    pub written: Vec<String>,
    // すでに同じ内容だったファイル
    pub unchanged: Vec<String>,
    // 内容が違うので書かなかったファイル（上書きしない）
    pub skipped: Vec<String>,
    // 問題のサンプルの数より大きい番号の sample-N（「再取得」でサンプルが減ったときなど）。消さずに知らせる
    // （Phase 10 作業 12。利用者が中身を変えているかもしれないので）
    pub leftover: Vec<String>,
}

// サンプルをフォルダに sample-N.in・sample-N.out として書く（N は 1 から）。
// すでにあるファイルは上書きしない。同じ内容なら unchanged、違えば skipped に入れる
pub fn save_samples(folder: &Path, samples: &[Sample]) -> Result<SavedSamples, String> {
    std::fs::create_dir_all(folder)
        .map_err(|_| "テストケースのフォルダを作れません".to_string())?;
    let mut saved = SavedSamples::default();
    for (index, sample) in samples.iter().enumerate() {
        for (extension, content) in [("in", &sample.input), ("out", &sample.output)] {
            let name = format!("sample-{}.{extension}", index + 1);
            let path = folder.join(&name);
            match std::fs::read(&path) {
                Ok(existing) if existing == content.as_bytes() => saved.unchanged.push(name),
                Ok(_) => saved.skipped.push(name),
                Err(_) => {
                    // 「なければ作る」を 1 回の操作で行う（確かめてから書くまでの間に作られても上書きしない）
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .map_err(|_| format!("{name} を書けません"))?;
                    std::io::Write::write_all(&mut file, content.as_bytes())
                        .map_err(|_| format!("{name} を書けません"))?;
                    saved.written.push(name);
                }
            }
        }
    }
    let mut leftover: Vec<(usize, String)> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let stem = name
                        .strip_suffix(".in")
                        .or_else(|| name.strip_suffix(".out"))?;
                    let number: usize = stem.strip_prefix("sample-")?.parse().ok()?;
                    (number > samples.len()).then_some((number, name))
                })
                .collect()
        })
        .unwrap_or_default();
    leftover.sort();
    saved.leftover = leftover.into_iter().map(|(_, name)| name).collect();
    Ok(saved)
}

// コンテストの問題の一覧の 1 行（選択欄に出す。D-117）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskEntry {
    // 問題 ID（例：abc475_a）
    pub task: String,
    // 例：「A」
    pub label: String,
    // 例：「mnclr」
    pub title: String,
}

// コンテストの問題の一覧のページ（`/contests/<ID>/tasks`）から、問題を読み取る。
// 表の各行の 1 列目のリンク（`/contests/<ID>/tasks/<問題 ID>`）と 2 列目の問題名を使う
pub fn parse_task_list(html: &str, contest: &str) -> Result<Vec<TaskEntry>, String> {
    let document = Document::from(html);
    let prefix = format!("/contests/{contest}/tasks/");
    let tasks: Vec<TaskEntry> = document
        .select("table tbody tr")
        .iter()
        .filter_map(|row| {
            let cells = row.select("td");
            let first = cells.first();
            let href = first.select("a").attr("href")?;
            let task = href.strip_prefix(prefix.as_str())?.to_string();
            ProblemId::new(contest, &task).ok()?;
            let title = row.select("td:nth-child(2)").text();
            Some(TaskEntry {
                task,
                label: squash(first.text().as_ref()),
                title: squash(title.as_ref()),
            })
        })
        .collect();
    if tasks.is_empty() {
        return Err(
            "問題の一覧が見つかりません（コンテスト ID を確かめてください。ページの作りが変わった可能性もあります）"
                .to_string(),
        );
    }
    Ok(tasks)
}

// 保存した問題の一覧の場所（contest は形を確かめてから渡す）
fn task_list_path(app_dir: &Path, contest: &str) -> PathBuf {
    app_dir.join("task-lists").join(format!("{contest}.json"))
}

// 実行の対象のパッケージ（作業領域のルートからの相対パス。例：joi2027yo1b/joi2027_yo1b_a）から問題を決める（D-155）。
// フォルダ名を問題 ID とし、コンテスト ID は次の順で決める。決まらなければ None（選択欄で選ぶ。D-117）
// 1. 親のフォルダ名のコンテストの、保存した問題の一覧に、その問題 ID があれば親のフォルダ名
//    （「コンテストを作る」「この問題を作る」で作ったもの。JOI の joi2027yo1b/joi2027_yo1b_a、ABC042 の abc042/arc058_a など）
// 2. 最後の `_` より前と親のフォルダ名を、`_`・`-` を除いて比べて同じなら親のフォルダ名（一覧を消したとき）
// 3. 最後の `_` より前（今までの決まり。problems/abc001_a → abc001）
pub fn package_problem(app_dir: &Path, package: &str) -> Option<ProblemId> {
    let mut parts = package.rsplit('/');
    let name = parts.next()?;
    let index = name.rfind('_');
    if let Some(parent) = parts.next()
        && let Ok(id) = ProblemId::new(parent, name)
    {
        let listed = std::fs::read_to_string(task_list_path(app_dir, parent))
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<TaskEntry>>(&text).ok())
            .is_some_and(|tasks| tasks.iter().any(|entry| entry.task == name));
        let squash = |text: &str| text.replace(['_', '-'], "");
        let same_name =
            index.is_some_and(|index| index > 0 && squash(&name[..index]) == squash(parent));
        if listed || same_name {
            return Some(id);
        }
    }
    let index = index.filter(|&index| index > 0 && index + 1 < name.len())?;
    ProblemId::new(&name[..index], name).ok()
}

// コンテストの問題の一覧を返す。保存してあればそれを使い、refresh のときか保存がなければ取得する（D-111・D-117）。
// 保存先はアプリのフォルダの task-lists/<コンテスト>.json
pub fn load_task_list(
    app_dir: &Path,
    contest: &str,
    refresh: bool,
) -> Result<Vec<TaskEntry>, String> {
    // ID の形を確かめる（問題 ID の部分は仮のもの）
    ProblemId::new(contest, "x")?;
    let path = task_list_path(app_dir, contest);
    if !refresh
        && let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(tasks) = serde_json::from_str::<Vec<TaskEntry>>(&text)
    {
        return Ok(tasks);
    }
    let url = format!("https://atcoder.jp/contests/{contest}/tasks?lang=ja");
    let tasks = parse_task_list(&fetch(&url)?, contest)?;
    let json =
        serde_json::to_vec_pretty(&tasks).map_err(|_| "問題の一覧を保存できません".to_string())?;
    storage::write_atomic(&path, &json)?;
    Ok(tasks)
}

// 公式の問題ページを、macOS の open コマンドで既定のブラウザで開く（D-110 の代替手段）。
// URL は ID を確かめてから IDE が組み立てたもの
pub fn open_in_browser(id: &ProblemId) -> Result<(), String> {
    let status = Command::new("/usr/bin/open")
        .arg(id.url())
        .status()
        .map_err(|error| format!("ブラウザを開けません：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("ブラウザを開けません".to_string())
    }
}

// パッケージごとに選んだ問題（D-117）。アプリのフォルダの problem-choices.json。
// キーはパッケージのディレクトリの絶対パス
fn choices_path(app_dir: &Path) -> PathBuf {
    app_dir.join("problem-choices.json")
}

fn read_choices(app_dir: &Path) -> BTreeMap<String, ProblemId> {
    std::fs::read_to_string(choices_path(app_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn choice_for(app_dir: &Path, package_dir: &Path) -> Option<ProblemId> {
    read_choices(app_dir).remove(package_dir.to_string_lossy().as_ref())
}

pub fn set_choice(app_dir: &Path, package_dir: &Path, id: ProblemId) -> Result<(), String> {
    let mut choices = read_choices(app_dir);
    choices.insert(package_dir.to_string_lossy().into_owned(), id);
    let json = serde_json::to_vec_pretty(&choices)
        .map_err(|_| "選んだ問題を保存できません".to_string())?;
    storage::write_atomic(&choices_path(app_dir), &json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tests::{TestKeychain, serve_once};

    #[test]
    fn recognizes_curl_not_found() {
        assert!(is_not_found(
            "curl: (56) The requested URL returned error: 404"
        ));
        assert!(is_not_found(
            "curl: (22) The requested URL returned error: 404"
        ));
        assert!(!is_not_found(
            "curl: (56) The requested URL returned error: 403"
        ));
        assert!(!is_not_found(
            "curl: (6) Could not resolve host: atcoder.jp"
        ));
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "Windows：段階 3（Keychain を資格情報マネージャーに）と段階 5（curl.exe）で直す。docs/windows-port-plan.md 7 章"
    )]
    fn reads_with_the_session_only_after_a_404_when_logged_in() {
        let keychain = TestKeychain::new("problem-fallback");
        let client = keychain.client();
        // 読めたもの・404 以外の失敗は、そのまま（セッションを使わない）
        assert_eq!(
            with_session_fallback("http://127.0.0.1:9/", Ok("page".to_string()), &client),
            Ok("page".to_string())
        );
        assert_eq!(
            with_session_fallback(
                "http://127.0.0.1:9/",
                Err(FetchError::Other("つながりません".to_string())),
                &client
            ),
            Err("つながりません".to_string())
        );
        // ログインしていなければ、送らずにログインを勧める
        let not_found =
            with_session_fallback("http://127.0.0.1:9/", Err(FetchError::NotFound), &client);
        assert_eq!(not_found, Err(NOT_FOUND_LOGGED_OUT.to_string()));
        // ログインしていれば、セッションを付けてもう 1 回読む
        keychain.0.save("session-value").unwrap();
        let (url, server) = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\ntask page",
        );
        assert_eq!(
            with_session_fallback(&url, Err(FetchError::NotFound), &client),
            Ok("task page".to_string())
        );
        assert!(
            server
                .join()
                .unwrap()
                .contains("Cookie: REVEL_SESSION=session-value")
        );
        // ログインしても読めなければ、その状態を出す
        let (url, server) =
            serve_once("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        assert!(
            with_session_fallback(&url, Err(FetchError::NotFound), &client)
                .unwrap_err()
                .contains("HTTP 404")
        );
        server.join().unwrap();
    }

    // 問題ページの作りをまねた小さな例（01 §2 の観察と、2026-10-25 に保存した問題の HTML による。実際のページはテストで取得しない）。
    // 取得したページの pre には id がない。「入力」の区画の pre（入力の形式）と、出力例の説明の中の pre は、サンプルではない
    const PAGE: &str = r#"<html><body>
<span class="h2">
    A - Happy Birthday!
    <a class="btn btn-default btn-sm" href="/contests/abc100/tasks/abc100_a/editorial">解説</a>
</span>
<p>実行時間制限: 2 sec / メモリ制限: 1024 MiB</p>
<div id="task-statement"><span class="lang">
<span class="lang-ja">
<p>配点 : <var>100</var> 点</p>
<div class="part"><section><h3>問題文</h3><p><var>A</var> 個と <var>B</var> 個</p></section></div>
<div class="io-style">
<div class="part"><section><h3>入力</h3><p>入力は以下の形式で与えられる。</p><pre><var>A</var> <var>B</var>
</pre></section></div>
</div>
<hr>
<div class="part">
<section>
<h3>入力例 1</h3><pre>5 4
</pre>
</section>
</div>
<div class="part"><section><h3>出力例 1</h3><pre>Yay!
</pre><p>説明の中の例：</p><pre>not a sample</pre></section></div>
<div class="part"><section><h3>入力例 2</h3><pre>8 8</pre></section></div>
<div class="part"><section><h3>出力例 2</h3><pre>Yay!</pre></section></div>
</span>
<span class="lang-en"><p>Score : <var>100</var> points</p>
<div class="part"><section><h3>Sample Input 1</h3><pre>5 4</pre></section></div></span>
</span></div>
</body></html>"#;

    #[test]
    fn parses_title_limits_statement_and_samples() {
        let problem = parse_problem(PAGE).unwrap();
        assert_eq!(problem.title, "A - Happy Birthday!");
        assert_eq!(problem.time_limit, "2 sec");
        assert_eq!(problem.memory_limit, "1024 MiB");
        assert!(problem.statement_html.contains("問題文"));
        assert!(problem.statement_html.contains("<var>A</var>"));
        // 英語の問題文は含めない
        assert!(!problem.statement_html.contains("Score"));
        assert_eq!(
            problem.samples,
            vec![
                Sample {
                    input: "5 4\n".to_string(),
                    output: "Yay!\n".to_string()
                },
                Sample {
                    input: "8 8\n".to_string(),
                    output: "Yay!\n".to_string()
                },
            ]
        );
    }

    #[test]
    fn reads_limits_on_english_page() {
        let english = PAGE.replace(
            "実行時間制限: 2 sec / メモリ制限: 1024 MiB",
            "Time Limit: 2 sec / Memory Limit: 1024 MiB",
        );
        let problem = parse_problem(&english).unwrap();
        assert_eq!(problem.time_limit, "2 sec");
        assert_eq!(problem.memory_limit, "1024 MiB");
    }

    #[test]
    fn reports_pages_with_other_structure() {
        assert!(parse_problem("<html><body><p>404 Not Found</p></body></html>").is_err());
        let without_statement = PAGE.replace("task-statement", "other");
        assert!(parse_problem(&without_statement).is_err());
        let odd_samples = PAGE.replace(
            r#"<div class="part"><section><h3>出力例 2</h3><pre>Yay!</pre></section></div>"#,
            "",
        );
        assert!(parse_problem(&odd_samples).is_err());
        let empty_sample = PAGE.replace("<h3>入力例 2</h3><pre>8 8</pre>", "<h3>入力例 2</h3>");
        // 中身のない区画は、その見出しを理由に出す
        assert!(
            parse_problem(&empty_sample)
                .unwrap_err()
                .contains("「入力例 2」")
        );
    }

    // D-155：パッケージのフォルダから問題を決める
    #[test]
    fn decides_the_problem_from_the_package_folder() {
        let dir = std::env::temp_dir().join(format!(
            "atcoder-studio-package-problem-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let id = |contest: &str, task: &str| Some(ProblemId::new(contest, task).unwrap());
        // 一覧がないとき：JOI は名前で比べて親のフォルダ名、ほかは今までの決まり
        assert_eq!(
            package_problem(&dir, "joi2027yo1b/joi2027_yo1b_a"),
            id("joi2027yo1b", "joi2027_yo1b_a")
        );
        assert_eq!(
            package_problem(&dir, "abc480/abc480_a"),
            id("abc480", "abc480_a")
        );
        assert_eq!(
            package_problem(&dir, "problems/abc001_a"),
            id("abc001", "abc001_a")
        );
        assert_eq!(package_problem(&dir, "abc001_a"), id("abc001", "abc001_a"));
        assert_eq!(
            package_problem(&dir, "abc042/arc058_a"),
            id("arc058", "arc058_a")
        );
        // 保存した一覧にあれば、親のフォルダ名（問題 ID に別のコンテストの名前が付く形）
        std::fs::create_dir_all(dir.join("task-lists")).unwrap();
        std::fs::write(
            dir.join("task-lists/abc042.json"),
            r#"[{"task":"arc058_a","label":"C","title":"x"}]"#,
        )
        .unwrap();
        assert_eq!(
            package_problem(&dir, "abc042/arc058_a"),
            id("abc042", "arc058_a")
        );
        // 一覧にない問題は、一覧からは決めない
        assert_eq!(
            package_problem(&dir, "abc042/arc058_b"),
            id("arc058", "arc058_b")
        );
        // 決まらないもの
        for package in [
            "",
            "perf-output",
            "solution",
            "x/abc001_",
            "x/_a",
            "X/abc001_a/Y",
        ] {
            assert_eq!(package_problem(&dir, package), None, "{package}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn accepts_only_simple_ids() {
        assert_eq!(
            ProblemId::new("abc430", "abc430_a").unwrap().url(),
            "https://atcoder.jp/contests/abc430/tasks/abc430_a"
        );
        assert!(ProblemId::new("abc430", "../x").is_err());
        assert!(ProblemId::new("ABC430", "abc430_a").is_err());
        assert!(ProblemId::new("", "abc430_a").is_err());
    }

    #[test]
    fn uses_saved_problem_without_fetching() {
        let dir =
            std::env::temp_dir().join(format!("atcoder-studio-problem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let id = ProblemId::new("abc100", "abc100_a").unwrap();
        let problem = parse_problem(PAGE).unwrap();
        storage::write_atomic(
            &cache_path(&dir, &id),
            &serde_json::to_vec(&problem).unwrap(),
        )
        .unwrap();
        // 保存してあるので、取得せずに返す（テストでは AtCoder につながない）
        assert_eq!(load_problem(&dir, &id, false).unwrap(), problem);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_task_list() {
        let html = r#"<table><thead><tr><th>問題</th></tr></thead><tbody>
<tr><td class="text-center no-break"><a href="/contests/abc475/tasks/abc475_a">A</a></td><td><a href="/contests/abc475/tasks/abc475_a">mnclr</a></td><td>2 sec</td></tr>
<tr><td><a href="/contests/abc475/tasks/abc475_b">B</a></td><td><a href="/contests/abc475/tasks/abc475_b">Change</a></td><td>2 sec</td></tr>
<tr><td><a href="https://example.com/x">C</a></td><td>外のリンク</td></tr>
</tbody></table>"#;
        assert_eq!(
            parse_task_list(html, "abc475").unwrap(),
            vec![
                TaskEntry {
                    task: "abc475_a".to_string(),
                    label: "A".to_string(),
                    title: "mnclr".to_string()
                },
                TaskEntry {
                    task: "abc475_b".to_string(),
                    label: "B".to_string(),
                    title: "Change".to_string()
                },
            ]
        );
        assert!(parse_task_list("<p>404</p>", "abc475").is_err());
    }

    #[test]
    fn remembers_choice_per_package() {
        let dir =
            std::env::temp_dir().join(format!("atcoder-studio-choice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let package = Path::new("/ws/A");
        assert_eq!(choice_for(&dir, package), None);
        let id = ProblemId::new("abc011", "abc011_a").unwrap();
        set_choice(&dir, package, id.clone()).unwrap();
        assert_eq!(choice_for(&dir, package), Some(id));
        assert_eq!(choice_for(&dir, Path::new("/ws/B")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saves_samples_without_overwriting() {
        let dir =
            std::env::temp_dir().join(format!("atcoder-studio-samples-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = dir.join("test");
        let samples = vec![
            Sample {
                input: "1\n".to_string(),
                output: "2\n".to_string(),
            },
            Sample {
                input: "3\n".to_string(),
                output: "4\n".to_string(),
            },
        ];
        let first = save_samples(&folder, &samples).unwrap();
        assert_eq!(first.written.len(), 4);
        assert_eq!(
            std::fs::read_to_string(folder.join("sample-2.out")).unwrap(),
            "4\n"
        );
        // 利用者が書き換えたファイルは上書きしない
        std::fs::write(folder.join("sample-1.out"), "edited\n").unwrap();
        let second = save_samples(&folder, &samples).unwrap();
        assert_eq!(second.written, Vec::<String>::new());
        assert_eq!(second.skipped, vec!["sample-1.out".to_string()]);
        assert_eq!(second.unchanged.len(), 3);
        assert_eq!(
            std::fs::read_to_string(folder.join("sample-1.out")).unwrap(),
            "edited\n"
        );
        assert!(second.leftover.is_empty());
        // サンプルが減ったら（「再取得」など）、残った番号のファイルを知らせる。消さない
        std::fs::write(folder.join("sample-3.in"), "5\n").unwrap();
        std::fs::write(folder.join("sample-10.out"), "6\n").unwrap();
        let third = save_samples(&folder, &samples).unwrap();
        assert_eq!(
            third.leftover,
            vec!["sample-3.in".to_string(), "sample-10.out".to_string()]
        );
        assert!(folder.join("sample-3.in").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn waits_between_fetches() {
        wait_for_interval();
        let started = Instant::now();
        wait_for_interval();
        assert!(started.elapsed() >= Duration::from_millis(900));
    }
}
