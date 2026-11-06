// 出力の比較（Phase 2 作業 5。D-092・D-093）と、サンプルテストの時間・メモリの制限（D-141）
//
// ローカルの比較・判定の規則であり、AtCoder のジャッジと同じ結果になることは保証しない（D-093・D-094）

use std::path::Path;

use serde::Deserialize;

// 比較の方式（D-092）。compare.json の "mode"
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    // 空白緩和（既定。D-093）
    Whitespace,
    // バイトの完全一致
    Exact,
    // 浮動小数点。abs・rel のどちらかの範囲内なら正解（指定したものだけを使う）
    Float { abs: Option<f64>, rel: Option<f64> },
    // 比較しない（「比較不可・目視確認」）
    None,
}

// サンプルテストの時間の制限の既定値（ミリ秒）と、メモリの制限の既定値（MiB）。
// compare.json の timeLimitMs・memoryLimitMiB で変えられる（D-141。ユーザーの判断、2026-10-23）
pub const DEFAULT_TIME_LIMIT_MS: u64 = 2500;
pub const DEFAULT_MEMORY_LIMIT_MIB: u64 = 1024;
// 指定できる範囲（パッケージを作るときの compare.json でも使う。D-161）
pub const MAX_TIME_LIMIT_MS: u64 = 60_000;
pub const MAX_MEMORY_LIMIT_MIB: u64 = 65_536;

// ランダムテストの回数の既定値と上限（compare.json の randomCount で変えられる。D-143。ユーザーの選択、2026-10-24）
pub const DEFAULT_RANDOM_COUNT: u64 = 100;
const MAX_RANDOM_COUNT: u64 = 10_000;

// サンプルテストの設定（テストのフォルダの compare.json。なければ既定値）
#[derive(Debug, Clone, PartialEq)]
pub struct TestSettings {
    pub mode: Mode,
    // 超えたら止めて TLE にする
    pub time_limit_ms: u64,
    // 最大使用メモリ（最大 RSS）が超えていたら MLE にする（実行中には止めない）
    pub memory_limit_mib: u64,
    // ランダムテストの回数（最初の失敗で止まるので、最大の回数。D-143）
    pub random_count: u64,
}

impl Default for TestSettings {
    fn default() -> Self {
        TestSettings {
            mode: Mode::Whitespace,
            time_limit_ms: DEFAULT_TIME_LIMIT_MS,
            memory_limit_mib: DEFAULT_MEMORY_LIMIT_MIB,
            random_count: DEFAULT_RANDOM_COUNT,
        }
    }
}

// compare.json の形（P2-05-1。D-141 で制限を加えた）。知らないキーは誤りとする。mode を省くと空白緩和
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompareFile {
    mode: Option<String>,
    abs: Option<f64>,
    rel: Option<f64>,
    #[serde(rename = "timeLimitMs")]
    time_limit_ms: Option<u64>,
    #[serde(rename = "memoryLimitMiB")]
    memory_limit_mib: Option<u64>,
    #[serde(rename = "randomCount")]
    random_count: Option<u64>,
}

// 許容誤差として使える値か（0 以上の有限の数）
fn valid_tolerance(value: Option<f64>) -> bool {
    value.is_none_or(|value| value.is_finite() && value >= 0.0)
}

// compare.json の中身から、比較の方式と制限を決める。誤りがあれば理由を返す（テストは実行しない）
pub fn parse_settings(text: &str) -> Result<TestSettings, String> {
    let file: CompareFile = serde_json::from_str(text).map_err(|error| {
        format!("compare.json を読めません（{error}）。例：{{\"mode\": \"float\", \"abs\": 1e-6, \"rel\": 1e-6, \"timeLimitMs\": 2500, \"memoryLimitMiB\": 1024, \"randomCount\": 100}}")
    })?;
    let time_limit_ms = file.time_limit_ms.unwrap_or(DEFAULT_TIME_LIMIT_MS);
    if !(1..=MAX_TIME_LIMIT_MS).contains(&time_limit_ms) {
        return Err(format!(
            "compare.json：timeLimitMs は 1 から {MAX_TIME_LIMIT_MS} までの整数（ミリ秒）にしてください"
        ));
    }
    let memory_limit_mib = file.memory_limit_mib.unwrap_or(DEFAULT_MEMORY_LIMIT_MIB);
    if !(1..=MAX_MEMORY_LIMIT_MIB).contains(&memory_limit_mib) {
        return Err(format!(
            "compare.json：memoryLimitMiB は 1 から {MAX_MEMORY_LIMIT_MIB} までの整数（MiB）にしてください"
        ));
    }
    let random_count = file.random_count.unwrap_or(DEFAULT_RANDOM_COUNT);
    if !(1..=MAX_RANDOM_COUNT).contains(&random_count) {
        return Err(format!(
            "compare.json：randomCount は 1 から {MAX_RANDOM_COUNT} までの整数（ランダムテストの回数）にしてください"
        ));
    }
    Ok(TestSettings {
        mode: mode_of(&file)?,
        time_limit_ms,
        memory_limit_mib,
        random_count,
    })
}

