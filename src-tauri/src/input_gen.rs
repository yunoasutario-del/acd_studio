// 問題文から proconio の input! の中身を作る（Phase 15 作業 1。D-194）
//
// - 問題文（日本語）の「入力」の区画の最初の pre を形式として読む：1 行の値の並び、横の列（A_1 … A_N）、
//   縦の組（X_1 Y_1 / ⋮ / X_N Y_N）、格子（S_{1,1}S_{1,2}…S_{1,W} / ⋮ / S_{H,1}…）。
//   テストケースが複数（T / case_1 … case_T）なら、2 つ目の pre を 1 ケースの形式として読み、main 用と solve 用に分ける。
//   クエリ（query_1 … query_Q）と読めない行は、そこまでを作り、コメントと注意で知らせる
// - 型は「制約」の区画から決める：文字列は Chars、1 文字は char、実数は f64。整数は、負の値がありうれば i64、
//   0 以上と分かれば usize、分からなければ i64（docs/phase15/01-pre-start.md §1・§4.1）
// - 作った形でサンプルの入力を読んでみて、合うかを確かめる
// - C++（Phase 17 作業 8。D-213）では、同じ形を宣言と `cin >>` の行にする。整数はすべて long long、名前は問題文のまま
//   （docs/phase16/11-cpp-input-plan.md）。C++ のファイルへの書き込みは input_cpp.rs

use std::collections::{HashMap, HashSet};

use dom_query::{Document, NodeRef};
use serde::Serialize;

use crate::language::Language;
use crate::problem::Sample;

// <var> の境目（間の空白ではない区切り）
const VAR_BREAK: char = '\u{1}';
// 不等号の目印（制約の読み取りの中だけで使う）
const LESS: char = '\u{2}';
// サンプルを読むときの、配列の長さの上限（誤った長さで長く回らないように）
const MAX_LEN: i128 = 10_000_000;

// 作った結果
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Generated {
    // 作った言語（Rust は proconio の input!、C++ は宣言と cin。D-213）
    pub language: Language,
    // solve の input! { } の中の行（例：`n: usize,`）。C++ なら solve() に書く行（例：`long long N;`）
    pub solve: Vec<String>,
    // テストケースが複数のとき、main の input! { } の中の行（例：`t: usize,`）。C++ なら main() に書く行
    pub main: Option<Vec<String>>,
    // 注意（作れなかった部分・決められなかった型・サンプルと合わないなど）
    pub notes: Vec<String>,
    // 形式を最後まで読めたか（クエリ・読めない行があれば false）
    pub complete: bool,
    pub samples_checked: usize,
    pub samples_matched: usize,
}

// 値の型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ty {
    Usize,
    I64,
    F64,
    Char,
    Chars,
}

impl Ty {
    fn name(self) -> &'static str {
        match self {
            Ty::Usize => "usize",
            Ty::I64 => "i64",
            Ty::F64 => "f64",
            Ty::Char => "char",
            Ty::Chars => "Chars",
        }
    }
}

// 長さ：coef × name + offset（name がなければ offset だけ）。name は小文字にした変数の名前
#[derive(Debug, Clone, PartialEq, Eq)]
struct Len {
    name: Option<String>,
    // 問題文のままの名前（C++ で使う）
    cpp: Option<String>,
    coef: i64,
    offset: i64,
}

impl Len {
    // 添字の式（`N`・`N-1`・`N - 1`・`2N`・`2N+1`・`5`）を読む
    fn parse(text: &str) -> Option<Len> {
        let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        let mut digits = String::new();
        while i < chars.len() && chars[i].is_ascii_digit() {
            digits.push(chars[i]);
            i += 1;
        }
        let mut name = String::new();
        while i < chars.len() && chars[i].is_ascii_alphabetic() {
            name.push(chars[i]);
            i += 1;
        }
        let rest: String = chars[i..].iter().collect();
        if name.is_empty() {
            return if rest.is_empty() && !digits.is_empty() {
                Some(Len {
                    name: None,
                    cpp: None,
                    coef: 0,
                    offset: digits.parse().ok()?,
                })
            } else {
                None
            };
        }
        let coef = if digits.is_empty() {
            1
        } else {
            digits.parse().ok()?
        };
        let offset = if rest.is_empty() {
            0
        } else if let Some(n) = rest.strip_prefix('+') {
            n.parse().ok()?
        } else {
            -rest.strip_prefix('-')?.parse::<i64>().ok()?
        };
        Some(Len {
            name: Some(name.to_ascii_lowercase()),
            cpp: Some(name),
            coef,
            offset,
        })
    }

    // 最初の添字（定数）と最後の添字の式から、数を決める（最後 − 最初 + 1）
    fn count(first: &str, last: &str) -> Option<Len> {
        let first = Len::parse(first)?;
        if first.name.is_some() {
            return None;
        }
        let mut len = Len::parse(last)?;
        len.offset = len.offset - first.offset + 1;
        if len.name.is_none() && len.offset <= 0 {
            return None;
        }
        Some(len)
    }

    fn render(&self) -> String {
        let Some(name) = &self.name else {
            return self.offset.to_string();
        };
        let term = if self.coef == 1 {
            name.clone()
        } else {
            format!("{} * {name}", self.coef)
        };
        match self.offset {
            0 => term,
            k if k > 0 => format!("{term} + {k}"),
            k => format!("{term} - {}", -k),
        }
    }

    fn eval(&self, env: &HashMap<String, i128>) -> Option<i128> {
        let base = match &self.name {
            Some(name) => *env.get(name)? * i128::from(self.coef),
            None => 0,
        };
        Some(base + i128::from(self.offset))
    }
}

// input! の中の 1 つの値の形
#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Value(Ty),
    Tuple(Vec<Ty>),
    Array(Len, Box<Kind>),
}

impl Kind {
    fn render(&self) -> String {
        match self {
            Kind::Value(ty) => ty.name().to_string(),
            Kind::Tuple(tys) => format!(
                "({})",
                tys.iter().map(|t| t.name()).collect::<Vec<_>>().join(", ")
            ),
            Kind::Array(len, inner) => format!("[{}; {}]", inner.render(), len.render()),
        }
    }
}

// 形式の字句
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    // 変数（`A`・`S`・`\text{query}`）と添字
    Name {
        base: String,
        sub: Option<String>,
        word: bool,
    },
    HDots,
    VDots,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
struct Item {
    tok: Tok,
    // 前に空白があるか（格子の行の、間あり・間なしを見分ける）
    spaced: bool,
}

// 波かっこの中身を読む（i は `{` の位置。読んだあとの位置を返す）
fn braced(chars: &[char], mut i: usize) -> (String, usize) {
    let mut depth = 0;
    let mut text = String::new();
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        match c {
            '{' => {
                depth += 1;
                if depth == 1 {
                    continue;
                }
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        text.push(c);
    }
    (text, i)
}

const COMMANDS: [&str; 13] = [
    "ldots",
    "cdots",
    "vdots",
    "ddots",
    "dots",
    "mathrm",
    "mathit",
    "mathbf",
    "mathtt",
    "text",
    "operatorname",
    "qquad",
    "quad",
];

// 形式の 1 行を字句に分ける
fn lex(line: &str) -> Vec<Item> {
    let chars: Vec<char> = line.chars().collect();
    let mut items: Vec<Item> = Vec::new();
    let mut spaced = true;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() || c == '~' {
            spaced = true;
            i += 1;
            continue;
        }
        if c == VAR_BREAK || c == '{' || c == '}' {
            i += 1;
            continue;
        }
        let tok = if c == '\\' {
            i += 1;
            if i < chars.len() && !chars[i].is_ascii_alphabetic() {
                // `\,`・`\ ` などは空白
                i += 1;
                spaced = true;
                continue;
            }
            let start = i;
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            let letters: String = chars[start..i].iter().collect();
            // `\ldotsS` のように続けて書かれていても、知っている名前の分だけ読む
            let command = COMMANDS
                .iter()
                .find(|known| letters.starts_with(*known))
                .copied();
            if let Some(known) = command {
                i = start + known.len();
            }
            match command {
                Some("ldots" | "cdots" | "dots") => Tok::HDots,
                Some("vdots") => Tok::VDots,
                Some("quad" | "qquad") => {
                    spaced = true;
                    continue;
                }
                Some("mathrm" | "mathit" | "mathbf" | "mathtt" | "text" | "operatorname") => {
                    while i < chars.len() && chars[i].is_whitespace() {
                        i += 1;
                    }
                    if i < chars.len() && chars[i] == '{' {
                        let (text, next) = braced(&chars, i);
                        i = next;
                        Tok::Name {
                            base: text.trim().to_string(),
                            sub: None,
                            word: true,
                        }
                    } else {
                        Tok::Other
                    }
                }
                _ => Tok::Other,
            }
        } else if c == '…' {
            i += 1;
            Tok::HDots
        } else if c == '⋮' {
            i += 1;
            Tok::VDots
        } else if c.is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            Tok::Name {
                base: chars[start..i].iter().collect(),
                sub: None,
                word: false,
            }
        } else if c == '_' {
            i += 1;
            let sub = if i < chars.len() && chars[i] == '{' {
                let (text, next) = braced(&chars, i);
                i = next;
                text
            } else if i < chars.len() {
                i += 1;
                chars[i - 1].to_string()
            } else {
                String::new()
            };
            if let Some(Item {
                tok: Tok::Name {
                    sub: slot @ None, ..
                },
                ..
            }) = items.last_mut()
            {
                *slot = Some(sub.trim().to_string());
                continue;
            }
            Tok::Other
        } else {
            i += 1;
            Tok::Other
        };
        items.push(Item { tok, spaced });
        spaced = false;
    }
    items
}

// 名前（小文字。添字が名前の一部のとき〔S_X〕は `s_x`）
fn var_name(base: &str, sub: Option<&str>) -> String {
    let mut name = base.to_ascii_lowercase();
    if let Some(sub) = sub {
        let sub: String = sub
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if !sub.is_empty() {
            name.push('_');
            name.push_str(&sub);
        }
    }
    name
}

// Rust の予約語（strict・reserved。edition 2024 の `gen` を含む）
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
    "where", "while", "abstract", "become", "box", "do", "final", "gen", "macro", "override",
    "priv", "try", "typeof", "unsized", "virtual", "yield",
];

// 名前が予約語なら、末尾に `_` を付ける（`fn` → `fn_`）。長さに使う名前も同じく直す。直した元の名前を renamed に足す
fn avoid_keywords(items: &mut [(String, Kind)], renamed: &mut Vec<String>) {
    fn fix_len(kind: &mut Kind) {
        if let Kind::Array(len, inner) = kind {
            if let Some(name) = &mut len.name
                && KEYWORDS.contains(&name.as_str())
            {
                name.push('_');
            }
            fix_len(inner);
        }
    }
    for (name, kind) in items {
        if KEYWORDS.contains(&name.as_str()) {
            renamed.push(name.clone());
            name.push('_');
        }
        fix_len(kind);
    }
}

fn is_const(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit())
}

// 1 行の中の、同じ変数の並び（A_1 A_2 … A_N）。最初と最後の添字（`,` で分けたもの）を返す
fn sequence(items: &[Item]) -> Option<(String, Vec<String>, Vec<String>, bool)> {
    let dots = items.iter().position(|item| item.tok == Tok::HDots)?;
    if dots == 0 || dots + 2 != items.len() {
        return None;
    }
    let mut base_seen: Option<String> = None;
    let mut subs = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if index == dots {
            continue;
        }
        let Tok::Name {
            base,
            sub: Some(sub),
            word: false,
        } = &item.tok
        else {
            return None;
        };
        if base_seen.as_ref().is_some_and(|seen| seen != base) {
            return None;
        }
        base_seen = Some(base.clone());
        subs.push(
            sub.split(',')
                .map(|s| s.trim().to_string())
                .collect::<Vec<_>>(),
        );
    }
    let first = subs.first()?.clone();
    let last = subs.last()?.clone();
    if subs.iter().any(|s| s.len() != first.len()) {
        return None;
    }
    // 1 つ目と 2 つ目の間に空白があるか（格子の間あり・間なし。`S_{1,1}S_{1,2}\dots S_{1,W}` は間なし）
    let spaced = items[1].spaced;
    Some((base_seen?, first, last, spaced))
}

// 1 行のすべてが、同じ定数の添字の変数（X_1 Y_1）なら、名前と添字
fn indexed_row(items: &[Item]) -> Option<(Vec<(String, bool)>, String)> {
    let mut bases = Vec::new();
    let mut index: Option<String> = None;
    for item in items {
        let Tok::Name {
            base,
            sub: Some(sub),
            word,
        } = &item.tok
        else {
            return None;
        };
        if sub.contains(',') || index.as_ref().is_some_and(|i| i != sub) {
            return None;
        }
        index = Some(sub.clone());
        bases.push((base.clone(), *word));
    }
    Some((bases, index?))
}

