// WebView から呼ぶアプリ独自のコマンド（D-055：build.rs で列挙し、capability で許可する）
//
// パスは、作業領域のルートからの相対パスだけを受け取る（05 T-08）。
// 作業領域のルートは Rust 側だけが持ち、WebView からは変えられない。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::cpp::{self, CppSettings};
use crate::debug::{
    self, BreakpointView, DebugControl, EvaluateContext, EvaluateView, FrameView, ScopeView,
    SourceBreakpoints, StepAction, VariableView,
};
use crate::history;
use crate::input_cpp;
use crate::input_gen;
use crate::input_py;
use crate::language::{LangSettings, Language};
use crate::lsp::{self, LspControl, LspEvent, Started};
use crate::problem::{self, ProblemId};
use crate::python::{Interpreter, PythonSettings};
use crate::run::{self, LogKind, RunControl, RunEvent};
use crate::sanitize::sanitize;
use crate::scaffold::{self, ContestCreated, ProblemCreated, ScaffoldControl, ScaffoldEvent};
use crate::search;
use crate::session::{self, Keychain, SessionState, SessionView};
use crate::snippets;
use crate::sources::{self, ReadOnlySource};
use crate::storage::{
    self, Appearance, KeyBinding, Layout, Loaded, LoadedSettings, ProblemSettings, SavedState,
    SubmitSettings,
};
use crate::submit::{self, SubmissionView, SubmitOutcome};
use crate::toolchain::{self, Target};
use crate::trust;
use crate::workspace::{
    DirEntry, ExpectedSaved, ReadResult, Stamp, WriteResult, count_files, create_file,
    create_folder, current_stamp, entry_path, is_folder, is_link, list_dir, locate_in_package,
    read_text, rename_file, resolve, trash_file, write_existing, write_expected,
};

// 作業領域のルート（canonicalize 済み）と、信頼しているか（D-128）。未選択なら None
#[derive(Default)]
pub struct Workspace(Mutex<Option<(PathBuf, bool)>>);

impl Workspace {
    fn current(&self) -> Result<Option<(PathBuf, bool)>, String> {
        Ok(self
            .0
            .lock()
            .map_err(|_| "作業領域の状態を読めません".to_string())?
            .clone())
    }

    fn set(&self, root: PathBuf, trusted: bool) -> Result<(), String> {
        *self
            .0
            .lock()
            .map_err(|_| "作業領域の状態を書けません".to_string())? = Some((root, trusted));
        Ok(())
    }

    // ファイルの読み書きなど、コードを実行しない操作に使う（信頼していなくてもよい）
    fn root(&self) -> Result<PathBuf, String> {
        self.current()?
            .map(|(root, _)| root)
            .ok_or_else(|| "フォルダが開かれていません".to_string())
    }

    // cargo・rust-analyzer・利用者のプログラムを動かす操作に使う。信頼していなければ拒否する（D-128）
    fn trusted_root(&self) -> Result<PathBuf, String> {
        match self.current()? {
            Some((root, true)) => Ok(root),
            Some((_, false)) => Err(UNTRUSTED.to_string()),
            None => Err("フォルダが開かれていません".to_string()),
        }
    }
}

const UNTRUSTED: &str = "このフォルダは信頼されていないので、コードを実行する操作はできません（ファイル一覧の上の「信頼する…」から信頼できます）";

// 作業領域を信頼しているか。信頼の記録（trusted.json）の信頼したフォルダにあれば信頼し、「信頼しない」を選んだ
// フォルダにあれば聞かずに信頼しない（Phase 10 作業 12。D-172）。どちらにもなければ、確認の画面を出して聞き、結果を記録する。
// 確認と記録は Rust 側だけで行う（WebView には、確かめずに信頼させるコマンドを渡さない。信頼し直すときも確認の画面を出す）
fn decide_trust(app: &AppHandle, root: &Path) -> Result<bool, String> {
    let (dir, record) = trust_record(app)?;
    if record.folders.iter().any(|folder| folder == root) {
        return Ok(true);
    }
    if record.denied.iter().any(|folder| folder == root) {
        return Ok(false);
    }
    ask_trust(app, &dir, record, root)
}

// 信頼の記録を読む。読めなければ退避して知らせ、何も記録していないものとして扱う
fn trust_record(app: &AppHandle) -> Result<(PathBuf, storage::TrustRecord), String> {
    ensure_primary(app)?;
    let dir = app_dir(app)?;
    let record = match storage::load_trusted(&dir)? {
        Loaded::Ok(record) => record,
        Loaded::Missing => storage::TrustRecord::default(),
        Loaded::Broken(backup) => {
            app.dialog()
                .message(format!(
                    "信頼したフォルダの記録を読めなかったので、別の名前で退避しました。どのフォルダも信頼していないものとして扱います。\n退避先：\n{}",
                    backup.to_string_lossy()
                ))
                .title("信頼の記録")
                .kind(MessageDialogKind::Warning)
                .blocking_show();
            storage::TrustRecord::default()
        }
    };
    Ok((dir, record))
}

// 確認の画面を出して聞き、結果（信頼する・しない）を記録する
fn ask_trust(
    app: &AppHandle,
    dir: &Path,
    mut record: storage::TrustRecord,
    root: &Path,
) -> Result<bool, String> {
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    let home = toolchain::home_dir();
    let message = trust::confirmation_message(&name, &trust::inspect(root, home.as_deref()));
    let trusted = app
        .dialog()
        .message(message)
        .title("このフォルダを信頼しますか")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "信頼する".to_string(),
            "信頼しない".to_string(),
        ))
        .blocking_show();
    record.folders.retain(|folder| folder != root);
    record.denied.retain(|folder| folder != root);
    if trusted {
        record.folders.push(root.to_path_buf());
    } else {
        record.denied.push(root.to_path_buf());
    }
    if storage::save_trusted(dir, &record).is_err() {
        app.dialog()
            .message("信頼の記録を保存できませんでした。次に開くときも確認します。")
            .title("信頼の記録")
            .kind(MessageDialogKind::Warning)
            .blocking_show();
    }
    Ok(trusted)
}

// 信頼していない作業領域を、信頼し直す（Phase 10 作業 12。D-172）。確認の画面を出して聞き、記録する。
// 信頼したら true（rust-analyzer・実行などが使えるようになる）
#[tauri::command]
pub async fn trust_workspace(
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<bool, String> {
    let Some((root, trusted)) = workspace.current()? else {
        return Err("フォルダが開かれていません".to_string());
    };
    if trusted {
        return Ok(true);
    }
    let (dir, record) = trust_record(&app)?;
    let trusted = ask_trust(&app, &dir, record, &root)?;
    workspace.set(root, trusted)?;
    Ok(trusted)
}

// フォルダを開いた結果。名前だけを渡す（D-065）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedWorkspace {
    name: String,
    // 信頼している（コードを実行する操作ができる）か
    trusted: bool,
}

// フォルダ選択の画面を開き、選ばれたフォルダを作業領域にする。
// 引数はない（WebView から作業領域のパスを渡すことはできない）。
// 戻り値は作業領域の名前と、信頼しているか（記録がなければ確認の画面で聞く。D-128）。キャンセルなら None
#[tauri::command]
pub async fn open_workspace(
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<Option<OpenedWorkspace>, String> {
    // 実行・テスト・デバッグ・問題を作っている間は、別のフォルダを開かない（Phase 10 作業 3。D-166）
    if app.state::<RunControl>().running() || app.state::<ScaffoldControl>().is_running() {
        return Err(
            "実行・テスト・デバッグ・問題を作っている間は、別のフォルダを開けません".to_string(),
        );
    }
    // async のコマンドはメインスレッドの外で動くので、選択の画面を待ってもメインスレッドは止まらない
    let Some(picked) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let root = picked
        .into_path()
        .map_err(|_| "選んだフォルダを使えません".to_string())?
        .canonicalize()
        .map_err(|_| "選んだフォルダが見つかりません".to_string())?;
    // 別のフォルダを開いたら、前の作業領域の rust-analyzer を止める（D-124）
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || handle.state::<LspControl>().stop())
        .await
        .map_err(|_| "言語サーバーを止められません".to_string())?;
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    // 確認の画面は、前の作業領域の rust-analyzer を止めたあとに出す（画面を待つ間に前のものが動き続けないように）
    let trusted = decide_trust(&app, &root)?;
    workspace.set(root, trusted)?;
    // 前の作業領域の置換は、もう元に戻さない（D-184）
    app.state::<search::SearchControl>().clear_undo();
    Ok(Some(OpenedWorkspace { name, trusted }))
}

// 作業領域の中のフォルダの中身を返す（workspace::list_dir）。見つからないフォルダは null（D-156）
#[tauri::command]
pub fn list_directory(
    path: String,
    workspace: State<'_, Workspace>,
) -> Result<Option<Vec<DirEntry>>, String> {
    list_dir(&workspace.root()?, &path)
}

