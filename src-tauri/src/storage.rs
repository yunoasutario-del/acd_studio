// IDE の作業データ（設定・レイアウト・前回の作業領域）の保存（作業 6）。
// D-034：自前の JSON。D-035：アプリのフォルダ。D-036：一時ファイルと名前の変更（W3）。
// D-039：読めないときは別名で退避して既定値で起動する。D-041：ファイルの権限は 0600。
//
// settings.json：キー割り当てと見た目（色・表示の入切・文字の大きさ）、提出の設定。利用者が手で書き換えてよい（D-052・D-159・D-162）
// state.json：レイアウト、前回の作業領域、開いていたタブ（IDE が書き続ける）
// trusted.json：信頼したフォルダの一覧（D-128。IDE が書く。手で書き換えて取り消してもよい）
// history.json：実行履歴（Phase 4a 作業 4。D-137。history.rs）

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cpp::CppSettings;
use crate::language::Language;
use crate::python::{Interpreter, PythonSettings};

// 保存するデータの版番号（D-039）。形を変えたら上げる
pub const VERSION: u32 = 1;

pub const SETTINGS_FILE: &str = "settings.json";
pub const STATE_FILE: &str = "state.json";
pub const TRUSTED_FILE: &str = "trusted.json";
const LOCK_FILE: &str = "instance.lock";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub sidebar_width: f64,
    pub sidebar_visible: bool,
    pub right_width: f64,
    pub right_visible: bool,
    pub bottom_ratio: f64,
    pub bottom_visible: bool,
    pub bottom_right_ratio: f64,
    pub bottom_right_visible: bool,
    // サイドバーの中身（Phase 12 作業 1。D-182）。値は画面側が確かめる（知らなければエクスプローラー）。
    // 前の版の state.json にはないので、なければエクスプローラー
    #[serde(default = "default_sidebar_view")]
    pub sidebar_view: String,
}

fn default_sidebar_view() -> String {
    "files".to_string()
}

// state.json の中身。作業領域のパスは Rust 側だけが扱う（WebView には名前だけを渡す。D-065）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedState {
    pub version: u32,
    pub layout: Layout,
    pub workspace: Option<PathBuf>,
    // 作業領域のルートからの相対パス
    pub tabs: Vec<String>,
    pub active_tab: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyBinding {
    pub command: String,
    pub key: String,
    #[serde(default)]
    pub meta: bool,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
}

// 初期のキー割り当て（D-022・D-051）。フロントエンドの src/keybindings.ts の既定と同じ内容
pub fn default_key_bindings() -> Vec<KeyBinding> {
    // modifiers は (⌘, ⌥, ⇧)
    let binding = |command: &str, key: &str, (meta, alt, shift): (bool, bool, bool)| KeyBinding {
        command: command.to_string(),
        key: key.to_string(),
        meta,
        ctrl: false,
        alt,
        shift,
    };
    vec![
        binding("toggleSidebar", "b", (true, false, false)),
        binding("save", "s", (true, false, false)),
        // 作業 7b（D-083）
        binding("closeTab", "w", (true, false, false)),
        binding("openFolder", "o", (true, false, false)),
        // Phase 2 作業 6（D-101）：⌘R 実行、⇧⌘R サンプルテスト、⌘. 停止
        binding("run", "r", (true, false, false)),
        binding("test", "r", (true, false, true)),
        binding("stop", ".", (true, false, false)),
        // Phase 3b-2 作業 4（D-133）：F12 定義へジャンプ、⇧F12 参照検索
        binding("goToDefinition", "f12", (false, false, false)),
        binding("findReferences", "f12", (false, false, true)),
        // Phase 3b-2 作業 6（D-133）：⇧⌥F 整形
        binding("formatDocument", "f", (false, true, true)),
        // Phase 5 作業 2（D-146 2.5）：F5 デバッグの開始・続行、F9 ブレークポイント、F10 ステップオーバー、
        // F11 ステップイン、⇧F11 ステップアウト
        binding("debug", "f5", (false, false, false)),
        binding("toggleBreakpoint", "f9", (false, false, false)),
        binding("stepOver", "f10", (false, false, false)),
        binding("stepIn", "f11", (false, false, false)),
        binding("stepOut", "f11", (false, false, true)),
        // Phase 8 作業 6（D-159）：⌘, 設定
        binding("openSettings", ",", (true, false, false)),
        // Phase 12 作業 1（D-182）：⇧⌘E エクスプローラー、⇧⌘F 検索、⇧⌘D 実行とデバッグ
        binding("showExplorer", "e", (true, false, true)),
        binding("showSearch", "f", (true, false, true)),
        binding("showDebug", "d", (true, false, true)),
    ]
}

// settings.json の見た目の設定（Phase 8 作業 6。D-159）。書いていない項目・読めない項目は既定の値
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    // 色の変数の名前（先頭の -- を除く）と値。色の変数の名前か、値が色として読めるかは、画面側で確かめる
    pub colors: BTreeMap<String, String>,
    pub indent_rainbow: bool,
    pub error_lens: bool,
    // パンくずリスト（Phase 18 作業 1。D-216）
    pub breadcrumbs: bool,
    pub editor_font_size: u32,
    pub problem_font_size: u32,
}

// settings.json の提出の設定（Phase 9 作業 2。D-162）。書いていない・読めない項目は true（今までどおり確かめる側）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitSettings {
    // 「提出」で、まずサンプルテストを動かす。すべて AC でなければ提出しない
    pub test_before_submit: bool,
    // false なら、サンプルテストがすべて AC のときだけ、確認のダイアログを省く
    pub confirm_dialog: bool,
}

impl Default for SubmitSettings {
    fn default() -> Self {
        SubmitSettings {
            test_before_submit: true,
            confirm_dialog: true,
        }
    }
}

fn parse_submit(value: &Value, problems: &mut Vec<String>) -> SubmitSettings {
    let mut submit = SubmitSettings::default();
    if let Some(section) = section(value, "submit", problems) {
        submit.test_before_submit = flag(section, "testBeforeSubmit", "submit", problems);
        submit.confirm_dialog = flag(section, "confirmDialog", "submit", problems);
    }
    submit
}

// settings.json の問題文の欄の設定（Phase 15 作業 3。D-196・D-197）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemSettings {
    // 見出しの行に出すボタン（PROBLEM_BUTTONS の名前）。出さないものは「…」の一覧に入る
    pub buttons: Vec<String>,
    // テストケースが複数のとき、main() に input! { t } と for _ in 0..t { solve(); } まで書く
    pub main_loop: bool,
    // 問題・コンテストを作るときに input! を入れておく（D-198）
    pub fill_input: bool,
    // 問題・コンテストを作るときの言語（Phase 16 作業 3。D-208）
    pub language: Language,
}

// 見出しの行に出せるボタンの名前（input! を作る・サンプルを保存・再取得・公式ページ・この問題を作る）
pub const PROBLEM_BUTTONS: [&str; 5] = [
    "makeInput",
    "saveSamples",
    "refresh",
    "openPage",
    "createProblem",
];

impl Default for ProblemSettings {
    fn default() -> Self {
        ProblemSettings {
            buttons: vec!["makeInput".to_string(), "saveSamples".to_string()],
            main_loop: false,
            fill_input: false,
            language: Language::Rust,
        }
    }
}