// 問題文のままの名前（C++ で使う。D-213）：base と添字。添字は英数字だけを `_` でつなぐ（`S_X`）
fn cpp_name(base: &str, sub: Option<&str>) -> String {
    let mut name = base.to_string();
    if let Some(sub) = sub {
        let sub: String = sub.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        if !sub.is_empty() {
            name.push('_');
            name.push_str(&sub);
        }
    }
    name
}

// 値の並び方（どの値がどの行にあるか。Python の行ごとに読む書き方と、行の分かれ方の確かめに使う。Phase 20 作業 7。D-227）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    // 1 行の値の並び（同じ行の値は、同じ番号）
    Line(usize),
    // 横の列（1 行に全部）
    Row,
    // 縦の並び（1 行に 1 つ、または組 1 つ）
    Column,
    // 格子（1 行に 1 段）
    Grid,
}

// 読んだ形式
#[derive(Debug, Default)]
struct Format {
    items: Vec<(String, Kind)>,
    // items と同じ順の、並び方
    layouts: Vec<Layout>,
    // items と同じ順の、問題文のままの名前（縦に並ぶ組は列ごと。C++ で使う。Rust の名前は小文字なので、別に持つ）
    cpp_names: Vec<Vec<String>>,
    // 型を決めるのに使った変数（制約から決められなかったものを注意に出す）
    used: Vec<String>,
    // 読めなかった（クエリ・読めない行）ときのコメントと注意
    stop: Option<Stop>,
}

#[derive(Debug, PartialEq)]
enum Stop {
    // クエリ（数の変数の名前）
    Query(String),
    // テストケース（数の変数の名前）
    Cases(String),
    // 読めない行（1 始まり）
    Unreadable(usize),
}

fn is_vdots(line: &[Item]) -> bool {
    line.len() == 1 && line[0].tok == Tok::VDots
}

// 形式（pre の行）を読む
fn read_format(lines: &[Vec<Item>], types: &Types) -> Format {
    let mut format = Format::default();
    let ty = |used: &mut Vec<String>, base: &str| {
        used.push(base.to_string());
        types.of(base)
    };
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        // 横の列・格子
        if let Some((base, first, last, spaced)) = sequence(line) {
            if first.len() == 1 {
                let ty = ty(&mut format.used, &base);
                let Some(len) = Len::count(&first[0], &last[0]) else {
                    format.stop = Some(Stop::Unreadable(i + 1));
                    return format;
                };
                format.cpp_names.push(vec![base.clone()]);
                format.items.push((
                    var_name(&base, None),
                    Kind::Array(len, Box::new(Kind::Value(ty))),
                ));
                format.layouts.push(Layout::Row);
                i += 1;
                continue;
            }
            if first.len() == 2 && is_const(&first[0]) {
                // 格子：⋮ までの行は、同じ列の始まりであること（三角形の並びは読まない）
                let mut j = i + 1;
                while j < lines.len() && !is_vdots(&lines[j]) {
                    match sequence(&lines[j]) {
                        Some((b, f, _, _)) if b == base && f.len() == 2 && f[1] == first[1] => {
                            j += 1
                        }
                        _ => break,
                    }
                }
                let last_row = (j < lines.len() && is_vdots(&lines[j]))
                    .then(|| lines.get(j + 1).and_then(|row| sequence(row)))
                    .flatten();
                let rows = last_row.and_then(|(b, f, l, _)| {
                    (b == base && f.len() == 2 && f[1] == first[1] && l[1] == last[1])
                        .then(|| Len::count(&first[0], &l[0]))
                        .flatten()
                });
                let (Some(rows), Some(cols)) = (rows, Len::count(&first[1], &last[1])) else {
                    format.stop = Some(Stop::Unreadable(i + 1));
                    return format;
                };
                let kind = if spaced {
                    let ty = ty(&mut format.used, &base);
                    Kind::Array(rows, Box::new(Kind::Array(cols, Box::new(Kind::Value(ty)))))
                } else {
                    // 間なしの格子は、行ごとに文字の並びとして読む
                    Kind::Array(rows, Box::new(Kind::Value(Ty::Chars)))
                };
                format.cpp_names.push(vec![base.clone()]);
                format.items.push((var_name(&base, None), kind));
                format.layouts.push(Layout::Grid);
                i = j + 2;
                continue;
            }
            format.stop = Some(Stop::Unreadable(i + 1));
            return format;
        }
        // 縦の並び（X_1 Y_1 / ⋮ / X_N Y_N）
        if let Some((bases, first)) = indexed_row(line).filter(|(_, index)| is_const(index)) {
            let mut j = i + 1;
            while j < lines.len() && !is_vdots(&lines[j]) {
                match indexed_row(&lines[j]) {
                    Some((b, index)) if b == bases && is_const(&index) => j += 1,
                    _ => break,
                }
            }
            let count = (j < lines.len() && is_vdots(&lines[j]))
                .then(|| lines.get(j + 1).and_then(|row| indexed_row(row)))
                .flatten()
                .filter(|(b, _)| *b == bases)
                .and_then(|(_, last)| Len::count(&first, &last).map(|len| (len, last)));
            let Some((len, last)) = count else {
                format.stop = Some(Stop::Unreadable(i + 1));
                return format;
            };
            if bases.iter().any(|(_, word)| *word) {
                let word = bases[0].0.to_ascii_lowercase();
                let count_name = var_name(&last, None);
                format.stop = Some(if bases.len() == 1 && word == "query" {
                    Stop::Query(count_name)
                } else if bases.len() == 1 && (word == "case" || word == "testcase") {
                    Stop::Cases(count_name)
                } else {
                    Stop::Unreadable(i + 1)
                });
                return format;
            }
            let kind = if bases.len() == 1 {
                Kind::Value(ty(&mut format.used, &bases[0].0))
            } else {
                Kind::Tuple(
                    bases
                        .iter()
                        .map(|(base, _)| ty(&mut format.used, base))
                        .collect(),
                )
            };
            let name: String = bases
                .iter()
                .map(|(base, _)| base.to_ascii_lowercase())
                .collect();
            format
                .cpp_names
                .push(bases.iter().map(|(base, _)| base.clone()).collect());
            format.items.push((name, Kind::Array(len, Box::new(kind))));
            format.layouts.push(Layout::Column);
            i = j + 2;
            continue;
        }
        // 1 行の値の並び（N K、S_X S_Y）
        let scalars: Option<Vec<(String, Ty, String)>> = line
            .iter()
            .map(|item| match &item.tok {
                Tok::Name {
                    base,
                    sub,
                    word: false,
                } if sub.as_deref().is_none_or(|s| !is_const(s)) => Some((
                    var_name(base, sub.as_deref()),
                    ty(&mut format.used, base),
                    cpp_name(base, sub.as_deref()),
                )),
                _ => None,
            })
            .collect();
        match scalars {
            Some(values) if !values.is_empty() => {
                for (name, ty, cpp) in values {
                    format.items.push((name, Kind::Value(ty)));
                    format.cpp_names.push(vec![cpp]);
                    format.layouts.push(Layout::Line(i));
                }
                i += 1;
            }
            _ => {
                format.stop = Some(Stop::Unreadable(i + 1));
                return format;
            }
        }
    }
    format
}

// 制約から決めた型
#[derive(Debug, Default)]
struct Types {
    // 変数ごとに：文字列・1 文字・実数と書かれているか、整数の下限が負か・0 以上か
    text: HashMap<String, Ty>,
    negative: HashSet<String>,
    nonnegative: HashSet<String>,
}

impl Types {
    fn of(&self, base: &str) -> Ty {
        if let Some(ty) = self.text.get(base) {
            return *ty;
        }
        if self.negative.contains(base) {
            Ty::I64
        } else if self.nonnegative.contains(base) {
            Ty::Usize
        } else {
            Ty::I64
        }
    }

    // 決められずに i64 にしたか（注意に出す）
    fn guessed(&self, base: &str) -> bool {
        !self.text.contains_key(base)
            && !self.negative.contains(base)
            && !self.nonnegative.contains(base)
    }
}

// 文の中に、変数 base が 1 つの名前として出てくるか
fn mentions(text: &str, base: &str) -> bool {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(found) = text[from..].find(base) {
        let start = from + found;
        let end = start + base.len();
        let before = start
            .checked_sub(1)
            .map(|k| bytes[k])
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'\\');
        let after = bytes.get(end).is_some_and(|b| b.is_ascii_alphabetic());
        if !before && !after {
            return true;
        }
        from = end;
    }
    false
}

