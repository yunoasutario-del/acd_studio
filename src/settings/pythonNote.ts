// 設定の画面の Python の欄の、処理系の探し方の説明（Phase 19 作業 3。D-221）。
// SettingsDialog.tsx の PythonSection から移した（Windows 移植の段階 2 S13。tests/pythonNote.test.ts で確かめるため）。
// windows は Windows で動いているか（画面からは panels/logLinks.ts の IS_WINDOWS を渡す）
const UNIX_NOTE =
  "処理系は /opt/homebrew/bin・PATH・/usr/local/bin から探します（決まったパスを使うときは、settings.json の python.cpython・python.pypy に書きます）。";

// Windows（段階 2 S13）：探し方は S7 のもの（src-tauri/src/python.rs の find_on_windows）。
// CPython はレジストリ（PEP 514）の 3.13 → PATH の python.exe（Microsoft Store の実行エイリアスは除く）、PyPy は PATH
const WINDOWS_NOTE =
  "CPython は、レジストリ（PEP 514）の 3.13、PATH の python.exe の順に探します（Microsoft Store の実行エイリアスは使いません）。PyPy は、PATH の pypy3.11.exe・pypy3.exe から探します（決まったパスを使うときは、settings.json の python.cpython・python.pypy に書きます）。";

export function pythonSearchNote(windows: boolean): string {
  return windows ? WINDOWS_NOTE : UNIX_NOTE;
}
