// フォルダ全体の検索（Phase 12 作業 2。D-183）。
//
// - 作業領域を下へたどり、1 行ずつ探す（行をまたぐ当たりは探さない）
// - シンボリックリンクはたどらない（外を指すもの・輪になるものを読まない）。`.git`・`target`・`node_modules` のフォルダは読まない
// - 1 MB を超えるファイル・NUL を含むファイル・UTF-8 でないファイルは読まない（飛ばした数を返す）
// - 当たりは MAX_MATCHES 件まで。新しい検索を始めたら、前の検索は途中でやめる（世代の番号で比べる）
// - 読むだけなので、信頼していない作業領域でも使える（ファイル一覧と同じ）
//
// 置換（Phase 12 作業 3。D-184）：検索と同じ条件で探し直して置き換える。検索のあとで変わったファイルは置き換えない。
// 置き換える前の中身をメモリに持ち（UNDO_LIMIT まで）、次の置換・作業領域の開き直し・アプリの終了まで元に戻せる
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::workspace::{self, MAX_FILE_SIZE, Stamp};

// 当たりの上限。超えたら止めて、途中までと知らせる
pub const MAX_MATCHES: usize = 2000;
// 1 行の見せる文字の上限（UTF-16 の数）。長い行は当たりの前後を切り出す
const PREVIEW_LEN: usize = 250;
// 当たりの前に見せる文字の数（長い行のとき）
const PREVIEW_BEFORE: usize = 40;
// 読まないフォルダの名前
const SKIPPED_DIRS: [&str; 3] = [".git", "target", "node_modules"];
// 正規表現の大きさの上限（大きすぎる式で固まらないように）
const REGEX_SIZE_LIMIT: usize = 1 << 20;

// 元に戻すために持つ中身の上限（置き換える前のファイルの大きさの合計）。超えるなら置き換えない
pub const UNDO_LIMIT: usize = 50 * 1024 * 1024;

// 検索の世代（新しい検索を始めるたびに増やし、古い検索は途中でやめる）と、直前の置換を元に戻すための中身
#[derive(Default)]
pub struct SearchControl {
    generation: AtomicU64,
    undo: Mutex<Option<Undo>>,
}