// 添字を外す（`S_{i,j}` → `S`、`A_i` → `A`）。添字の中の変数（`S_N` の `N`）を、その行の主語と見ないように
fn strip_subscripts(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut result = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '_' && i + 1 < chars.len() {
            i = if chars[i + 1] == '{' {
                braced(&chars, i + 1).1
            } else {
                i + 2
            };
            continue;
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

// 「は」のあとの文から、文字列・1 文字・実数・0 以上の整数・負の値のありうる整数を決める
fn described(tail: &str) -> Option<Result<Ty, bool>> {
    if tail.contains("文字列") {
        return Some(Ok(Ty::Chars));
    }
    if tail.contains("実数") {
        return Some(Ok(Ty::F64));
    }
    if tail.contains("文字") {
        return Some(Ok(Ty::Char));
    }
    if let Some(ty) = choices(tail) {
        return Some(Ok(ty));
    }
    // 整数の範囲：Err(true) は 0 以上、Err(false) は負の値がありうる
    if [
        "正整数",
        "正の整数",
        "非負整数",
        "非負の整数",
        "自然数",
        "順列",
    ]
    .iter()
    .any(|word| tail.contains(word))
    {
        return Some(Err(true));
    }
    if let Some(lower) = tail.split("以上").next().filter(|_| tail.contains("以上")) {
        let lower = lower.trim();
        if lower.contains('-') || lower.contains('−') {
            return Some(Err(false));
        }
        if lower.chars().last().is_some_and(|c| c.is_ascii_digit()) {
            return Some(Err(true));
        }
    }
    None
}

// 下限の式が 0 以上か（Some(true)）、負か（Some(false)）。変数で始まるときは、その変数が 0 以上と分かっていれば 0 以上
fn lower_sign(left: &str, nonnegative: &HashSet<String>) -> Option<bool> {
    let left = left.trim();
    if left.starts_with('-') || left.starts_with('−') {
        return Some(false);
    }
    let first = left.chars().next()?;
    if first.is_ascii_digit() {
        return Some(true);
    }
    let name: String = left
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    (first.is_ascii_alphabetic() && nonnegative.contains(&name)).then_some(true)
}

fn read_types(constraints: &[String], bases: &HashSet<String>) -> Types {
    let mut types = Types::default();
    let lines: Vec<String> = constraints
        .iter()
        .map(|line| strip_subscripts(line))
        .collect();
    for line in &lines {
        // 「S は英小文字からなる文字列」：「は」の前に出てくる変数について
        if let Some(position) = line.find('は') {
            let (head, tail) = line.split_at(position);
            let tail = &tail['は'.len_utf8()..];
            for base in bases.iter().filter(|base| mentions(head, base)) {
                match described(tail) {
                    Some(Ok(ty)) => {
                        types.text.entry(base.clone()).or_insert(ty);
                    }
                    Some(Err(true)) => {
                        types.nonnegative.insert(base.clone());
                    }
                    Some(Err(false)) => {
                        types.negative.insert(base.clone());
                    }
                    None => {}
                }
            }
        }
        // 「t \in \lbrace 1,2 \rbrace」：集合の数が負か
        if let Some(position) = line
            .find("\\in")
            .filter(|p| !line[p + 3..].starts_with(|c: char| c.is_ascii_alphabetic()))
        {
            let (left, right) = line.split_at(position);
            let sign = if right.contains('-') || right.contains('−') {
                Some(false)
            } else if right.chars().any(|c| c.is_ascii_digit()) {
                Some(true)
            } else {
                None
            };
            for base in bases.iter().filter(|base| mentions(left, base)) {
                match sign {
                    Some(true) => types.nonnegative.insert(base.clone()),
                    Some(false) => types.negative.insert(base.clone()),
                    None => false,
                };
            }
        }
        // 「|A| \leq 10^9」は負の値がありうる
        for base in bases {
            if line.contains(&format!("|{base}")) {
                types.negative.insert(base.clone());
            }
        }
    }
    // 「-10^9 \leq A_i \leq 10^9」：一番左の下限が負か、0 以上か。左端が変数のとき（N-1 \leq M）に、
    // その変数が先に決まっているとよいので 2 回読む。かっこの中（(1 \leq i \leq N)）は別の不等式として読む
    for _ in 0..2 {
        for line in &lines {
            let marked = line
                .replace("\\leqq", &LESS.to_string())
                .replace("\\leq", &LESS.to_string())
                .replace("\\lt", &LESS.to_string())
                .replace("\\le", &LESS.to_string())
                .replace(['≦', '≤', '<'], &LESS.to_string());
            for chain in marked.split(['(', ')', '（', '）', '．', '。']) {
                let segments: Vec<&str> = chain.split(LESS).collect();
                if segments.len() < 2 {
                    continue;
                }
                let left = segments[0]
                    .rsplit([',', '、'])
                    .next()
                    .unwrap_or(segments[0]);
                let Some(nonnegative) = lower_sign(left, &types.nonnegative) else {
                    continue;
                };
                for base in bases {
                    if segments[1..].iter().any(|segment| mentions(segment, base)) {
                        if nonnegative {
                            types.nonnegative.insert(base.clone());
                        } else {
                            types.negative.insert(base.clone());
                        }
                    }
                }
            }
        }
    }
    types
}

// 「L, R, U, D のいずれか」「. または # のいずれか」「Yes と No のいずれか」：数字でない選択肢だけなら、
// どれも 1 文字なら char、ほかは Chars
fn choices(tail: &str) -> Option<Ty> {
    let list = tail
        .split("のいずれか")
        .next()
        .filter(|_| tail.contains("のいずれか"))?;
    let choices: Vec<&str> = list
        .split([',', '、'])
        .flat_map(|part| part.split("または"))
        .flat_map(|part| part.split('と'))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if choices.is_empty()
        || choices
            .iter()
            .any(|c| c.chars().any(|c| c.is_ascii_digit()))
    {
        return None;
    }
    Some(if choices.iter().all(|c| c.chars().count() == 1) {
        Ty::Char
    } else {
        Ty::Chars
    })
}

// pre の中身を行に分ける（<var> の境目に VAR_BREAK を入れる）
fn pre_lines(pre: &NodeRef) -> Vec<Vec<Item>> {
    let mut text = String::new();
    for child in pre.children() {
        if child.is_text() {
            text.push_str(&child.text());
        } else {
            text.push(VAR_BREAK);
            text.push_str(&child.text());
            text.push(VAR_BREAK);
        }
    }
    text.lines()
        .map(lex)
        .filter(|items| !items.is_empty())
        .collect()
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// 形式の変数の名前（制約で探すもの）
fn bases_of(lines: &[Vec<Item>]) -> HashSet<String> {
    lines
        .iter()
        .flatten()
        .filter_map(|item| match &item.tok {
            Tok::Name {
                base, word: false, ..
            } => Some(base.clone()),
            _ => None,
        })
        .collect()
}

// サンプルの入力を、作った形で読んでみる
fn read_value(
    kind: &Kind,
    tokens: &mut std::slice::Iter<&str>,
    env: &HashMap<String, i128>,
    name: &str,
) -> Result<Option<i128>, String> {
    match kind {
        Kind::Value(ty) => {
            let token = tokens
                .next()
                .ok_or_else(|| format!("値が足りません（{name}）"))?;
            let bad = || format!("{name} を {} として読めません（「{token}」）", ty.name());
            match ty {
                Ty::Usize => token
                    .parse::<u64>()
                    .map(|v| Some(i128::from(v)))
                    .map_err(|_| bad()),
                Ty::I64 => token
                    .parse::<i64>()
                    .map(|v| Some(i128::from(v)))
                    .map_err(|_| bad()),
                Ty::F64 => token.parse::<f64>().map(|_| None).map_err(|_| bad()),
                Ty::Char => (token.chars().count() == 1).then_some(None).ok_or_else(bad),
                Ty::Chars => Ok(None),
            }
        }
        Kind::Tuple(tys) => {
            for ty in tys {
                read_value(&Kind::Value(*ty), tokens, env, name)?;
            }
            Ok(None)
        }
        Kind::Array(len, inner) => {
            let count = len
                .eval(env)
                .ok_or_else(|| format!("{name} の長さ（{}）が決まりません", len.render()))?;
            if !(0..=MAX_LEN).contains(&count) {
                return Err(format!(
                    "{name} の長さ（{}）が {count} になります",
                    len.render()
                ));
            }
            for _ in 0..count {
                read_value(inner, tokens, env, name)?;
            }
            Ok(None)
        }
    }
}

fn read_items(
    items: &[(String, Kind)],
    tokens: &mut std::slice::Iter<&str>,
) -> Result<HashMap<String, i128>, String> {
    let mut env = HashMap::new();
    for (name, kind) in items {
        if let Some(value) = read_value(kind, tokens, &env, name)? {
            env.insert(name.clone(), value);
        }
    }
    Ok(env)
}

fn check_sample(
    main: Option<&[(String, Kind)]>,
    solve: &[(String, Kind)],
    complete: bool,
    input: &str,
) -> Result<(), String> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let mut iter = tokens.iter();
    match main {
        Some(main) => {
            let env = read_items(main, &mut iter)?;
            let cases = main
                .first()
                .and_then(|(name, _)| env.get(name))
                .copied()
                .unwrap_or(0);
            if !(0..=MAX_LEN).contains(&cases) {
                return Err(format!("テストケースの数が {cases} になります"));
            }
            for _ in 0..cases {
                read_items(solve, &mut iter)?;
            }
        }
        None => {
            read_items(solve, &mut iter)?;
        }
    }
    let rest = iter.len();
    if complete && rest > 0 {
        return Err(format!("値が {rest} 個余ります"));
    }
    Ok(())
}

// 入力例を、行の分かれ方まで確かめる（Python。D-227）。1 行の値・横の列は 1 行、縦の並びは長さだけの行、
// 格子は段の数だけの行で、どの行も値が余らない・足りないこと
struct Lines<'a> {
    lines: Vec<Vec<&'a str>>,
    next: usize,
}

impl<'a> Lines<'a> {
    fn new(input: &'a str) -> Self {
        let mut lines: Vec<Vec<&str>> = input
            .lines()
            .map(|line| line.split_whitespace().collect())
            .collect();
        while lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
        Lines { lines, next: 0 }
    }

    // 次の 1 行を、read で読み切る
    fn line<T>(
        &mut self,
        read: impl FnOnce(&mut std::slice::Iter<&str>) -> Result<T, String>,
    ) -> Result<T, String> {
        let number = self.next + 1;
        let tokens = self
            .lines
            .get(self.next)
            .ok_or_else(|| format!("{number} 行目がありません"))?;
        self.next += 1;
        let mut iter = tokens.iter();
        let value = read(&mut iter).map_err(|reason| format!("{number} 行目：{reason}"))?;
        let rest = iter.len();
        if rest > 0 {
            return Err(format!("{number} 行目：値が {rest} 個余ります"));
        }
        Ok(value)
    }

    fn items(
        &mut self,
        items: &[(String, Kind)],
        layouts: &[Layout],
    ) -> Result<HashMap<String, i128>, String> {
        let mut env = HashMap::new();
        let mut i = 0;
        while i < items.len() {
            match layouts.get(i).copied().unwrap_or(Layout::Row) {
                Layout::Line(line) => {
                    let end = (i..items.len())
                        .find(|&j| layouts.get(j) != Some(&Layout::Line(line)))
                        .unwrap_or(items.len());
                    let read = self.line(|tokens| {
                        let mut values = Vec::new();
                        for (name, kind) in &items[i..end] {
                            values.push((name.clone(), read_value(kind, tokens, &env, name)?));
                        }
                        Ok(values)
                    })?;
                    for (name, value) in read {
                        if let Some(value) = value {
                            env.insert(name, value);
                        }
                    }
                    i = end;
                    continue;
                }
                Layout::Row => {
                    let (name, kind) = &items[i];
                    self.line(|tokens| read_value(kind, tokens, &env, name))?;
                }
                Layout::Column | Layout::Grid => {
                    let (name, kind) = &items[i];
                    let Kind::Array(len, inner) = kind else {
                        return Err(format!("{name} の形を読めません"));
                    };
                    let count = len
                        .eval(&env)
                        .filter(|count| (0..=MAX_LEN).contains(count))
                        .ok_or_else(|| {
                            format!("{name} の長さ（{}）が決まりません", len.render())
                        })?;
                    for _ in 0..count {
                        self.line(|tokens| read_value(inner, tokens, &env, name))?;
                    }
                }
            }
            i += 1;
        }
        Ok(env)
    }
}

fn check_sample_lines(
    main: Option<&[(String, Kind)]>,
    solve: &[(String, Kind)],
    layouts: &[Layout],
    complete: bool,
    input: &str,
) -> Result<(), String> {
    let mut lines = Lines::new(input);
    match main {
        Some(main) => {
            let env = lines.items(main, &[Layout::Line(0)])?;
            let cases = main
                .first()
                .and_then(|(name, _)| env.get(name))
                .copied()
                .unwrap_or(0);
            if !(0..=MAX_LEN).contains(&cases) {
                return Err(format!("テストケースの数が {cases} になります"));
            }
            for _ in 0..cases {
                lines.items(solve, layouts)?;
            }
        }
        None => {
            lines.items(solve, layouts)?;
        }
    }
    let rest = lines.lines.len() - lines.next;
    if complete && rest > 0 {
        return Err(format!("行が {rest} 行余ります"));
    }
    Ok(())
}

// 1 文字の格子（[[char; w]; h]）を、行ごとの文字列（[Chars; h]）にしたもの。なければ None
fn char_grid_as_rows(items: &[(String, Kind)]) -> Option<Vec<(String, Kind)>> {
    let mut changed = false;
    let rows = items
        .iter()
        .map(|(name, kind)| match kind {
            Kind::Array(rows, inner)
                if matches!(&**inner, Kind::Array(_, cell) if **cell == Kind::Value(Ty::Char)) =>
            {
                changed = true;
                (
                    name.clone(),
                    Kind::Array(rows.clone(), Box::new(Kind::Value(Ty::Chars))),
                )
            }
            _ => (name.clone(), kind.clone()),
        })
        .collect();
    changed.then_some(rows)
}

fn render(items: &[(String, Kind)]) -> Vec<String> {
    items
        .iter()
        .map(|(name, kind)| format!("{name}: {},", kind.render()))
        .collect()
}

// 問題文（日本語の HTML）とサンプルから、input! の中身を作る（テストで使う。ほかは generate_for で言語を選ぶ）
#[cfg(test)]
pub fn generate(statement_html: &str, samples: &[Sample]) -> Result<Generated, String> {
    generate_for(statement_html, samples, Language::Rust)
}

// language の書き方で作る（C++ は宣言と cin の行。D-213。Python は行ごとに input() で読む行。D-227）
pub fn generate_for(
    statement_html: &str,
    samples: &[Sample],
    language: Language,
) -> Result<Generated, String> {
    let document = Document::from(statement_html);
    let mut pres: Vec<Vec<Vec<Item>>> = Vec::new();
    let mut constraints: Vec<String> = Vec::new();
    for section in document.select("section").iter() {
        let heading = squash(section.select("h3").first().text().as_ref());
        if heading == "入力" && pres.is_empty() {
            pres = section
                .select("pre")
                .nodes()
                .iter()
                .map(pre_lines)
                .collect();
        } else if heading == "制約" && constraints.is_empty() {
            constraints = section
                .select("li")
                .iter()
                .map(|li| squash(li.text().as_ref()))
                .collect();
        }
    }
    let Some(first) = pres.first() else {
        return Err("問題文に「入力」の形式が見つかりません".to_string());
    };
    let mut bases = bases_of(first);
    if let Some(second) = pres.get(1) {
        bases.extend(bases_of(second));
    }
    let types = read_types(&constraints, &bases);
    let mut notes = Vec::new();

    let top = read_format(first, &types);
    // テストケースが複数：1 行目の数と case の並びだけなら、2 つ目の pre を 1 ケースの形式として読む
    let (mut main, mut body, place) = match &top.stop {
        Some(Stop::Cases(count))
            if top.items.len() == 1 && top.items[0].0 == *count && pres.len() >= 2 =>
        {
            let main = vec![(count.clone(), Kind::Value(Ty::Usize))];
            (Some(main), read_format(&pres[1], &types), "1 ケースの形式")
        }
        _ => (None, top, "入力の形式"),
    };
    let complete = body.stop.is_none();
    // 入力例と合うか。Python は行の分かれ方も確かめる（D-227）
    let layouts = body.layouts.clone();
    let check = |main: Option<&[(String, Kind)]>, items: &[(String, Kind)], input: &str| {
        if language == Language::Python {
            check_sample_lines(main, items, &layouts, complete, input)
        } else {
            check_sample(main, items, complete, input)
        }
    };
    // 形式では間ありの 1 文字の格子が、入力例では間なし（行ごとの文字列）のときは、入力例に合わせる
    let matches_all = |items: &[(String, Kind)]| {
        !samples.is_empty()
            && samples
                .iter()
                .all(|s| check(main.as_deref(), items, &s.input).is_ok())
    };
    if let Some(rows) = char_grid_as_rows(&body.items)
        && !matches_all(&body.items)
        && matches_all(&rows)
    {
        body.items = rows;
        notes.push(format!(
            "1 文字の格子は、入力例に合わせて行ごとの文字列（{}）で読みます",
            match language {
                Language::Rust => "Chars",
                Language::Cpp | Language::Python => "string",
            }
        ));
    }
    // C++ の名前（問題文のまま）。長さ・テストケースの数の名前は、形式に出てくる変数の名前から戻す
    let keywords = match language {
        Language::Python => PYTHON_KEYWORDS,
        Language::Rust | Language::Cpp => CPP_KEYWORDS,
    };
    let mut cpp = CppNames::new(main.as_deref(), &body.cpp_names, &bases, keywords);
    let mut renamed = Vec::new();
    let (mut solve, main_lines) = match language {
        Language::Rust => {
            if let Some(main) = &mut main {
                avoid_keywords(main, &mut renamed);
            }
            avoid_keywords(&mut body.items, &mut renamed);
            (render(&body.items), main.as_deref().map(render))
        }
        Language::Cpp => {
            renamed = cpp.avoid_keywords();
            (
                cpp.render(&body.items, &cpp.solve),
                main.as_deref().map(|items| cpp.render(items, &cpp.main)),
            )
        }
        Language::Python => {
            renamed = cpp.avoid_keywords();
            (
                cpp.render_python(&body.items, &cpp.solve, &body.layouts),
                main.as_deref()
                    .map(|items| cpp.render_python(items, &cpp.main, &[Layout::Line(0)])),
            )
        }
    };
    // コメントの書き方
    let comment = match language {
        Language::Python => "#",
        Language::Rust | Language::Cpp => "//",
    };
    if !renamed.is_empty() {
        notes.push(format!(
            "{} の予約語と同じ名前は、末尾に _ を付けました：{}",
            language.label(),
            renamed
                .iter()
                .map(|name| format!("{name} → {name}_"))
                .collect::<Vec<_>>()
                .join("、")
        ));
    }
    match &body.stop {
        None => {}
        Some(Stop::Query(count)) => {
            solve.push(format!(
                "{comment} クエリ（{count} 個）は形が決まらないので、ここでは読んでいません"
            ));
            notes.push(format!(
                "クエリ（{count} 個）は読んでいません。クエリごとに {} で読んでください",
                match language {
                    Language::Rust => "input!",
                    Language::Cpp => "cin",
                    Language::Python => "input()",
                }
            ));
        }
        Some(Stop::Cases(_)) => {
            solve.push(format!("{comment} テストケースの形式を読めませんでした"));
            notes.push("テストケースの形式を読めませんでした".to_string());
        }
        Some(Stop::Unreadable(line)) => {
            solve.push(format!(
                "{comment} {place}の {line} 行目から先は読めませんでした"
            ));
            notes.push(format!("{place}の {line} 行目から先は読めませんでした"));
        }
    }
    if body.items.is_empty() && main.is_none() {
        return Err(notes
            .pop()
            .unwrap_or_else(|| "入力の形式を読めませんでした".to_string()));
    }

    // 同じ名前（C++ は問題文のままの名前。縦に並ぶ組は列ごと）
    let declared: Vec<String> = match language {
        Language::Rust => main
            .iter()
            .flatten()
            .chain(&body.items)
            .map(|(name, _)| name.clone())
            .collect(),
        Language::Cpp | Language::Python => cpp
            .main
            .iter()
            .chain(&cpp.solve)
            .flatten()
            .cloned()
            .collect(),
    };
    let mut seen = HashSet::new();
    for name in declared {
        if !seen.insert(name.clone()) {
            notes.push(format!(
                "名前 {name} が 2 度出てきます。片方の名前を変えてください"
            ));
        }
    }
    let mut guessed: Vec<String> = body
        .used
        .iter()
        .filter(|base| types.guessed(base))
        .map(|base| base.to_ascii_lowercase())
        .collect();
    guessed.sort();
    guessed.dedup();
    // C++ では整数はすべて long long なので、決められなかったことは知らせない
    if !guessed.is_empty() && language == Language::Rust {
        notes.push(format!(
            "制約から型を決められなかったので i64 にしました：{}",
            guessed.join("、")
        ));
    }

    let mut matched = 0;
    for (index, sample) in samples.iter().enumerate() {
        match check(main.as_deref(), &body.items, &sample.input) {
            Ok(()) => matched += 1,
            Err(reason) => notes.push(format!("入力例 {} と合いません（{reason}）", index + 1)),
        }
    }
    Ok(Generated {
        language,
        solve,
        main: main_lines,
        notes,
        complete,
        samples_checked: samples.len(),
        samples_matched: matched,
    })
}

// ---- C++ の書き方（Phase 17 作業 8。D-213） ----

// C++ の予約語（C++23。代わりの綴り `and` などを含む）
const CPP_KEYWORDS: &[&str] = &[
    "alignas",
    "alignof",
    "and",
    "and_eq",
    "asm",
    "auto",
    "bitand",
    "bitor",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "char8_t",
    "char16_t",
    "char32_t",
    "class",
    "compl",
    "concept",
    "const",
    "consteval",
    "constexpr",
    "constinit",
    "const_cast",
    "continue",
    "co_await",
    "co_return",
    "co_yield",
    "decltype",
    "default",
    "delete",
    "do",
    "double",
    "dynamic_cast",
    "else",
    "enum",
    "explicit",
    "export",
    "extern",
    "false",
    "float",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "not",
    "not_eq",
    "nullptr",
    "operator",
    "or",
    "or_eq",
    "private",
    "protected",
    "public",
    "register",
    "reinterpret_cast",
    "requires",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "static_assert",
    "static_cast",
    "struct",
    "switch",
    "template",
    "this",
    "thread_local",
    "throw",
    "true",
    "try",
    "typedef",
    "typeid",
    "typename",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "wchar_t",
    "while",
    "xor",
    "xor_eq",
];

// 型：整数はすべて long long（ユーザーの選択。docs/phase16/11-cpp-input-plan.md §3 の 1）
fn cpp_type(ty: Ty) -> &'static str {
    match ty {
        Ty::Usize | Ty::I64 => "long long",
        Ty::F64 => "double",
        Ty::Char => "char",
        Ty::Chars => "string",
    }
}