// フォルダ全体の検索（Phase 12 作業 2。D-183）。読むだけなので、信頼していない作業領域でも使える。
// 新しい検索を始めたら、前の検索は途中でやめる（cancelled の結果を返す）
#[tauri::command]
pub async fn search_workspace(
    query: search::SearchQuery,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<search::SearchResult, String> {
    let root = workspace.root()?;
    let regex = search::build_regex(&query)?;
    let generation = app.state::<search::SearchControl>().begin();
    tauri::async_runtime::spawn_blocking(move || {
        search::search(
            &root,
            &regex,
            &app.state::<search::SearchControl>(),
            generation,
        )
    })
    .await
    .map_err(|_| "検索できません".to_string())
}

// 置換（Phase 12 作業 3。D-184）。検索と同じ条件で探し直し、検索したときから変わっていないファイルだけを置き換える。
// 置き換える前に OS の確認のダイアログを出す。ファイルの編集と同じく、信頼していない作業領域でも使える
#[tauri::command]
pub async fn replace_in_workspace(
    query: search::SearchQuery,
    replacement: String,
    targets: Vec<search::ReplaceTarget>,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<search::ReplaceResult, String> {
    let root = workspace.root()?;
    let regex = search::build_regex(&query)?;
    tauri::async_runtime::spawn_blocking(move || {
        let short = |text: &str| {
            let mut chars = text.chars();
            let head: String = chars.by_ref().take(60).collect();
            if chars.next().is_some() {
                format!("{head}…")
            } else {
                head
            }
        };
        let dialog_app = app.clone();
        search::replace_files(
            &root,
            &regex,
            &replacement,
            query.regex,
            &targets,
            &app.state::<search::SearchControl>(),
            |plan| {
                let mut message = format!(
                    "{} 個のファイルの {} 件を置き換えます。\n\n探す：{}\n置き換え：{}",
                    plan.files,
                    plan.matches,
                    short(&query.pattern),
                    short(&replacement)
                );
                if plan.changed > 0 {
                    message.push_str(&format!(
                        "\n\n検索のあとで変わった {} 個のファイルは置き換えません。",
                        plan.changed
                    ));
                }
                message.push_str("\n\n置き換えたあと、次の置換かフォルダを開き直すまでは「置換を元に戻す」で戻せます。");
                dialog_app
                    .dialog()
                    .message(message)
                    .title("置換")
                    .kind(MessageDialogKind::Warning)
                    .buttons(MessageDialogButtons::OkCancelCustom(
                        "置き換える".to_string(),
                        "キャンセル".to_string(),
                    ))
                    .blocking_show()
            },
        )
    })
    .await
    .map_err(|_| "置換できません".to_string())?
}

// 直前の置換を元に戻す（D-184）
#[tauri::command]
pub async fn undo_replace(
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<search::UndoResult, String> {
    let root = workspace.root()?;
    tauri::async_runtime::spawn_blocking(move || {
        search::undo_replace(&root, &app.state::<search::SearchControl>())
    })
    .await
    .map_err(|_| "元に戻せません".to_string())?
}

// ファイルを UTF-8 の文字列として返す（workspace::read_text）
#[tauri::command]
pub fn read_file(path: String, workspace: State<'_, Workspace>) -> Result<ReadResult, String> {
    read_text(&workspace.root()?, &path)
}

// 既存のファイルを上書きする（workspace::write_existing）
#[tauri::command]
// expected が null なら、外部での変更を確かめずに書く（利用者が「上書き」を選んだとき）
pub fn write_file(
    path: String,
    content: String,
    expected: Option<Stamp>,
    workspace: State<'_, Workspace>,
) -> Result<WriteResult, String> {
    write_existing(&workspace.root()?, &path, &content, expected.as_ref())
}

// 開いているファイルの今の更新日時と大きさ（ウィンドウにフォーカスが戻ったときに確かめる。D-038）。
// ファイルがなければ null
#[tauri::command]
pub fn file_stamps(
    paths: Vec<String>,
    workspace: State<'_, Workspace>,
) -> Result<Vec<Option<Stamp>>, String> {
    let root = workspace.root()?;
    Ok(paths
        .iter()
        .map(|path| current_stamp(&root, path))
        .collect())
}

// 外部で変更されたファイルに未保存の編集があるとき、「再読み込み／上書き」を選んでもらう（D-038）。
// 戻り値は、再読み込みを選んだら true
#[tauri::command]
pub async fn ask_external_change(name: String, app: AppHandle) -> Result<bool, String> {
    // async のコマンドはメインスレッドの外で動くので、画面を待ってもメインスレッドは止まらない
    Ok(app
        .dialog()
        .message(format!(
            "「{name}」がほかのプログラムで変更されました。\n再読み込みすると、IDE で編集した内容は失われます。上書きすると、ほかのプログラムでの変更が失われます。"
        ))
        .title("外部での変更")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "再読み込み".to_string(),
            "上書き".to_string(),
        ))
        .blocking_show())
}

// 保存できなかったファイルの変更を捨てるかを、macOS 標準の画面で聞く（Phase 10 作業 1。D-164）。
// purpose は "close"（タブを閉じる）・"quit"（終了する）・"folder"（別のフォルダを開く）。捨てるなら true
#[tauri::command]
pub async fn ask_discard_changes(
    paths: Vec<String>,
    purpose: String,
    app: AppHandle,
) -> Result<bool, String> {
    let action = match purpose.as_str() {
        "close" => "タブを閉じ",
        "quit" => "終了し",
        "folder" => "別のフォルダを開き",
        _ => return Err("不正な操作です".to_string()),
    };
    // 多すぎるときは先の 20 個だけを出す
    let mut list: Vec<String> = paths
        .iter()
        .take(20)
        .map(|path| format!("・{path}"))
        .collect();
    if paths.len() > 20 {
        list.push(format!("ほか {} 個", paths.len() - 20));
    }
    // async のコマンドはメインスレッドの外で動くので、画面を待ってもメインスレッドは止まらない
    Ok(app
        .dialog()
        .message(format!(
            "次のファイルを保存できませんでした（ほかのプログラムで消された・書き込めないなど）。\n{}\n\n保存していない変更を捨てて{action}ますか？",
            list.join("\n")
        ))
        .title("保存できないファイル")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "変更を捨てる".to_string(),
            "キャンセル".to_string(),
        ))
        .blocking_show())
}

// 作業領域の中のフォルダに、空のファイルを作る（workspace::create_file）。戻り値は作ったファイルの相対パス
#[tauri::command]
pub fn create_new_file(
    dir: String,
    name: String,
    workspace: State<'_, Workspace>,
) -> Result<String, String> {
    create_file(&workspace.root()?, &dir, &name)
}

// 作業領域の中のフォルダに、新しいフォルダを作る（workspace::create_folder。D-157）。戻り値は作ったフォルダの相対パス
#[tauri::command]
pub fn create_new_folder(
    dir: String,
    name: String,
    workspace: State<'_, Workspace>,
) -> Result<String, String> {
    create_folder(&workspace.root()?, &dir, &name)
}

// 実行・テスト・デバッグ・問題を作っている間は、フォルダの名前の変更・削除を断る（D-157）
fn refuse_while_busy(app: &AppHandle) -> Result<(), String> {
    if app.state::<RunControl>().running() || app.state::<ScaffoldControl>().is_running() {
        return Err(
            "実行・テスト・デバッグ・問題を作っている間は、フォルダの名前の変更・削除はできません"
                .to_string(),
        );
    }
    Ok(())
}

// ファイル・フォルダの名前を変える（workspace::rename_file。フォルダは D-157）。戻り値は変えた後の相対パス
#[tauri::command]
pub fn rename_existing_file(
    path: String,
    new_name: String,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<String, String> {
    let root = workspace.root()?;
    if is_folder(&root, &path)? {
        refuse_while_busy(&app)?;
    }
    rename_file(&root, &path, &new_name)
}

// サンプルテストの実際の出力を、期待出力（`sample-N.out`・`random-N.out`）として保存する（Phase 4a 作業 3。D-137。`random-N` は D-143）。
// path は作業領域のルートからの相対パス。すでに内容の違う `.out` があるときは、上書きの確認の画面を Rust 側で開く
// （WebView から確認なしに上書きはできない。D-072 と同じ考え方）。cargo もプログラムも動かさず、
// 既存の write_file と同じ範囲のファイルしか書かない（`sample-N.in`・`random-N.in` と並ぶ同じ名前の `.out` だけ）ので、信頼は求めない
#[tauri::command]
pub async fn save_expected(
    path: String,
    content: String,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<ExpectedSaved, String> {
    let root = workspace.root()?;
    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
    // async のコマンドはメインスレッドの外で動くので、確認の画面を待ってもメインスレッドは止まらない
    write_expected(&root, &path, &content, || {
        app.dialog()
            .message(format!(
                "「{name}」を上書きしますか？\n今の内容は失われます（元に戻せません）。"
            ))
            .title("期待出力の上書き")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "上書きする".to_string(),
                "キャンセル".to_string(),
            ))
            .blocking_show()
    })
}

// 削除の確認に出す、フォルダの中のファイルを数える上限（D-157）
const FOLDER_COUNT_LIMIT: usize = 10_000;