// problem の欄の buttons・mainLoop。欄がオブジェクトでないことは parse_appearance が知らせる
fn parse_problem_settings(value: &Value, problems: &mut Vec<String>) -> ProblemSettings {
    let mut settings = ProblemSettings::default();
    let Some(section) = value.get("problem").filter(|section| section.is_object()) else {
        return settings;
    };
    match section.get("buttons") {
        None => {}
        Some(Value::Array(items)) => {
            let mut buttons = Vec::new();
            for item in items {
                match item.as_str().filter(|name| PROBLEM_BUTTONS.contains(name)) {
                    Some(name) if !buttons.iter().any(|b: &String| b == name) => {
                        buttons.push(name.to_string())
                    }
                    Some(_) => {}
                    None => problems.push(format!("problem.buttons の「{item}」")),
                }
            }
            settings.buttons = buttons;
        }
        Some(_) => problems.push(format!(
            "problem.buttons（{} の一覧にしてください）",
            PROBLEM_BUTTONS.join("・")
        )),
    }
    for (key, slot) in [
        ("mainLoop", &mut settings.main_loop),
        ("fillInput", &mut settings.fill_input),
    ] {
        match section.get(key) {
            None => {}
            Some(Value::Bool(on)) => *slot = *on,
            Some(_) => problems.push(format!("problem.{key}（true か false にしてください）")),
        }
    }
    match section.get("language") {
        None => {}
        Some(value) => match serde_json::from_value::<Language>(value.clone()) {
            Ok(language) => settings.language = language,
            Err(_) => problems
                .push("problem.language（rust・cpp・python のどれかにしてください）".to_string()),
        },
    }
    settings
}

// settings.json の cpp の欄（Phase 16 作業 1。D-205）。compiler は g++ の絶対パス（空なら探す）
fn parse_cpp_settings(value: &Value, problems: &mut Vec<String>) -> CppSettings {
    let mut settings = CppSettings::default();
    if let Some(section) = section(value, "cpp", problems) {
        match section.get("compiler") {
            None => {}
            Some(Value::String(path)) => settings.compiler = path.trim().to_string(),
            Some(_) => {
                problems.push("cpp.compiler（g++ のパスの文字列にしてください）".to_string())
            }
        }
    }
    settings
}

// C++ をビルドするときに読む（D-205）。読めない・ないときは既定（g++ を探す）
pub fn cpp_settings(dir: &Path) -> CppSettings {
    match reread_settings(dir) {
        Ok(Some(loaded)) => loaded.cpp,
        _ => CppSettings::default(),
    }
}

// settings.json の python の欄（Phase 19 作業 1。D-221）。interpreter は cpython か pypy、
// cpython・pypy は処理系の絶対パス（空なら探す）
fn parse_python_settings(value: &Value, problems: &mut Vec<String>) -> PythonSettings {
    let mut settings = PythonSettings::default();
    if let Some(section) = section(value, "python", problems) {
        match section.get("interpreter") {
            None => {}
            Some(Value::String(name)) if name == "cpython" => {
                settings.interpreter = Interpreter::Cpython
            }
            Some(Value::String(name)) if name == "pypy" => settings.interpreter = Interpreter::Pypy,
            Some(_) => problems
                .push("python.interpreter（\"cpython\" か \"pypy\" にしてください）".to_string()),
        }
        for (key, slot) in [
            ("cpython", &mut settings.cpython),
            ("pypy", &mut settings.pypy),
        ] {
            match section.get(key) {
                None => {}
                Some(Value::String(path)) => *slot = path.trim().to_string(),
                Some(_) => problems.push(format!(
                    "python.{key}（処理系のパスの文字列にしてください）"
                )),
            }
        }
    }
    settings
}

// Python を動かすときに読む（D-221）。読めない・ないときは既定（CPython を探す）
pub fn python_settings(dir: &Path) -> PythonSettings {
    match reread_settings(dir) {
        Ok(Some(loaded)) => loaded.python,
        _ => PythonSettings::default(),
    }
}

// 問題・コンテストを作るときに読む（D-198）。読めない・ないときは既定（input! は入れない）
pub fn problem_settings(dir: &Path) -> ProblemSettings {
    match reread_settings(dir) {
        Ok(Some(loaded)) => loaded.problem,
        _ => ProblemSettings::default(),
    }
}

// 提出のときに読む（D-162）。読めない・ないときは既定（確かめる側）
pub fn submit_settings(dir: &Path) -> SubmitSettings {
    match reread_settings(dir) {
        Ok(Some(loaded)) => loaded.submit,
        _ => SubmitSettings::default(),
    }
}

// 文字の大きさ（px）の既定と範囲
pub const DEFAULT_FONT_SIZE: u32 = 13;
const MIN_FONT_SIZE: u32 = 8;
const MAX_FONT_SIZE: u32 = 32;
// 色の変数の名前・値の長さの上限
const MAX_COLOR_NAME: usize = 64;
const MAX_COLOR_VALUE: usize = 100;

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            colors: BTreeMap::new(),
            indent_rainbow: true,
            error_lens: true,
            breadcrumbs: true,
            editor_font_size: DEFAULT_FONT_SIZE,
            problem_font_size: DEFAULT_FONT_SIZE,
        }
    }
}

// 見た目の設定を読む。読めない項目は既定の値にして、その項目の名前（と理由）を返す
pub fn parse_appearance(value: &Value) -> (Appearance, Vec<String>) {
    let mut appearance = Appearance::default();
    let mut problems = Vec::new();
    match value.get("colors") {
        None => {}
        Some(Value::Object(colors)) => {
            for (name, color) in colors {
                let name_ok = !name.is_empty()
                    && name.len() <= MAX_COLOR_NAME
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
                match color.as_str() {
                    Some(color)
                        if name_ok
                            && !color.trim().is_empty()
                            && color.len() <= MAX_COLOR_VALUE =>
                    {
                        appearance.colors.insert(name.clone(), color.to_string());
                    }
                    _ => {
                        let shown: String = name.chars().take(MAX_COLOR_NAME).collect();
                        problems.push(format!("colors の「{shown}」"));
                    }
                }
            }
        }
        Some(_) => problems.push("colors（名前と色の組にしてください）".to_string()),
    }
    if let Some(editor) = section(value, "editor", &mut problems) {
        appearance.indent_rainbow = flag(editor, "indentRainbow", "editor", &mut problems);
        appearance.error_lens = flag(editor, "errorLens", "editor", &mut problems);
        appearance.breadcrumbs = flag(editor, "breadcrumbs", "editor", &mut problems);
        appearance.editor_font_size = font_size(editor, "editor", &mut problems);
    }
    if let Some(problem) = section(value, "problem", &mut problems) {
        appearance.problem_font_size = font_size(problem, "problem", &mut problems);
    }
    (appearance, problems)
}

// editor・problem の欄。なければ None。オブジェクトでなければ知らせて None
fn section<'a>(value: &'a Value, name: &str, problems: &mut Vec<String>) -> Option<&'a Value> {
    match value.get(name) {
        None => None,
        Some(section) if section.is_object() => Some(section),
        Some(_) => {
            problems.push(name.to_string());
            None
        }
    }
}