// Python の予約語（ソフトキーワードを含む）と、作るコードで使う組み込みの名前（同じ名前の変数を作ると、
// あとの行で使えなくなる。D-227）
const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "case", "class",
    "continue", "def", "del", "elif", "else", "except", "finally", "float", "for", "from",
    "global", "if", "import", "in", "input", "int", "is", "lambda", "list", "map", "match",
    "nonlocal", "not", "or", "pass", "raise", "range", "return", "try", "type", "while", "with",
    "yield",
];

// C++・Python で使う名前（問題文のまま。D-213・D-227）
struct CppNames {
    // main()・solve() で読む値の、問題文のままの名前（items と同じ順。縦に並ぶ組は列ごと）
    main: Vec<Vec<String>>,
    solve: Vec<Vec<String>>,
    // 長さ・テストケースの数に出てくる名前（小文字）→ 問題文のままの名前
    lens: HashMap<String, String>,
    // 末尾に `_` を付ける名前（その言語の予約語など）
    keywords: &'static [&'static str],
}

impl CppNames {
    // main は main() で読む値（テストケースの数。名前は小文字なので、形式の変数の名前から戻す）
    fn new(
        main: Option<&[(String, Kind)]>,
        solve: &[Vec<String>],
        bases: &HashSet<String>,
        keywords: &'static [&'static str],
    ) -> CppNames {
        let mut sorted: Vec<&String> = bases.iter().collect();
        sorted.sort();
        let mut lens = HashMap::new();
        for base in sorted {
            lens.entry(base.to_ascii_lowercase())
                .or_insert_with(|| base.clone());
        }
        let main = main
            .unwrap_or_default()
            .iter()
            .map(|(name, _)| vec![lens.get(name).cloned().unwrap_or_else(|| name.clone())])
            .collect();
        CppNames {
            main,
            solve: solve.to_vec(),
            lens,
            keywords,
        }
    }

    // 予約語と同じ名前の末尾に `_` を付ける。直した元の名前を返す
    fn avoid_keywords(&mut self) -> Vec<String> {
        let mut renamed = Vec::new();
        let all = self
            .main
            .iter_mut()
            .chain(self.solve.iter_mut())
            .flatten()
            .chain(self.lens.values_mut());
        let keywords = self.keywords;
        for name in all {
            if keywords.contains(&name.as_str()) {
                if !renamed.contains(name) {
                    renamed.push(name.clone());
                }
                name.push('_');
            }
        }
        renamed.sort();
        renamed
    }

    fn len(&self, len: &Len) -> String {
        let Some(name) = len.cpp.clone().or_else(|| len.name.clone()) else {
            return len.offset.to_string();
        };
        let name = if self.keywords.contains(&name.as_str()) {
            format!("{name}_")
        } else {
            name
        };
        let term = if len.coef == 1 {
            name
        } else {
            format!("{} * {name}", len.coef)
        };
        match len.offset {
            0 => term,
            k if k > 0 => format!("{term} + {k}"),
            k => format!("{term} - {}", -k),
        }
    }

    // ループの変数の名前：候補のうち、問題の変数の名前・予約語とぶつからない最初のもの
    fn fresh(&self, candidates: &[String]) -> String {
        let taken = |name: &str| {
            self.keywords.contains(&name)
                || self
                    .main
                    .iter()
                    .chain(&self.solve)
                    .flatten()
                    .any(|used| used == name)
                || self.lens.values().any(|used| used == name)
        };
        candidates
            .iter()
            .find(|name| !taken(name))
            .cloned()
            .unwrap_or_else(|| {
                (1..)
                    .map(|n| format!("v{n}"))
                    .find(|name| !taken(name))
                    .unwrap_or_default()
            })
    }

    fn element(&self, array: &str) -> String {
        let lower = array.to_ascii_lowercase();
        let mut candidates = Vec::new();
        if lower != array {
            candidates.push(lower);
        }
        candidates.extend(["x", "v", "e"].map(String::from));
        self.fresh(&candidates)
    }

    // 続けて同じ型なら 1 つにまとめた宣言（`long long N, M;`）。items は (名前、長さを付けたもの, 型)
    fn declarations(items: &[(String, String)]) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let mut last_ty: Option<&str> = None;
        for (name, ty) in items {
            if last_ty == Some(ty.as_str())
                && let Some(line) = lines.last_mut()
            {
                line.pop();
                line.push_str(&format!(", {name};"));
            } else {
                lines.push(format!("{ty} {name};"));
            }
            last_ty = Some(ty);
        }
        lines
    }

    // items を読む行。names は items と同じ順の、問題文のままの名前
    fn render(&self, items: &[(String, Kind)], names: &[Vec<String>]) -> Vec<String> {
        let of = |index: usize| -> Vec<String> {
            names
                .get(index)
                .cloned()
                .unwrap_or_else(|| vec![items[index].0.clone()])
        };
        let one = |index: usize| of(index).join("_");
        let mut lines = Vec::new();
        let mut i = 0;
        while i < items.len() {
            // 続く 1 つの値は、まとめて宣言して 1 つの cin で読む
            let mut values = Vec::new();
            while let Some((_, Kind::Value(ty))) = items.get(i) {
                values.push((one(i), cpp_type(*ty).to_string()));
                i += 1;
            }
            if !values.is_empty() {
                lines.extend(CppNames::declarations(&values));
                let names: Vec<&str> = values.iter().map(|(name, _)| name.as_str()).collect();
                lines.push(format!("cin >> {};", names.join(" >> ")));
                continue;
            }
            let index = i;
            let (name, kind) = &items[i];
            i += 1;
            let Kind::Array(len, inner) = kind else {
                lines.push(format!("// {name} は読めない形です"));
                continue;
            };
            let len = self.len(len);
            match &**inner {
                Kind::Value(ty) => {
                    let array = one(index);
                    let element = self.element(&array);
                    lines.push(format!("vector<{}> {array}({len});", cpp_type(*ty)));
                    lines.push(format!("for (auto& {element} : {array}) cin >> {element};"));
                }
                Kind::Tuple(tys) => {
                    let columns = of(index);
                    let index = self.fresh(&["i", "j", "k"].map(String::from));
                    let typed: Vec<(String, String)> = columns
                        .iter()
                        .zip(tys)
                        .map(|(column, ty)| {
                            (
                                format!("{column}({len})"),
                                format!("vector<{}>", cpp_type(*ty)),
                            )
                        })
                        .collect();
                    lines.extend(CppNames::declarations(&typed));
                    let reads: Vec<String> = columns
                        .iter()
                        .map(|column| format!("{column}[{index}]"))
                        .collect();
                    lines.push(format!(
                        "for (int {index} = 0; {index} < {len}; {index}++) cin >> {};",
                        reads.join(" >> ")
                    ));
                }
                Kind::Array(cols, cell) if matches!(&**cell, Kind::Value(_)) => {
                    let Kind::Value(ty) = &**cell else {
                        unreachable!()
                    };
                    let array = one(index);
                    let cols = self.len(cols);
                    let row = self.fresh(&["row", "r"].map(String::from));
                    let element = self.element(&array);
                    let ty = cpp_type(*ty);
                    lines.push(format!(
                        "vector<vector<{ty}>> {array}({len}, vector<{ty}>({cols}));"
                    ));
                    lines.push(format!(
                        "for (auto& {row} : {array}) for (auto& {element} : {row}) cin >> {element};"
                    ));
                }
                _ => lines.push(format!("// {name} は読めない形です")),
            }
        }
        lines
    }

    // Python：items を行ごとに読む行（D-227）。names は items と同じ順の、問題文のままの名前、layouts は並び方
    fn render_python(
        &self,
        items: &[(String, Kind)],
        names: &[Vec<String>],
        layouts: &[Layout],
    ) -> Vec<String> {
        let of = |index: usize| -> Vec<String> {
            names
                .get(index)
                .cloned()
                .unwrap_or_else(|| vec![items[index].0.clone()])
        };
        let one = |index: usize| of(index).join("_");
        let mut lines = Vec::new();
        let mut i = 0;
        while i < items.len() {
            let layout = layouts.get(i).copied().unwrap_or(Layout::Row);
            // 同じ行の値は、まとめて 1 行で読む
            if let Layout::Line(line) = layout {
                let mut values = Vec::new();
                while let (Some((_, Kind::Value(ty))), Some(Layout::Line(at))) =
                    (items.get(i), layouts.get(i))
                {
                    if *at != line {
                        break;
                    }
                    values.push((one(i), *ty));
                    i += 1;
                }
                if values.is_empty() {
                    lines.push(format!("# {} は読めない形です", one(i)));
                    i += 1;
                    continue;
                }
                lines.extend(python_line(&values));
                continue;
            }
            let index = i;
            i += 1;
            let Kind::Array(len, inner) = &items[index].1 else {
                lines.push(format!("# {} は読めない形です", one(index)));
                continue;
            };
            let len = self.len(len);
            match (layout, &**inner) {
                (Layout::Row, Kind::Value(ty)) => {
                    lines.push(format!("{} = {}", one(index), python_split(*ty)));
                }
                (Layout::Column, Kind::Value(ty)) => {
                    lines.push(format!(
                        "{} = [{} for _ in range({len})]",
                        one(index),
                        python_convert(*ty, "input()")
                    ));
                }
                (Layout::Column, Kind::Tuple(tys)) => {
                    let columns = of(index);
                    let row = self.fresh(&["i", "j", "k"].map(String::from));
                    for (column, ty) in columns.iter().zip(tys) {
                        let zero = if python_number(*ty) { "0" } else { "\"\"" };
                        lines.push(format!("{column} = [{zero}] * {len}"));
                    }
                    lines.push(format!("for {row} in range({len}):"));
                    let targets: Vec<(String, Ty)> = columns
                        .iter()
                        .zip(tys)
                        .map(|(column, ty)| (format!("{column}[{row}]"), *ty))
                        .collect();
                    lines.extend(
                        python_line(&targets)
                            .into_iter()
                            .map(|line| format!("{INDENT}{line}")),
                    );
                }
                (Layout::Grid, Kind::Array(_, cell)) => {
                    let Kind::Value(ty) = &**cell else {
                        lines.push(format!("# {} は読めない形です", one(index)));
                        continue;
                    };
                    lines.push(format!(
                        "{} = [{} for _ in range({len})]",
                        one(index),
                        python_split(*ty)
                    ));
                }
                (Layout::Grid, Kind::Value(ty)) => {
                    lines.push(format!(
                        "{} = [{} for _ in range({len})]",
                        one(index),
                        python_convert(*ty, "input()")
                    ));
                }
                _ => lines.push(format!("# {} は読めない形です", one(index))),
            }
        }
        lines
    }
}