// ファイル・フォルダをゴミ箱に移す（リンクはリンク自体を消す。D-102。フォルダは D-157）。確認の画面は Rust 側で開き、WebView から確認なしに削除はできない（D-072）。
// unsaved が true なら、確認の画面に未保存の変更があることを添える。戻り値は、ゴミ箱に移したか
#[tauri::command]
pub async fn delete_file(
    path: String,
    unsaved: bool,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<bool, String> {
    let root = workspace.root()?;
    // 確認の前に、作業領域の中のファイル・フォルダであることを確かめる
    let entry = entry_path(&root, &path)?;
    let name = path.rsplit('/').next().unwrap_or(&path);
    // リンクはゴミ箱に移さず、リンク自体を消す（D-102）
    let link = is_link(&root, &path)?;
    let folder = !link && is_folder(&root, &path)?;
    if folder {
        refuse_while_busy(&app)?;
    }
    let (mut message, ok_label) = if link {
        (
            format!(
                "リンク「{name}」を、ゴミ箱に移さずに消しますか？\n元に戻せません。リンクの先は残ります。"
            ),
            "削除する",
        )
    } else if folder {
        // 中のファイルの数（D-157）。数えるのは 10,000 個まで
        let count = match count_files(&entry, FOLDER_COUNT_LIMIT) {
            FOLDER_COUNT_LIMIT => "10,000 個以上".to_string(),
            count => format!("{count} 個"),
        };
        (
            format!("「{name}」フォルダをゴミ箱に移しますか？\n中のファイル {count} も移ります。"),
            "ゴミ箱に移す",
        )
    } else {
        (format!("「{name}」をゴミ箱に移しますか？"), "ゴミ箱に移す")
    };
    if unsaved {
        message.push_str("\n保存されていない変更があります。削除すると、その変更は失われます。");
    }
    // async のコマンドはメインスレッドの外で動くので、確認の画面を待ってもメインスレッドは止まらない
    let confirmed = app
        .dialog()
        .message(message)
        .title("ファイルの削除")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            ok_label.to_string(),
            "キャンセル".to_string(),
        ))
        .blocking_show();
    if !confirmed {
        return Ok(false);
    }
    trash_file(&root, &path)?;
    Ok(true)
}

// 開いているファイルから、実行の対象のバイナリを決める（D-088 の 1・2）。決まらなければ null。
// cargo を起動するので、メインスレッドの外で動かす
#[tauri::command]
pub async fn resolve_target(
    path: String,
    workspace: State<'_, Workspace>,
) -> Result<Option<Target>, String> {
    // cargo を動かすので、信頼していない作業領域では拒否する（D-128）
    let root = workspace.trusted_root()?;
    let file = resolve(&root, &path).map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || toolchain::resolve_target(&root, &file))
        .await
        .map_err(|_| "実行の対象を決められません".to_string())?
}

// 対象のパッケージのディレクトリ。作業領域の中にあり、言語のしるしのファイル（Rust は Cargo.toml、C++ は main.cpp）があるもの（D-204）
fn package_dir(root: &Path, target: &Target) -> Result<PathBuf, String> {
    let dir = resolve(root, &target.package).map_err(|error| error.to_string())?;
    if !target.language.is_package(&dir) {
        return Err(match target.language {
            Language::Rust => "Cargo パッケージが見つかりません".to_string(),
            Language::Cpp => "C++ のパッケージ（main.cpp）が見つかりません".to_string(),
            Language::Python => "Python のパッケージ（main.py）が見つかりません".to_string(),
        });
    }
    Ok(dir)
}

// 通常実行（Phase 2 作業 4）とサンプルテスト（作業 5。test が true）。ビルドと実行が終わるまで返らない。経過は on_event で送る（09 K-07）。
// パッケージのディレクトリは、作業領域の中にあり Cargo.toml があることを確かめてから使う。
// cases は、サンプルテストで実行するケースの名前（`sample-N`・`random-N`。1 ケースだけ・失敗したケースだけの再実行。Phase 4a 作業 2）。
// 終わったら、結果を実行履歴に残す（Phase 4a 作業 4。D-137）。残せなくても実行は失敗にせず、実行ログに知らせる
#[tauri::command]
pub async fn start_run(
    target: Target,
    test: bool,
    cases: Option<Vec<String>>,
    on_event: tauri::ipc::Channel<RunEvent>,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<(), String> {
    // ビルドと実行は利用者のコードを動かすので、信頼していない作業領域では拒否する（D-128）
    let root = workspace.trusted_root()?;
    let dir = package_dir(&root, &target)?;
    // C++ のコンパイラ・AC Library（D-205・D-206）と Python の処理系（D-221）の設定
    let langs = lang_settings_of(&app);
    let kind = match (test, cases.is_some()) {
        (false, _) => history::Kind::Run,
        (true, false) => history::Kind::Test,
        (true, true) => history::Kind::PartialTest,
    };
    let time_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    let (package, bin) = (target.package.clone(), target.bin.clone());
    let channel = on_event.clone();
    let runner = app.clone();
    let finish = tauri::async_runtime::spawn_blocking(move || {
        run::start(
            dir,
            target,
            langs,
            test,
            cases,
            &runner.state::<RunControl>(),
            on_event,
        )
    })
    .await
    .map_err(|_| "実行できません".to_string())??;
    record_history(&app, &channel, &root, time_ms, package, bin, kind, finish);
    Ok(())
}

// ランダムテスト（Phase 4b 作業 2。D-143）。ビルドして、生成器・愚直解・解答を繰り返し動かす。終わるまで返らない。
// 経過は on_event で送る。終わったら、結果を実行履歴に残す。
// 生成器・愚直解・解答は利用者のコードなので、信頼していない作業領域では拒否する（D-128）。
// 失敗したケースは、テストのフォルダ（作業領域の中）に `random-N.in`・`.out` として保存する
#[tauri::command]
pub async fn start_random(
    target: Target,
    on_event: tauri::ipc::Channel<RunEvent>,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<(), String> {
    let root = workspace.trusted_root()?;
    let dir = package_dir(&root, &target)?;
    let langs = lang_settings_of(&app);
    let time_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    // 1 回目の seed。毎回変わるように時刻から決める。生成器が 32 ビットの整数で読んでも収まるように、
    // 回数の上限を足しても 2^31 を超えない範囲にする
    let start_seed = time_ms % run::SEED_RANGE;
    let (package, bin) = (target.package.clone(), target.bin.clone());
    let channel = on_event.clone();
    let runner = app.clone();
    let owned_root = root.clone();
    let finish = tauri::async_runtime::spawn_blocking(move || {
        run::start_random(
            dir,
            owned_root,
            target,
            langs,
            start_seed,
            &runner.state::<RunControl>(),
            on_event,
        )
    })
    .await
    .map_err(|_| "実行できません".to_string())??;
    record_history(
        &app,
        &channel,
        &root,
        time_ms,
        package,
        bin,
        history::Kind::RandomTest,
        finish,
    );
    Ok(())
}

// 対話テスト（Phase 4c 作業 1。D-144）。ビルドして、解答とジャッジをつないでケースごとに動かす。終わるまで返らない。
// cases は、実行するケースの名前（`interactive-N`。1 ケースだけ・失敗したケースだけの再実行）。経過は on_event で送る。
// 終わったら、結果を実行履歴に残す。解答・ジャッジは利用者のコードなので、信頼していない作業領域では拒否する（D-128）
#[tauri::command]
pub async fn start_interactive(
    target: Target,
    cases: Option<Vec<String>>,
    on_event: tauri::ipc::Channel<RunEvent>,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<(), String> {
    let root = workspace.trusted_root()?;
    let dir = package_dir(&root, &target)?;
    let langs = lang_settings_of(&app);
    let time_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    let (package, bin) = (target.package.clone(), target.bin.clone());
    let channel = on_event.clone();
    let runner = app.clone();
    let finish = tauri::async_runtime::spawn_blocking(move || {
        run::start_interactive(
            dir,
            target,
            langs,
            cases,
            &runner.state::<RunControl>(),
            on_event,
        )
    })
    .await
    .map_err(|_| "実行できません".to_string())??;
    record_history(
        &app,
        &channel,
        &root,
        time_ms,
        package,
        bin,
        history::Kind::InteractiveTest,
        finish,
    );
    Ok(())
}

// 終わり方を実行履歴に残す（Phase 4a 作業 4。D-137）。残せなくても実行は失敗にせず、実行ログに知らせる
#[allow(clippy::too_many_arguments)]
fn record_history(
    app: &AppHandle,
    channel: &tauri::ipc::Channel<RunEvent>,
    root: &Path,
    time_ms: u64,
    package: String,
    bin: String,
    kind: history::Kind,
    finish: run::Finish,
) {
    // UTF-8 で表せないパスの作業領域は、記録しない（trusted.json と同じ）
    let Some(workspace) = root.to_str().map(str::to_string) else {
        return;
    };
    let entry = history::Entry {
        time_ms,
        workspace,
        package,
        bin,
        kind,
        finish,
    };
    let recorded = ensure_primary(app)
        .and_then(|_| app_dir(app))
        .and_then(|dir| history::record(&dir, entry));
    if let Err(error) = recorded {
        let _ = channel.send(RunEvent::Log {
            kind: LogKind::Info,
            text: format!("\n実行履歴を残せませんでした：{error}\n"),
        });
    }
}

// 今の作業領域の実行履歴を、新しい順に返す（Phase 4a 作業 4。D-137）。作業領域のパスは渡さない
#[tauri::command]
// 実行履歴を読めず退避したときは、macOS 標準の画面で知らせる（Phase 10 作業 15。D-173）。
// async のコマンドはメインスレッドの外で動くので、画面を待ってもメインスレッドは止まらない
pub async fn load_history(
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<Vec<history::EntryView>, String> {
    let root = workspace.root()?;
    let Some(root) = root.to_str() else {
        return Ok(Vec::new());
    };
    ensure_primary(&app)?;
    let entries = history::entries_for(&app_dir(&app)?, root)?;
    if let Some(backup) = history::take_broken() {
        app.dialog()
            .message(format!(
                "実行履歴（history.json）を読めなかったので、別の名前で退避し、空から始めました。\n退避先：\n{}",
                backup.to_string_lossy()
            ))
            .title("実行履歴")
            .kind(MessageDialogKind::Warning)
            .blocking_show();
    }
    Ok(entries)
}

// 今の作業領域の実行履歴を消す。確認の画面は Rust 側で開く（ほかの作業領域の分は残す）。戻り値は、消したか
#[tauri::command]
pub async fn clear_history(
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<bool, String> {
    let root = workspace.root()?;
    let Some(root) = root.to_str() else {
        return Ok(false);
    };
    ensure_primary(&app)?;
    let dir = app_dir(&app)?;
    let count = history::count_for(&dir, root)?;
    if count == 0 {
        return Ok(false);
    }
    // async のコマンドはメインスレッドの外で動くので、確認の画面を待ってもメインスレッドは止まらない
    let confirmed = app
        .dialog()
        .message(format!(
            "この作業領域の実行履歴（{count} 件）を消去しますか？\n元に戻せません。"
        ))
        .title("実行履歴の消去")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "消去する".to_string(),
            "キャンセル".to_string(),
        ))
        .blocking_show();
    if !confirmed {
        return Ok(false);
    }
    history::clear_for(&dir, root)?;
    Ok(true)
}

// 実行中の処理を止める（D-009・D-013）
#[tauri::command]
pub fn stop_run(control: State<'_, RunControl>) {
    control.stop();
}

// デバッグ（Phase 5 作業 1。D-145・D-146）。dev でビルドし、lldb-dap で起動して、終わる・止めるまで返らない。
// case は標準入力にするケース（`sample-N`・`random-N`。None なら input.txt）、breakpoints は始めるときのブレークポイント
// （ファイルは作業領域のルートからの相対パス）。経過とデバッグの知らせは on_event で送る。止めるのは stop_run。
// ほかの処理と同時には動かさない（D-013）。実行履歴には残さない（D-146 2.10）。
// 利用者のプログラムを動かすので、信頼していない作業領域では拒否する（D-128）
#[tauri::command]
pub async fn start_debug(
    target: Target,
    case: Option<String>,
    breakpoints: Vec<SourceBreakpoints>,
    on_event: tauri::ipc::Channel<RunEvent>,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<(), String> {
    let root = workspace.trusted_root()?;
    let dir = package_dir(&root, &target)?;
    let langs = lang_settings_of(&app);
    // Python の debugpy の場所（D-226）
    let python_tools = app_dir(&app).ok().map(|dir| dir.join("python"));
    let runner = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        run::start_debug(
            dir,
            root,
            target,
            case,
            breakpoints,
            langs,
            python_tools,
            &runner.state::<RunControl>(),
            &runner.state::<DebugControl>(),
            on_event,
        )
    })
    .await
    .map_err(|_| "デバッグできません".to_string())??;
    Ok(())
}

// 今のデバッグに、決まった操作を 1 つ行う。答えを待つので、メインスレッドの外で動かす
async fn with_debug<T: Send + 'static>(
    app: AppHandle,
    action: impl FnOnce(&debug::Client) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let client = app.state::<DebugControl>().current()?;
        action(&client)
    })
    .await
    .map_err(|_| "デバッグの操作ができません".to_string())?
}

