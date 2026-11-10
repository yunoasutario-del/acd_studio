// 「提出結果」の「ログアウト」のボタンの説明（Windows 移植の段階 3 T8）。
// SubmissionPanel.tsx から移した（tests/sessionNote.test.ts で確かめるため）。
// windows は Windows で動いているか（画面からは panels/logLinks.ts の IS_WINDOWS を渡す）
const UNIX_LOGOUT_TITLE = "Keychain のセッションを消します（AtCoder には何も送りません）";

// Windows（段階 3 T5）：セッションは資格情報マネージャーに置く（src-tauri/src/session.rs）
const WINDOWS_LOGOUT_TITLE = "資格情報マネージャーのセッションを消します（AtCoder には何も送りません）";

export function logoutTitle(windows: boolean): string {
  return windows ? WINDOWS_LOGOUT_TITLE : UNIX_LOGOUT_TITLE;
}