// Python で数として読む型
fn python_number(ty: Ty) -> bool {
    matches!(ty, Ty::Usize | Ty::I64 | Ty::F64)
}

// 1 つの値の読み方（文字列・文字はそのまま）
fn python_convert(ty: Ty, text: &str) -> String {
    match ty {
        Ty::Usize | Ty::I64 => format!("int({text})"),
        Ty::F64 => format!("float({text})"),
        Ty::Char | Ty::Chars => text.to_string(),
    }
}

// 1 行を空白で分けて、リストにする読み方
fn python_split(ty: Ty) -> String {
    match ty {
        Ty::Usize | Ty::I64 => "list(map(int, input().split()))".to_string(),
        Ty::F64 => "list(map(float, input().split()))".to_string(),
        Ty::Char | Ty::Chars => "input().split()".to_string(),
    }
}

// 1 行の値（targets は代入先と型）を読む行。同じ型なら 1 行、混ざるときは文字列で読んでから数にする
fn python_line(targets: &[(String, Ty)]) -> Vec<String> {
    let names: Vec<&str> = targets.iter().map(|(name, _)| name.as_str()).collect();
    if let [(name, ty)] = targets {
        return vec![format!("{name} = {}", python_convert(*ty, "input()"))];
    }
    let all = |pick: &dyn Fn(Ty) -> bool| targets.iter().all(|(_, ty)| pick(*ty));
    if all(&|ty| matches!(ty, Ty::Usize | Ty::I64)) {
        return vec![format!("{} = map(int, input().split())", names.join(", "))];
    }
    if all(&|ty| ty == Ty::F64) {
        return vec![format!(
            "{} = map(float, input().split())",
            names.join(", ")
        )];
    }
    let mut lines = vec![format!("{} = input().split()", names.join(", "))];
    for (name, ty) in targets {
        if python_number(*ty) {
            lines.push(format!("{name} = {}", python_convert(*ty, name)));
        }
    }
    lines
}

// ---- テンプレートの input! を置き換える（Phase 15 作業 4。D-198） ----
//
// 問題を作るときはエディタを通らないので、画面側（src/editor/inputMacro.ts。エディタの構文木を使う）と同じ置き換えを、
// テンプレートの文字を読んで行う。コメント（`//`・`/* */`〔入れ子〕）・文字列（`"…"`・`r"…"`・`r#"…"#`・`b"…"`）・
// 文字（`'x'`。ライフタイム `'a` と見分ける）は飛ばし、名前と `{ } ( ) ! ;` だけを拾う

const INDENT: &str = "    ";

#[derive(Debug, Clone, PartialEq)]
enum Lexeme {
    Ident(String),
    Punct(char),
}

#[derive(Debug, Clone)]
struct Token {
    lexeme: Lexeme,
    start: usize,
    end: usize,
}

fn is_ident(lexeme: &Lexeme, name: &str) -> bool {
    matches!(lexeme, Lexeme::Ident(ident) if ident == name)
}

fn is_punct(lexeme: &Lexeme, c: char) -> bool {
    *lexeme == Lexeme::Punct(c)
}

// 生の文字列（r"…"・r#"…"#・br"…"）。始まりなら、閉じたあとの位置
fn raw_string_end(text: &str, at: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = at;
    if bytes.get(i) == Some(&b'b') {
        i += 1;
    }
    if bytes.get(i) != Some(&b'r') {
        return None;
    }
    i += 1;
    let hashes = bytes[i..].iter().take_while(|&&b| b == b'#').count();
    i += hashes;
    if bytes.get(i) != Some(&b'"') {
        return None;
    }
    let closing = format!("\"{}", "#".repeat(hashes));
    text[i + 1..]
        .find(&closing)
        .map(|found| i + 1 + found + closing.len())
        .or(Some(text.len()))
}

