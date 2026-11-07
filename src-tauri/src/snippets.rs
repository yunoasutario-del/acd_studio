// スニペット（Phase 13 作業 1。D-186）。アプリのフォルダの言語ごとのファイル（VS Code の書き方）を読み、
// エディタ（@codemirror/autocomplete の snippet）の書き方に直して画面に渡す。
//
// - ファイルは言語ごと（Phase 17 作業 9。D-213）：snippets-rust.json（.rs）・snippets-cpp.json（C++）・
//   snippets-python.json（.py。Phase 20 作業 8。D-227）。VS Code の rust.json・cpp.json・python.json と同じ分け方。前の snippets.json は、snippets-rust.json がなければ、読むときに名前を変える
//
// - コメント（// と /* */）と末尾のカンマを許す（VS Code のスニペットのファイルと同じ）。1 MB まで
// - 読めない項目は、それだけを使わずに知らせる。ファイル全体が読めなければ Err（画面は前のものを使い続ける）
// - 直し方（VS Code → エディタ）：$1 → ${1}、${1:既定} はそのまま、$0 → ${0}、\$ \} \\ → 文字、
//   選択肢 ${1|a,b|} → ${1:a}、変数（$TM_SELECTED_TEXT など）→ 既定の文字か空、入れ子の欄 → 内側の文字を既定にした 1 つの欄、
//   変換（${1/…/…/}）→ ${1}、既定の文字の中の { } → 外す（エディタの書き方では書けない）
#[cfg(windows)]
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::language::Language;
use crate::storage;

// 前のファイルの名前（Phase 13〜16。Rust だけだった）
const OLD_FILE: &str = "snippets.json";
// 読むファイルの大きさの上限
const MAX_SIZE: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub name: String,
    // 呼び出す文字（1 つ以上）
    pub prefixes: Vec<String>,
    // エディタの書き方に直した本文
    pub template: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Loaded {
    pub snippets: Vec<Snippet>,
    // 読めなかった項目（「名前」と理由）
    pub problems: Vec<String>,
}

// VS Code のスニペットを取り込んだ結果（Phase 13 作業 2。D-187）
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    // 確認のダイアログでキャンセルした
    pub cancelled: bool,
    // 取り込んだスニペットの数と、読めない項目の数（読めない項目も写す。使うときに知らせる）
    pub count: usize,
    pub problems: usize,
    // 前のファイルを退避した先のファイルの名前
    pub backup: Option<String>,
}

// 言語のスニペットのファイルの名前
pub fn file_name(language: Language) -> &'static str {
    match language {
        Language::Rust => "snippets-rust.json",
        Language::Cpp => "snippets-cpp.json",
        Language::Python => "snippets-python.json",
    }
}

// VS Code の、言語のスニペットのファイルの名前（フォルダは vscode_source）
pub fn vscode_file_name(language: Language) -> &'static str {
    match language {
        Language::Rust => "rust.json",
        Language::Cpp => "cpp.json",
        Language::Python => "python.json",
    }
}

// VS Code の言語のスニペットのファイルの場所（Unix）：home の下の Library/Application Support/Code/User/snippets
#[cfg(any(not(windows), test))]
fn unix_vscode_source(home: Option<PathBuf>, language: Language) -> Result<PathBuf, String> {
    let home = home.ok_or_else(|| "ホームのフォルダが分かりません".to_string())?;
    Ok(home
        .join("Library/Application Support/Code/User/snippets")
        .join(vscode_file_name(language)))
}

#[cfg(not(windows))]
pub fn vscode_source(language: Language) -> Result<PathBuf, String> {
    unix_vscode_source(crate::toolchain::home_dir(), language)
}

// VS Code の言語のスニペットのファイルの場所（Windows）：%APPDATA%\Code\User\snippets（段階 3 T4）。
// 環境の値は引数で受け取り、絶対パスのときだけ使う（空の値・相対パスは、作業フォルダの下を指しうるので断る）
#[cfg(windows)]
fn windows_vscode_source(appdata: Option<&OsStr>, language: Language) -> Result<PathBuf, String> {
    let appdata = appdata
        .map(Path::new)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            "VS Code のスニペットの場所が分かりません（環境変数 APPDATA が空か、絶対パスではありません）"
                .to_string()
        })?;
    Ok(appdata
        .join(r"Code\User\snippets")
        .join(vscode_file_name(language)))
}

