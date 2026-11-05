// 設定の画面の Python の欄の、処理系の探し方の説明（src/settings/pythonNote.ts）のテスト（Windows 移植の段階 2 S13）。
// 動かし方は logLinks.test.ts と同じ（node --test）
import { test } from "node:test";
import assert from "node:assert/strict";
import { pythonSearchNote } from "../src/settings/pythonNote.ts";

test("macOS：今までの文のまま", () => {
  assert.equal(
    pythonSearchNote(false),
    "処理系は /opt/homebrew/bin・PATH・/usr/local/bin から探します（決まったパスを使うときは、settings.json の python.cpython・python.pypy に書きます）。",
  );
});

test("Windows：S7 の探し方（レジストリ → PATH）を書き、macOS の場所は出さない", () => {
  const note = pythonSearchNote(true);
  for (const part of ["レジストリ（PEP 514）", "python.exe", "pypy3.11.exe・pypy3.exe", "python.cpython・python.pypy"]) {
    assert.ok(note.includes(part), `「${part}」がない：${note}`);
  }
  for (const part of ["/opt/homebrew", "/usr/local", "Homebrew"]) {
    assert.ok(!note.includes(part), `「${part}」がある：${note}`);
  }
});