fn tokens(text: &str) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if text[i..].starts_with("//") {
            i = text[i..].find('\n').map_or(text.len(), |n| i + n);
        } else if text[i..].starts_with("/*") {
            let mut depth = 0;
            while i < bytes.len() {
                if text[i..].starts_with("/*") {
                    depth += 1;
                    i += 2;
                } else if text[i..].starts_with("*/") {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    // 日本語などの文字の途中で止まらないように、1 文字ずつ進める
                    i += text[i..].chars().next().map_or(1, char::len_utf8);
                }
            }
        } else if let Some(end) = raw_string_end(text, i) {
            i = end;
        } else if b == b'"' || (b == b'b' && bytes.get(i + 1) == Some(&b'"')) {
            i += if b == b'b' { 2 } else { 1 };
            while i < bytes.len() && bytes[i] != b'"' {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
        } else if b == b'\'' {
            // 'x'・'\n'・'\u{…}' は文字。'a（閉じない）はライフタイム
            let rest = &text[i + 1..];
            let mut chars = rest.char_indices();
            match chars.next() {
                Some((_, '\\')) => {
                    // エスケープ（'\''・'\n'・'\u{…}'）：\ の次の 1 文字を飛ばし、そのあとの ' で閉じる
                    let after = 1 + rest[1..].chars().next().map_or(0, char::len_utf8);
                    i = rest[after..]
                        .find('\'')
                        .map_or(text.len(), |n| i + 1 + after + n + 1);
                }
                Some((_, c)) if chars.next().map(|(_, next)| next) == Some('\'') => {
                    i += 1 + c.len_utf8() + 1;
                }
                _ => i += 1,
            }
        } else if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            result.push(Token {
                lexeme: Lexeme::Ident(text[start..i].to_string()),
                start,
                end: i,
            });
        } else if b.is_ascii_digit() {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
        } else if matches!(b, b'{' | b'}' | b'(' | b')' | b'!' | b';') {
            result.push(Token {
                lexeme: Lexeme::Punct(b as char),
                start: i,
                end: i + 1,
            });
            i += 1;
        } else {
            // ほかの記号と、ASCII でない文字（文字の途中では止まらない）
            i += text[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    result
}

// open（`{` の位置）に対応する `}` の位置
fn matching_brace(tokens: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        if is_punct(&token.lexeme, '{') {
            depth += 1;
        } else if is_punct(&token.lexeme, '}') {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

// `fn name` の、`fn` と本体の `{`・`}` の位置（最初のもの）
fn find_fn(tokens: &[Token], name: &str) -> Option<(usize, usize, usize)> {
    let at = tokens
        .windows(2)
        .position(|w| is_ident(&w[0].lexeme, "fn") && is_ident(&w[1].lexeme, name))?;
    let open = (at + 2..tokens.len()).find(|&k| is_punct(&tokens[k].lexeme, '{'))?;
    Some((at, open, matching_brace(tokens, open)?))
}

// 本体（open..close）の中の最初の `input! { … }` の、`input` と `{`・`}` の位置
fn find_input(tokens: &[Token], open: usize, close: usize) -> Option<(usize, usize, usize)> {
    (open + 1..close.saturating_sub(2)).find_map(|k| {
        if is_ident(&tokens[k].lexeme, "input")
            && is_punct(&tokens[k + 1].lexeme, '!')
            && is_punct(&tokens[k + 2].lexeme, '{')
        {
            let end = matching_brace(tokens, k + 2).filter(|&end| end < close)?;
            Some((k, k + 2, end))
        } else {
            None
        }
    })
}

fn line_indent(text: &str, at: usize) -> &str {
    let line_start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    let line = &text[line_start..];
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

// fn の input! の中身を lines にする変更（なければ fn の `{` の直後に入れる）。(始まり, 終わり, 文字, 入れたか)
fn input_edit(
    text: &str,
    tokens: &[Token],
    function: (usize, usize, usize),
    lines: &[String],
) -> (usize, usize, String, bool) {
    let (at, open, close) = function;
    if let Some((name, brace, end)) = find_input(tokens, open, close) {
        let base = line_indent(text, tokens[name].start);
        let inner = format!("{base}{INDENT}");
        let body: Vec<String> = lines.iter().map(|line| format!("{inner}{line}")).collect();
        (
            tokens[brace].end,
            tokens[end].start,
            format!("\n{}\n{base}", body.join("\n")),
            false,
        )
    } else {
        let base = format!("{}{INDENT}", line_indent(text, tokens[at].start));
        let body: Vec<String> = lines
            .iter()
            .map(|line| format!("{base}{INDENT}{line}"))
            .collect();
        let position = tokens[open].end;
        (
            position,
            position,
            format!("\n{base}input! {{\n{}\n{base}}}", body.join("\n")),
            true,
        )
    }
}

// テンプレート（main.rs の中身）の solve() の input! を、作ったものにする。テストケースが複数なら main() も
// （main_loop なら input! がなくても入れ、直下の `solve();` を for にする。画面側の planInputMacros と同じ決まり）。
// fn solve がなければ Err
pub fn fill_template(text: &str, generated: &Generated, main_loop: bool) -> Result<String, String> {
    let tokens = tokens(text);
    let solve = find_fn(&tokens, "solve").ok_or_else(|| "fn solve が見つかりません".to_string())?;
    let mut edits = vec![input_edit(text, &tokens, solve, &generated.solve)];
    if let Some(main_lines) = &generated.main
        && let Some(main) = find_fn(&tokens, "main")
    {
        let (_, open, close) = main;
        if main_loop {
            edits.push(input_edit(text, &tokens, main, main_lines));
            let count = main_lines
                .first()
                .and_then(|line| line.split(':').next())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or("t");
            // 本体の直下（深さ 1）で、文の始まりにある `solve();`
            let mut depth = 0;
            for k in open..close {
                match &tokens[k].lexeme {
                    Lexeme::Punct('{') => depth += 1,
                    Lexeme::Punct('}') => depth -= 1,
                    _ => {}
                }
                let starts_statement = k > open
                    && (is_punct(&tokens[k - 1].lexeme, '{')
                        || is_punct(&tokens[k - 1].lexeme, ';')
                        || is_punct(&tokens[k - 1].lexeme, '}'));
                if depth == 1
                    && starts_statement
                    && k + 3 < close
                    && is_ident(&tokens[k].lexeme, "solve")
                    && is_punct(&tokens[k + 1].lexeme, '(')
                    && is_punct(&tokens[k + 2].lexeme, ')')
                    && is_punct(&tokens[k + 3].lexeme, ';')
                {
                    let base = line_indent(text, tokens[k].start);
                    edits.push((
                        tokens[k].start,
                        tokens[k + 3].end,
                        format!("for _ in 0..{count} {{\n{base}{INDENT}solve();\n{base}}}"),
                        false,
                    ));
                    break;
                }
            }
        } else if find_input(&tokens, open, close).is_some() {
            edits.push(input_edit(text, &tokens, main, main_lines));
        }
    }
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.0));
    let mut result = text.to_string();
    for (start, end, insert, _) in edits {
        result.replace_range(start..end, &insert);
    }
    Ok(result)
}

// 作るものの名前（知らせの文に使う）
pub fn input_name(language: Language) -> &'static str {
    match language {
        Language::Rust => "input! ",
        Language::Cpp | Language::Python => "入力のコード",
    }
}

// 問題を作るときの知らせ（D-198）
pub fn fill_note(generated: &Generated) -> String {
    let name = input_name(generated.language);
    let mut caveats = Vec::new();
    if !generated.complete {
        caveats.push("一部だけ");
    }
    if generated.samples_matched < generated.samples_checked {
        caveats.push("入力例と合わないものがあります");
    }
    if caveats.is_empty() {
        format!("{name}を入れました")
    } else {
        format!("{name}を入れました（{}）", caveats.join("・"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(input: &str) -> Sample {
        Sample {
            input: input.to_string(),
            output: String::new(),
        }
    }

    // 手元の CPython 3.13 で script を動かし、標準出力を返す（なければ None）
    fn run_python(script: &str, input: &str) -> Option<String> {
        use std::io::Write;
        let interpreter =
            crate::python::find_interpreter(&crate::python::PythonSettings::default()).ok()?;
        let mut child = std::process::Command::new(interpreter)
            .args(["-I", "-B", "-c", script])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        // Windows の Python は print の改行を \r\n で出すので、比べる前に \n に直す（段階 2 S7）
        #[cfg(windows)]
        let stdout = stdout.replace("\r\n", "\n");
        Some(stdout)
    }

    // 手で書いた問題文（実際の問題文は使わない）
    fn statement(input_pres: &[&str], constraints: &[&str]) -> String {
        let pres: String = input_pres
            .iter()
            .map(|pre| format!("<pre>{pre}</pre>"))
            .collect();
        let items: String = constraints
            .iter()
            .map(|c| format!("<li><var>{c}</var></li>"))
            .collect();
        format!(
            "<div class=\"part\"><section><h3>制約</h3><ul>{items}</ul></section></div>\
             <div class=\"part\"><section><h3>入力</h3><p>入力は以下の形式で与えられる。</p>{pres}</section></div>\
             <div class=\"part\"><section><h3>入力例 1</h3><pre>1</pre></section></div>"
        )
    }

    #[test]
    fn reads_values_rows_and_pairs() {
        let html = statement(
            &[
                "<var>N</var> <var>K</var>\n<var>A_1</var> <var>A_2</var> <var>\\ldots</var> <var>A_N</var>\n<var>X_1</var> <var>Y_1</var>\n<var>\\vdots</var>\n<var>X_K</var> <var>Y_K</var>\n<var>B_0</var> <var>\\dots</var> <var>B_{N-1}</var>\n<var>C_1 \\cdots C_{N - 1}</var>\n",
            ],
            &[
                "1 \\leq N,K \\leq 2 \\times 10^5",
                "-10^9 \\leq A_i \\leq 10^9",
                "1 \\leq X_i \\lt Y_i \\leq N",
                "0 \\leq B_i \\leq 10^{18}",
                "|C_i| \\leq 10",
            ],
        );
        let generated = generate(&html, &[sample("3 2\n-1 0 5\n1 2\n2 3\n7 8 9\n-2 3\n")]).unwrap();
        assert_eq!(
            generated.solve,
            [
                "n: usize,",
                "k: usize,",
                "a: [i64; n],",
                "xy: [(usize, usize); k],",
                "b: [usize; n],",
                "c: [i64; n - 1],",
            ]
        );
        assert_eq!(generated.main, None);
        assert!(generated.complete);
        assert_eq!(generated.notes, Vec::<String>::new());
        assert_eq!(
            (generated.samples_checked, generated.samples_matched),
            (1, 1)
        );
    }

    #[test]
    fn adds_an_underscore_to_keyword_names() {
        let html = statement(
            &[
                "<var>M</var> <var>N</var>\n<var>F_1</var> <var>N_1</var>\n<var>\\vdots</var>\n<var>F_M</var> <var>N_M</var>\n<var>I_1</var> <var>F_1</var>\n<var>\\vdots</var>\n<var>I_N</var> <var>F_N</var>\n",
            ],
            &["1 \\leq M,N \\leq 10", "1 \\leq F_i,N_i,I_i \\leq 10"],
        );
        let generated = generate(&html, &[sample("1 2\n3 4\n5 6\n7 8\n")]).unwrap();
        assert_eq!(
            generated.solve,
            [
                "m: usize,",
                "n: usize,",
                "fn_: [(usize, usize); m],",
                "if_: [(usize, usize); n],",
            ]
        );
        assert_eq!(
            generated.notes,
            ["Rust の予約語と同じ名前は、末尾に _ を付けました：fn → fn_、if → if_"]
        );
        assert_eq!(generated.samples_matched, 1);
    }

    #[test]
    fn reads_strings_chars_and_grids() {
        let html = statement(
            &[
                "<var>H</var> <var>W</var>\n<var>S</var>\n<var>C</var>\n<var>G_{1,1}G_{1,2}\\ldots</var><var>G_{1,W}</var>\n<var>\\vdots</var>\n<var>G_{H,1}G_{H,2}\\ldots</var><var>G_{H,W}</var>\n<var>A_{1,1}</var> <var>\\ldots</var> <var>A_{1,W}</var>\n<var>\\vdots</var>\n<var>A_{H,1}</var> <var>\\ldots</var> <var>A_{H,W}</var>\n<var>T_1</var>\n<var>\\vdots</var>\n<var>T_H</var>\n<var>R</var>",
            ],
            &[
                "1 \\leq H,W \\leq 100",
                "S は英小文字からなる長さ W の文字列",
                "C は英大文字",
                "G_{i,j} は . または # のいずれか",
                "A_{i, j} は L, R, U, D のいずれかである (1 \\leq i \\leq H)",
                "T_i は長さ 3 の文字列",
                "R は実数",
            ],
        );
        let input = "2 3\nabc\nX\n.#.\n##.\nL R U\nD D L\nfoo\nbar\n0.5\n";
        let generated = generate(&html, &[sample(input)]).unwrap();
        assert_eq!(
            generated.solve,
            [
                "h: usize,",
                "w: usize,",
                "s: Chars,",
                "c: char,",
                "g: [Chars; h],",
                "a: [[char; w]; h],",
                "t: [Chars; h],",
                "r: f64,",
            ]
        );
        assert_eq!(generated.notes, Vec::<String>::new());
        assert_eq!(generated.samples_matched, 1);
    }

    #[test]
    fn splits_test_cases_into_main_and_solve() {
        let html = statement(
            &[
                "<var>T</var>\n<var>\\text{case}_1</var>\n<var>\\text{case}_2</var>\n<var>\\vdots</var>\n<var>\\text{case}_T</var>",
                "<var>N</var>\n<var>A_1</var> <var>A_2</var> <var>\\ldots</var> <var>A_N</var>",
            ],
            &[
                "1 \\leq T \\leq 10",
                "1 \\leq N \\leq 10",
                "1 \\leq A_i \\leq 10",
            ],
        );
        let generated = generate(&html, &[sample("2\n1\n5\n3\n1 2 3\n")]).unwrap();
        assert_eq!(generated.main, Some(vec!["t: usize,".to_string()]));
        assert_eq!(generated.solve, ["n: usize,", "a: [usize; n],"]);
        assert!(generated.complete);
        assert_eq!(generated.samples_matched, 1);
    }

    #[test]
    fn stops_at_queries_and_unreadable_lines() {
        // クエリ：そこまでを作り、残りの値は誤りとしない
        let html = statement(
            &[
                "<var>N</var> <var>Q</var>\n<var>A_1</var> <var>\\ldots</var> <var>A_N</var>\n<var>\\text{query}_1</var>\n<var>\\text{query}_2</var>\n<var>\\vdots</var>\n<var>\\text{query}_Q</var>",
                "<var>1</var> <var>x</var>",
            ],
            &["1 \\leq N, Q \\leq 10", "1 \\leq A_i \\leq 10"],
        );
        let generated = generate(&html, &[sample("2 2\n1 2\n1 5\n2\n")]).unwrap();
        assert_eq!(
            generated.solve,
            [
                "n: usize,",
                "q: usize,",
                "a: [usize; n],",
                "// クエリ（q 個）は形が決まらないので、ここでは読んでいません",
            ]
        );
        assert!(!generated.complete);
        assert_eq!(generated.samples_matched, 1);
        assert!(generated.notes[0].starts_with("クエリ（q 個）は読んでいません"));

        // 三角形の並びは読まない
        let html = statement(
            &[
                "<var>N</var>\n<var>C_{1,2}</var> <var>C_{1,3}</var> <var>\\ldots</var> <var>C_{1,N}</var>\n<var>C_{2,3}</var> <var>\\ldots</var> <var>C_{2,N}</var>\n<var>\\vdots</var>\n<var>C_{N-1,N}</var>",
            ],
            &["2 \\leq N \\leq 10", "1 \\leq C_{i,j} \\leq 10"],
        );
        let generated = generate(&html, &[sample("3\n1 2\n3\n")]).unwrap();
        assert_eq!(
            generated.solve,
            [
                "n: usize,",
                "// 入力の形式の 2 行目から先は読めませんでした"
            ]
        );
        assert_eq!(
            generated.notes,
            ["入力の形式の 2 行目から先は読めませんでした"]
        );

        // 形式がない
        assert!(generate("<section><h3>出力</h3></section>", &[]).is_err());
    }

    #[test]
    fn checks_samples_and_reports_guesses() {
        let html = statement(
            &[
                "<var>N</var> <var>M</var>\n<var>A_1</var> <var>\\ldots</var> <var>A_N</var>\n<var>P</var>",
            ],
            &["1 \\leq N \\leq 10", "1 \\leq A_i \\leq M"],
        );
        let samples = [
            sample("2 5\n1 2\n7\n"),
            // 足りない
            sample("3 5\n1 2\n"),
            // usize に負の数
            sample("2 5\n-1 2\n7\n"),
            // 余る
            sample("1 5\n1\n7 8\n"),
        ];
        let generated = generate(&html, &samples).unwrap();
        // M は「1 \leq A_i \leq M」から 0 以上と分かる。P は制約にない
        assert_eq!(
            generated.solve,
            ["n: usize,", "m: usize,", "a: [usize; n],", "p: i64,"]
        );
        assert_eq!(
            (generated.samples_checked, generated.samples_matched),
            (4, 1)
        );
        assert_eq!(
            generated.notes,
            [
                "制約から型を決められなかったので i64 にしました：p",
                "入力例 2 と合いません（値が足りません（a））",
                "入力例 3 と合いません（a を usize として読めません（「-1」））",
                "入力例 4 と合いません（値が 1 個余ります）",
            ]
        );
    }

    #[test]
    fn reads_constraint_phrases() {
        let html = statement(
            &[
                "<var>N</var> <var>M</var> <var>K</var>\n<var>S_1</var>\n<var>\\vdots</var>\n<var>S_N</var>\n<var>Y_1</var> <var>\\dots</var> <var>Y_N</var>\n<var>T_1</var> <var>R_1</var>\n<var>\\vdots</var>\n<var>T_M</var> <var>R_M</var>\n<var>A_1</var> <var>\\ldots</var> <var>A_K</var>",
            ],
            &[
                // 添字の N は主語ではない
                "S_1, S_2, \\ldots, S_N は相異なる文字列",
                "N は正整数",
                // 左端が 0 以上の変数
                "N-1 \\leq M \\leq 10",
                "Y は (1, \\dots, N) の順列",
                "T_i \\in \\lbrace 1,2 \\rbrace",
                "R_i は Yes と No のいずれかである",
                // かっこの中は別の不等式（N を負としない）
                "-10^9 \\le A_i \\le 10^9 (1 \\le i \\le K)",
                "0 \\leq K \\leq 5",
            ],
        );
        let generated = generate(&html, &[sample("2 1 2\nab\ncd\n2 1\n1 Yes\n-3 4\n")]).unwrap();
        assert_eq!(
            generated.solve,
            [
                "n: usize,",
                "m: usize,",
                "k: usize,",
                "s: [Chars; n],",
                "y: [usize; n],",
                "tr: [(usize, Chars); m],",
                "a: [i64; k],",
            ]
        );
        assert_eq!(generated.notes, Vec::<String>::new());
        assert_eq!(generated.samples_matched, 1);
    }

    #[test]
    fn falls_back_to_rows_for_char_grids() {
        // 形式は間ありでも、入力例が間なしなら行ごとの文字列にする
        let html = statement(
            &[
                "<var>H</var> <var>W</var>\n<var>S_{1,1} S_{1,2} \\cdots S_{1,W}</var>\n<var>\\vdots</var>\n<var>S_{H,1} S_{H,2} \\cdots S_{H,W}</var>",
            ],
            &["1 \\leq H,W \\leq 10", "S_{i,j} は L, R のいずれかである"],
        );
        let generated = generate(&html, &[sample("2 3\nLRL\nRRL\n")]).unwrap();
        assert_eq!(
            generated.solve,
            ["h: usize,", "w: usize,", "s: [Chars; h],"]
        );
        assert_eq!(
            generated.notes,
            ["1 文字の格子は、入力例に合わせて行ごとの文字列（Chars）で読みます"]
        );
        // 入力例も間ありなら、そのまま
        let generated = generate(&html, &[sample("2 3\nL R L\nR R L\n")]).unwrap();
        assert_eq!(
            generated.solve,
            ["h: usize,", "w: usize,", "s: [[char; w]; h],"]
        );
        assert_eq!(generated.notes, Vec::<String>::new());
    }

    // テンプレートの input! の置き換え（D-198）。期待する結果は、画面側（src/editor/inputMacro.ts）に同じ例を通した結果
    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn fill(text: &str, solve: Vec<String>, main: Option<Vec<String>>, main_loop: bool) -> String {
        let generated = Generated {
            language: Language::Rust,
            solve,
            main,
            notes: Vec::new(),
            complete: true,
            samples_checked: 0,
            samples_matched: 0,
        };
        fill_template(text, &generated, main_loop).unwrap_or_else(|e| format!("ERR {e}"))
    }

    #[test]
    fn fills_the_template_like_the_editor() {
        // テンプレートの solve() の input! を置き換える
        assert_eq!(
            fill(
                r##"use proconio::input;

#[proconio::fastout]
fn solve() {
    input! {
        n: usize,
        a: [usize; n],
    }

    println!("{:?}", a);
}

fn main() {
    solve();
    // 深い再帰DFSでスタックが足りない場合は上の行の代わりにこちらを使う
    // std::thread::Builder::new().stack_size(1 << 28).spawn(solve).unwrap().join().unwrap();
}
"##,
                lines(&["h: usize,", "w: usize,", "s: [Chars; h],"]),
                None,
                false,
            ),
            r##"use proconio::input;

#[proconio::fastout]
fn solve() {
    input! {
        h: usize,
        w: usize,
        s: [Chars; h],
    }

    println!("{:?}", a);
}

fn main() {
    solve();
    // 深い再帰DFSでスタックが足りない場合は上の行の代わりにこちらを使う
    // std::thread::Builder::new().stack_size(1 << 28).spawn(solve).unwrap().join().unwrap();
}
"##,
        );
        // テストケース：main() に input! があれば置き換える（for は書かない）
        assert_eq!(
            fill(
                r##"use proconio::input;

#[proconio::fastout]
fn solve() {
    input! {
        n: usize,
        a: [usize; n],
    }

    println!("{:?}", a);
}

fn main() {
    input! { t: usize }
    for _ in 0..t {
        solve();
    }
    // 深い再帰DFSでスタックが足りない場合は上の行の代わりにこちらを使う
    // std::thread::Builder::new().stack_size(1 << 28).spawn(solve).unwrap().join().unwrap();
}
"##,
                lines(&[
                    "n: usize,",
                    "// クエリ（q 個）は形が決まらないので、ここでは読んでいません"
                ]),
                Some(lines(&["t: usize,"])),
                false,
            ),
            r##"use proconio::input;

#[proconio::fastout]
fn solve() {
    input! {
        n: usize,
        // クエリ（q 個）は形が決まらないので、ここでは読んでいません
    }

    println!("{:?}", a);
}

fn main() {
    input! {
        t: usize,
    }
    for _ in 0..t {
        solve();
    }
    // 深い再帰DFSでスタックが足りない場合は上の行の代わりにこちらを使う
    // std::thread::Builder::new().stack_size(1 << 28).spawn(solve).unwrap().join().unwrap();
}
"##,
        );
        // for を書く：main() の先頭に input!、直下の solve(); を for に（コメントの中の solve は呼び出しではない）
        assert_eq!(
            fill(
                r##"use proconio::input;

#[proconio::fastout]
fn solve() {
    input! {
        n: usize,
        a: [usize; n],
    }

    println!("{:?}", a);
}

fn main() {
    solve();
    // 深い再帰DFSでスタックが足りない場合は上の行の代わりにこちらを使う
    // std::thread::Builder::new().stack_size(1 << 28).spawn(solve).unwrap().join().unwrap();
}
"##,
                lines(&["n: usize,", "a: [usize; n],"]),
                Some(lines(&["t: usize,"])),
                true,
            ),
            r##"use proconio::input;

#[proconio::fastout]
fn solve() {
    input! {
        n: usize,
        a: [usize; n],
    }

    println!("{:?}", a);
}

fn main() {
    input! {
        t: usize,
    }
    for _ in 0..t {
        solve();
    }
    // 深い再帰DFSでスタックが足りない場合は上の行の代わりにこちらを使う
    // std::thread::Builder::new().stack_size(1 << 28).spawn(solve).unwrap().join().unwrap();
}
"##,
        );
        // for を書く：すでに for の中で呼んでいれば、そのまま
        assert_eq!(
            fill(
                r##"use proconio::input;

#[proconio::fastout]
fn solve() {
    input! {
        n: usize,
        a: [usize; n],
    }

    println!("{:?}", a);
}

fn main() {
    input! { t: usize }
    for _ in 0..t {
        solve();
    }
    // 深い再帰DFSでスタックが足りない場合は上の行の代わりにこちらを使う
    // std::thread::Builder::new().stack_size(1 << 28).spawn(solve).unwrap().join().unwrap();
}
"##,
                lines(&["n: usize,"]),
                Some(lines(&["t: usize,"])),
                true,
            ),
            r##"use proconio::input;

#[proconio::fastout]
fn solve() {
    input! {
        n: usize,
    }

    println!("{:?}", a);
}

fn main() {
    input! {
        t: usize,
    }
    for _ in 0..t {
        solve();
    }
    // 深い再帰DFSでスタックが足りない場合は上の行の代わりにこちらを使う
    // std::thread::Builder::new().stack_size(1 << 28).spawn(solve).unwrap().join().unwrap();
}
"##,
        );
        // solve() に input! がなければ、先頭に入れる
        assert_eq!(
            fill(
                r##"fn solve() {
    let x = 1;
}
"##,
                lines(&["n: usize,"]),
                None,
                false,
            ),
            r##"fn solve() {
    input! {
        n: usize,
    }
    let x = 1;
}
"##,
        );
        // コメント・文字列・文字の中の fn solve・input!・{ } は数えない
        assert_eq!(
            fill(
                r##"// fn solve() { input! { x: i64 } }
/* fn solve() { /* 入れ子 { */ } */
fn helper<'a>(s: &'a str) -> char {
    let _ = "fn solve() { input! { y: u8 } }";
    let _ = r#"input! { "{" }"#;
    let _ = '{';
    let _ = '\'';
    let _ = b"}";
    '}'
}
fn solve() {
    let s = "input! {";
    input! {
        n: usize, // { コメントの中の波かっこ }
    }
    let c = '}';
}
fn main() {
    let x = solve_twice();
    if true { solve(); }
    solve();
}
"##,
                lines(&["n: usize,"]),
                Some(lines(&["t: usize,"])),
                true,
            ),
            r##"// fn solve() { input! { x: i64 } }
/* fn solve() { /* 入れ子 { */ } */
fn helper<'a>(s: &'a str) -> char {
    let _ = "fn solve() { input! { y: u8 } }";
    let _ = r#"input! { "{" }"#;
    let _ = '{';
    let _ = '\'';
    let _ = b"}";
    '}'
}
fn solve() {
    let s = "input! {";
    input! {
        n: usize,
    }
    let c = '}';
}
fn main() {
    input! {
        t: usize,
    }
    let x = solve_twice();
    if true { solve(); }
    for _ in 0..t {
        solve();
    }
}
"##,
        );
        // fn solve がなければ置き換えない
        assert_eq!(
            fill("fn main() {}\n", lines(&["n: usize,"]), None, false),
            "ERR fn solve が見つかりません"
        );
    }

    #[test]
    fn notes_how_the_input_was_filled() {
        let mut generated = Generated {
            language: Language::Rust,
            solve: Vec::new(),
            main: None,
            notes: Vec::new(),
            complete: true,
            samples_checked: 2,
            samples_matched: 2,
        };
        assert_eq!(fill_note(&generated), "input! を入れました");
        generated.complete = false;
        generated.samples_matched = 1;
        assert_eq!(
            fill_note(&generated),
            "input! を入れました（一部だけ・入力例と合わないものがあります）"
        );
    }

    #[test]
    fn reads_lengths_and_names() {
        assert_eq!(Len::count("1", "N").unwrap().render(), "n");
        assert_eq!(Len::count("0", "N-1").unwrap().render(), "n");
        assert_eq!(Len::count("1", "N - 1").unwrap().render(), "n - 1");
        assert_eq!(Len::count("1", "2N").unwrap().render(), "2 * n");
        assert_eq!(Len::count("1", "N+1").unwrap().render(), "n + 1");
        assert_eq!(Len::count("1", "5").unwrap().render(), "5");
        assert_eq!(Len::count("N", "M"), None);
        assert_eq!(Len::count("1", "K_i"), None);
        assert_eq!(var_name("S", Some("X")), "s_x");
        assert!(mentions("1 \\leq N \\leq 10", "N"));
        assert!(!mentions("\\times 10", "t"));
        assert!(!mentions("NM", "N"));
    }

    // ---- C++（Phase 17 作業 8。D-213） ----

    #[test]
    fn writes_cpp_declarations_and_reads() {
        let html = statement(
            &[
                "<var>N</var> <var>K</var>\n<var>A_1</var> <var>A_2</var> <var>\\ldots</var> <var>A_N</var>\n<var>X_1</var> <var>Y_1</var>\n<var>\\vdots</var>\n<var>X_K</var> <var>Y_K</var>\n<var>B_0</var> <var>\\dots</var> <var>B_{N-1}</var>\n<var>C_1 \\cdots C_{N - 1}</var>\n",
            ],
            &[
                "1 \\leq N,K \\leq 2 \\times 10^5",
                "-10^9 \\leq A_i \\leq 10^9",
                "1 \\leq X_i \\lt Y_i \\leq N",
                "0 \\leq B_i \\leq 10^{18}",
            ],
        );
        let generated = generate_for(
            &html,
            &[sample("3 2\n-1 0 5\n1 2\n2 3\n7 8 9\n-2 3\n")],
            Language::Cpp,
        )
        .unwrap();
        assert_eq!(generated.language, Language::Cpp);
        assert_eq!(
            generated.solve,
            [
                "long long N, K;",
                "cin >> N >> K;",
                "vector<long long> A(N);",
                "for (auto& a : A) cin >> a;",
                "vector<long long> X(K), Y(K);",
                "for (int i = 0; i < K; i++) cin >> X[i] >> Y[i];",
                "vector<long long> B(N);",
                "for (auto& b : B) cin >> b;",
                "vector<long long> C(N - 1);",
                "for (auto& c : C) cin >> c;",
            ]
        );
        assert_eq!(generated.main, None);
        // C++ では整数はすべて long long なので、型を決められなかった注意（C）は出さない
        assert_eq!(generated.notes, Vec::<String>::new());
        assert_eq!(generated.samples_matched, 1);
    }

    #[test]
    fn writes_cpp_strings_grids_and_cases() {
        let html = statement(
            &[
                "<var>H</var> <var>W</var>\n<var>S</var>\n<var>C</var>\n<var>G_{1,1}G_{1,2}\\ldots</var><var>G_{1,W}</var>\n<var>\\vdots</var>\n<var>G_{H,1}G_{H,2}\\ldots</var><var>G_{H,W}</var>\n<var>A_{1,1}</var> <var>\\ldots</var> <var>A_{1,W}</var>\n<var>\\vdots</var>\n<var>A_{H,1}</var> <var>\\ldots</var> <var>A_{H,W}</var>\n<var>R</var>",
            ],
            &[
                "1 \\leq H,W \\leq 100",
                "S は英小文字からなる長さ W の文字列",
                "C は英大文字",
                "G_{i,j} は . または # のいずれか",
                "1 \\leq A_{i,j} \\leq 9",
                "R は実数",
            ],
        );
        let input = "2 3\nabc\nX\n.#.\n##.\n1 2 3\n4 5 6\n0.5\n";
        let generated = generate_for(&html, &[sample(input)], Language::Cpp).unwrap();
        assert_eq!(
            generated.solve,
            [
                "long long H, W;",
                "string S;",
                "char C;",
                "cin >> H >> W >> S >> C;",
                "vector<string> G(H);",
                "for (auto& g : G) cin >> g;",
                "vector<vector<long long>> A(H, vector<long long>(W));",
                "for (auto& row : A) for (auto& a : row) cin >> a;",
                "double R;",
                "cin >> R;",
            ]
        );
        assert_eq!(generated.samples_matched, 1);

        let html = statement(
            &[
                "<var>T</var>\n<var>\\text{case}_1</var>\n<var>\\text{case}_2</var>\n<var>\\vdots</var>\n<var>\\text{case}_T</var>",
                "<var>N</var>\n<var>A_1</var> <var>A_2</var> <var>\\ldots</var> <var>A_N</var>",
            ],
            &[
                "1 \\leq T \\leq 10",
                "1 \\leq N \\leq 10",
                "1 \\leq A_i \\leq 10",
            ],
        );
        let generated =
            generate_for(&html, &[sample("2\n1\n5\n3\n1 2 3\n")], Language::Cpp).unwrap();
        assert_eq!(
            generated.main,
            Some(vec!["long long T;".to_string(), "cin >> T;".to_string()])
        );
        assert_eq!(
            generated.solve,
            [
                "long long N;",
                "cin >> N;",
                "vector<long long> A(N);",
                "for (auto& a : A) cin >> a;",
            ]
        );
        assert_eq!(generated.samples_matched, 1);
    }

    // Python（Phase 20 作業 7。D-227）：行ごとに input() で読む。名前は問題文のまま
    #[test]
    fn writes_python_lines() {
        let html = statement(
            &[
                "<var>N</var> <var>M</var>\n<var>A_1</var> <var>A_2</var> <var>\\ldots</var> <var>A_N</var>\n<var>X_1</var> <var>Y_1</var>\n<var>\\vdots</var>\n<var>X_M</var> <var>Y_M</var>\n<var>B_1</var>\n<var>\\vdots</var>\n<var>B_N</var>\n<var>K</var> <var>T</var>\n<var>U_1</var> <var>\\ldots</var> <var>U_K</var>\n",
            ],
            &[
                "1 \\leq N,M,K \\leq 10",
                "1 \\leq A_i,X_i,Y_i,B_i \\leq 10",
                "T は英小文字からなる文字列",
                "U_i は英小文字からなる文字列",
            ],
        );
        let input = "3 2\n1 2 3\n1 2\n2 3\n4\n5\n6\n2 abc\nx yz\n";
        let generated = generate_for(&html, &[sample(input)], Language::Python).unwrap();
        assert_eq!(
            generated.solve,
            [
                "N, M = map(int, input().split())",
                "A = list(map(int, input().split()))",
                "X = [0] * M",
                "Y = [0] * M",
                "for i in range(M):",
                "    X[i], Y[i] = map(int, input().split())",
                "B = [int(input()) for _ in range(N)]",
                "K, T = input().split()",
                "K = int(K)",
                "U = input().split()",
            ]
        );
        assert_eq!(generated.language, Language::Python);
        assert_eq!(generated.notes, Vec::<String>::new());
        assert_eq!(generated.samples_matched, 1);
        // 手元の処理系で動かして、入力例を読めること（なければ飛ばす）
        let script: String = generated
            .solve
            .iter()
            .map(|line| format!("    {line}\n"))
            .collect();
        let script =
            format!("def main():\n{script}    print(N, M, A, X, Y, B, K, T, U)\n\n\nmain()\n");
        match run_python(&script, input) {
            Some(out) => assert_eq!(
                out,
                "3 2 [1, 2, 3] [1, 2] [2, 3] [4, 5, 6] 2 abc ['x', 'yz']\n"
            ),
            None => eprintln!("python3.13 がないので、動かすのは飛ばします"),
        }
        // 行の分かれ方が違う入力例は、値の数が合っていても合わない（A が 2 行に分かれている）
        let split = "3 2\n1 2\n3\n1 2\n2 3\n4\n5\n6\n2 abc\nx yz\n";
        let generated = generate_for(&html, &[sample(split)], Language::Python).unwrap();
        assert_eq!(generated.samples_matched, 0);
        assert!(
            generated
                .notes
                .contains(&"入力例 1 と合いません（2 行目：値が足りません（a））".to_string()),
            "{:?}",
            generated.notes
        );
        // Rust・C++ は空白で区切って読むので、合う
        assert_eq!(
            generate_for(&html, &[sample(split)], Language::Cpp)
                .unwrap()
                .samples_matched,
            1
        );
        // 行が余る・足りない
        let extra = format!("{input}9\n");
        assert!(
            generate_for(&html, &[sample(&extra)], Language::Python)
                .unwrap()
                .notes
                .contains(&"入力例 1 と合いません（行が 1 行余ります）".to_string())
        );
        let short = "3 2\n1 2 3\n1 2\n";
        assert!(
            generate_for(&html, &[sample(short)], Language::Python)
                .unwrap()
                .notes
                .contains(&"入力例 1 と合いません（4 行目がありません）".to_string())
        );
    }

    #[test]
    fn writes_python_strings_grids_and_cases() {
        let html = statement(
            &[
                "<var>H</var> <var>W</var>\n<var>S</var>\n<var>C</var>\n<var>G_{1,1}G_{1,2}\\ldots</var><var>G_{1,W}</var>\n<var>\\vdots</var>\n<var>G_{H,1}G_{H,2}\\ldots</var><var>G_{H,W}</var>\n<var>A_{1,1}</var> <var>\\ldots</var> <var>A_{1,W}</var>\n<var>\\vdots</var>\n<var>A_{H,1}</var> <var>\\ldots</var> <var>A_{H,W}</var>\n<var>R</var>",
            ],
            &[
                "1 \\leq H,W \\leq 100",
                "S は英小文字からなる長さ W の文字列",
                "C は英大文字",
                "G_{i,j} は . または # のいずれか",
                "1 \\leq A_{i,j} \\leq 9",
                "R は実数",
            ],
        );
        let input = "2 3\nabc\nX\n.#.\n##.\n1 2 3\n4 5 6\n0.5\n";
        let generated = generate_for(&html, &[sample(input)], Language::Python).unwrap();
        assert_eq!(
            generated.solve,
            [
                "H, W = map(int, input().split())",
                "S = input()",
                "C = input()",
                "G = [input() for _ in range(H)]",
                "A = [list(map(int, input().split())) for _ in range(H)]",
                "R = float(input())",
            ]
        );
        assert_eq!(generated.samples_matched, 1);
        let script: String = generated
            .solve
            .iter()
            .map(|line| format!("{line}\n"))
            .collect();
        if let Some(out) = run_python(&format!("{script}print(H, W, S, C, G, A, R)\n"), input) {
            assert_eq!(out, "2 3 abc X ['.#.', '##.'] [[1, 2, 3], [4, 5, 6]] 0.5\n");
        }

        let html = statement(
            &[
                "<var>T</var>\n<var>\\text{case}_1</var>\n<var>\\text{case}_2</var>\n<var>\\vdots</var>\n<var>\\text{case}_T</var>",
                "<var>N</var>\n<var>A_1</var> <var>A_2</var> <var>\\ldots</var> <var>A_N</var>",
            ],
            &[
                "1 \\leq T \\leq 10",
                "1 \\leq N \\leq 10",
                "1 \\leq A_i \\leq 10",
            ],
        );
        let generated =
            generate_for(&html, &[sample("2\n1\n5\n3\n1 2 3\n")], Language::Python).unwrap();
        assert_eq!(generated.main, Some(vec!["T = int(input())".to_string()]));
        assert_eq!(
            generated.solve,
            ["N = int(input())", "A = list(map(int, input().split()))"]
        );
        assert_eq!(generated.samples_matched, 1);
        // クエリは読まずにコメント
        let html = statement(
            &[
                "<var>Q</var>\n<var>\\text{query}_1</var>\n<var>\\vdots</var>\n<var>\\text{query}_Q</var>",
            ],
            &["1 \\leq Q \\leq 10"],
        );
        let generated = generate_for(&html, &[], Language::Python).unwrap();
        assert_eq!(
            generated.solve,
            [
                "Q = int(input())",
                "# クエリ（q 個）は形が決まらないので、ここでは読んでいません",
            ]
        );
        assert!(
            generated.notes.contains(
                &"クエリ（q 個）は読んでいません。クエリごとに input() で読んでください"
                    .to_string()
            ),
            "{:?}",
            generated.notes
        );
    }

    #[test]
    fn avoids_python_keywords() {
        // 予約語（if）・作るコードで使う名前（map）・ループの変数（i）とぶつかる
        let html = statement(
            &[
                "<var>if</var> <var>i</var>\n<var>map_1</var> <var>\\ldots</var> <var>map_i</var>\n<var>P_1</var> <var>Q_1</var>\n<var>\\vdots</var>\n<var>P_i</var> <var>Q_i</var>\n",
            ],
            &["1 \\leq if,i \\leq 10"],
        );
        let generated = generate_for(&html, &[], Language::Python).unwrap();
        assert_eq!(
            generated.solve,
            [
                "if_, i = map(int, input().split())",
                "map_ = list(map(int, input().split()))",
                "P = [0] * i",
                "Q = [0] * i",
                "for j in range(i):",
                "    P[j], Q[j] = map(int, input().split())",
            ]
        );
        assert!(
            generated.notes.contains(
                &"Python の予約語と同じ名前は、末尾に _ を付けました：if → if_、map → map_"
                    .to_string()
            ),
            "{:?}",
            generated.notes
        );
    }

    #[test]
    fn avoids_cpp_keywords_and_reports_same_names() {
        // 変数の名前が予約語（new）・ループの変数（a・x・i）とぶつかる。小文字の a と大文字の A は別の名前
        let html = statement(
            &[
                "<var>new</var> <var>a</var> <var>x</var> <var>i</var>\n<var>A_1</var> <var>\\ldots</var> <var>A_a</var>\n<var>P_1</var> <var>Q_1</var>\n<var>\\vdots</var>\n<var>P_a</var> <var>Q_a</var>\n",
            ],
            &["1 \\leq a,x,i \\leq 10"],
        );
        let generated = generate_for(&html, &[], Language::Cpp).unwrap();
        assert_eq!(
            generated.solve,
            [
                "long long new_, a, x, i;",
                "cin >> new_ >> a >> x >> i;",
                "vector<long long> A(a);",
                "for (auto& v : A) cin >> v;",
                "vector<long long> P(a), Q(a);",
                "for (int j = 0; j < a; j++) cin >> P[j] >> Q[j];",
            ]
        );
        assert!(
            generated.notes.contains(
                &"C++ の予約語と同じ名前は、末尾に _ を付けました：new → new_".to_string()
            ),
            "{:?}",
            generated.notes
        );
        // 縦の組の列の名前が、ほかの変数と同じ（N）
        let html = statement(
            &[
                "<var>M</var> <var>N</var>\n<var>F_1</var> <var>N_1</var>\n<var>\\vdots</var>\n<var>F_M</var> <var>N_M</var>\n",
            ],
            &["1 \\leq M,N \\leq 10", "1 \\leq F_i,N_i \\leq 10"],
        );
        let generated = generate_for(&html, &[], Language::Cpp).unwrap();
        assert!(
            generated
                .notes
                .contains(&"名前 N が 2 度出てきます。片方の名前を変えてください".to_string()),
            "{:?}",
            generated.notes
        );
        // Rust では fn・n で別の名前なので、知らせない
        let generated = generate(&html, &[]).unwrap();
        assert!(!generated.notes.iter().any(|note| note.contains("2 度")));
    }
}