#[cfg(windows)]
pub fn vscode_source(language: Language) -> Result<PathBuf, String> {
    windows_vscode_source(std::env::var_os("APPDATA").as_deref(), language)
}

// 見つからないときの文（Unix）
#[cfg(any(not(windows), test))]
fn unix_missing_text(language: Language) -> String {
    format!(
        "VS Code の {} のスニペット（~/Library/Application Support/Code/User/snippets/{}）が見つかりません",
        language.label(),
        vscode_file_name(language)
    )
}

#[cfg(not(windows))]
fn missing_text(language: Language, _source: &Path) -> String {
    unix_missing_text(language)
}

// 見つからないときの文（Windows）：実際の場所を出す
#[cfg(windows)]
fn missing_text(language: Language, source: &Path) -> String {
    format!(
        "VS Code の {} のスニペット（{}）が見つかりません",
        language.label(),
        source.display()
    )
}

// 言語のスニペットのファイルの場所。Rust で snippets-rust.json がなく、前の snippets.json があれば、名前を変える
// （ユーザーの指示、2026-10-28。docs/phase16/13-snippets-plan.md）。変えられなければ、前の名前のまま読む
fn path(dir: &Path, language: Language) -> std::path::PathBuf {
    let path = dir.join(file_name(language));
    if language == Language::Rust && fs::symlink_metadata(&path).is_err() {
        let old = dir.join(OLD_FILE);
        if fs::symlink_metadata(&old).is_ok() && fs::rename(&old, &path).is_err() {
            return old;
        }
    }
    path
}

// VS Code のスニペット（source）を、言語のファイルに写す。読めない・大きすぎるファイルは写さない。
// confirm（スニペットの数、読めない項目の数、今のファイルがあるか）で確かめてもらい、OK なら、
// 今のファイルを退避してから、source の中身をそのまま（コメント・並びを保って）書く
pub fn import(
    dir: &Path,
    source: &Path,
    language: Language,
    confirm: impl FnOnce(usize, usize, bool) -> bool,
) -> Result<Imported, String> {
    let metadata = fs::metadata(source).map_err(|_| missing_text(language, source))?;
    if metadata.len() > MAX_SIZE {
        return Err("VS Code のスニペットが 1 MB を超えるので、取り込みません".to_string());
    }
    let text = fs::read_to_string(source)
        .map_err(|_| "VS Code のスニペットを読めません（UTF-8 でないなど）".to_string())?;
    let loaded =
        parse(&text).map_err(|error| format!("VS Code のスニペットを取り込みません：{error}"))?;
    let target = path(dir, language);
    let exists = fs::symlink_metadata(&target).is_ok();
    if !confirm(loaded.snippets.len(), loaded.problems.len(), exists) {
        return Ok(Imported {
            cancelled: true,
            ..Imported::default()
        });
    }
    let backup = if exists {
        let moved = storage::back_up(&target).map_err(|_| {
            format!(
                "今の {} を退避できないので、取り込みません",
                file_name(language)
            )
        })?;
        moved
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    } else {
        None
    };
    storage::write_atomic(&dir.join(file_name(language)), text.as_bytes())?;
    Ok(Imported {
        cancelled: false,
        count: loaded.snippets.len(),
        problems: loaded.problems.len(),
        backup,
    })
}

// 言語のファイルがなければ、空のもの（{ }）を作る（「ファイルを開く」の前）
pub fn ensure_file(dir: &Path, language: Language) -> Result<std::path::PathBuf, String> {
    let path = path(dir, language);
    if fs::symlink_metadata(&path).is_err() {
        storage::write_atomic(&path, b"{\n}\n")?;
    }
    Ok(path)
}

// 言語のファイルの更新日時と大きさ（窓に戻ったときに比べ、変わっていれば読み直す）。なければ None
pub fn stamp(dir: &Path, language: Language) -> Option<(std::time::SystemTime, u64)> {
    let metadata = fs::metadata(path(dir, language)).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

// 言語のファイルを読む。なければ空。大きすぎる・JSON として読めないときは Err
pub fn load(dir: &Path, language: Language) -> Result<Loaded, String> {
    let path = path(dir, language);
    let name = file_name(language);
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Loaded::default()),
        Err(_) => return Err(format!("{name} を読めません")),
    };
    if metadata.len() > MAX_SIZE {
        return Err(format!("{name} が 1 MB を超えるので読みません"));
    }
    let text = fs::read_to_string(&path)
        .map_err(|_| format!("{name} を読めません（UTF-8 でないなど）"))?;
    parse(&text)
        .map_err(|error| format!("{name} を読めません（{error}）。前のスニペットのまま使います"))
}