fn mode_of(file: &CompareFile) -> Result<Mode, String> {
    let tolerances = file.abs.is_some() || file.rel.is_some();
    let mode = file.mode.as_deref().unwrap_or("whitespace");
    match mode {
        "whitespace" | "exact" | "none" if tolerances => Err(format!(
            "compare.json：mode が \"{mode}\" のときは abs・rel を書かないでください（mode を省くと whitespace）"
        )),
        "whitespace" => Ok(Mode::Whitespace),
        "exact" => Ok(Mode::Exact),
        "none" => Ok(Mode::None),
        "float" => {
            // 許容誤差に既定値を設けない（原典 §6.2、D-092）
            if !tolerances {
                return Err(
                    "compare.json：mode が \"float\" のときは、abs（絶対誤差）か rel（相対誤差）の少なくとも一方を書いてください".to_string(),
                );
            }
            if !valid_tolerance(file.abs) || !valid_tolerance(file.rel) {
                return Err("compare.json：abs・rel は 0 以上の数にしてください".to_string());
            }
            Ok(Mode::Float {
                abs: file.abs,
                rel: file.rel,
            })
        }
        other => Err(format!(
            "compare.json：mode \"{other}\" は使えません。whitespace・exact・float・none のどれかにしてください"
        )),
    }
}

// テストのフォルダの compare.json を読む。なければ空白緩和と既定の制限（D-092・D-141）
pub fn read_settings(test_dir: &Path) -> Result<TestSettings, String> {
    let path = test_dir.join("compare.json");
    if !path.is_file() {
        return Ok(TestSettings::default());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("compare.json を読めません（{error}）"))?;
    parse_settings(&text)
}

// AtCoder のルールの空白文字（CR・LF・Space・Tab。04 §4.1）
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

fn tokens(text: &str) -> Vec<&str> {
    text.split(is_space)
        .filter(|token| !token.is_empty())
        .collect()
}

// 空白緩和（W-2。D-093）：空白で区切った語の列が一致し、期待出力が空白で始まる（終わる）なら、
// 出力も空白で始まる（終わる）
fn whitespace_equal(expected: &str, actual: &str) -> bool {
    let starts = |text: &str| text.starts_with(is_space);
    let ends = |text: &str| text.ends_with(is_space);
    tokens(expected) == tokens(actual)
        && (!starts(expected) || starts(actual))
        && (!ends(expected) || ends(actual))
}

// 浮動小数点：各語を比べる。両方が数なら、指定した誤差のどちらかの範囲内で正解。数でない語は完全一致
fn float_equal(expected: &str, actual: &str, abs: Option<f64>, rel: Option<f64>) -> bool {
    let expected = tokens(expected);
    let actual = tokens(actual);
    expected.len() == actual.len()
        && expected
            .iter()
            .zip(&actual)
            .all(|(e, a)| match (e.parse::<f64>(), a.parse::<f64>()) {
                (Ok(e_value), Ok(a_value)) if e_value.is_finite() && a_value.is_finite() => {
                    let diff = (e_value - a_value).abs();
                    abs.is_some_and(|abs| diff <= abs)
                        || rel.is_some_and(|rel| diff <= rel * e_value.abs())
                        || e == a
                }
                _ => e == a,
            })
}

// Windows だけ（段階 2 S6）：\r のすぐあとに \n が来る組だけを \n に直す（\r だけのものは残す）。
// C++（MinGW）・Python の解答はパイプへ \r\n を出し、Windows のエディタで作った期待出力も \r\n になりうるため
#[cfg(windows)]
pub(crate) fn crlf_to_lf(bytes: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .enumerate()
        .filter(|(index, byte)| !(**byte == b'\r' && bytes.get(index + 1) == Some(&b'\n')))
        .map(|(_, byte)| *byte)
        .collect()
}