// 入切。なければ true
fn flag(section: &Value, key: &str, prefix: &str, problems: &mut Vec<String>) -> bool {
    match section.get(key) {
        None => true,
        Some(Value::Bool(on)) => *on,
        Some(_) => {
            problems.push(format!("{prefix}.{key}（true か false にしてください）"));
            true
        }
    }
}

// 文字の大きさ。なければ既定。8〜32 の整数でなければ知らせて既定
fn font_size(section: &Value, prefix: &str, problems: &mut Vec<String>) -> u32 {
    match section.get("fontSize") {
        None => DEFAULT_FONT_SIZE,
        Some(size) => match size.as_u64() {
            Some(size) if (MIN_FONT_SIZE as u64..=MAX_FONT_SIZE as u64).contains(&size) => {
                size as u32
            }
            _ => {
                problems.push(format!(
                    "{prefix}.fontSize（{MIN_FONT_SIZE}〜{MAX_FONT_SIZE} の整数にしてください）"
                ));
                DEFAULT_FONT_SIZE
            }
        },
    }
}

// 読み込みの結果
#[derive(Debug, PartialEq)]
pub enum Loaded<T> {
    // ファイルがなかった（初めての起動）
    Missing,
    Ok(T),
    // 読めなかったので退避した。退避先のパス
    Broken(PathBuf),
}

// Windows には Unix のパーミッション（0700・0600）がない。Unix の DirBuilderExt・OpenOptionsExt の mode と
// 同じ名前で何もしないものを置き、Unix と同じ書き方のまま動くようにする。アプリのフォルダ（%APPDATA% の下）は
// 利用者だけが読み書きできる ACL になっていると思われる（推測。未確認。docs/windows-port-plan.md 3.3）
#[cfg(windows)]
trait ModeShim {
    fn mode(&mut self, mode: u32) -> &mut Self;
}

#[cfg(windows)]
impl ModeShim for fs::DirBuilder {
    fn mode(&mut self, _mode: u32) -> &mut Self {
        self
    }
}

#[cfg(windows)]
impl ModeShim for fs::OpenOptions {
    fn mode(&mut self, _mode: u32) -> &mut Self {
        self
    }
}

// アプリのフォルダを作る（所有者だけが読み書きできる 0700）
pub fn ensure_dir(dir: &Path) -> Result<(), String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|_| "アプリのフォルダを作れません".to_string())
}

// W3：同じフォルダの一時ファイルに書き、sync_all で確定させ、名前の変更で置き換える（D-036）。
// 権限は 0600（D-041）。OS のクラッシュ・電源断に対する完全な耐久性は保証しない
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| "保存先が不正です".to_string())?;
    ensure_dir(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // 同じファイルへの書き込みが同時に起きても混ざらないように、書くたびに進む番号も入れる（Phase 10 作業 14）
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.tmp-{}-{serial}", std::process::id()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        // すでにあった一時ファイルの権限も 0600 にそろえる
        #[cfg(unix)]
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|_| "設定を保存できません".to_string())
}

// 退避したファイルを、種類ごとに残す数（新しいものから。Phase 10 作業 15。D-173）
const BACKUPS_KEPT: usize = 5;

// 読めないファイルを別名で退避する（例：state.broken-1792130000.json。同じ秒なら -1・-2 を付ける）。
// 同じ種類の退避が BACKUPS_KEPT を超えたら、古いものから消す（IDE が作った退避だけ）
fn move_aside(path: &Path) -> Result<PathBuf, String> {
    move_aside_as(path, "broken").map_err(|_| "読めない設定ファイルを退避できません".to_string())
}

// 置き換える前のファイルを別名で退避する（例：snippets.backup-1792130000.json。Phase 13 作業 2。D-187）。
// 残す数は、読めないファイルの退避と同じ（種類ごとに BACKUPS_KEPT）
pub fn back_up(path: &Path) -> Result<PathBuf, String> {
    move_aside_as(path, "backup")
}

// path を stem.<label>-<秒>[-<番号>].json に移し、同じ種類の古い退避を消す
fn move_aside_as(path: &Path, label: &str) -> Result<PathBuf, String> {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut backup = path.with_file_name(format!("{stem}.{label}-{secs}.json"));
    let mut serial = 0;
    while fs::symlink_metadata(&backup).is_ok() {
        serial += 1;
        backup = path.with_file_name(format!("{stem}.{label}-{secs}-{serial}.json"));
    }
    fs::rename(path, &backup).map_err(|_| "ファイルを退避できません".to_string())?;
    prune_backups(path, &stem, label);
    Ok(backup)
}

// stem.<label>-<秒>[-<番号>].json のうち、新しい BACKUPS_KEPT 個だけを残す
fn prune_backups(path: &Path, stem: &str, label: &str) {
    let Some(dir) = path.parent() else {
        return;
    };
    let prefix = format!("{stem}.{label}-");
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut backups: Vec<((u64, u64), PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let rest = name.strip_prefix(&prefix)?.strip_suffix(".json")?;
            let (secs, serial) = rest.split_once('-').unwrap_or((rest, "0"));
            let key = (secs.parse().ok()?, serial.parse().ok()?);
            entry.path().is_file().then(|| (key, entry.path()))
        })
        .collect();
    backups.sort();
    let excess = backups.len().saturating_sub(BACKUPS_KEPT);
    for (_, old) in backups.into_iter().take(excess) {
        let _ = fs::remove_file(old);
    }
}

// JSON として読めて、版が今のものなら、その中身
fn versioned(bytes: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .filter(|value| value.get("version").and_then(Value::as_u64) == Some(VERSION as u64))
}

// JSON のファイルを読む。なければ Missing。JSON として読めない、版が違う、形が違うときは退避して Broken（D-039）
pub(crate) fn load_json<T>(
    path: &Path,
    parse: impl FnOnce(Value) -> Option<T>,
) -> Result<Loaded<T>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Loaded::Missing),
        Err(_) => return Err("設定ファイルを読めません".to_string()),
    };
    match versioned(&bytes).and_then(parse) {
        Some(value) => Ok(Loaded::Ok(value)),
        None => Ok(Loaded::Broken(move_aside(path)?)),
    }
}

pub fn load_state(dir: &Path) -> Result<Loaded<SavedState>, String> {
    load_json(&dir.join(STATE_FILE), |value| {
        serde_json::from_value(value).ok()
    })
}

pub fn save_state(dir: &Path, state: &SavedState) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(state).map_err(|_| "設定を保存できません".to_string())?;
    write_atomic(&dir.join(STATE_FILE), &bytes)
}

// 信頼の記録（D-128）。folders は信頼したフォルダ、denied は「信頼しない」を選んだフォルダ（Phase 10 作業 12。
// 次からは聞かない。D-172）。パスは canonicalize 済みのもので、完全に同じものだけに当てはめる
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrustRecord {
    pub folders: Vec<PathBuf>,
    pub denied: Vec<PathBuf>,
}

fn paths_of(value: &Value) -> Option<Vec<PathBuf>> {
    value
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(PathBuf::from))
        .collect()
}