// ファイルの中身を読む
pub fn parse(text: &str) -> Result<Loaded, String> {
    let value: Value = serde_json::from_str(&strip_jsonc(text))
        .map_err(|error| format!("JSON の誤り：{} 行目", error.line()))?;
    let Value::Object(entries) = value else {
        return Err("一番外側が { } ではありません".to_string());
    };
    let mut loaded = Loaded::default();
    for (name, entry) in entries {
        match parse_entry(&name, &entry) {
            Ok(snippet) => loaded.snippets.push(snippet),
            Err(reason) => loaded.problems.push(format!("「{name}」（{reason}）")),
        }
    }
    Ok(loaded)
}

fn strings(value: Option<&Value>) -> Option<Vec<String>> {
    match value? {
        Value::String(text) => Some(vec![text.clone()]),
        Value::Array(items) => items
            .iter()
            .map(|item| item.as_str().map(str::to_string))
            .collect(),
        _ => None,
    }
}

fn parse_entry(name: &str, entry: &Value) -> Result<Snippet, String> {
    if !entry.is_object() {
        return Err("{ } ではない".to_string());
    }
    let prefixes: Vec<String> = strings(entry.get("prefix"))
        .ok_or("prefix がないか、文字・文字の一覧ではない")?
        .into_iter()
        .filter(|prefix| !prefix.trim().is_empty())
        .collect();
    if prefixes.is_empty() {
        return Err("prefix が空".to_string());
    }
    let body = strings(entry.get("body")).ok_or("body がないか、文字・文字の一覧ではない")?;
    let description = match entry.get("description") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(_) => return Err("description が文字ではない".to_string()),
    };
    Ok(Snippet {
        name: name.to_string(),
        prefixes,
        template: to_template(&body.join("\n")),
        description,
    })
}

// コメント（文字列の外の // と /* */）と、末尾のカンマ（文字列の外の , のあとが空白・コメントだけで } か ]）を除く
pub fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    // 空白とコメントを飛ばした次の位置
    let skip = |mut at: usize| -> usize {
        loop {
            while at < chars.len() && chars[at].is_whitespace() {
                at += 1;
            }
            if chars.get(at) == Some(&'/') && chars.get(at + 1) == Some(&'/') {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                }
            } else if chars.get(at) == Some(&'/') && chars.get(at + 1) == Some(&'*') {
                at += 2;
                while at < chars.len() && !(chars[at] == '*' && chars.get(at + 1) == Some(&'/')) {
                    at += 1;
                }
                at = (at + 2).min(chars.len());
            } else {
                return at;
            }
        }
    };
    while index < chars.len() {
        let c = chars[index];
        if c == '"' {
            // 文字列はそのまま写す（\" で終わらない）
            out.push(c);
            index += 1;
            while index < chars.len() {
                let d = chars[index];
                out.push(d);
                index += 1;
                if d == '\\' {
                    if let Some(&next) = chars.get(index) {
                        out.push(next);
                        index += 1;
                    }
                } else if d == '"' {
                    break;
                }
            }
        } else if c == '/' && matches!(chars.get(index + 1), Some('/') | Some('*')) {
            let next = skip(index);
            // コメントは空白に置き換える（行の数を保ち、誤りの行を合わせる）
            for skipped in &chars[index..next] {
                if *skipped == '\n' {
                    out.push('\n');
                }
            }
            index = next;
        } else if c == ',' && matches!(chars.get(skip(index + 1)), Some('}') | Some(']')) {
            index += 1;
        } else {
            out.push(c);
            index += 1;
        }
    }
    out
}

// VS Code の書き方の本文を、エディタの書き方に直す
pub fn to_template(body: &str) -> String {
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        index = convert_at(&chars, index, &mut out);
    }
    out
}

