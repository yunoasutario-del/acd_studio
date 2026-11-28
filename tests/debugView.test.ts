// デバッグの画面の小さな純粋な関数（src/panels/debugView.ts）のテスト（windows-debug-ui-fixes の件 1・件 5）。
// 動かし方は logLinks.test.ts と同じ（node --test）。パスは偽のもの
import { test } from "node:test";
import assert from "node:assert/strict";
import { exitText, failureLog, failureStatus, fileName } from "../src/panels/debugView.ts";

test("件 1：panic・例外・abort で止まったときの文に「でで」がない（状態の文と、実行ログの文）", () => {
  const cases: [string, string, string][] = [
    ["panic", "panic で止まっています", "panic で止まりました。"],
    ["throw", "例外で止まっています", "例外で止まりました。"],
    ["abort", "abort で止まっています", "abort で止まりました。"],
    ["exception", "例外で止まっています", "例外で止まりました。"],
  ];
  // 合わないものを全部集めてから比べる
  const wrong: string[] = [];
  for (const [reason, status, log] of cases) {
    const gotStatus = failureStatus(reason);
    const gotLog = failureLog(reason);
    if (gotStatus !== status || gotLog !== log) {
      wrong.push(`${reason} → ${gotStatus} / ${gotLog}`);
    }
  }
  assert.deepEqual(wrong, [], `合わない：\n${wrong.join("\n")}`);
});

test("件 1：ほかの理由（breakpoint・step・pause・理由なし）では文を作らない（今と同じ）", () => {
  for (const reason of ["breakpoint", "step", "pause", null]) {
    assert.equal(failureStatus(reason), null, String(reason));
    assert.equal(failureLog(reason), null, String(reason));
  }
});

test("件 5：コールスタックとブレークポイントの一覧のファイルの名前は、「/」と「\\」のどちらで区切られていても名前だけ", () => {
  const cases: [string, string][] = [
    // Windows の形（今は全部が出る）
    ["C:\\w\\a\\main.cpp", "main.cpp"],
    ["C:\\w\\.rustup\\lib\\rustlib\\src\\rust\\library\\core\\src\\fmt\\rt.rs", "rt.rs"],
    // 「/」と「\」が混ざった形
    ["C:\\w/lib\\stl_vector.h", "stl_vector.h"],
    // Unix の形・作業領域からの相対の形・名前だけ（今と同じ）
    ["/w/a/main.rs", "main.rs"],
    ["t1/t1_a/src/main.rs", "main.rs"],
    ["main.py", "main.py"],
  ];
  const wrong: string[] = [];
  for (const [path, name] of cases) {
    const got = fileName(path);
    if (got !== name) {
      wrong.push(`${path} → ${got}`);
    }
  }
  assert.deepEqual(wrong, [], `合わない：\n${wrong.join("\n")}`);
});

test("件 6：終了コードの文は、Rust が作った名前（label）があればそれ、なければ「終了コード N」（src/ipc/run.ts の exitLabel と同じ決まり）", () => {
  const cases: [number | null, string | null | undefined, string | null][] = [
    // Windows の負の値（Rust の run::exit_code_text が 16 進と名前にする）
    [-1073740791, "終了コード 0xC0000409（異常終了：…）", "終了コード 0xC0000409（異常終了：…）"],
    [0, "終了コード 0", "終了コード 0"],
    // label のない知らせ（前の形）と、空の label は、code から作る（exitLabel と同じく || で戻す）
    [3, undefined, "終了コード 3"],
    [3, "", "終了コード 3"],
    // 終了コードが届かなかった
    [null, null, null],
  ];
  const wrong: string[] = [];
  for (const [code, label, text] of cases) {
    const got = exitText(code, label);
    if (got !== text) {
      wrong.push(`${code} / ${label} → ${got}`);
    }
  }
  assert.deepEqual(wrong, [], `合わない：\n${wrong.join("\n")}`);
});
