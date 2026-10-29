mod commands;
mod compare;
mod cpp;
mod debug;
mod history;
mod input_cpp;
mod input_gen;
mod input_py;
mod interactive;
mod language;
mod lsp;
mod menu;
mod problem;
mod process;
mod python;
mod random;
mod run;
mod sanitize;
mod scaffold;
mod search;
mod session;
mod snippets;
mod sources;
mod storage;
mod submit;
mod toolchain;
mod trust;
#[cfg(windows)]
mod win;
mod workspace;

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{Emitter, Manager, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

// 単一起動のロック（D-040）。プロセスが終わるまで持ち続ける。
// これを持たないプロセス（2 つ目の起動）は、IDE 専用のファイルを読み書きしない（commands::ensure_primary）
pub(crate) struct InstanceLock(#[allow(dead_code)] std::fs::File);

// 閉じる・終える前の保存の途中なら true（D-084）。保存の途中にもう一度閉じようとしたら、そのまま閉じる
#[derive(Default)]
pub struct Closing(pub AtomicBool);

// 閉じる・終える前に、フロントエンドに保存を頼む。フロントエンドは保存し終えたら finish_close を呼ぶ
fn request_close<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    let closing = app.state::<Closing>();
    if closing.0.swap(true, Ordering::SeqCst) {
        // すでに保存の途中（フロントエンドが応えないときなど）。そのまま閉じる
        return false;
    }
    let _ = app.emit("close-requested", ());
    true
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // フォルダ選択の画面を Rust 側から開くために使う。WebView には dialog の権限を与えない（capability に入れない）
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::Workspace::default())
        .manage(Closing::default())
        .manage(run::RunControl::default())
        .manage(lsp::LspControl::default())
        .manage(debug::DebugControl::default())
        .manage(session::SessionState::default())
        .manage(submit::SubmitState::default())
        .manage(scaffold::ScaffoldControl::default())
        .manage(commands::SettingsStamp::default())
        .manage(search::SearchControl::default())
        .manage(commands::SnippetsStamp::default())
        // メニュー（D-083）
        .menu(menu::build)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if id == menu::QUIT {
                if !request_close(app) {
                    app.exit(0);
                }
            } else if id == menu::OPEN_FOLDER
                || id == menu::CLOSE_TAB
                || id == menu::RESTART_RUST_ANALYZER
                || id == menu::OPEN_SETTINGS
            {
                let _ = app.emit("menu-command", id.to_string());
            }
        })
        // ウィンドウを閉じる前に保存する（D-084）。main の窓だけ（ログインの窓を閉じても IDE を終えない。D-148）
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
                && request_close(window.app_handle())
            {
                api.prevent_close();
            }
        })
        // 2 つ目の起動なら知らせて終わる（D-040）
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            match storage::acquire_instance_lock(&dir) {
                Ok(Some(lock)) => {
                    app.manage(InstanceLock(lock));
                }
                Ok(None) => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.hide();
                    }
                    let handle = app.handle().clone();
                    app.dialog()
                        .message("AtCoder Studio はすでに起動しています。")
                        .title("AtCoder Studio")
                        .kind(MessageDialogKind::Info)
                        .show(move |_| handle.exit(0));
                }
                Err(error) => return Err(error.into()),
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_workspace,
            commands::list_directory,
            commands::read_file,
            commands::write_file,
            commands::create_new_file,
            commands::rename_existing_file,
            commands::create_new_folder,
            commands::delete_file,
            commands::save_expected,
            commands::file_stamps,
            commands::ask_external_change,
            commands::load_app_state,
            commands::save_app_state,
            commands::ask_discard_changes,
            commands::trust_workspace,
            commands::search_workspace,
            commands::replace_in_workspace,
            commands::undo_replace,
            commands::reload_snippets,
            commands::open_snippets_file,
            commands::import_vscode_snippets,
            commands::template_files,
            commands::open_templates_folder,
            commands::make_input,
            commands::fill_cpp_input,
            commands::fill_python_input,
            commands::reload_settings,
            commands::save_settings,
            commands::open_settings_file,
            commands::finish_close,
            commands::cancel_close,
            commands::resolve_target,
            commands::start_run,
            commands::start_random,
            commands::start_interactive,
            commands::stop_run,
            commands::start_debug,
            commands::debug_step,
            commands::debug_set_breakpoints,
            commands::debug_stack,
            commands::debug_scopes,
            commands::debug_variables,
            commands::debug_evaluate,
            commands::load_history,
            commands::clear_history,
            commands::load_problem,
            commands::load_task_list,
            commands::open_problem_page,
            commands::atcoder_login,
            commands::atcoder_session,
            commands::atcoder_logout,
            commands::atcoder_submit,
            commands::atcoder_submissions,
            commands::contest_status,
            commands::atcoder_refresh_submission,
            commands::open_submit_page,
            commands::open_submission_page,
            commands::create_problem_package,
            commands::create_contest_packages,
            commands::stop_scaffold,
            commands::problem_choice,
            commands::package_problem,
            commands::set_problem_choice,
            commands::save_samples,
            commands::lsp_start,
            commands::lsp_send,
            commands::lsp_locate,
            commands::locate_source,
            commands::read_source,
            commands::lsp_stop
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // アプリが終わるときは、実行中のビルド・プログラム・デバッグを止めてから終わる。
            // 利用者のプロセスは別のプロセスグループなので、IDE が終わっても残るため（D-095、AC2-12）。
            // デバッグも同じ止める印で止まる（D-146 2.10）
            if let tauri::RunEvent::Exit = event {
                app.state::<run::RunControl>().stop_and_wait();
                // rust-analyzer も止める（D-124）
                app.state::<lsp::LspControl>().stop();
            }
        });
}