// 1 つ（文字・エスケープ・欄）を直して out に足し、次の位置を返す
fn convert_at(chars: &[char], index: usize, out: &mut String) -> usize {
    let c = chars[index];
    if c == '\\' {
        if let Some(&next) = chars.get(index + 1)
            && matches!(next, '$' | '}' | '\\')
        {
            push_literal(out, next);
            return index + 2;
        }
        out.push(c);
        return index + 1;
    }
    if c != '$' {
        push_literal(out, c);
        return index + 1;
    }
    // $1・$NAME
    let digits = take_while(chars, index + 1, |c| c.is_ascii_digit());
    if digits > 0 {
        let number: String = chars[index + 1..index + 1 + digits].iter().collect();
        out.push_str(&format!("${{{}}}", trim_number(&number)));
        return index + 1 + digits;
    }
    let word = take_while(chars, index + 1, |c| c.is_ascii_alphanumeric() || c == '_');
    if word > 0 {
        // 変数は値を入れない（空）
        return index + 1 + word;
    }
    if chars.get(index + 1) != Some(&'{') {
        out.push('$');
        return index + 1;
    }
    // ${…}
    let Some(close) = matching_brace(chars, index + 1) else {
        // 閉じていなければ、文字のまま
        out.push('$');
        return index + 1;
    };
    let inner = &chars[index + 2..close];
    let digits = take_while(inner, 0, |c| c.is_ascii_digit());
    if digits > 0 {
        let number = trim_number(&inner[..digits].iter().collect::<String>());
        let rest = &inner[digits..];
        match rest.first() {
            None => out.push_str(&format!("${{{number}}}")),
            Some(':') => {
                let default = plain_text(&rest[1..]);
                if default.is_empty() {
                    out.push_str(&format!("${{{number}}}"));
                } else {
                    out.push_str(&format!("${{{number}:{default}}}"));
                }
            }
            Some('|') => {
                let first = first_choice(&rest[1..]);
                if first.is_empty() {
                    out.push_str(&format!("${{{number}}}"));
                } else {
                    out.push_str(&format!("${{{number}:{first}}}"));
                }
            }
            // 変換（${1/…/…/}）など：欄だけにする
            Some(_) => out.push_str(&format!("${{{number}}}")),
        }
        return close + 1;
    }
    // ${NAME}・${NAME:既定}：既定の文字があれば、それを文字として入れる
    let name = take_while(inner, 0, |c| c.is_ascii_alphanumeric() || c == '_');
    if name > 0 && inner.get(name) == Some(&':') {
        out.push_str(&plain_text(&inner[name + 1..]));
    }
    close + 1
}

// 文字を 1 つ足す。エディタ（CodeMirror）のスニペットは `${…}`・`#{…}` を欄として読み、`\{`・`\}` を `{`・`}` に戻すので、
// 文字のまま残すために、`$`・`#`・`\` のすぐあとの `{` と、`\` のすぐあとの `}` を `\{`・`\}` にする（`\\{` は `\{` と読まれる）
fn push_literal(out: &mut String, c: char) {
    let escape = match c {
        '{' => matches!(out.chars().last(), Some('$' | '#' | '\\')),
        '}' => out.ends_with('\\'),
        _ => false,
    };
    if escape {
        out.push('\\');
    }
    out.push(c);
}

fn take_while(chars: &[char], from: usize, accept: impl Fn(char) -> bool) -> usize {
    chars[from.min(chars.len())..]
        .iter()
        .take_while(|c| accept(**c))
        .count()
}

