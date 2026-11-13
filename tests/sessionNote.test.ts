// 「提出結果」の「ログアウト」のボタンの説明（src/panels/sessionNote.ts）のテスト（Windows 移植の段階 3 T8）。
// 動かし方は logLinks.test.ts と同じ（node --test）
import { test } from "node:test";
import assert from "node:assert/strict";
import { logoutTitle } from "../src/panels/sessionNote.ts";

test("macOS：今までの文のまま", () => {
  assert.equal(logoutTitle(false), "Keychain のセッションを消します（AtCoder には何も送りません）");
});

test("Windows：資格情報マネージャーと書き、Keychain は出さない", () => {
  const title = logoutTitle(true);
  assert.equal(title, "資格情報マネージャーのセッションを消します（AtCoder には何も送りません）");
  assert.ok(!title.includes("Keychain"), title);
});