// 比べた結果。None は「比較不可・目視確認」
pub fn judge(mode: &Mode, expected: &[u8], actual: &[u8]) -> Option<bool> {
    match mode {
        Mode::None => None,
        #[cfg(not(windows))]
        Mode::Exact => Some(expected == actual),
        // Windows では、両方の \r\n を \n に直してから比べる（段階 2 S6）
        #[cfg(windows)]
        Mode::Exact => Some(crlf_to_lf(expected) == crlf_to_lf(actual)),
        Mode::Whitespace => Some(whitespace_equal(
            &String::from_utf8_lossy(expected),
            &String::from_utf8_lossy(actual),
        )),
        Mode::Float { abs, rel } => Some(float_equal(
            &String::from_utf8_lossy(expected),
            &String::from_utf8_lossy(actual),
            *abs,
            *rel,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(expected: &str, actual: &str) -> Option<bool> {
        judge(&Mode::Whitespace, expected.as_bytes(), actual.as_bytes())
    }

    #[test]
    fn whitespace_rule_follows_w2() {
        assert_eq!(ws("1 2\n", "1 2\n"), Some(true));
        // 空白の置き換え・追加は正解
        assert_eq!(ws("1 2\n", "1\n2\n"), Some(true));
        assert_eq!(ws("1 2\n", " 1  2 \n\n"), Some(true));
        assert_eq!(ws("1 2\n", "1\t2\r\n"), Some(true));
        // 末尾の改行がないのは不正解（W-2）
        assert_eq!(ws("1 2\n", "1 2"), Some(false));
        // 語の削除・連結は不正解
        assert_eq!(ws("1 2\n", "12\n"), Some(false));
        assert_eq!(ws("1 2\n", "1\n"), Some(false));
        // 期待出力が空白で始まらなければ、先頭の空白は問わない
        assert_eq!(ws("Yes\n", "\nYes\n"), Some(true));
        // 空の出力を期待する
        assert_eq!(ws("", ""), Some(true));
        assert_eq!(ws("", "\n"), Some(true));
        assert_eq!(ws("", "0\n"), Some(false));
    }

    #[test]
    fn exact_compares_bytes() {
        assert_eq!(judge(&Mode::Exact, b"1 2\n", b"1 2\n"), Some(true));
        // Windows では CRLF を LF に直してから比べる（下の exact_ignores_crlf_on_windows）
        #[cfg(not(windows))]
        assert_eq!(judge(&Mode::Exact, b"1 2\n", b"1 2\r\n"), Some(false));
        assert_eq!(judge(&Mode::Exact, b"1 2\n", b"1 3\n"), Some(false));
    }

    #[cfg(windows)]
    #[test]
    fn exact_ignores_crlf_on_windows() {
        // 段階 2 S6：C++（MinGW）・Python の解答はパイプへ \r\n を出す。Windows では、期待出力と実際の出力の両方で、
        // \r のすぐあとに \n が来る組だけを \n に直してから比べる
        assert_eq!(
            judge(&Mode::Exact, b"1 2\n3\n", b"1 2\r\n3\r\n"),
            Some(true)
        );
        // Windows のエディタで作った期待出力（CRLF）と、Rust の解答の出力（LF）
        assert_eq!(judge(&Mode::Exact, b"1 2\r\n", b"1 2\n"), Some(true));
        // \r だけのものは直さない（違いとして見つかる）
        assert_eq!(judge(&Mode::Exact, b"1 2\n", b"1 2\r"), Some(false));
        assert_eq!(judge(&Mode::Exact, b"1 2\n", b"1\r2\n"), Some(false));
        assert_eq!(judge(&Mode::Exact, b"1 2\n", b"1 2\r\r\n"), Some(false));
        // 直したあとも中身が違えば不正解
        assert_eq!(judge(&Mode::Exact, b"1 2\n", b"1 3\r\n"), Some(false));
    }

    #[test]
    fn float_uses_given_tolerances_only() {
        let both = Mode::Float {
            abs: Some(1e-6),
            rel: Some(1e-6),
        };
        assert_eq!(judge(&both, b"0.5\n", b"0.5000004\n"), Some(true));
        assert_eq!(judge(&both, b"0.5\n", b"0.51\n"), Some(false));
        // 大きい値は相対誤差で正解
        assert_eq!(judge(&both, b"1000000\n", b"1000000.5\n"), Some(true));
        // 数でない語は完全一致
        assert_eq!(judge(&both, b"Yes 0.5\n", b"Yes 0.5\n"), Some(true));
        assert_eq!(judge(&both, b"Yes 0.5\n", b"No 0.5\n"), Some(false));
        // 語の数が違えば不正解
        assert_eq!(judge(&both, b"1 2\n", b"1\n"), Some(false));
        // 絶対誤差だけを指定したときは、相対誤差では正解にしない
        let abs_only = Mode::Float {
            abs: Some(1e-6),
            rel: None,
        };
        assert_eq!(judge(&abs_only, b"1000000\n", b"1000000.5\n"), Some(false));
    }

    #[test]
    fn none_is_not_judged() {
        assert_eq!(judge(&Mode::None, b"1\n", b"2\n"), None);
    }

    fn parse_mode(text: &str) -> Result<Mode, String> {
        parse_settings(text).map(|settings| settings.mode)
    }

    #[test]
    fn parses_limits_in_compare_json() {
        // 書かなければ既定値（2500 ms・1024 MiB）
        assert_eq!(
            parse_settings(r#"{"mode":"exact"}"#).unwrap().time_limit_ms,
            2500
        );
        assert_eq!(parse_settings("{}"), Ok(TestSettings::default()));
        // 制限だけを書いてもよい（mode は空白緩和）
        assert_eq!(
            parse_settings(r#"{"timeLimitMs":3000,"memoryLimitMiB":256}"#),
            Ok(TestSettings {
                mode: Mode::Whitespace,
                time_limit_ms: 3000,
                memory_limit_mib: 256,
                random_count: 100,
            })
        );
        // ランダムテストの回数（既定 100、1〜10000。D-143）
        assert_eq!(parse_settings("{}").unwrap().random_count, 100);
        assert_eq!(
            parse_settings(r#"{"randomCount":1}"#).unwrap().random_count,
            1
        );
        assert_eq!(
            parse_settings(r#"{"randomCount":10000}"#)
                .unwrap()
                .random_count,
            10000
        );
        assert_eq!(
            parse_settings(r#"{"timeLimitMs":60000}"#)
                .unwrap()
                .time_limit_ms,
            60000
        );
        // 範囲の外、整数でない、負、文字列は誤り
        for text in [
            r#"{"timeLimitMs":0}"#,
            r#"{"timeLimitMs":60001}"#,
            r#"{"timeLimitMs":2.5}"#,
            r#"{"timeLimitMs":-1}"#,
            r#"{"timeLimitMs":"2500"}"#,
            r#"{"memoryLimitMiB":0}"#,
            r#"{"memoryLimitMiB":65537}"#,
            r#"{"timeLimitMS":2500}"#,
            r#"{"randomCount":0}"#,
            r#"{"randomCount":10001}"#,
            r#"{"randomCount":2.5}"#,
            r#"{"randomCount":-1}"#,
            r#"{"randomCount":"100"}"#,
            r#"{"randomcount":100}"#,
        ] {
            assert!(parse_settings(text).is_err(), "{text}");
        }
        // mode を省いて abs を書くのは誤り
        assert!(parse_settings(r#"{"abs":1e-6}"#).is_err());
    }

    #[test]
    fn parses_compare_json() {
        assert_eq!(parse_mode(r#"{"mode":"exact"}"#), Ok(Mode::Exact));
        assert_eq!(parse_mode(r#"{"mode":"none"}"#), Ok(Mode::None));
        assert_eq!(
            parse_mode(r#"{"mode":"float","abs":1e-6}"#),
            Ok(Mode::Float {
                abs: Some(1e-6),
                rel: None
            })
        );
        // 誤り：既定値を設けないので、float には誤差が要る
        assert!(parse_mode(r#"{"mode":"float"}"#).is_err());
        assert!(parse_mode(r#"{"mode":"float","abs":-1}"#).is_err());
        assert!(parse_mode(r#"{"mode":"exact","abs":1e-6}"#).is_err());
        assert!(parse_mode(r#"{"mode":"near"}"#).is_err());
        assert!(parse_mode(r#"{"mode":"exact","eps":1}"#).is_err());
        assert!(parse_mode("not json").is_err());
    }
}