// 続行・一時停止・ステップオーバー・ステップイン・ステップアウト（D-146 2.5）
#[tauri::command]
pub async fn debug_step(
    action: StepAction,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<(), String> {
    workspace.trusted_root()?;
    with_debug(app, move |client| client.step(action)).await
}

// デバッグ中に、1 つのファイルのブレークポイントを lines のとおりに置き直す（D-146 2.6）。
// path は作業領域のルートからの相対パスで、作業領域の中の .rs と C++ のファイルだけ（D-211）
#[tauri::command]
pub async fn debug_set_breakpoints(
    path: String,
    lines: Vec<u32>,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<Vec<BreakpointView>, String> {
    workspace.trusted_root()?;
    with_debug(app, move |client| client.set_breakpoints(&path, &lines)).await
}

// 止まっているスレッドのコールスタック（D-146 2.7）
#[tauri::command]
pub async fn debug_stack(
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<Vec<FrameView>, String> {
    workspace.trusted_root()?;
    with_debug(app, |client| client.stack()).await
}

#[tauri::command]
pub async fn debug_scopes(
    frame_id: i64,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<Vec<ScopeView>, String> {
    workspace.trusted_root()?;
    with_debug(app, move |client| client.scopes(frame_id)).await
}

#[tauri::command]
pub async fn debug_variables(
    reference: i64,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<Vec<VariableView>, String> {
    workspace.trusted_root()?;
    with_debug(app, move |client| client.variables(reference)).await
}

// 式を評価する（デバッグコンソールは repl、ウォッチ式は watch。D-145 の 3）。止まっているときだけ。
// 式も LLDB のコマンドも、利用者の権限で動く（docs/phase5/01-pre-start-research.md §8）
#[tauri::command]
pub async fn debug_evaluate(
    expression: String,
    frame_id: Option<i64>,
    context: EvaluateContext,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<EvaluateView, String> {
    workspace.trusted_root()?;
    with_debug(app, move |client| {
        client.evaluate(&expression, frame_id, context)
    })
    .await
}

// C++ のコンパイラの設定（D-205）と AC Library の場所（D-206）。読めないときは既定（g++ を探す）
fn cpp_settings_of(app: &AppHandle) -> CppSettings {
    app_dir(app)
        .map(|dir| CppSettings {
            ac_library: cpp::ac_library_dir(&dir),
            ..storage::cpp_settings(&dir)
        })
        .unwrap_or_default()
}

// 実行・テスト・デバッグに渡す言語ごとの設定（C++ と Python。Phase 19 作業 1。D-221）
fn lang_settings_of(app: &AppHandle) -> LangSettings {
    LangSettings {
        cpp: cpp_settings_of(app),
        python: app_dir(app)
            .map(|dir| storage::python_settings(&dir))
            .unwrap_or_default(),
    }
}

// 言語サーバーにつなぐ C++ のファイルの拡張子（D-210）
const CPP_EXTENSIONS: &[&str] = &["cpp", "cc", "cxx", "hpp", "hh", "h"];

// 開いているファイルの言語サーバーを起動する（Phase 3b-1 作業 2。D-124。C++ は Phase 17 作業 5。D-210）。
// .rs は最寄りの Cargo.toml のパッケージで rust-analyzer を、C++ のファイルは最寄りの C++ のパッケージ（main.cpp）で clangd を動かす。
// 同じパッケージ・同じ session で動いていれば何もしない。別のパッケージか別の session（画面の再読み込み）、別の言語
// で動いていれば、止めてから起動する。session は画面を読み込むたびに画面が作る文字列。
// サーバーからのメッセージ・標準エラー・終了は on_event で送る（09 K-07）。on_event は起動したときだけ使い、
// 何もしないときは捨てる。Tauri のチャンネルは呼び出しごとに番号を 0 から振り直すので、画面は呼ぶたびに
// 新しいチャンネルを渡す。そのほかのファイルと、パッケージの外のファイルでは None を返し、起動しない
#[tauri::command]
pub async fn lsp_start(
    path: String,
    session: String,
    on_event: tauri::ipc::Channel<LspEvent>,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<Option<Started>, String> {
    // 言語サーバーは開いただけでコードを動かすことがあるので、信頼していない作業領域では拒否する（D-128）
    let root = workspace.trusted_root()?;
    let file = resolve(&root, &path).map_err(|error| error.to_string())?;
    let extension = file
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_string();
    let cpp = cpp_settings_of(&app);
    let python = lang_settings_of(&app).python;
    // 止めるときに正式な終了を待ち、g++ に標準ヘッダーの場所を聞くので、メインスレッドの外で動かす
    tauri::async_runtime::spawn_blocking(move || {
        let launch = if extension == "rs" {
            let Some(package) = toolchain::nearest_manifest_dir(&root, &file) else {
                return Ok(None);
            };
            // コンテストのフォルダの中なら、そのフォルダの問題をまとめて扱う（Phase 10 作業 11。D-171）
            let (dir, projects) = lsp::analyzer_place(&root, &package);
            lsp::rust_analyzer(dir, projects)?
        } else if CPP_EXTENSIONS.contains(&extension.as_str()) {
            let Some((package, Language::Cpp)) = toolchain::nearest_package(&root, &file) else {
                return Ok(None);
            };
            let (dir, projects) = lsp::clangd_place(&root, package);
            let compiler = cpp::find_compiler(&cpp)?;
            let options = cpp::compilation_database(&compiler, &projects, &cpp);
            lsp::clangd(dir, projects, &compiler, options)?
        } else if extension == "py" {
            // Python のパッケージ（main.py）で basedpyright を動かす（Phase 20 作業 5。D-225）。
            // 処理系が見つからなくても動かす（標準ライブラリは basedpyright の typeshed で分かる）
            let Some((package, Language::Python)) = toolchain::nearest_package(&root, &file) else {
                return Ok(None);
            };
            let (dir, projects) = lsp::basedpyright_place(&root, package);
            let interpreter = crate::python::find_interpreter(&python).ok();
            lsp::basedpyright(dir, projects, interpreter.as_deref())?
        } else {
            return Ok(None);
        };
        let sink = std::sync::Arc::new(move |event: LspEvent| {
            let _ = on_event.send(event);
        });
        let name = launch.name();
        let dir = launch.dir().to_path_buf();
        let already_running =
            app.state::<LspControl>()
                .start(root.clone(), launch, session, sink)?;
        Ok(Some(lsp::started(&root, &dir, already_running, name)))
    })
    .await
    .map_err(|_| "言語サーバーを起動できません".to_string())?
}

// rust-analyzer が返した場所（file: の URI）を、作業領域の中のファイルなら、ルートからの相対パスにする
// （定義ジャンプ・参照検索でタブに開く場所を決める。Phase 3b-2 作業 4。D-133）。外・ない・フォルダ・リンクで外を指すものは null。
// ファイルは読まないので、信頼していなくても使える
#[tauri::command]
pub fn lsp_locate(uri: String, workspace: State<'_, Workspace>) -> Result<Option<String>, String> {
    Ok(lsp::locate_in_workspace(&workspace.root()?, &uri))
}

// エラー一覧・実行ログのファイル名（例：`src/main.rs`）を、作業領域の中のファイルの相対パスにする
// （そこへ移動する。Phase 4a 作業 1。D-137）。作業領域の外・ない・フォルダ・リンクで外を指すものは null。
// ファイルは読まないので、信頼していなくても使える
#[tauri::command]
pub fn locate_source(
    package: String,
    file: String,
    workspace: State<'_, Workspace>,
) -> Result<Option<String>, String> {
    Ok(locate_in_package(&workspace.root()?, &package, &file))
}

// 作業領域の外の標準ライブラリ・依存クレートのソースを、読み取り専用で読む（定義ジャンプの行き先。
// Phase 3b-2 作業 5。D-133）。読めるのは ~/.rustup/toolchains と ~/.cargo/registry/src の中の .rs だけで、
// 作業領域にも信頼にも関係しない（書く経路はない。ファイル操作の境界 AC1-13 は変えない）
// C++ のヘッダー（使う g++ の標準ヘッダー・SDK の usr/include・AC Library）も読める（Phase 17 作業 5。D-210）。
// g++ に場所を聞くので、メインスレッドの外で動かす
#[tauri::command]
pub async fn read_source(uri: String, app: AppHandle) -> Result<ReadOnlySource, String> {
    let cpp = cpp_settings_of(&app);
    let python = lang_settings_of(&app).python;
    tauri::async_runtime::spawn_blocking(move || {
        let cpp_roots = cpp::find_compiler(&cpp)
            .map(|compiler| cpp::header_dirs(&compiler, cpp.ac_library.as_deref()))
            .unwrap_or_default();
        // Python：basedpyright の typeshed と、設定の処理系の標準ライブラリ・site-packages（D-225）
        let mut python_roots: Vec<PathBuf> = lsp::basedpyright_typeshed().into_iter().collect();
        if let Ok(interpreter) = crate::python::find_interpreter(&python) {
            python_roots.extend(crate::python::library_dirs(&interpreter));
        }
        sources::read_source(&uri, &cpp_roots, &python_roots)
    })
    .await
    .map_err(|_| "ファイルを読めません".to_string())?
}

// 画面から rust-analyzer へ、LSP のメッセージ（JSON の文字列）を 1 つ送る（D-123）。許可リストにない方法、
// 作業領域の外の textDocument.uri、大きすぎる本文はエラーにする（D-131）。
// initialize の場所と設定は Rust 側で上書きする。メッセージは積むだけで、書き込みは待たない。
// 呼ばれた順に書く（同期のコマンドなので、画面が呼んだ順に届く。推測）
#[tauri::command]
pub fn lsp_send(message: String, control: State<'_, LspControl>) -> Result<(), String> {
    control.send(&message)
}

// rust-analyzer を止める（作業領域を閉じるとき、開発者ツールからの停止。D-124・D-126）
#[tauri::command]
pub async fn lsp_stop(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<LspControl>().stop())
        .await
        .map_err(|_| "言語サーバーを止められません".to_string())
}

// 画面に渡す問題（Phase 3a 作業 3）。問題文は無害化してから渡す（D-113・D-118）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemView {
    contest: String,
    task: String,
    url: String,
    title: String,
    time_limit: String,
    memory_limit: String,
    statement_html: String,
    sample_count: usize,
}

// 問題を返す。保存してあればそれを使い、なければ（または refresh のとき）取得する（D-110〜D-112）。
// 取得は curl を起動して待つので、メインスレッドの外で動かす
#[tauri::command]
pub async fn load_problem(
    contest: String,
    task: String,
    refresh: bool,
    app: AppHandle,
) -> Result<ProblemView, String> {
    let id = ProblemId::new(&contest, &task)?;
    let dir = app_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let problem = problem::load_problem(&dir, &id, refresh)?;
        Ok(ProblemView {
            url: id.url(),
            contest: id.contest,
            task: id.task,
            title: problem.title,
            time_limit: problem.time_limit,
            memory_limit: problem.memory_limit,
            statement_html: sanitize(&problem.statement_html),
            sample_count: problem.samples.len(),
        })
    })
    .await
    .map_err(|_| "問題を読み込めません".to_string())?
}

// コンテストの問題の一覧（選択欄。D-117）。保存してあればそれを使う（D-111）
#[tauri::command]
pub async fn load_task_list(
    contest: String,
    refresh: bool,
    app: AppHandle,
) -> Result<Vec<problem::TaskEntry>, String> {
    let dir = app_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || problem::load_task_list(&dir, &contest, refresh))
        .await
        .map_err(|_| "問題の一覧を読み込めません".to_string())?
}

// AtCoder のログインの窓を開く（Phase 6 作業 1。D-147・D-148）。開いていれば前に出す。
// ログインが済んだか・失敗したかは、イベント atcoder-session で main の窓に知らせる
#[tauri::command]
pub fn atcoder_login(app: AppHandle) -> Result<(), String> {
    session::open_login_window(&app)
}

// ログインの状態（Keychain に置いてあるか。値は読まない）と、確かめたユーザー名
#[tauri::command]
pub async fn atcoder_session(app: AppHandle) -> Result<SessionView, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<SessionState>().view(&Keychain::app()))
        .await
        .map_err(|_| "ログインの状態を読めません".to_string())?
}