// 読めないときは退避する（呼び出し側は何も信頼していないものとして扱う）。denied はなくてよい（前の版の記録）
pub fn load_trusted(dir: &Path) -> Result<Loaded<TrustRecord>, String> {
    load_json(&dir.join(TRUSTED_FILE), |value| {
        let folders = paths_of(value.get("folders")?)?;
        let denied = match value.get("denied") {
            None => Vec::new(),
            Some(denied) => paths_of(denied)?,
        };
        Some(TrustRecord { folders, denied })
    })
}

pub fn save_trusted(dir: &Path, record: &TrustRecord) -> Result<(), String> {
    // UTF-8 で書けないパスは記録できない（その作業領域は開くたびに確かめる）
    let strings = |paths: &[PathBuf]| -> Vec<String> {
        paths
            .iter()
            .filter_map(|path| path.to_str().map(str::to_string))
            .collect()
    };
    let value = serde_json::json!({
        "version": VERSION,
        "folders": strings(&record.folders),
        "denied": strings(&record.denied),
    });
    let bytes =
        serde_json::to_vec_pretty(&value).map_err(|_| "設定を保存できません".to_string())?;
    write_atomic(&dir.join(TRUSTED_FILE), &bytes)
}

// settings.json のキー割り当てと見た目。正しくない割り当て・項目は、それだけを使わずに知らせる（D-052 の保留事項・D-159）
#[derive(Debug, PartialEq)]
pub struct LoadedSettings {
    // key が空のものは「割り当てなし」（D-159）
    pub key_bindings: Vec<KeyBinding>,
    // 読めなかった割り当て（JSON の文字列）
    pub invalid: Vec<String>,
    pub appearance: Appearance,
    pub submit: SubmitSettings,
    pub problem: ProblemSettings,
    pub cpp: CppSettings,
    pub python: PythonSettings,
    // 読めなかった見た目・提出・問題文の欄・C++・Python の項目
    pub appearance_problems: Vec<String>,
}

// keyBindings はなくてもよい（書いていないコマンドは既定の割り当て）。あるのに一覧でなければ、読めないファイルとする
fn parse_settings(value: Value) -> Option<LoadedSettings> {
    let items = match value.get("keyBindings") {
        None => Vec::new(),
        Some(items) => items.as_array()?.clone(),
    };
    let mut key_bindings = Vec::new();
    let mut invalid = Vec::new();
    for item in items {
        match serde_json::from_value::<KeyBinding>(item.clone()) {
            Ok(binding) => key_bindings.push(binding),
            _ => invalid.push(item.to_string()),
        }
    }
    let (appearance, mut appearance_problems) = parse_appearance(&value);
    let submit = parse_submit(&value, &mut appearance_problems);
    let problem = parse_problem_settings(&value, &mut appearance_problems);
    let cpp = parse_cpp_settings(&value, &mut appearance_problems);
    let python = parse_python_settings(&value, &mut appearance_problems);
    Some(LoadedSettings {
        key_bindings,
        invalid,
        appearance,
        submit,
        problem,
        cpp,
        python,
        appearance_problems,
    })
}

pub fn load_settings(dir: &Path) -> Result<Loaded<LoadedSettings>, String> {
    load_json(&dir.join(SETTINGS_FILE), parse_settings)
}

