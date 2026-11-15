// 言語サーバーが終わったときの実行ログの行と状態（src/editor/lspExit.ts。実行ログの直し ①②）のテスト。
// 動かし方は logLinks.test.ts と同じ（node --test）
import { test } from "node:test";
import assert from "node:assert/strict";
import { exitReport } from "../src/editor/lspExit.ts";

// 終了コード：0（正式な終了）、-525598719（Windows で Job を止めたとき。KILLED_EXIT_CODE）、null（macOS でシグナルで止めたとき）
const CODES = [0, -525598719, null];

test("① IDE が止めたときは、読み込みの前でもあとでも、終了コードによらず「止めました」（状態は停止）", () => {
  // 合わないものを全部集めてから比べる
  const wrong: string[] = [];
  for (const ready of [false, true]) {
    for (const code of CODES) {
      const report = exitReport("rust-analyzer", { ready, endedByItself: false, code });
      if (report.line !== "[rust-analyzer] 止めました" || report.status !== "stopped") {
        wrong.push(`ready=${ready} code=${code} → ${report.line}（${report.status}）`);
      }
    }
  }
  assert.deepEqual(wrong, [], `合わない：\n${wrong.join("\n")}`);
});

test("自分で終わった・読み込みのあと：「止まりました（終了コード）」（状態は停止。今と同じ）", () => {
  assert.deepEqual(exitReport("clangd", { ready: true, endedByItself: true, code: 0 }), {
    line: "[clangd] 止まりました（終了コード 0）",
    status: "stopped",
  });
  assert.deepEqual(exitReport("clangd", { ready: true, endedByItself: true, code: null }), {
    line: "[clangd] 止まりました（終了コード なし）",
    status: "stopped",
  });
});

test("自分で終わった・読み込みの前：「起動に失敗しました」（状態は失敗。今と同じ）", () => {
  assert.deepEqual(exitReport("basedpyright", { ready: false, endedByItself: true, code: 3 }), {
    line: "[basedpyright] 起動に失敗しました。読み込みが終わる前に止まりました（終了コード 3）",
    status: "failed",
  });
});

test("② 行頭の名前は、引数で渡したサーバーの名前（終わった接続のもの）", () => {
  for (const server of ["rust-analyzer", "clangd", "basedpyright"]) {
    for (const endedByItself of [false, true]) {
      const { line } = exitReport(server, { ready: true, endedByItself, code: 0 });
      assert.ok(line.startsWith(`[${server}] `), `名前が違う：${line}`);
    }
  }
});