// ログアウト：Keychain の値を消す（サーバーには何も送らない。D-148）
#[tauri::command]
pub async fn atcoder_logout(app: AppHandle) -> Result<SessionView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        session::logout(&app.state::<SessionState>(), &Keychain::app())
    })
    .await
    .map_err(|_| "ログアウトできません".to_string())?
}

// AtCoder に提出する（Phase 6 作業 2。D-147・D-148）。開催期間・ログイン・フォームを確かめ、
// Rust 側の確認のダイアログで「提出する」が選ばれたときだけ送る（画面からは、ダイアログを通らずに送れない）。
// 送ったら、判定は別のスレッドで追い、イベント atcoder-submission で知らせる。
// 提出するのは bin のソースそのまま（D-148）。ソースを読むのに cargo metadata を動かすので、信頼していない作業領域では拒否する（D-128）
#[tauri::command]
pub async fn atcoder_submit(
    target: Target,
    contest: String,
    task: String,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<SubmitOutcome, String> {
    let id = ProblemId::new(&contest, &task)?;
    let root = workspace.trusted_root()?;
    // ほかの処理（実行・テスト・デバッグ）の間は送らない（D-013）
    if app.state::<RunControl>().running() {
        return Err("別の処理を実行中です".to_string());
    }
    let dir = package_dir(&root, &target)?;
    // C++ のコンパイラの設定（版の注意に使う。D-209）
    let cpp_settings = storage::cpp_settings(&app_dir(&app)?);
    // Python の処理系の設定（提出の言語と、import・版の注意に使う。D-224）
    let python_settings = storage::python_settings(&app_dir(&app)?);
    // 実行履歴に残す先。UTF-8 で表せないパスの作業領域は、記録しない（record_history と同じ）
    let place = root.to_str().and_then(|workspace| {
        ensure_primary(&app).ok()?;
        Some(submit::HistoryPlace {
            dir: app_dir(&app).ok()?,
            workspace: workspace.to_string(),
        })
    });
    // 提出の設定は、画面から受け取らず、settings.json から読む（D-162）
    let settings = storage::submit_settings(&app_dir(&app)?);
    // パッケージのフォルダから決まる問題（D-155）。画面で紐付けを外して別の問題を選んだときに知らせる（D-178）
    let folder_problem = problem::package_problem(&app_dir(&app)?, &target.package);
    tauri::async_runtime::spawn_blocking(move || {
        // 提出するソースと、言語ごとの検査（Rust は D-148、C++ は D-209、Python は D-224）
        let (mut prepared, bins) = match target.language {
            Language::Rust => {
                let cargo =
                    toolchain::find_cargo().ok_or_else(|| "cargo が見つかりません".to_string())?;
                let metadata = toolchain::metadata_json(&cargo, &dir)?;
                (
                    submit::prepare(&root, &dir, &metadata, &target.bin, &id.task)?,
                    toolchain::bin_names(&metadata, &dir)?,
                )
            }
            Language::Cpp => {
                let version = cpp::find_compiler(&cpp_settings)
                    .and_then(|compiler| cpp::version(&compiler))
                    .ok()
                    .map(|(_, number)| number);
                (
                    submit::prepare_cpp(&root, &dir, version.as_deref())?,
                    vec![cpp::BIN.to_string()],
                )
            }
            Language::Python => (
                submit::prepare_python(&root, &dir, &python_settings)?,
                vec![crate::python::BIN.to_string()],
            ),
        };
        prepared
            .warnings
            .extend(submit::folder_problem_warning(folder_problem.as_ref(), &id));
        let source_path = root.join(&prepared.file);
        // サンプルテストのケースがあるか（テストのフォルダの sample-N・random-N。D-162）
        let has_cases = toolchain::test_dir(&dir, &bins, &target.bin)
            .dir
            .is_some_and(|tests| run::has_test_cases(&tests));
        submit::submit(
            &app,
            prepared,
            Some(source_path),
            &target,
            &id.contest,
            &id.task,
            place,
            submit::SubmitChecks {
                settings,
                has_cases,
            },
        )
    })
    .await
    .map_err(|_| "提出できません".to_string())?
}

// このアプリを起動してから IDE で出した提出（新しい順。保存しない）
#[tauri::command]
pub fn atcoder_submissions(state: State<'_, submit::SubmitState>) -> Vec<SubmissionView> {
    state.views()
}

// 「更新」：その提出の判定を読み直す（まだ判定中なら、また追う）
#[tauri::command]
pub fn atcoder_refresh_submission(key: u64, app: AppHandle) -> Result<SubmissionView, String> {
    submit::refresh(&app, key)
}

// 公式の提出のページを既定のブラウザで開く（期間の外・送れなかったときの「コードをコピーして公式の提出ページを開く」。
// コードのコピーは画面が行う）。ログインは要らない
#[tauri::command]
pub fn open_submit_page(contest: String, task: String) -> Result<(), String> {
    submit::open_submit_page(&contest, &task)
}

// 提出の詳細（submission_id があるとき）か、自分の提出の一覧を、既定のブラウザで開く
#[tauri::command]
pub fn open_submission_page(contest: String, submission_id: Option<u64>) -> Result<(), String> {
    submit::open_submission_page(&contest, submission_id)
}

// サイドバーの「コンテスト」：コンテストの問題ごとの提出状況（Phase 18 作業 4。D-218）。
// 「自分の提出」のページを、セッションを付けて読む（読むのは画面が頼んだときだけ）
#[tauri::command]
pub async fn contest_status(
    contest: String,
    app: AppHandle,
) -> Result<submit::StatusOutcome, String> {
    let dir = app_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        submit::contest_status(&submit::Remote::app(), &dir, &contest)
    })
    .await
    .map_err(|_| "提出状況を読めません".to_string())
}

