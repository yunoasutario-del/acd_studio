// アプリのメニュー（作業 7b。D-021・D-033 を置き換える D-083）。
// Tauri 2.12.4 の既定のメニュー（Menu::default）をもとに、次を変えている：
// - 項目の文字をすべて日本語にし、中身のない Help を外す（Phase 10 作業 16。D-174）
// - 「Close Window」（File・Window）を外す。⌘W をアプリのキー割り当ての「タブを閉じる」に使うため
// - File に「フォルダを開く…」「タブを閉じる」「rust-analyzer を再起動」を加える（再起動は Phase 3b-2 作業 3。D-130。Phase 17 作業 5 で「言語サーバーを再起動」。D-210）。
//   ショートカットはメニューに付けず、キー割り当ての表（settings.json。D-052）で扱う
// - Quit を独自の項目にする。終える前に未保存のタブを保存するため（D-084）
// - アプリの項目に「設定…」を加える（Phase 8 作業 6。D-159。ショートカットはキー割り当ての表の ⌘,）

// AboutMetadata は macOS のメニューだけで使う
#[cfg_attr(not(target_os = "macos"), allow(unused_imports))]
use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Runtime};

// メニューの項目の ID。フロントエンドに "menu-command" のイベントで渡す
pub const OPEN_FOLDER: &str = "openFolder";
pub const CLOSE_TAB: &str = "closeTab";
pub const RESTART_RUST_ANALYZER: &str = "restartRustAnalyzer";
pub const OPEN_SETTINGS: &str = "openSettings";
pub const QUIT: &str = "quit";

// Windows（macOS 以外）のメニュー（Windows 移植の段階 1 C8。docs/windows-port-plan.md 9.1 の 16）：最小限の変更として、
// アプリ名のメニューをやめ、「ファイル」に設定・終了をまとめる。メニューは窓の中に出る。
// services・hide_others・fullscreen は、muda の文書で Windows では使えないとされているので付けない。
// 編集の項目が Windows で使えるかは、muda の文書に使えないという記述がないことからの推測（動かして確かめる）
#[cfg(not(target_os = "macos"))]
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    Menu::with_items(
        app,
        &[
            &Submenu::with_items(
                app,
                "ファイル",
                true,
                &[
                    &MenuItem::with_id(app, OPEN_FOLDER, "フォルダを開く…", true, None::<&str>)?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItem::with_id(app, CLOSE_TAB, "タブを閉じる", true, None::<&str>)?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItem::with_id(
                        app,
                        RESTART_RUST_ANALYZER,
                        "言語サーバーを再起動",
                        true,
                        None::<&str>,
                    )?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItem::with_id(app, OPEN_SETTINGS, "設定…", true, None::<&str>)?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItem::with_id(app, QUIT, "終了", true, Some("CmdOrCtrl+Q"))?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "編集",
                true,
                &[
                    &PredefinedMenuItem::undo(app, Some("取り消す"))?,
                    &PredefinedMenuItem::redo(app, Some("やり直す"))?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::cut(app, Some("カット"))?,
                    &PredefinedMenuItem::copy(app, Some("コピー"))?,
                    &PredefinedMenuItem::paste(app, Some("ペースト"))?,
                    &PredefinedMenuItem::select_all(app, Some("すべてを選択"))?,
                ],
            )?,
        ],
    )
}

#[cfg(target_os = "macos")]
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let pkg_info = app.package_info();
    let config = app.config();
    let about_metadata = AboutMetadata {
        name: Some(pkg_info.name.clone()),
        version: Some(pkg_info.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config.bundle.publisher.clone().map(|p| vec![p]),
        ..Default::default()
    };

    Menu::with_items(
        app,
        &[
            &Submenu::with_items(
                app,
                pkg_info.name.clone(),
                true,
                &[
                    &PredefinedMenuItem::about(
                        app,
                        Some(&format!("{} について", pkg_info.name)),
                        Some(about_metadata),
                    )?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItem::with_id(app, OPEN_SETTINGS, "設定…", true, None::<&str>)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::services(app, Some("サービス"))?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::hide(app, Some(&format!("{} を隠す", pkg_info.name)))?,
                    &PredefinedMenuItem::hide_others(app, Some("ほかを隠す"))?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItem::with_id(
                        app,
                        QUIT,
                        format!("{} を終了", pkg_info.name),
                        true,
                        Some("CmdOrCtrl+Q"),
                    )?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "ファイル",
                true,
                &[
                    &MenuItem::with_id(app, OPEN_FOLDER, "フォルダを開く…", true, None::<&str>)?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItem::with_id(app, CLOSE_TAB, "タブを閉じる", true, None::<&str>)?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItem::with_id(
                        app,
                        RESTART_RUST_ANALYZER,
                        "言語サーバーを再起動",
                        true,
                        None::<&str>,
                    )?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "編集",
                true,
                &[
                    &PredefinedMenuItem::undo(app, Some("取り消す"))?,
                    &PredefinedMenuItem::redo(app, Some("やり直す"))?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::cut(app, Some("カット"))?,
                    &PredefinedMenuItem::copy(app, Some("コピー"))?,
                    &PredefinedMenuItem::paste(app, Some("ペースト"))?,
                    &PredefinedMenuItem::select_all(app, Some("すべてを選択"))?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "表示",
                true,
                &[&PredefinedMenuItem::fullscreen(
                    app,
                    Some("フルスクリーンにする"),
                )?],
            )?,
            &Submenu::with_items(
                app,
                "ウインドウ",
                true,
                &[
                    &PredefinedMenuItem::minimize(app, Some("しまう"))?,
                    &PredefinedMenuItem::maximize(app, Some("拡大/縮小"))?,
                ],
            )?,
        ],
    )
}