impl SearchControl {
    pub fn begin(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    // 作業領域を開き直したら、元に戻せなくする
    pub fn clear_undo(&self) {
        if let Ok(mut undo) = self.undo.lock() {
            *undo = None;
        }
    }
}

// 直前の置換を元に戻すための中身。stamp は置き換えたあとの更新日時と大きさ（そのあと変わったものは戻さない）
struct Undo {
    root: PathBuf,
    entries: Vec<UndoEntry>,
}

struct UndoEntry {
    path: String,
    original: String,
    stamp: Stamp,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub pattern: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

// 1 件の当たり。line・column は 1 から（column は行の中の UTF-16 の位置。エディタの移動と同じ数え方）。
// preview は見せる行（前の空白を除き、長ければ切り出す）、start・end はその中の当たりの位置（UTF-16）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineMatch {
    pub line: u32,
    pub column: u32,
    pub preview: String,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMatches {
    // 作業領域のルートからの相対パス（区切りは `/`）
    pub path: String,
    // 読んだときの更新日時と大きさ（置換のときに、そのあと外で変わっていないかを確かめる。作業 3）
    pub stamp: Stamp,
    pub matches: Vec<LineMatch>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub files: Vec<FileMatches>,
    // 当たりの数
    pub total: usize,
    // 上限で止めた
    pub truncated: bool,
    // 読まなかったファイルの数（大きすぎる・文字のファイルでない・読めない）
    pub skipped: usize,
    // 新しい検索が始まったので、途中でやめた（画面はこの結果を使わない）
    pub cancelled: bool,
}

// 探す式を作る。正規表現でなければ文字どおりに探す。単語単位は \b で囲む
pub fn build_regex(query: &SearchQuery) -> Result<Regex, String> {
    if query.pattern.is_empty() {
        return Err("探す文字を入れてください".to_string());
    }
    let mut pattern = if query.regex {
        query.pattern.clone()
    } else {
        regex::escape(&query.pattern)
    };
    if query.whole_word {
        pattern = format!(r"\b(?:{pattern})\b");
    }
    RegexBuilder::new(&pattern)
        .case_insensitive(!query.case_sensitive)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(describe_regex_error)
}

// regex の誤りの文は複数行の英語なので、理由の行（"error: …"）だけを添える
fn describe_regex_error(error: regex::Error) -> String {
    match error {
        regex::Error::CompiledTooBig(_) => "正規表現が大きすぎます".to_string(),
        error => {
            let text = error.to_string();
            let reason = text
                .lines()
                .find_map(|line| line.trim().strip_prefix("error: "))
                .unwrap_or("書き方の誤り");
            format!("正規表現として読めません（{reason}）")
        }
    }
}

// 作業領域を探す。generation が今の世代でなくなったら、途中でやめる
pub fn search(
    root: &Path,
    regex: &Regex,
    control: &SearchControl,
    generation: u64,
) -> SearchResult {
    let mut result = SearchResult {
        files: Vec::new(),
        total: 0,
        truncated: false,
        skipped: 0,
        cancelled: false,
    };
    walk(root, "", regex, control, generation, &mut result);
    result
}

// フォルダの中を名前の順にたどる（フォルダとファイルを混ぜて、大文字・小文字を区別しない順）。
// やめるとき（上限・世代が変わった）は false
fn walk(
    root: &Path,
    relative: &str,
    regex: &Regex,
    control: &SearchControl,
    generation: u64,
    result: &mut SearchResult,
) -> bool {
    let Ok(entries) = fs::read_dir(root.join(relative)) else {
        return true;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
    for entry in entries {
        if !control.is_current(generation) {
            result.cancelled = true;
            return false;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = if relative.is_empty() {
            name.clone()
        } else {
            format!("{relative}/{name}")
        };
        // file_type はリンクをたどらない。リンクは飛ばす
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if SKIPPED_DIRS.contains(&name.as_str()) {
                continue;
            }
            if !walk(root, &path, regex, control, generation, result) {
                return false;
            }
        } else if kind.is_file() && !search_file(root, &path, regex, result) {
            result.truncated = true;
            return false;
        }
    }
    true
}

// 1 つのファイルを探す。上限に達したら false
fn search_file(root: &Path, relative: &str, regex: &Regex, result: &mut SearchResult) -> bool {
    let full = root.join(relative);
    let text = match read_text(&full) {
        Some(text) => text,
        None => {
            result.skipped += 1;
            return true;
        }
    };
    let Ok(stamp) = workspace::stamp_of(&full) else {
        result.skipped += 1;
        return true;
    };
    let mut matches = Vec::new();
    let mut full_stop = false;
    'lines: for (index, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        for found in regex.find_iter(line) {
            // 空の当たり（`a*` など）は数えない
            if found.start() == found.end() {
                continue;
            }
            if result.total >= MAX_MATCHES {
                full_stop = true;
                break 'lines;
            }
            matches.push(line_match(index, line, found.start(), found.end()));
            result.total += 1;
        }
    }
    if !matches.is_empty() {
        result.files.push(FileMatches {
            path: relative.to_string(),
            stamp,
            matches,
        });
    }
    !full_stop
}

// 文字のファイルとして読む。大きすぎる・NUL を含む・UTF-8 でない・読めないときは None
pub fn read_text(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > MAX_FILE_SIZE {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

// 当たり（line の中のバイトの位置）から、見せる行と位置を作る
fn line_match(index: usize, line: &str, start: usize, end: usize) -> LineMatch {
    let column = utf16_len(&line[..start]) + 1;
    // 前の空白を除く（当たりが空白の中なら除かない）
    let trimmed = line.len() - line.trim_start().len();
    let from = if trimmed <= start { trimmed } else { 0 };
    let mut shown = &line[from..];
    let mut offset = from;
    // 長い行は、当たりの少し前から切り出す
    if utf16_len(shown) > PREVIEW_LEN {
        let before: Vec<(usize, char)> = line[offset..start].char_indices().collect();
        if before.len() > PREVIEW_BEFORE {
            let cut = before[before.len() - PREVIEW_BEFORE].0;
            offset += cut;
            shown = &line[offset..];
        }
        let mut width = 0;
        let mut stop = shown.len();
        for (position, ch) in shown.char_indices() {
            width += ch.len_utf16();
            if width > PREVIEW_LEN && offset + position >= end {
                stop = position;
                break;
            }
        }
        shown = &shown[..stop];
    }
    let preview_start = utf16_len(&line[offset..start]);
    let preview_end = preview_start + utf16_len(&line[start..end.min(offset + shown.len())]);
    LineMatch {
        line: (index + 1) as u32,
        column: column as u32,
        preview: shown.to_string(),
        start: preview_start as u32,
        end: preview_end as u32,
    }
}

// 置き換えるファイルと、検索したときの更新日時と大きさ
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceTarget {
    pub path: String,
    pub stamp: Stamp,
}

// 置き換える前に確かめてもらう内容（確認のダイアログに出す）
#[derive(Debug, Clone, PartialEq)]
pub struct ReplacePlan {
    pub files: usize,
    pub matches: usize,
    // 検索のあとで変わったので置き換えないファイル
    pub changed: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceResult {
    // 確認のダイアログでキャンセルした
    pub cancelled: bool,
    // 置き換えた件数と、書き換えたファイル
    pub replaced: usize,
    pub files: Vec<String>,
    // 検索のあとで変わったので置き換えなかったファイル
    pub changed: Vec<String>,
    // 読めない・書けないなどで置き換えられなかったファイル
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoResult {
    pub restored: Vec<String>,
    // 置き換えたあとで変わったので戻さなかったファイル
    pub changed: Vec<String>,
    pub failed: Vec<String>,
}

// 置換の文の `$数字` のすぐあとに英数字か `_` が続くとき、`${数字}` にする。
// regex は `$` のあとの英数字と `_` を最長で組の名前として読む（`$1_x` は `1_x` という名前の組）が、
// VS Code と同じく `$1` を 1 番目の組として扱うため。`$$`（`$` の文字）と `${name}` は変えない
fn brace_group_numbers(replacement: &str) -> String {
    let mut out = String::with_capacity(replacement.len());
    let mut rest = replacement;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some(tail) = after.strip_prefix('$') {
            out.push_str("$$");
            rest = tail;
            continue;
        }
        let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let (number, tail) = after.split_at(digits);
        let named = tail
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        if digits > 0 && named {
            out.push_str("${");
            out.push_str(number);
            out.push('}');
        } else {
            out.push('$');
            out.push_str(number);
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

// 1 行ずつ置き換えた中身と件数（検索と同じく、行をまたがず、空の当たりは置き換えない。改行の形は変えない）。
// expand（正規表現のとき）なら、置換の文の `$1`・`${name}` を当たりの組で置き換える。そうでなければ文字どおり
pub fn replace_text(text: &str, regex: &Regex, replacement: &str, expand: bool) -> (String, usize) {
    let braced;
    let replacement = if expand {
        braced = brace_group_numbers(replacement);
        braced.as_str()
    } else {
        replacement
    };
    let mut out = String::with_capacity(text.len());
    let mut count = 0;
    for (index, raw) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let (line, cr) = match raw.strip_suffix('\r') {
            Some(line) => (line, true),
            None => (raw, false),
        };
        let mut last = 0;
        for captures in regex.captures_iter(line) {
            let Some(found) = captures.get(0) else {
                continue;
            };
            if found.start() == found.end() {
                continue;
            }
            out.push_str(&line[last..found.start()]);
            if expand {
                captures.expand(replacement, &mut out);
            } else {
                out.push_str(replacement);
            }
            last = found.end();
            count += 1;
        }
        out.push_str(&line[last..]);
        if cr {
            out.push('\r');
        }
    }
    (out, count)
}

// 置き換える。confirm で確かめてもらい（false ならやめる）、書いたファイルの元の中身を、元に戻すために持つ
pub fn replace_files(
    root: &Path,
    regex: &Regex,
    replacement: &str,
    expand: bool,
    targets: &[ReplaceTarget],
    control: &SearchControl,
    confirm: impl FnOnce(&ReplacePlan) -> bool,
) -> Result<ReplaceResult, String> {
    let mut result = ReplaceResult::default();
    // 置き換える中身を先に作る
    let mut planned = Vec::new();
    let mut size = 0;
    for target in targets {
        let Ok(full) = workspace::resolve(root, &target.path) else {
            result.failed.push(target.path.clone());
            continue;
        };
        if workspace::stamp_of(&full).ok().as_ref() != Some(&target.stamp) {
            result.changed.push(target.path.clone());
            continue;
        }
        let Some(original) = read_text(&full) else {
            result.failed.push(target.path.clone());
            continue;
        };
        let (replaced, count) = replace_text(&original, regex, replacement, expand);
        if count == 0 {
            continue;
        }
        size += original.len();
        planned.push((target, original, replaced, count));
    }
    if planned.is_empty() {
        return Ok(result);
    }
    if size > UNDO_LIMIT {
        return Err(
            "置き換えるファイルが大きすぎます（元に戻すための中身が 50 MB を超えます）。探す範囲を絞ってください"
                .to_string(),
        );
    }
    let plan = ReplacePlan {
        files: planned.len(),
        matches: planned.iter().map(|(_, _, _, count)| count).sum(),
        changed: result.changed.len(),
    };
    if !confirm(&plan) {
        result.cancelled = true;
        return Ok(result);
    }
    let mut entries = Vec::new();
    for (target, original, replaced, count) in planned {
        // 確認を待つ間に変わっていたら書かない（Conflict）
        match workspace::write_existing(root, &target.path, &replaced, Some(&target.stamp)) {
            Ok(workspace::WriteResult::Saved { stamp }) => {
                result.replaced += count;
                result.files.push(target.path.clone());
                entries.push(UndoEntry {
                    path: target.path.clone(),
                    original,
                    stamp,
                });
            }
            Ok(workspace::WriteResult::Conflict) => result.changed.push(target.path.clone()),
            Err(_) => result.failed.push(target.path.clone()),
        }
    }
    // 前の置換は、もう元に戻せない（直前のものだけ）
    let mut undo = control
        .undo
        .lock()
        .map_err(|_| "置換の記録を読めません".to_string())?;
    *undo = (!entries.is_empty()).then(|| Undo {
        root: root.to_path_buf(),
        entries,
    });
    Ok(result)
}

// 直前の置換を元に戻す。置き換えたあとで変わったファイルは戻さない。一度戻したら、もう戻せない
pub fn undo_replace(root: &Path, control: &SearchControl) -> Result<UndoResult, String> {
    let mut held = control
        .undo
        .lock()
        .map_err(|_| "置換の記録を読めません".to_string())?;
    match held.as_ref() {
        None => return Err("元に戻せる置換がありません".to_string()),
        Some(undo) if undo.root != root => {
            return Err("フォルダが変わったので、元に戻せません".to_string());
        }
        Some(_) => {}
    }
    let Some(undo) = held.take() else {
        return Err("元に戻せる置換がありません".to_string());
    };
    drop(held);
    let mut result = UndoResult::default();
    for entry in undo.entries {
        match workspace::write_existing(root, &entry.path, &entry.original, Some(&entry.stamp)) {
            Ok(workspace::WriteResult::Saved { .. }) => result.restored.push(entry.path),
            Ok(workspace::WriteResult::Conflict) => result.changed.push(entry.path),
            Err(_) => result.failed.push(entry.path),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "atcoder-studio-search-{}-{}",
                name,
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempDir(path.canonicalize().unwrap())
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn query(pattern: &str) -> SearchQuery {
        SearchQuery {
            pattern: pattern.to_string(),
            case_sensitive: false,
            whole_word: false,
            regex: false,
        }
    }

    fn run(root: &Path, query: &SearchQuery) -> SearchResult {
        let control = SearchControl::default();
        let generation = control.begin();
        search(root, &build_regex(query).unwrap(), &control, generation)
    }

    fn paths(result: &SearchResult) -> Vec<&str> {
        result.files.iter().map(|file| file.path.as_str()).collect()
    }

    #[cfg(unix)]
    #[test]
    fn finds_lines_and_skips_excluded_places() {
        let dir = TempDir::new("walk");
        let root = &dir.0;
        fs::create_dir_all(root.join("a/src")).unwrap();
        fs::write(
            root.join("a/src/main.rs"),
            "fn main() {\n    let x = 1;\r\n    x\n}\n",
        )
        .unwrap();
        fs::write(root.join("B.txt"), "let\nlet let\n").unwrap();
        for skipped in ["target", ".git", "node_modules"] {
            fs::create_dir_all(root.join(skipped)).unwrap();
            fs::write(root.join(skipped).join("x.rs"), "let").unwrap();
        }
        fs::create_dir_all(root.join(".cargo")).unwrap();
        fs::write(root.join(".cargo/config.toml"), "let = 1").unwrap();
        // 読まないファイル：NUL・UTF-8 でない・大きすぎる
        fs::write(root.join("bin.dat"), b"let\0").unwrap();
        fs::write(root.join("latin1.txt"), b"let \xff").unwrap();
        fs::write(root.join("big.txt"), "let ".repeat(300_000)).unwrap();
        // リンクはたどらない（外を指すもの・フォルダのリンク）
        let outside = TempDir::new("walk-outside");
        fs::write(outside.0.join("secret.rs"), "let").unwrap();
        symlink(outside.0.join("secret.rs"), root.join("link.rs")).unwrap();
        symlink(&outside.0, root.join("linkdir")).unwrap();

        let result = run(root, &query("LET"));
        assert_eq!(
            paths(&result),
            vec![".cargo/config.toml", "a/src/main.rs", "B.txt"]
        );
        assert_eq!(result.total, 5);
        assert_eq!(result.skipped, 3);
        assert!(!result.truncated && !result.cancelled);
        let main = &result.files[1].matches[0];
        // 行・列は 1 から。前の空白は見せる行から除く。\r は除く
        assert_eq!((main.line, main.column), (2, 5));
        assert_eq!(main.preview, "let x = 1;");
        assert_eq!((main.start, main.end), (0, 3));
        let second = &result.files[2].matches[2];
        assert_eq!((second.line, second.column, second.start), (2, 5, 4));
    }

    #[test]
    fn honors_case_word_and_regex() {
        let dir = TempDir::new("options");
        fs::write(dir.0.join("a.rs"), "Vec vec vector\nlet v = vec![1];\n").unwrap();
        let count = |query: &SearchQuery| run(&dir.0, query).total;
        assert_eq!(count(&query("vec")), 4);
        assert_eq!(
            count(&SearchQuery {
                case_sensitive: true,
                ..query("vec")
            }),
            3
        );
        assert_eq!(
            count(&SearchQuery {
                whole_word: true,
                ..query("vec")
            }),
            3
        );
        // 正規表現でないときは、記号を文字どおりに探す
        assert_eq!(count(&query("vec![")), 1);
        assert_eq!(count(&query(".")), 0);
        assert_eq!(
            count(&SearchQuery {
                regex: true,
                ..query(r"\bvec!\[")
            }),
            1
        );
        // 空の当たりは数えない
        assert_eq!(
            count(&SearchQuery {
                regex: true,
                ..query("x*")
            }),
            0
        );
    }

    #[test]
    fn explains_bad_patterns() {
        let error = build_regex(&SearchQuery {
            regex: true,
            ..query("(")
        })
        .unwrap_err();
        assert!(error.starts_with("正規表現として読めません（"), "{error}");
        assert!(!error.contains('\n'));
        let error = build_regex(&SearchQuery {
            regex: true,
            ..query(r"\w{1000}{1000}")
        })
        .unwrap_err();
        assert_eq!(error, "正規表現が大きすぎます");
        assert!(build_regex(&query("")).is_err());
    }

    #[test]
    fn stops_at_the_limit_and_when_superseded() {
        let dir = TempDir::new("limit");
        fs::write(dir.0.join("a.txt"), "x\n".repeat(MAX_MATCHES)).unwrap();
        fs::write(dir.0.join("b.txt"), "x\n").unwrap();
        let result = run(&dir.0, &query("x"));
        assert_eq!(result.total, MAX_MATCHES);
        assert!(result.truncated);
        assert_eq!(paths(&result), vec!["a.txt"]);
        // 新しい検索が始まっていれば、途中でやめる
        let control = SearchControl::default();
        let old = control.begin();
        control.begin();
        let result = search(&dir.0, &build_regex(&query("x")).unwrap(), &control, old);
        assert!(result.cancelled);
        assert!(result.files.is_empty());
    }

    #[test]
    fn cuts_long_lines_around_the_match_and_counts_utf16() {
        // 列は UTF-16 で数える（エディタと同じ）。「あ」は 1、絵文字は 2
        let found = line_match(0, "あ😀x", "あ😀".len(), "あ😀x".len());
        assert_eq!((found.column, found.start, found.end), (4, 3, 4));
        let long = format!("{}needle{}", "a".repeat(500), "b".repeat(500));
        let start = 500;
        let found = line_match(0, &long, start, start + 6);
        assert_eq!(found.column, 501);
        assert_eq!(
            &found.preview[found.start as usize..found.end as usize],
            "needle"
        );
        assert_eq!(found.start as usize, PREVIEW_BEFORE);
        assert!(utf16_len(&found.preview) <= PREVIEW_LEN + 1);
    }

    #[test]
    fn replaces_line_by_line_keeping_line_endings() {
        let regex = build_regex(&query("let")).unwrap();
        let (text, count) = replace_text("let a\r\nlet b let\n\nx", &regex, "$1 const", false);
        // 正規表現でないときは、置換の文を文字どおりに使う
        assert_eq!(text, "$1 const a\r\n$1 const b $1 const\n\nx");
        assert_eq!(count, 3);
        let regex = build_regex(&SearchQuery {
            regex: true,
            ..query(r"(\w+)\.len\(\)")
        })
        .unwrap();
        let (text, count) = replace_text("a.len() + bb.len()", &regex, "len($1)", true);
        assert_eq!((text.as_str(), count), ("len(a) + len(bb)", 2));
        // `$1` のあとに文字が続いても 1 番目の組。`$$` は `$` の文字、`${1}`・`${name}` は今のまま
        let regex = build_regex(&SearchQuery {
            regex: true,
            ..query(r"(?<w>\w+)")
        })
        .unwrap();
        for (replacement, expected) in [
            ("$1_x", "foo_x bar_x"),
            ("$1a", "fooa bara"),
            ("$$1", "$1 $1"),
            ("$$$1_", "$foo_ $bar_"),
            ("${1}x", "foox barx"),
            ("${w}_y", "foo_y bar_y"),
            ("$1-$1", "foo-foo bar-bar"),
            ("$", "$ $"),
            ("$x", " "),
        ] {
            assert_eq!(
                replace_text("foo bar", &regex, replacement, true).0,
                expected,
                "{replacement}"
            );
        }
        // 空の当たりは置き換えない
        let regex = build_regex(&SearchQuery {
            regex: true,
            ..query("x*")
        })
        .unwrap();
        assert_eq!(
            replace_text("abxc", &regex, "-", true),
            ("ab-c".to_string(), 1)
        );
    }

    #[test]
    fn replaces_confirms_and_undoes() {
        let dir = TempDir::new("replace");
        let root = &dir.0;
        fs::write(root.join("a.rs"), "let x = 1;\nlet y = 2;\n").unwrap();
        fs::write(root.join("b.rs"), "let z;\n").unwrap();
        fs::write(root.join("c.rs"), "let w;\n").unwrap();
        let control = SearchControl::default();
        let found = run(root, &query("let"));
        let targets: Vec<ReplaceTarget> = found
            .files
            .iter()
            .map(|file| ReplaceTarget {
                path: file.path.clone(),
                stamp: file.stamp.clone(),
            })
            .collect();
        // 検索のあとで変わったファイルは置き換えない（大きさを変えて確実に違う印にする）
        fs::write(root.join("c.rs"), "let w; // 変えた\n").unwrap();
        let regex = build_regex(&query("let")).unwrap();
        // キャンセルなら何も変えない
        let mut seen = None;
        let cancelled = replace_files(root, &regex, "const", false, &targets, &control, |plan| {
            seen = Some(plan.clone());
            false
        })
        .unwrap();
        assert!(cancelled.cancelled);
        assert_eq!(
            seen,
            Some(ReplacePlan {
                files: 2,
                matches: 3,
                changed: 1
            })
        );
        assert_eq!(
            fs::read_to_string(root.join("a.rs")).unwrap(),
            "let x = 1;\nlet y = 2;\n"
        );
        assert!(undo_replace(root, &control).is_err());

        let done =
            replace_files(root, &regex, "const", false, &targets, &control, |_| true).unwrap();
        assert_eq!(done.replaced, 3);
        assert_eq!(done.files, vec!["a.rs", "b.rs"]);
        assert_eq!(done.changed, vec!["c.rs"]);
        assert_eq!(
            fs::read_to_string(root.join("a.rs")).unwrap(),
            "const x = 1;\nconst y = 2;\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("c.rs")).unwrap(),
            "let w; // 変えた\n"
        );

        // 置き換えたあとで変わったファイルは戻さない
        fs::write(root.join("b.rs"), "const z; // あとで変えた\n").unwrap();
        let undone = undo_replace(root, &control).unwrap();
        assert_eq!(undone.restored, vec!["a.rs"]);
        assert_eq!(undone.changed, vec!["b.rs"]);
        assert_eq!(
            fs::read_to_string(root.join("a.rs")).unwrap(),
            "let x = 1;\nlet y = 2;\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("b.rs")).unwrap(),
            "const z; // あとで変えた\n"
        );
        // 一度戻したら、もう戻せない
        assert!(undo_replace(root, &control).is_err());
    }

    #[test]
    fn refuses_paths_outside_and_other_roots() {
        let dir = TempDir::new("replace-outside");
        let outside = TempDir::new("replace-outside-x");
        fs::write(outside.0.join("o.rs"), "let").unwrap();
        fs::write(dir.0.join("a.rs"), "let").unwrap();
        let control = SearchControl::default();
        let regex = build_regex(&query("let")).unwrap();
        let stamp = workspace::stamp_of(&outside.0.join("o.rs")).unwrap();
        let name = outside
            .0
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let targets = [ReplaceTarget {
            path: format!("../{name}/o.rs"),
            stamp,
        }];
        let result =
            replace_files(&dir.0, &regex, "x", false, &targets, &control, |_| true).unwrap();
        assert_eq!(result.failed.len(), 1);
        assert_eq!(fs::read_to_string(outside.0.join("o.rs")).unwrap(), "let");
        // 別のフォルダでは、元に戻さない
        let targets = [ReplaceTarget {
            path: "a.rs".to_string(),
            stamp: workspace::stamp_of(&dir.0.join("a.rs")).unwrap(),
        }];
        replace_files(&dir.0, &regex, "x", false, &targets, &control, |_| true).unwrap();
        assert!(undo_replace(&outside.0, &control).is_err());
        // 断っても、元のフォルダでは戻せる
        assert_eq!(
            undo_replace(&dir.0, &control).unwrap().restored,
            vec!["a.rs"]
        );
    }
}