// 公式の問題ページをブラウザで開く（取得・読み取りに失敗したときの代替手段。D-110）
#[tauri::command]
pub fn open_problem_page(contest: String, task: String) -> Result<(), String> {
    problem::open_in_browser(&ProblemId::new(&contest, &task)?)
}

// サンプルを、パッケージのテストケースのフォルダ（D-106）に書く（D-112。利用者が「サンプルを保存」を押したとき）。
// フォルダは test/<バイナリ名>/ があればそれ、なければバイナリが 1 つなら test/、複数なら test/<バイナリ名>/ を作る。
// 戻り値の folder は、パッケージのディレクトリからの相対パス。
// cargo metadata を動かすので、信頼していない作業領域では拒否する（D-128。Phase 4a 作業 3 で、検査の抜けを直した）
#[derive(serde::Serialize)]
pub struct SavedSamplesView {
    folder: String,
    #[serde(flatten)]
    saved: problem::SavedSamples,
}

#[tauri::command]
pub async fn save_samples(
    target: Target,
    contest: String,
    task: String,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<SavedSamplesView, String> {
    let id = ProblemId::new(&contest, &task)?;
    // cargo metadata を動かすので、信頼していない作業領域では拒否する（D-128）
    let root = workspace.trusted_root()?;
    let dir = package_dir(&root, &target)?;
    let app_dir = app_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        // バイナリの一覧（テストのフォルダを決めるのに使う）。C++ は main だけ（D-204）
        let bins = match target.language {
            Language::Rust => {
                let cargo =
                    toolchain::find_cargo().ok_or_else(|| "cargo が見つかりません".to_string())?;
                toolchain::bin_names(&toolchain::metadata_json(&cargo, &dir)?, &dir)?
            }
            Language::Cpp => vec![cpp::BIN.to_string()],
            Language::Python => vec![crate::python::BIN.to_string()],
        };
        if !bins.contains(&target.bin) {
            return Err(format!("バイナリ「{}」が見つかりません", target.bin));
        }
        let folder = toolchain::test_dir(&dir, &bins, &target.bin)
            .dir
            .unwrap_or_else(|| {
                if bins.len() == 1 {
                    dir.join("test")
                } else {
                    dir.join("test").join(&target.bin)
                }
            });
        // テストケースのフォルダ（と test/）が、リンクで作業領域の外を指していないことを確かめる
        for path in [dir.join("test"), folder.clone()] {
            if path.exists()
                && !path
                    .canonicalize()
                    .is_ok_and(|real| real.starts_with(&root))
            {
                return Err("テストケースのフォルダが作業領域の外にあります".to_string());
            }
        }
        let problem = problem::load_problem(&app_dir, &id, false)?;
        let saved = problem::save_samples(&folder, &problem.samples)?;
        let relative = folder
            .strip_prefix(&dir)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(SavedSamplesView {
            folder: relative,
            saved,
        })
    })
    .await
    .map_err(|_| "サンプルを保存できません".to_string())?
}

// 選択欄で選んだ問題のパッケージ（`<コンテスト>/<問題 ID>/`）を、作業領域の直下に作る（Phase 7a。D-151）。
// 問題は保存してあればそれを使い、なければ取得する（D-111・D-149）。すでにあるファイルは変えない。
// ファイルを書くだけで、コードは動かさないので、信頼は問わない
#[tauri::command]
pub async fn create_problem_package(
    contest: String,
    task: String,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<ProblemCreated, String> {
    let id = ProblemId::new(&contest, &task)?;
    let root = workspace.root()?;
    let dir = app_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let control = app.state::<ScaffoldControl>();
        let _running = control.begin()?;
        let template = template_for(&dir)?;
        scaffold::create_problem(&root, &id, &template, &|id| {
            problem::load_problem(&dir, id, false)
        })
    })
    .await
    .map_err(|_| "問題を作れません".to_string())?
}

// テンプレートと、問題を作るときに input! を入れるか（settings.json の problem.fillInput・mainLoop。D-198。Rust だけ）。
// 言語は settings.json の problem.language（D-208。コンテストごとには選ばない。ユーザーの指摘、2026-10-28）。
// C++ は compile_flags.txt の中身（AC Library があればその場所）も入れる
fn template_for(dir: &Path) -> Result<scaffold::Template, String> {
    let settings = storage::problem_settings(dir);
    let language = settings.language;
    let mut template = scaffold::load_template(dir, language)?;
    // 入力のコードを入れておく設定（D-198。C++ は Phase 17 作業 8。D-213）
    template.fill = settings.fill_input.then_some(scaffold::FillInput {
        main_loop: settings.main_loop,
    });
    if language == Language::Cpp {
        template.compile_flags = cpp::compile_flags(cpp::ac_library_dir(dir).as_deref());
    }
    Ok(template)
}

// コンテストの問題をすべて、作業領域の直下の `<コンテスト>/` に作る（Phase 7a。D-151）。
// 問題の一覧を読み、問題のページを 1 問ずつ取る（1 秒あける・保存してあれば使う）。経過は on_event で送る。止めるのは stop_scaffold
#[tauri::command]
pub async fn create_contest_packages(
    contest: String,
    on_event: tauri::ipc::Channel<ScaffoldEvent>,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<ContestCreated, String> {
    let root = workspace.root()?;
    let dir = app_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let control = app.state::<ScaffoldControl>();
        let (_running, cancel) = control.begin()?;
        let template = template_for(&dir)?;
        scaffold::create_contest(
            &root,
            &contest,
            &template,
            &|contest| problem::load_task_list(&dir, contest, false),
            &|id| problem::load_problem(&dir, id, false),
            &cancel,
            &|event| {
                let _ = on_event.send(event);
            },
        )
    })
    .await
    .map_err(|_| "コンテストの問題を作れません".to_string())?
}

// コンテストの問題を作るのを止める（今の問題を取り終えたところで止まる）
#[tauri::command]
pub fn stop_scaffold(control: State<'_, ScaffoldControl>) {
    control.stop();
}

// パッケージ（作業領域のルートからの相対パス）で選んだ問題。なければ null（D-117）
#[tauri::command]
pub fn problem_choice(
    package: String,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<Option<ProblemId>, String> {
    let dir = resolve(&workspace.root()?, &package).map_err(|error| error.to_string())?;
    Ok(problem::choice_for(&app_dir(&app)?, &dir))
}

// 実行の対象のパッケージ（作業領域のルートからの相対パス）のフォルダから決まる問題。決まらなければ null（D-117・D-155）。
// 名前と、アプリのフォルダの保存した問題の一覧を見るだけなので、信頼は問わない
#[tauri::command]
pub fn package_problem(package: String, app: AppHandle) -> Result<Option<ProblemId>, String> {
    Ok(problem::package_problem(&app_dir(&app)?, &package))
}

// パッケージで選んだ問題を覚える（D-117）
#[tauri::command]
pub fn set_problem_choice(
    package: String,
    contest: String,
    task: String,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<(), String> {
    let id = ProblemId::new(&contest, &task)?;
    let dir = resolve(&workspace.root()?, &package).map_err(|error| error.to_string())?;
    problem::set_choice(&app_dir(&app)?, &dir, id)
}

// アプリのフォルダ（D-035：macOS では ~/Library/Application Support/<識別子>）
fn app_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|_| "アプリのフォルダが分かりません".to_string())
}