// 先頭の 0 を除く（$01 → 1。$0 は 0 のまま）
fn trim_number(number: &str) -> String {
    let trimmed = number.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

// open（{ の位置）に対応する } の位置。\} は数えない
fn matching_brace(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut index = open;
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 1,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

// 欄の既定の文字：入れ子の欄・変数は中の文字にし、エスケープを外し、{ } と改行を除く
fn plain_text(chars: &[char]) -> String {
    let mut converted = String::new();
    let mut index = 0;
    while index < chars.len() {
        index = convert_at(chars, index, &mut converted);
    }
    // 入れ子の欄（${2:x} に直ったもの）は、中の文字だけにする
    let mut out = String::new();
    let flat: Vec<char> = converted.chars().collect();
    let mut index = 0;
    while index < flat.len() {
        if flat[index] == '$'
            && flat.get(index + 1) == Some(&'{')
            && let Some(close) = matching_brace(&flat, index + 1)
        {
            let inner: String = flat[index + 2..close].iter().collect();
            if let Some((_, text)) = inner.split_once(':') {
                out.push_str(text);
            }
            index = close + 1;
            continue;
        }
        out.push(flat[index]);
        index += 1;
    }
    // { } は除くので、文字として残すために付けた \（push_literal の `\{`・`\}`）も外す
    out.replace("\\{", "{")
        .replace("\\}", "}")
        .chars()
        .filter(|c| !matches!(c, '{' | '}' | '\n' | '\r'))
        .collect()
}

// 選択肢（a,b|）の最初のもの。\, \| はその文字
fn first_choice(chars: &[char]) -> String {
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '\\' if index + 1 < chars.len() => {
                out.push(chars[index + 1]);
                index += 2;
                continue;
            }
            ',' | '|' => break,
            c => out.push(c),
        }
        index += 1;
    }
    out.chars().filter(|c| !matches!(c, '{' | '}')).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_comments_and_trailing_commas_outside_strings() {
        let text = "{\n  // コメント\n  \"a\": { \"prefix\": \"x // y\", \"body\": [\"1,\", \"/* z */\",], }, /* 末尾 */\n}";
        let stripped = strip_jsonc(text);
        let value: Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(value["a"]["prefix"], "x // y");
        assert_eq!(value["a"]["body"][0], "1,");
        assert_eq!(value["a"]["body"][1], "/* z */");
        // 行の数は変わらない（誤りの行を合わせる）
        assert_eq!(stripped.matches('\n').count(), text.matches('\n').count());
        // 文字列の中の \" で終わらない
        let value: Value = serde_json::from_str(&strip_jsonc(r#"{"a": "q\"//,"}"#)).unwrap();
        assert_eq!(value["a"], "q\"//,");
    }

    #[test]
    fn converts_vscode_syntax() {
        assert_eq!(
            to_template("for $1 in $2 {\n\t$0\n}"),
            "for ${1} in ${2} {\n\t${0}\n}"
        );
        assert_eq!(to_template("let ${1:n} = ${2:0};"), "let ${1:n} = ${2:0};");
        assert_eq!(to_template("${0}"), "${0}");
        assert_eq!(to_template(r"\$x \} \\ a\b"), r"$x } \ a\b");
        assert_eq!(to_template("${1|i64,usize|}"), "${1:i64}");
        assert_eq!(
            to_template("// $TM_FILENAME ${CLIPBOARD} ${TM_SELECTED_TEXT:sel}"),
            "//   sel"
        );
        assert_eq!(to_template("${1:Vec<${2:i64}>}"), "${1:Vec<i64>}");
        assert_eq!(to_template("${1:{x}}"), "${1:x}");
        assert_eq!(to_template("${1/(.*)/${1:/upcase}/}"), "${1}");
        assert_eq!(to_template("${1:}"), "${1}");
        assert_eq!(to_template("$01"), "${1}");
        // 文字としての $・閉じていない ${ は、エディタで文字のまま出る
        assert_eq!(to_template("a $ b ${1"), r"a $ b $\{1");
        // 文字の `${`・`#{`・`\{`・`\}` は、エディタで欄やエスケープとして読まれないようにする
        assert_eq!(
            to_template(r#"println!("\${}", x);"#),
            r#"println!("$\{}", x);"#
        );
        assert_eq!(to_template("let m = #{x};"), r"let m = #\{x};");
        assert_eq!(to_template(r#"r"\\{""#), r#"r"\\{""#);
        assert_eq!(to_template(r"\\\}"), r"\\}");
        assert_eq!(to_template(r"${NAME:a$}{x}"), r"a$\{x}");
        assert_eq!(to_template("${1:x} {}"), "${1:x} {}");
        // 欄の既定の文字では { } を除くので、付けた \ も残さない（文字としての \ は残す）
        assert_eq!(to_template("${1:a#{b}}"), "${1:a#b}");
        assert_eq!(to_template("${NAME:a#{b}}"), "a#b");
        assert_eq!(to_template(r"${1:p\\q}"), r"${1:p\q}");
        assert_eq!(
            to_template("println!(\"{}\", ${1:x});"),
            "println!(\"{}\", ${1:x});"
        );
    }

    #[test]
    fn reads_entries_and_reports_bad_ones() {
        let text = r#"{
            "Read": { "prefix": ["rd", "読む"], "body": ["let ${1:n}: usize = 0;", "$0"], "description": "読む" },
            "One": { "prefix": "one", "body": "1" },
            "NoBody": { "prefix": "nb" },
            "EmptyPrefix": { "prefix": ["", " "], "body": "x" },
            "BadDesc": { "prefix": "bd", "body": "x", "description": 1 },
            "NotObject": 3,
        }"#;
        let mut loaded = parse(text).unwrap();
        // 並びは JSON の読み方による（名前の順か、ファイルの順）ので、名前の順にして比べる
        loaded.snippets.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(
            loaded.snippets,
            vec![
                Snippet {
                    name: "One".to_string(),
                    prefixes: vec!["one".to_string()],
                    template: "1".to_string(),
                    description: None,
                },
                Snippet {
                    name: "Read".to_string(),
                    prefixes: vec!["rd".to_string(), "読む".to_string()],
                    template: "let ${1:n}: usize = 0;\n${0}".to_string(),
                    description: Some("読む".to_string()),
                },
            ]
        );
        assert_eq!(loaded.problems.len(), 4);
        assert!(
            loaded
                .problems
                .iter()
                .any(|problem| problem.starts_with("「NoBody」（body"))
        );
        assert!(parse("{ \"a\": ").unwrap_err().contains("JSON の誤り"));
        assert!(parse("[]").is_err());
    }

    #[test]
    fn imports_with_backup_and_confirmation() {
        let base =
            std::env::temp_dir().join(format!("atcoder-studio-import-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let dir = base.join("app");
        fs::create_dir_all(&dir).unwrap();
        let source = base.join("rust.json");
        // VS Code のファイルがない
        assert!(
            import(&dir, &source, Language::Rust, |_, _, _| true)
                .unwrap_err()
                .contains("見つかりません")
        );
        // 読めない JSON は写さない
        fs::write(&source, "{ \"a\": ").unwrap();
        assert!(import(&dir, &source, Language::Rust, |_, _, _| true).is_err());
        let text = "{\n  // コメント\n  \"a\": { \"prefix\": \"a\", \"body\": \"b\" },\n  \"bad\": { \"prefix\": \"x\" },\n}\n";
        fs::write(&source, text).unwrap();
        // snippets.json がないとき：退避しない
        let mut asked = None;
        let first = import(&dir, &source, Language::Rust, |count, problems, exists| {
            asked = Some((count, problems, exists));
            true
        })
        .unwrap();
        assert_eq!(asked, Some((1, 1, false)));
        assert_eq!(
            (first.count, first.problems, first.backup.clone()),
            (1, 1, None)
        );
        // 中身はそのまま（コメントも残す）
        assert_eq!(
            fs::read_to_string(dir.join(file_name(Language::Rust))).unwrap(),
            text
        );
        // キャンセルなら変えない
        fs::write(dir.join(file_name(Language::Rust)), "{}").unwrap();
        assert!(
            import(&dir, &source, Language::Rust, |_, _, exists| !exists)
                .unwrap()
                .cancelled
        );
        assert_eq!(
            fs::read_to_string(dir.join(file_name(Language::Rust))).unwrap(),
            "{}"
        );
        // あるとき：退避してから写す
        let second = import(&dir, &source, Language::Rust, |_, _, _| true).unwrap();
        let backup = second.backup.unwrap();
        assert!(backup.starts_with("snippets-rust.backup-"), "{backup}");
        assert_eq!(fs::read_to_string(dir.join(&backup)).unwrap(), "{}");
        assert_eq!(
            fs::read_to_string(dir.join(file_name(Language::Rust))).unwrap(),
            text
        );
        // 空のファイルを作るのは、ないときだけ
        fs::remove_file(dir.join(file_name(Language::Rust))).unwrap();
        ensure_file(&dir, Language::Rust).unwrap();
        assert_eq!(
            parse(&fs::read_to_string(dir.join(file_name(Language::Rust))).unwrap()).unwrap(),
            Loaded::default()
        );
        fs::write(dir.join(file_name(Language::Rust)), "keep").unwrap();
        ensure_file(&dir, Language::Rust).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join(file_name(Language::Rust))).unwrap(),
            "keep"
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn loads_from_the_app_folder() {
        let dir =
            std::env::temp_dir().join(format!("atcoder-studio-snippets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // なければ空
        assert_eq!(load(&dir, Language::Rust).unwrap(), Loaded::default());
        fs::write(
            dir.join(file_name(Language::Rust)),
            "{ \"a\": { \"prefix\": \"a\", \"body\": \"b\" } }",
        )
        .unwrap();
        assert_eq!(load(&dir, Language::Rust).unwrap().snippets.len(), 1);
        // 大きすぎるものは読まない
        fs::write(
            dir.join(file_name(Language::Rust)),
            " ".repeat(MAX_SIZE as usize + 1),
        )
        .unwrap();
        assert!(load(&dir, Language::Rust).unwrap_err().contains("1 MB"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_files_per_language_and_renames_the_old_file() {
        let dir = std::env::temp_dir().join(format!(
            "atcoder-studio-snippets-languages-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let entry =
            |prefix: &str| format!("{{ \"a\": {{ \"prefix\": \"{prefix}\", \"body\": \"b\" }} }}");
        // 前の snippets.json は、読むときに snippets-rust.json になる
        fs::write(dir.join("snippets.json"), entry("old")).unwrap();
        let loaded = load(&dir, Language::Rust).unwrap();
        assert_eq!(loaded.snippets[0].prefixes, ["old"]);
        assert!(!dir.join("snippets.json").exists());
        assert_eq!(
            fs::read_to_string(dir.join("snippets-rust.json")).unwrap(),
            entry("old")
        );
        // snippets-rust.json があれば、前のファイルは変えない（読まない）
        fs::write(dir.join("snippets.json"), entry("again")).unwrap();
        assert_eq!(
            load(&dir, Language::Rust).unwrap().snippets[0].prefixes,
            ["old"]
        );
        assert!(dir.join("snippets.json").exists());
        // C++ は別のファイル。Rust のものは出ない
        assert_eq!(load(&dir, Language::Cpp).unwrap(), Loaded::default());
        fs::write(dir.join("snippets-cpp.json"), entry("cin")).unwrap();
        assert_eq!(
            load(&dir, Language::Cpp).unwrap().snippets[0].prefixes,
            ["cin"]
        );
        assert!(stamp(&dir, Language::Cpp).is_some());
        // 読めない JSON は、ファイルの名前つきで知らせる
        fs::write(dir.join("snippets-cpp.json"), "{ \"a\": ").unwrap();
        assert!(
            load(&dir, Language::Cpp)
                .unwrap_err()
                .starts_with("snippets-cpp.json を読めません（JSON の誤り")
        );
        // C++ の取り込みは cpp.json の案内、退避は snippets-cpp.backup-…
        let missing =
            import(&dir, &dir.join("none.json"), Language::Cpp, |_, _, _| true).unwrap_err();
        #[cfg(not(windows))]
        assert!(missing.contains("snippets/cpp.json"), "{missing}");
        #[cfg(windows)]
        assert_windows_missing_text(&missing, &dir.join("none.json"));
        let source = dir.join("cpp.json");
        fs::write(&source, entry("rep")).unwrap();
        let done = import(&dir, &source, Language::Cpp, |_, _, _| true).unwrap();
        assert!(done.backup.unwrap().starts_with("snippets-cpp.backup-"));
        assert_eq!(
            fs::read_to_string(dir.join("snippets-cpp.json")).unwrap(),
            entry("rep")
        );
        // Python も別のファイル（snippets-python.json）。取り込みは python.json の案内、退避は snippets-python.backup-…
        assert_eq!(load(&dir, Language::Python).unwrap(), Loaded::default());
        fs::write(dir.join("snippets-python.json"), entry("inp")).unwrap();
        assert_eq!(
            load(&dir, Language::Python).unwrap().snippets[0].prefixes,
            ["inp"]
        );
        assert_eq!(
            load(&dir, Language::Cpp).unwrap().snippets[0].prefixes,
            ["rep"]
        );
        let missing = import(&dir, &dir.join("none.json"), Language::Python, |_, _, _| {
            true
        })
        .unwrap_err();
        #[cfg(not(windows))]
        assert!(missing.contains("snippets/python.json"), "{missing}");
        #[cfg(windows)]
        assert_windows_missing_text(&missing, &dir.join("none.json"));
        let source = dir.join("python.json");
        fs::write(&source, entry("dfs")).unwrap();
        let done = import(&dir, &source, Language::Python, |_, _, _| true).unwrap();
        assert!(done.backup.unwrap().starts_with("snippets-python.backup-"));
        assert_eq!(
            load(&dir, Language::Python).unwrap().snippets[0].prefixes,
            ["dfs"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    // Unix の経路：場所を組み立てるだけで、ファイルは読まない（Windows のテストでも動く）
    #[test]
    fn unix_reads_from_library_application_support() {
        for (language, name) in [
            (Language::Rust, "rust.json"),
            (Language::Cpp, "cpp.json"),
            (Language::Python, "python.json"),
        ] {
            assert_eq!(
                unix_vscode_source(Some(PathBuf::from("/Users/me")), language).unwrap(),
                Path::new("/Users/me/Library/Application Support/Code/User/snippets").join(name)
            );
        }
    }

    #[test]
    fn unix_needs_the_home_folder() {
        assert_eq!(
            unix_vscode_source(None, Language::Rust),
            Err("ホームのフォルダが分かりません".to_string())
        );
    }

    #[test]
    fn unix_missing_text_stays_the_same() {
        assert_eq!(
            unix_missing_text(Language::Rust),
            "VS Code の Rust のスニペット（~/Library/Application Support/Code/User/snippets/rust.json）が見つかりません"
        );
        assert_eq!(
            unix_missing_text(Language::Cpp),
            "VS Code の C++ のスニペット（~/Library/Application Support/Code/User/snippets/cpp.json）が見つかりません"
        );
        assert_eq!(
            unix_missing_text(Language::Python),
            "VS Code の Python のスニペット（~/Library/Application Support/Code/User/snippets/python.json）が見つかりません"
        );
    }

    // Windows の経路：APPDATA の値は引数で渡し、本物の環境変数は変えない。テストのファイルは一時フォルダに置く。
    // 本物の %APPDATA%\Code\User\snippets と、IDE のフォルダのスニペットは読まない

    // 実際の場所が分かり、macOS のパスが出ない
    #[cfg(windows)]
    fn assert_windows_missing_text(message: &str, source: &Path) {
        assert!(message.contains(&source.display().to_string()), "{message}");
        assert!(message.contains("見つかりません"), "{message}");
        for mac in ["Library", "Application Support", "~/"] {
            assert!(!message.contains(mac), "{message}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_reads_from_appdata_code_user_snippets() {
        // 組み立てるだけで読まないので、この場所はなくてよい
        let appdata = r"C:\Users\me\AppData\Roaming";
        for (language, name) in [
            (Language::Rust, "rust.json"),
            (Language::Cpp, "cpp.json"),
            (Language::Python, "python.json"),
        ] {
            assert_eq!(
                windows_vscode_source(Some(OsStr::new(appdata)), language),
                Ok(PathBuf::from(format!(
                    r"{appdata}\Code\User\snippets\{name}"
                )))
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_refuses_empty_and_relative_appdata() {
        let values = [
            None,
            Some(""),
            Some("."),
            Some("Roaming"),
            Some(r"a\b"),
            Some("C:foo"),
            Some(r"\foo"),
        ];
        for value in values {
            let result = windows_vscode_source(value.map(OsStr::new), Language::Rust);
            let message = result.expect_err(&format!("{value:?}"));
            assert!(message.contains("APPDATA"), "{value:?}：{message}");
            for wrong in ["Library", "ホーム"] {
                assert!(!message.contains(wrong), "{value:?}：{message}");
            }
        }
    }

    // 本物の環境変数の値を読むのは APPDATA だけ（場所を組み立てるだけで、ファイルは読まない）
    #[cfg(windows)]
    #[test]
    fn windows_source_uses_the_appdata_variable() {
        assert_eq!(
            vscode_source(Language::Cpp),
            windows_vscode_source(std::env::var_os("APPDATA").as_deref(), Language::Cpp)
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_imports_from_a_temporary_appdata() {
        let base = std::env::temp_dir().join(format!(
            "atcoder-studio-snippets-appdata-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let appdata = base.join("appdata");
        let app_dir = base.join("app");
        let folder = appdata.join(r"Code\User\snippets");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir_all(&app_dir).unwrap();
        let text = "{ \"a\": { \"prefix\": \"rep\", \"body\": \"b\" } }";
        fs::write(folder.join("cpp.json"), text).unwrap();
        // ある：一時フォルダの appdata から、一時フォルダの app_dir に写す
        let source = windows_vscode_source(Some(appdata.as_os_str()), Language::Cpp).unwrap();
        let done = import(&app_dir, &source, Language::Cpp, |_, _, _| true).unwrap();
        assert_eq!(done.count, 1);
        assert_eq!(
            fs::read_to_string(app_dir.join("snippets-cpp.json")).unwrap(),
            text
        );
        // ない：実際の場所が文に出る
        let source = windows_vscode_source(Some(appdata.as_os_str()), Language::Python).unwrap();
        let missing = import(&app_dir, &source, Language::Python, |_, _, _| true).unwrap_err();
        assert_windows_missing_text(&missing, &source);
        assert!(
            missing.starts_with("VS Code の Python のスニペット（"),
            "{missing}"
        );
        let _ = fs::remove_dir_all(&base);
    }
}