// 窓に戻ったときの読み直し（D-159）。退避はしない。なければ Ok(None)。
// 読めない（書いている途中で崩れているなど）ときは Err で、呼び出し側は今の設定のまま使う
pub fn reread_settings(dir: &Path) -> Result<Option<LoadedSettings>, String> {
    let bytes = match fs::read(dir.join(SETTINGS_FILE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("settings.json を読めません".to_string()),
    };
    versioned(&bytes)
        .and_then(parse_settings)
        .map(Some)
        .ok_or_else(|| {
            format!(
                "settings.json を読めません（JSON の誤り、版が {VERSION} でない、または keyBindings が一覧でない）。今の設定のまま使います"
            )
        })
}

// settings.json の更新日時と大きさ。窓に戻ったときに、変わったかを比べる（D-159）。なければ None
pub fn settings_stamp(dir: &Path) -> Option<(SystemTime, u64)> {
    let metadata = fs::metadata(dir.join(SETTINGS_FILE)).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

// 設定の画面の保存（作業 6b。D-159）。今のファイルの keyBindings・colors と、editor・problem・submit・python の
// 設定の画面で扱う項目だけを入れ替え、ほかの項目は残す（python は interpreter だけ。処理系のパスは残す。Phase 19 作業 3。D-221）。読めないファイル（JSON の誤り・版の違い）には書かない
pub fn write_settings(
    dir: &Path,
    key_bindings: &[KeyBinding],
    appearance: &Appearance,
    submit: &SubmitSettings,
    problem_settings: &ProblemSettings,
    interpreter: Interpreter,
) -> Result<(), String> {
    let path = dir.join(SETTINGS_FILE);
    let mut value = match fs::read(&path) {
        Ok(bytes) => versioned(&bytes).filter(Value::is_object).ok_or_else(|| {
            "settings.json を読めないので、保存しません。テキストエディタで直してから、もう一度保存してください"
                .to_string()
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(_) => return Err("settings.json を読めません".to_string()),
    };
    let editor = serde_json::json!({
        "indentRainbow": appearance.indent_rainbow,
        "errorLens": appearance.error_lens,
        "breadcrumbs": appearance.breadcrumbs,
        "fontSize": appearance.editor_font_size,
    });
    let problem = serde_json::json!({
        "fontSize": appearance.problem_font_size,
        "buttons": problem_settings.buttons,
        "mainLoop": problem_settings.main_loop,
        "fillInput": problem_settings.fill_input,
        "language": problem_settings.language,
    });
    // 読むときと同じ決まりで確かめる（画面が確かめたものでも、範囲の外の値・知らない名前などは書かない）
    let checked = serde_json::json!({
        "colors": appearance.colors,
        "editor": editor,
        "problem": problem,
    });
    let (_, mut problems) = parse_appearance(&checked);
    parse_problem_settings(&checked, &mut problems);
    if !problems.is_empty() || key_bindings.iter().any(|binding| binding.key.len() > 32) {
        return Err(format!(
            "保存できない値があるので、保存しません：{}",
            problems.join("、")
        ));
    }
    let object = value
        .as_object_mut()
        .ok_or_else(|| "settings.json を読めません".to_string())?;
    object.insert("version".to_string(), VERSION.into());
    object.insert(
        "keyBindings".to_string(),
        serde_json::to_value(key_bindings).map_err(|_| "設定を保存できません".to_string())?,
    );
    object.insert(
        "colors".to_string(),
        serde_json::to_value(&appearance.colors).map_err(|_| "設定を保存できません".to_string())?,
    );
    merge_section(object, "editor", editor);
    merge_section(object, "problem", problem);
    merge_section(
        object,
        "submit",
        serde_json::json!({
            "testBeforeSubmit": submit.test_before_submit,
            "confirmDialog": submit.confirm_dialog,
        }),
    );
    merge_section(
        object,
        "python",
        serde_json::json!({ "interpreter": interpreter }),
    );
    let bytes =
        serde_json::to_vec_pretty(&value).map_err(|_| "設定を保存できません".to_string())?;
    write_atomic(&path, &bytes)
}

// editor・problem の欄の項目を入れ替える。欄にほかの項目があれば残す（オブジェクトでなければ置き換える）
fn merge_section(object: &mut serde_json::Map<String, Value>, name: &str, items: Value) {
    match object.get_mut(name) {
        Some(Value::Object(section)) => {
            if let Value::Object(items) = items {
                section.extend(items);
            }
        }
        _ => {
            object.insert(name.to_string(), items);
        }
    }
}

// 初めての起動のときに、既定の内容の settings.json を作る（利用者が書き換えられるように）
pub fn write_default_settings(dir: &Path) -> Result<(), String> {
    let value = serde_json::json!({
        "version": VERSION,
        "keyBindings": default_key_bindings(),
        "colors": {},
        "editor": { "indentRainbow": true, "errorLens": true, "breadcrumbs": true, "fontSize": DEFAULT_FONT_SIZE },
        "problem": {
            "fontSize": DEFAULT_FONT_SIZE,
            "buttons": ProblemSettings::default().buttons,
            "mainLoop": false,
            "fillInput": false,
            "language": "rust",
        },
        "submit": { "testBeforeSubmit": true, "confirmDialog": true },
        "cpp": { "compiler": "" },
        "python": { "interpreter": "cpython", "cpython": "", "pypy": "" },
    });
    let bytes =
        serde_json::to_vec_pretty(&value).map_err(|_| "設定を保存できません".to_string())?;
    write_atomic(&dir.join(SETTINGS_FILE), &bytes)
}

// 単一起動（D-040）。アプリのフォルダのロック用のファイルに排他のロックをかける。
// 取れたら Some(ファイル)。プロセスが終わるまで持ち続ける。ほかのプロセスが持っていれば None
pub fn acquire_instance_lock(dir: &Path) -> Result<Option<fs::File>, String> {
    ensure_dir(dir)?;
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join(LOCK_FILE))
        .map_err(|_| "ロック用のファイルを開けません".to_string())?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(fs::TryLockError::WouldBlock) => Ok(None),
        Err(_) => Err("ロックを取れません".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "atcoder-studio-storage-{}-{}",
                name,
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            TempDir(path.join("app"))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            if let Some(parent) = self.0.parent() {
                let _ = fs::remove_dir_all(parent);
            }
        }
    }

    fn sample_state() -> SavedState {
        SavedState {
            version: VERSION,
            layout: Layout {
                sidebar_width: 300.0,
                sidebar_visible: true,
                right_width: 400.0,
                right_visible: false,
                bottom_ratio: 0.25,
                bottom_visible: true,
                bottom_right_ratio: 0.375,
                bottom_right_visible: true,
                sidebar_view: "debug".to_string(),
            },
            workspace: Some(PathBuf::from("/tmp/ws")),
            tabs: vec!["src/main.rs".to_string()],
            active_tab: Some("src/main.rs".to_string()),
        }
    }

    #[test]
    fn saves_and_loads_state_with_private_permissions() {
        let dir = TempDir::new("state");
        assert_eq!(load_state(&dir.0), Ok(Loaded::Missing));
        save_state(&dir.0, &sample_state()).unwrap();
        assert_eq!(load_state(&dir.0), Ok(Loaded::Ok(sample_state())));
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.0.join(STATE_FILE)), 0o600);
        assert_eq!(mode(&dir.0), 0o700);
        // 一時ファイルは残らない
        let names: Vec<_> = fs::read_dir(&dir.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![STATE_FILE.to_string()]);
    }

    #[test]
    fn loads_state_without_sidebar_view() {
        // Phase 12 より前の state.json（sidebarView がない）も読め、エクスプローラーになる（D-182）
        let dir = TempDir::new("old-state");
        ensure_dir(&dir.0).unwrap();
        let mut value = serde_json::to_value(sample_state()).unwrap();
        value["layout"]
            .as_object_mut()
            .unwrap()
            .remove("sidebarView");
        fs::write(dir.0.join(STATE_FILE), value.to_string()).unwrap();
        let Ok(Loaded::Ok(state)) = load_state(&dir.0) else {
            panic!("読めない");
        };
        assert_eq!(state.layout.sidebar_view, "files");
    }

    #[test]
    fn concurrent_writes_do_not_mix() {
        let dir = TempDir::new("concurrent");
        ensure_dir(&dir.0).unwrap();
        let path = dir.0.join("same.json");
        // 同じファイルに、違う中身を同時に何度も書く。どの時点の中身も、どれか 1 つの書き込みの全体になる
        let writers: Vec<_> = (0..8)
            .map(|index| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let content = format!("{index}").repeat(20_000);
                    for _ in 0..20 {
                        write_atomic(&path, content.as_bytes()).unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.len(), 20_000);
        let first = text.chars().next().unwrap();
        assert!(text.chars().all(|c| c == first));
        // 一時ファイルは残らない
        let names: Vec<_> = fs::read_dir(&dir.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["same.json".to_string()]);
    }

    #[test]
    fn moves_broken_or_old_files_aside() {
        let dir = TempDir::new("broken");
        ensure_dir(&dir.0).unwrap();
        fs::write(dir.0.join(STATE_FILE), "{ not json").unwrap();
        let Ok(Loaded::Broken(backup)) = load_state(&dir.0) else {
            panic!("退避されるはず");
        };
        assert!(backup.exists());
        assert!(!dir.0.join(STATE_FILE).exists());
        // 版が違うファイルも退避する
        fs::write(
            dir.0.join(SETTINGS_FILE),
            r#"{"version": 99, "keyBindings": []}"#,
        )
        .unwrap();
        assert!(matches!(load_settings(&dir.0), Ok(Loaded::Broken(_))));
    }

    #[test]
    fn keeps_only_the_newest_backups() {
        let dir = TempDir::new("backups");
        ensure_dir(&dir.0).unwrap();
        // 古い退避（秒が小さい）を 6 つと、ほかの種類の退避・利用者のファイルを置く
        for secs in 1..=6 {
            fs::write(dir.0.join(format!("state.broken-{secs}.json")), "x").unwrap();
        }
        fs::write(dir.0.join("settings.broken-1.json"), "x").unwrap();
        fs::write(dir.0.join("state.broken-mine.json"), "x").unwrap();
        fs::write(dir.0.join(STATE_FILE), "{ broken").unwrap();
        let Ok(Loaded::Broken(backup)) = load_state(&dir.0) else {
            panic!("退避されるはず");
        };
        let mut names: Vec<String> = fs::read_dir(&dir.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        // 今の退避と、新しい 4 つ（3〜6）だけが残る。ほかの種類と、形の違う名前は消さない
        let backup_name = backup.file_name().unwrap().to_string_lossy().into_owned();
        let mut expected = vec![
            "settings.broken-1.json".to_string(),
            "state.broken-3.json".to_string(),
            "state.broken-4.json".to_string(),
            "state.broken-5.json".to_string(),
            "state.broken-6.json".to_string(),
            "state.broken-mine.json".to_string(),
            backup_name,
        ];
        expected.sort();
        assert_eq!(names, expected);
        // 同じ秒に 2 回退避しても、前の退避を上書きしない
        fs::write(dir.0.join(STATE_FILE), "{ broken").unwrap();
        let Ok(Loaded::Broken(second)) = load_state(&dir.0) else {
            panic!("退避されるはず");
        };
        assert_ne!(second, backup);
    }

    #[test]
    fn saves_and_loads_trusted_folders() {
        let dir = TempDir::new("trusted");
        assert_eq!(load_trusted(&dir.0), Ok(Loaded::Missing));
        let record = TrustRecord {
            folders: vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b c")],
            denied: vec![PathBuf::from("/tmp/no")],
        };
        save_trusted(&dir.0, &record).unwrap();
        assert_eq!(load_trusted(&dir.0), Ok(Loaded::Ok(record)));
        // 前の版の記録（denied がない）も読める
        fs::write(
            dir.0.join(TRUSTED_FILE),
            r#"{"version": 1, "folders": ["/tmp/a"]}"#,
        )
        .unwrap();
        assert_eq!(
            load_trusted(&dir.0),
            Ok(Loaded::Ok(TrustRecord {
                folders: vec![PathBuf::from("/tmp/a")],
                denied: Vec::new()
            }))
        );
        let mode = fs::metadata(dir.0.join(TRUSTED_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        // 文字列でない要素や、形の違うファイルは退避する（何も信頼していないものとして扱われる）
        for bad in [
            r#"{"version": 1, "folders": ["/tmp/a", 3]}"#,
            r#"{"version": 1, "folders": "/tmp/a"}"#,
            r#"{"version": 1}"#,
            r#"{"version": 1, "folders": [], "denied": [1]}"#,
        ] {
            fs::write(dir.0.join(TRUSTED_FILE), bad).unwrap();
            assert!(
                matches!(load_trusted(&dir.0), Ok(Loaded::Broken(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn keeps_valid_key_bindings_and_reports_invalid_ones() {
        let dir = TempDir::new("settings");
        write_default_settings(&dir.0).unwrap();
        let Ok(Loaded::Ok(loaded)) = load_settings(&dir.0) else {
            panic!("読めるはず");
        };
        assert_eq!(loaded.key_bindings, default_key_bindings());
        assert_eq!(loaded.appearance, Appearance::default());
        assert!(loaded.appearance_problems.is_empty());
        // key が空のものは「割り当てなし」として読む（D-159）
        fs::write(
            dir.0.join(SETTINGS_FILE),
            r#"{"version": 1, "keyBindings": [
                {"command": "toggleSidebar", "key": "e", "meta": true},
                {"command": "save"},
                {"command": "stepIn", "key": ""},
                {"command": "run", "key": 3}
            ]}"#,
        )
        .unwrap();
        let Ok(Loaded::Ok(loaded)) = load_settings(&dir.0) else {
            panic!("読めるはず");
        };
        assert_eq!(loaded.key_bindings.len(), 2);
        assert_eq!(loaded.key_bindings[0].key, "e");
        assert_eq!(
            (
                loaded.key_bindings[1].command.as_str(),
                loaded.key_bindings[1].key.as_str()
            ),
            ("stepIn", "")
        );
        assert_eq!(loaded.invalid.len(), 2);
        // keyBindings がなくても読める（見た目だけを書いた settings.json）
        fs::write(
            dir.0.join(SETTINGS_FILE),
            r#"{"version": 1, "editor": {"fontSize": 18}}"#,
        )
        .unwrap();
        let Ok(Loaded::Ok(loaded)) = load_settings(&dir.0) else {
            panic!("読めるはず");
        };
        assert!(loaded.key_bindings.is_empty());
        assert_eq!(loaded.appearance.editor_font_size, 18);
        // あるのに一覧でなければ、読めないファイル
        fs::write(
            dir.0.join(SETTINGS_FILE),
            r#"{"version": 1, "keyBindings": {}}"#,
        )
        .unwrap();
        assert!(matches!(load_settings(&dir.0), Ok(Loaded::Broken(_))));
    }

    #[test]
    fn reads_appearance_and_reports_bad_items() {
        let (appearance, problems) = parse_appearance(&serde_json::json!({
            "colors": { "indent-rainbow-1": "rgba(255, 255, 64, 0.25)", "syntax-keyword": "#ff8080" },
            "editor": { "indentRainbow": false, "errorLens": true, "breadcrumbs": false, "fontSize": 18 },
            "problem": { "fontSize": 20 }
        }));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(appearance.colors.len(), 2);
        assert_eq!(appearance.colors["syntax-keyword"], "#ff8080");
        assert!(!appearance.indent_rainbow);
        assert!(appearance.error_lens);
        assert!(!appearance.breadcrumbs);
        assert_eq!(
            (appearance.editor_font_size, appearance.problem_font_size),
            (18, 20)
        );
        // 範囲の外・数でない大きさ、真偽でない入切、名前・値の形の違う色は、既定の値にして知らせる
        let long = "a".repeat(MAX_COLOR_VALUE + 1);
        let (appearance, problems) = parse_appearance(&serde_json::json!({
            "colors": { "Bad_Name": "#fff", "fg-default": 3, "bg-base": long, "accent-focus": " ", "fg-strong": "#fff" },
            "editor": { "indentRainbow": "no", "breadcrumbs": 0, "fontSize": 40 },
            "problem": { "fontSize": "大" }
        }));
        assert_eq!(
            appearance.colors.keys().collect::<Vec<_>>(),
            vec!["fg-strong"]
        );
        assert!(appearance.indent_rainbow);
        assert!(appearance.breadcrumbs);
        assert_eq!(
            (appearance.editor_font_size, appearance.problem_font_size),
            (DEFAULT_FONT_SIZE, DEFAULT_FONT_SIZE)
        );
        assert_eq!(problems.len(), 8, "{problems:?}");
        assert!(problems.iter().any(|p| p.starts_with("editor.breadcrumbs")));
        assert!(problems.iter().any(|p| p.starts_with("editor.fontSize")));
        assert!(problems.iter().any(|p| p.starts_with("problem.fontSize")));
        assert!(
            problems
                .iter()
                .any(|p| p.starts_with("editor.indentRainbow"))
        );
        // 範囲の端は使える
        for size in [MIN_FONT_SIZE, MAX_FONT_SIZE] {
            let (appearance, problems) =
                parse_appearance(&serde_json::json!({ "editor": { "fontSize": size } }));
            assert!(problems.is_empty());
            assert_eq!(appearance.editor_font_size, size);
        }
        for size in [MIN_FONT_SIZE - 1, MAX_FONT_SIZE + 1] {
            let (_, problems) =
                parse_appearance(&serde_json::json!({ "editor": { "fontSize": size } }));
            assert_eq!(problems.len(), 1);
        }
        // 欄がオブジェクトでないとき
        let (_, problems) =
            parse_appearance(&serde_json::json!({ "colors": [], "editor": 1, "problem": "x" }));
        assert_eq!(problems.len(), 3);
    }

    #[test]
    fn rereads_settings_without_moving_them_aside() {
        let dir = TempDir::new("reread");
        assert_eq!(reread_settings(&dir.0), Ok(None));
        assert_eq!(settings_stamp(&dir.0), None);
        write_default_settings(&dir.0).unwrap();
        let first = settings_stamp(&dir.0).unwrap();
        let loaded = reread_settings(&dir.0).unwrap().unwrap();
        assert_eq!(loaded.key_bindings, default_key_bindings());
        // 書いている途中で崩れた JSON は、退避せずに Err
        fs::write(dir.0.join(SETTINGS_FILE), "{ \"version\": 1, ").unwrap();
        assert!(reread_settings(&dir.0).is_err());
        assert!(dir.0.join(SETTINGS_FILE).exists());
        let names: Vec<_> = fs::read_dir(&dir.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![SETTINGS_FILE.to_string()]);
        // 中身が変われば、大きさか更新日時が変わる
        assert_ne!(settings_stamp(&dir.0).unwrap(), first);
    }

    #[test]
    fn writes_only_the_settings_screen_items() {
        let dir = TempDir::new("write");
        ensure_dir(&dir.0).unwrap();
        fs::write(
            dir.0.join(SETTINGS_FILE),
            r##"{"version": 1, "note": "残す", "keyBindings": [], "colors": {"bg-base": "#000"},
                "editor": {"fontSize": 13, "other": 1}, "problem": 5}"##,
        )
        .unwrap();
        let mut appearance = Appearance::default();
        appearance
            .colors
            .insert("syntax-keyword".to_string(), "#ff8080".to_string());
        appearance.indent_rainbow = false;
        appearance.breadcrumbs = false;
        appearance.editor_font_size = 18;
        appearance.problem_font_size = 20;
        let bindings = vec![KeyBinding {
            command: "stepIn".to_string(),
            key: String::new(),
            meta: false,
            ctrl: false,
            alt: false,
            shift: false,
        }];
        write_settings(
            &dir.0,
            &bindings,
            &appearance,
            &SubmitSettings::default(),
            &ProblemSettings::default(),
            Interpreter::Cpython,
        )
        .unwrap();
        let value: Value =
            serde_json::from_slice(&fs::read(dir.0.join(SETTINGS_FILE)).unwrap()).unwrap();
        // 知らない項目と、editor の中のほかの項目は残る。colors は送ったものだけになる
        assert_eq!(value["note"], "残す");
        assert_eq!(value["editor"]["other"], 1);
        assert_eq!(
            value["colors"],
            serde_json::json!({"syntax-keyword": "#ff8080"})
        );
        let Ok(Loaded::Ok(loaded)) = load_settings(&dir.0) else {
            panic!("読めるはず");
        };
        assert_eq!(loaded.key_bindings, bindings);
        assert_eq!(loaded.appearance, appearance);
        assert!(loaded.appearance_problems.is_empty());
        let mode = fs::metadata(dir.0.join(SETTINGS_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        // 範囲の外の大きさは書かない
        let mut bad = appearance.clone();
        bad.editor_font_size = 40;
        assert!(
            write_settings(
                &dir.0,
                &bindings,
                &bad,
                &SubmitSettings::default(),
                &ProblemSettings::default(),
                Interpreter::Cpython
            )
            .is_err()
        );
        assert_eq!(load_settings(&dir.0), Ok(Loaded::Ok(loaded)));
        // 読めないファイルには書かない（直してもらう）。ファイルはそのまま
        fs::write(dir.0.join(SETTINGS_FILE), "{ broken").unwrap();
        assert!(
            write_settings(
                &dir.0,
                &bindings,
                &appearance,
                &SubmitSettings::default(),
                &ProblemSettings::default(),
                Interpreter::Cpython
            )
            .is_err()
        );
        assert_eq!(fs::read(dir.0.join(SETTINGS_FILE)).unwrap(), b"{ broken");
        // なければ作る
        fs::remove_file(dir.0.join(SETTINGS_FILE)).unwrap();
        write_settings(
            &dir.0,
            &bindings,
            &appearance,
            &SubmitSettings::default(),
            &ProblemSettings::default(),
            Interpreter::Cpython,
        )
        .unwrap();
        assert!(matches!(load_settings(&dir.0), Ok(Loaded::Ok(_))));
    }

    #[test]
    fn reads_and_writes_submit_settings() {
        let dir = TempDir::new("submit");
        // なければ・読めなければ、確かめる側（両方 true）
        assert_eq!(submit_settings(&dir.0), SubmitSettings::default());
        assert!(
            SubmitSettings::default().test_before_submit
                && SubmitSettings::default().confirm_dialog
        );
        ensure_dir(&dir.0).unwrap();
        fs::write(dir.0.join(SETTINGS_FILE), "{ broken").unwrap();
        assert_eq!(submit_settings(&dir.0), SubmitSettings::default());
        fs::write(
            dir.0.join(SETTINGS_FILE),
            r#"{"version": 1, "submit": {"testBeforeSubmit": false, "confirmDialog": "no"}}"#,
        )
        .unwrap();
        let Ok(Loaded::Ok(loaded)) = load_settings(&dir.0) else {
            panic!("読めるはず");
        };
        assert_eq!(
            loaded.submit,
            SubmitSettings {
                test_before_submit: false,
                confirm_dialog: true
            }
        );
        assert_eq!(loaded.appearance_problems.len(), 1);
        // 書いて読み直す（ほかの項目は残る）
        let submit = SubmitSettings {
            test_before_submit: true,
            confirm_dialog: false,
        };
        write_settings(
            &dir.0,
            &[],
            &Appearance::default(),
            &submit,
            &ProblemSettings::default(),
            Interpreter::Cpython,
        )
        .unwrap();
        assert_eq!(submit_settings(&dir.0), submit);
        // 初めての起動で作る settings.json にも書く
        write_default_settings(&dir.0).unwrap();
        assert_eq!(submit_settings(&dir.0), SubmitSettings::default());
    }

    #[test]
    fn reads_python_settings() {
        let read = |text: &str| parse_settings(serde_json::from_str(text).unwrap()).unwrap();
        let loaded = read(
            r#"{"version": 1, "python": {"interpreter": "pypy", "pypy": " /opt/x/pypy3 ", "cpython": ""}}"#,
        );
        assert_eq!(loaded.python.interpreter, Interpreter::Pypy);
        assert_eq!(loaded.python.pypy, "/opt/x/pypy3");
        assert!(loaded.appearance_problems.is_empty());
        // ない・読めないときは CPython を探す。読めない項目は知らせる
        assert_eq!(read(r#"{"version": 1}"#).python, PythonSettings::default());
        let wrong = read(r#"{"version": 1, "python": {"interpreter": "jython", "cpython": 3}}"#);
        assert_eq!(wrong.python, PythonSettings::default());
        assert_eq!(
            wrong.appearance_problems,
            [
                "python.interpreter（\"cpython\" か \"pypy\" にしてください）",
                "python.cpython（処理系のパスの文字列にしてください）"
            ]
        );
    }

    // 設定の画面は処理系だけを書く。処理系のパスなど、ほかの項目は残す（Phase 19 作業 3。D-221）
    #[test]
    fn writes_the_python_interpreter() {
        let dir = TempDir::new("python-write");
        fs::create_dir_all(&dir.0).unwrap();
        fs::write(
            dir.0.join(SETTINGS_FILE),
            r#"{"version": 1, "python": {"interpreter": "cpython", "pypy": "/opt/x/pypy3", "other": 1}}"#,
        )
        .unwrap();
        let write = |interpreter| {
            write_settings(
                &dir.0,
                &[],
                &Appearance::default(),
                &SubmitSettings::default(),
                &ProblemSettings::default(),
                interpreter,
            )
            .unwrap()
        };
        write(Interpreter::Pypy);
        let settings = python_settings(&dir.0);
        assert_eq!(settings.interpreter, Interpreter::Pypy);
        assert_eq!(settings.pypy, "/opt/x/pypy3");
        let text = fs::read_to_string(dir.0.join(SETTINGS_FILE)).unwrap();
        assert!(text.contains(r#""other": 1"#));
        write(Interpreter::Cpython);
        assert_eq!(python_settings(&dir.0).interpreter, Interpreter::Cpython);
        // 初めての起動で作る settings.json にも書く
        write_default_settings(&dir.0).unwrap();
        let text = fs::read_to_string(dir.0.join(SETTINGS_FILE)).unwrap();
        assert!(text.contains(r#""interpreter": "cpython""#));
        assert_eq!(python_settings(&dir.0), PythonSettings::default());
    }

    #[test]
    fn reads_cpp_settings() {
        let read = |text: &str| parse_settings(serde_json::from_str(text).unwrap()).unwrap();
        let loaded = read(r#"{"version": 1, "cpp": {"compiler": " /opt/x/g++-15 "}}"#);
        assert_eq!(loaded.cpp.compiler, "/opt/x/g++-15");
        assert!(loaded.appearance_problems.is_empty());
        // ない・読めないときは探す（空）。読めない項目は知らせる
        assert_eq!(read(r#"{"version": 1}"#).cpp, CppSettings::default());
        let wrong = read(r#"{"version": 1, "cpp": {"compiler": 15}}"#);
        assert_eq!(wrong.cpp, CppSettings::default());
        assert_eq!(
            wrong.appearance_problems,
            ["cpp.compiler（g++ のパスの文字列にしてください）"]
        );
        assert_eq!(
            read(r#"{"version": 1, "cpp": "x"}"#).appearance_problems,
            ["cpp"]
        );
    }

    #[test]
    fn reads_and_writes_problem_settings() {
        let dir = TempDir::new("problem-settings");
        ensure_dir(&dir.0).unwrap();
        let load = || {
            let Ok(Loaded::Ok(loaded)) = load_settings(&dir.0) else {
                panic!("読めるはず");
            };
            loaded
        };
        // 書いていなければ既定（input! を作る・サンプルを保存、for は書かない）
        fs::write(dir.0.join(SETTINGS_FILE), r#"{"version": 1}"#).unwrap();
        assert_eq!(load().problem, ProblemSettings::default());
        // 知らない名前・重なりは使わず、知らない名前は知らせる。並びは書いた順のまま読む
        fs::write(
            dir.0.join(SETTINGS_FILE),
            r#"{"version": 1, "problem": {"fontSize": 15, "buttons": ["refresh", "nope", "refresh", "openPage"], "mainLoop": true, "fillInput": true, "language": "cpp"}}"#,
        )
        .unwrap();
        let loaded = load();
        assert_eq!(
            loaded.problem,
            ProblemSettings {
                buttons: vec!["refresh".to_string(), "openPage".to_string()],
                main_loop: true,
                fill_input: true,
                language: Language::Cpp,
            }
        );
        assert_eq!(loaded.appearance.problem_font_size, 15);
        assert_eq!(
            loaded.appearance_problems,
            ["problem.buttons の「\"nope\"」"]
        );
        // 一覧でない・真偽でない
        fs::write(
            dir.0.join(SETTINGS_FILE),
            r#"{"version": 1, "problem": {"buttons": "refresh", "mainLoop": 1, "fillInput": "yes", "language": "java"}}"#,
        )
        .unwrap();
        let loaded = load();
        assert_eq!(loaded.problem, ProblemSettings::default());
        assert_eq!(loaded.appearance_problems.len(), 4);
        assert!(
            loaded.appearance_problems.contains(
                &"problem.language（rust・cpp・python のどれかにしてください）".to_string()
            )
        );
        // 書いて読み直す（問題文の文字の大きさなど、ほかの項目は残る）
        fs::write(
            dir.0.join(SETTINGS_FILE),
            r#"{"version": 1, "problem": {"fontSize": 15, "other": 1}}"#,
        )
        .unwrap();
        let appearance = Appearance {
            problem_font_size: 15,
            ..Appearance::default()
        };
        let problem = ProblemSettings {
            buttons: PROBLEM_BUTTONS.iter().map(|b| b.to_string()).collect(),
            main_loop: true,
            fill_input: true,
            language: Language::Cpp,
        };
        write_settings(
            &dir.0,
            &[],
            &appearance,
            &SubmitSettings::default(),
            &problem,
            Interpreter::Cpython,
        )
        .unwrap();
        let loaded = load();
        assert_eq!(loaded.problem, problem);
        assert_eq!(problem_settings(&dir.0), problem);
        assert_eq!(loaded.appearance.problem_font_size, 15);
        let text = fs::read_to_string(dir.0.join(SETTINGS_FILE)).unwrap();
        assert!(text.contains(r#""other": 1"#));
        // 知らない名前は書かない
        let bad = ProblemSettings {
            buttons: vec!["nope".to_string()],
            ..ProblemSettings::default()
        };
        assert!(
            write_settings(
                &dir.0,
                &[],
                &appearance,
                &SubmitSettings::default(),
                &bad,
                Interpreter::Cpython
            )
            .is_err()
        );
        // 初めての起動で作る settings.json にも書く
        write_default_settings(&dir.0).unwrap();
        assert_eq!(load().problem, ProblemSettings::default());
    }

    #[test]
    fn default_key_bindings_do_not_collide() {
        let bindings = default_key_bindings();
        for (i, a) in bindings.iter().enumerate() {
            for b in &bindings[i + 1..] {
                assert!(
                    (&a.key, a.meta, a.ctrl, a.alt, a.shift)
                        != (&b.key, b.meta, b.ctrl, b.alt, b.shift),
                    "{} と {} が同じキー",
                    a.command,
                    b.command
                );
            }
        }
        // ⇧⌥F：整形（D-133）
        let format = bindings
            .iter()
            .find(|b| b.command == "formatDocument")
            .unwrap();
        assert_eq!(
            (format.key.as_str(), format.meta, format.alt, format.shift),
            ("f", false, true, true)
        );
    }

    #[test]
    fn allows_only_one_instance_lock() {
        let dir = TempDir::new("lock");
        let first = acquire_instance_lock(&dir.0).unwrap();
        assert!(first.is_some());
        assert!(acquire_instance_lock(&dir.0).unwrap().is_none());
        drop(first);
        // ほかのテストが並行してプロセスを起動すると、fork から exec までの間だけ、子がロックの
        // ファイルを持つことがある（推測）。その間は取れないので、少しだけ取り直す
        let again = (0..50).find_map(|_| {
            let lock = acquire_instance_lock(&dir.0).unwrap();
            if lock.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            lock
        });
        assert!(again.is_some());
    }
}