// 単一起動のロック（D-040）を持つプロセスか確かめる。2 つ目の起動は、知らせる画面を閉じるまで WebView が動くので、
// その間に settings.json・state.json を読み書き・退避しないようにする
fn ensure_primary(app: &AppHandle) -> Result<(), String> {
    if app.try_state::<crate::InstanceLock>().is_none() {
        return Err("AtCoder Studio はすでに起動しています".to_string());
    }
    Ok(())
}

// 起動したときに WebView に返す内容。作業領域はパスではなく名前だけを渡す
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupState {
    layout: Option<Layout>,
    workspace_name: Option<String>,
    tabs: Vec<String>,
    active_tab: Option<String>,
    // 前回の作業領域を信頼しているか（記録がなければ、確認の画面で聞いた結果。D-128）
    workspace_trusted: bool,
    settings: SettingsView,
}

// settings.json の中身（キー割り当てと見た目。D-052・D-159）。起動したときと、窓に戻って読み直したときに渡す
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    key_bindings: Vec<KeyBinding>,
    // 読めなかったキー割り当て（JSON の文字列）
    invalid_key_bindings: Vec<String>,
    appearance: Appearance,
    // 提出の設定（D-162）。画面は表示と設定の画面に使うだけで、提出のときは Rust 側が読み直す
    submit: SubmitSettings,
    // 問題文の欄の設定（D-196・D-197）
    problem: ProblemSettings,
    // Python の設定（Phase 19 作業 3。D-221）。設定の画面は処理系を選ぶのに使う。動かすときは Rust 側が読み直す
    python: PythonSettings,
    // 読めなかった見た目・提出・問題文の欄の項目
    appearance_problems: Vec<String>,
}

impl SettingsView {
    fn from_loaded(loaded: LoadedSettings) -> Self {
        SettingsView {
            key_bindings: loaded.key_bindings,
            invalid_key_bindings: loaded.invalid,
            appearance: loaded.appearance,
            submit: loaded.submit,
            problem: loaded.problem,
            python: loaded.python,
            appearance_problems: loaded.appearance_problems,
        }
    }

    fn defaults() -> Self {
        SettingsView {
            key_bindings: storage::default_key_bindings(),
            invalid_key_bindings: Vec::new(),
            appearance: Appearance::default(),
            submit: SubmitSettings::default(),
            problem: ProblemSettings::default(),
            python: PythonSettings::default(),
            appearance_problems: Vec::new(),
        }
    }
}

// 最後に読んだときの、言語ごとのスニペットのファイルの更新日時と大きさ（Phase 13 作業 1。D-186。言語ごとは Phase 17 作業 9。D-213）
#[derive(Default)]
pub struct SnippetsStamp(Mutex<std::collections::HashMap<&'static str, Option<(SystemTime, u64)>>>);

// 言語のスニペットのファイルを読む（D-186）。force でなければ、最後に読んだときから変わっていないとき null（窓に戻ったときに呼ぶ）。
// 読めないときは Err（画面は前のスニペットを使い続ける。同じ中身では二度知らせない）
#[tauri::command]
pub fn reload_snippets(
    force: bool,
    language: Language,
    app: AppHandle,
    stamp: State<'_, SnippetsStamp>,
) -> Result<Option<snippets::Loaded>, String> {
    let dir = app_dir(&app)?;
    let now = snippets::stamp(&dir, language);
    {
        let mut last = stamp
            .0
            .lock()
            .map_err(|_| "スニペットの状態を読めません".to_string())?;
        let key = snippets::file_name(language);
        if !force && last.get(key).is_some_and(|seen| *seen == now) {
            return Ok(None);
        }
        last.insert(key, now);
    }
    snippets::load(&dir, language).map(Some)
}

// 雛形のファイル（templates/main.rs・templates/dependencies.toml）があるか（Phase 13 作業 3。D-188）
#[tauri::command]
pub fn template_files(app: AppHandle) -> Result<scaffold::TemplateFiles, String> {
    Ok(scaffold::template_files(&app_dir(&app)?))
}

// 問題文から input! の中身を作る（Phase 15 作業 1。D-194）。保存してある問題だけを使い、取得し直さない。
// ファイルを書かないので、信頼は要らない。
// 言語は、作業領域の `<コンテスト>/<問題>/` がパッケージならその言語、なければ設定の「問題を作るときの言語」
// （C++ なら宣言と cin の行。Phase 17 作業 8。D-213）
#[tauri::command]
pub async fn make_input(
    contest: String,
    task: String,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<input_gen::Generated, String> {
    let id = ProblemId::new(&contest, &task)?;
    let dir = app_dir(&app)?;
    let package = workspace
        .root()
        .ok()
        .and_then(|root| resolve(&root, &format!("{}/{}", id.contest, id.task)).ok());
    tauri::async_runtime::spawn_blocking(move || {
        let language = package
            .and_then(|package| toolchain::package_language(&package))
            .unwrap_or_else(|| storage::problem_settings(&dir).language);
        let problem = problem::load_saved(&dir, &id)?;
        input_gen::generate_for(&problem.statement_html, &problem.samples, language)
    })
    .await
    .map_err(|_| "input! を作れません".to_string())?
}

// 作った C++ の入力のコードを、text（エディタの C++ のファイルの中身）の solve()（なければ main()）に書いた結果を返す
// （「solve() の入力を置き換える」。Phase 17 作業 8。D-213）。画面がエディタの文書を置き換える。ファイルは読み書きしない
#[tauri::command]
pub fn fill_cpp_input(
    text: String,
    solve: Vec<String>,
    main: Option<Vec<String>>,
    main_loop: bool,
) -> Result<input_cpp::Filled, String> {
    if text.len() as u64 > crate::workspace::MAX_FILE_SIZE {
        return Err("ファイルが大きすぎます".to_string());
    }
    input_cpp::fill(&text, &solve, main.as_deref(), main_loop)
}

// 作った Python の入力のコードを、text（エディタの .py の中身）の solve()（なければ main()）に書いた結果を返す
// （「solve() の入力を置き換える」。Phase 20 作業 7。D-227）。画面がエディタの文書を置き換える。ファイルは読み書きしない
#[tauri::command]
pub fn fill_python_input(
    text: String,
    solve: Vec<String>,
    main: Option<Vec<String>>,
    main_loop: bool,
) -> Result<input_cpp::Filled, String> {
    if text.len() as u64 > crate::workspace::MAX_FILE_SIZE {
        return Err("ファイルが大きすぎます".to_string());
    }
    input_py::fill(&text, &solve, main.as_deref(), main_loop)
}

// テンプレートのフォルダ（アプリのフォルダの templates/）を Finder で開く。なければ空のフォルダを作る（D-188）
#[tauri::command]
pub fn open_templates_folder(app: AppHandle) -> Result<(), String> {
    let dir = scaffold::ensure_template_dir(&app_dir(&app)?)?;
    let status = std::process::Command::new("/usr/bin/open")
        .arg(&dir)
        .status()
        .map_err(|error| format!("テンプレートのフォルダを開けません：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("テンプレートのフォルダを開けません".to_string())
    }
}

// 言語のスニペットのファイルを既定のテキストエディタで開く（Phase 13 作業 2。D-187）。なければ空のもの（{ }）を作る。
// 書き換えて IDE の窓に戻ると読み直す
#[tauri::command]
pub fn open_snippets_file(language: Language, app: AppHandle) -> Result<(), String> {
    let path = snippets::ensure_file(&app_dir(&app)?, language)?;
    let name = snippets::file_name(language);
    let status = std::process::Command::new("/usr/bin/open")
        .arg("-t")
        .arg(&path)
        .status()
        .map_err(|error| format!("{name} を開けません：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{name} を開けません"))
    }
}

// VS Code の言語のスニペット（rust.json・cpp.json）を、言語のファイルに写す（D-187・D-213）。押したときだけ。
// 確認のダイアログのあと、今のファイルを退避してから写す
#[tauri::command]
pub async fn import_vscode_snippets(
    language: Language,
    app: AppHandle,
) -> Result<snippets::Imported, String> {
    let dir = app_dir(&app)?;
    let home = toolchain::home_dir().ok_or_else(|| "ホームのフォルダが分かりません".to_string())?;
    // Windows の VS Code の場所（%APPDATA%\Code\User\snippets）にするのは段階 3
    let source = home
        .join("Library/Application Support/Code/User/snippets")
        .join(snippets::vscode_file_name(language));
    tauri::async_runtime::spawn_blocking(move || {
        snippets::import(&dir, &source, language, |count, problems, exists| {
            let name = snippets::file_name(language);
            let mut message = format!(
                "VS Code の {} のスニペット {count} 件を {name} に取り込みます。",
                language.label()
            );
            if problems > 0 {
                message.push_str(&format!(
                    "\n読めない項目が {problems} 個あります（そのまま写し、使うときに知らせます）。"
                ));
            }
            if exists {
                let stem = name.trim_end_matches(".json");
                message.push_str(&format!(
                    "\n\n今の {name} は、同じフォルダに退避します（{stem}.backup-…json）。"
                ));
            }
            app.dialog()
                .message(message)
                .title("スニペットの取り込み")
                .kind(MessageDialogKind::Info)
                .buttons(MessageDialogButtons::OkCancelCustom(
                    "取り込む".to_string(),
                    "キャンセル".to_string(),
                ))
                .blocking_show()
        })
    })
    .await
    .map_err(|_| "取り込めません".to_string())?
}

// 最後に読んだときの settings.json の更新日時と大きさ。窓に戻ったときに比べ、変わっていれば読み直す（D-159）
#[derive(Default)]
pub struct SettingsStamp(Mutex<Option<(SystemTime, u64)>>);

impl SettingsStamp {
    // 今のファイルの印が、最後に読んだ（書いた）ときと同じか
    fn is_current(&self, dir: &Path) -> Result<bool, String> {
        let last = self
            .0
            .lock()
            .map_err(|_| "設定の状態を読めません".to_string())?;
        Ok(*last == storage::settings_stamp(dir))
    }

    // 今のファイルの印を覚え、前と違えば true
    fn update(&self, dir: &Path) -> Result<bool, String> {
        let now = storage::settings_stamp(dir);
        let mut last = self
            .0
            .lock()
            .map_err(|_| "設定の状態を読めません".to_string())?;
        let changed = *last != now;
        *last = now;
        Ok(changed)
    }
}

// 起動したときに、保存してあった設定・レイアウト・前回の作業領域を読む（作業 6）。
// 読めないファイルは別名で退避し、退避先を macOS 標準の画面で知らせる（D-039）
#[tauri::command]
pub async fn load_app_state(
    app: AppHandle,
    workspace: State<'_, Workspace>,
    stamp: State<'_, SettingsStamp>,
) -> Result<StartupState, String> {
    ensure_primary(&app)?;
    let dir = app_dir(&app)?;
    let mut backups = Vec::new();

    let settings = match storage::load_settings(&dir)? {
        Loaded::Ok(loaded) => SettingsView::from_loaded(loaded),
        Loaded::Missing => {
            storage::write_default_settings(&dir)?;
            SettingsView::defaults()
        }
        Loaded::Broken(backup) => {
            backups.push(backup);
            storage::write_default_settings(&dir)?;
            SettingsView::defaults()
        }
    };
    stamp.update(&dir)?;

    let mut startup = StartupState {
        layout: None,
        workspace_name: None,
        tabs: Vec::new(),
        active_tab: None,
        workspace_trusted: false,
        settings,
    };
    match storage::load_state(&dir)? {
        Loaded::Ok(saved) => {
            startup.layout = Some(saved.layout);
            // 前回の作業領域が今もあれば開く。パスは Rust 側で確かめ直す
            if let Some(root) = saved.workspace.and_then(|p| p.canonicalize().ok())
                && root.is_dir()
            {
                startup.workspace_name = root
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned());
                startup.tabs = saved.tabs;
                startup.active_tab = saved.active_tab;
                let trusted = decide_trust(&app, &root)?;
                startup.workspace_trusted = trusted;
                workspace.set(root, trusted)?;
            }
        }
        Loaded::Missing => {}
        Loaded::Broken(backup) => backups.push(backup),
    }

    if !backups.is_empty() {
        let list = backups
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        app.dialog()
            .message(format!(
                "設定ファイルを読めなかったので、別の名前で退避し、既定の内容で起動しました。\n退避先：\n{list}"
            ))
            .title("設定ファイル")
            .kind(MessageDialogKind::Warning)
            .blocking_show();
    }
    Ok(startup)
}

// 窓に戻ったときに、settings.json が変わっていれば読み直す（D-159）。変わっていなければ None。
// なくなっていれば既定の内容（ファイルは作らない）。読めなければ Err（退避はせず、画面は今の設定のまま使う）
#[tauri::command]
pub fn reload_settings(
    app: AppHandle,
    stamp: State<'_, SettingsStamp>,
) -> Result<Option<SettingsView>, String> {
    ensure_primary(&app)?;
    let dir = app_dir(&app)?;
    if !stamp.update(&dir)? {
        return Ok(None);
    }
    Ok(Some(match storage::reread_settings(&dir)? {
        Some(loaded) => SettingsView::from_loaded(loaded),
        None => SettingsView::defaults(),
    }))
}

// 設定の画面の保存の結果（作業 6b・Phase 9 作業 2）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SaveSettingsOutcome {
    Saved,
    // 最後に読んだあとに settings.json が外で変わっていたので、書かなかった（画面は読み直す）
    ChangedOutside,
    // 確認のダイアログを省く設定を、OS のダイアログで「キャンセル」した（D-162）
    Cancelled,
}

// 設定の画面の保存（作業 6b。D-159）。提出の確認のダイアログを省く設定（submit.confirmDialog を false）に
// 変えるときは、OS のダイアログで確かめる（D-162。画面だけでは省くようにできない）。
// async のコマンドはメインスレッドの外で動くので、確認の画面を待ってもメインスレッドは止まらない
#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    stamp: State<'_, SettingsStamp>,
    key_bindings: Vec<KeyBinding>,
    appearance: Appearance,
    submit: SubmitSettings,
    problem: ProblemSettings,
    interpreter: Interpreter,
) -> Result<SaveSettingsOutcome, String> {
    ensure_primary(&app)?;
    let dir = app_dir(&app)?;
    if !stamp.is_current(&dir)? {
        return Ok(SaveSettingsOutcome::ChangedOutside);
    }
    if storage::submit_settings(&dir).confirm_dialog && !submit.confirm_dialog {
        let confirmed = app
            .dialog()
            .message(
                "提出の確認のダイアログを省くようにしますか？\n\n提出の前のサンプルテストが、今のソースですべて AC で、提出の前の検査の注意がないときだけ、確認せずに AtCoder に送ります。サンプルがない問題などでは、今までどおり確認します。",
            )
            .title("提出の確認")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "省く".to_string(),
                "キャンセル".to_string(),
            ))
            .blocking_show();
        if !confirmed {
            return Ok(SaveSettingsOutcome::Cancelled);
        }
        // ダイアログを待つ間に外で変わっていれば書かない
        if !stamp.is_current(&dir)? {
            return Ok(SaveSettingsOutcome::ChangedOutside);
        }
    }
    storage::write_settings(
        &dir,
        &key_bindings,
        &appearance,
        &submit,
        &problem,
        interpreter,
    )?;
    stamp.update(&dir)?;
    Ok(SaveSettingsOutcome::Saved)
}

// settings.json を、macOS の open コマンドで既定のテキストエディタで開く（D-159）。なければ既定の内容で作る
#[tauri::command]
pub fn open_settings_file(app: AppHandle) -> Result<(), String> {
    ensure_primary(&app)?;
    let dir = app_dir(&app)?;
    let path = dir.join(storage::SETTINGS_FILE);
    if !path.exists() {
        storage::write_default_settings(&dir)?;
    }
    let status = std::process::Command::new("/usr/bin/open")
        .arg("-t")
        .arg(&path)
        .status()
        .map_err(|error| format!("settings.json を開けません：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("settings.json を開けません".to_string())
    }
}

// レイアウト・開いているタブを保存する（作業 6）。作業領域のパスは Rust 側が持っているものを使う
#[tauri::command]
pub fn save_app_state(
    layout: Layout,
    tabs: Vec<String>,
    active_tab: Option<String>,
    app: AppHandle,
    workspace: State<'_, Workspace>,
) -> Result<(), String> {
    ensure_primary(&app)?;
    let root = workspace.current()?.map(|(root, _)| root);
    let state = SavedState {
        version: storage::VERSION,
        layout,
        workspace: root,
        tabs,
        active_tab,
    };
    storage::save_state(&app_dir(&app)?, &state)
}

// 閉じる・終える前の保存が終わったら呼ぶ。アプリを終える（D-084）
#[tauri::command]
pub fn finish_close(app: AppHandle) {
    app.exit(0);
}

// 閉じる・終える前の保存に失敗したら呼ぶ。閉じるのをやめる（D-084）
#[tauri::command]
pub fn cancel_close(closing: State<'_, crate::Closing>) {
    closing.0.store(false, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_to_run_code_in_an_untrusted_workspace() {
        let workspace = Workspace::default();
        assert!(workspace.root().is_err());
        assert!(workspace.trusted_root().is_err());
        // 信頼していなくても、ファイルの読み書きに使うルートは返す
        workspace.set(PathBuf::from("/ws"), false).unwrap();
        assert_eq!(workspace.root(), Ok(PathBuf::from("/ws")));
        assert_eq!(workspace.trusted_root(), Err(UNTRUSTED.to_string()));
        // 別のフォルダを開き直すと、信頼の状態も入れ替わる
        workspace.set(PathBuf::from("/other"), true).unwrap();
        assert_eq!(workspace.trusted_root(), Ok(PathBuf::from("/other")));
        workspace.set(PathBuf::from("/ws"), false).unwrap();
        assert_eq!(workspace.trusted_root(), Err(UNTRUSTED.to_string()));
    }
}
