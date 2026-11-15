// 言語サーバーに didClose を送ってよいか（src/editor/lspSync.ts。実行ログの直し ⑥）のテスト。
// 動かし方は logLinks.test.ts と同じ（node --test）
import { test } from "node:test";
import assert from "node:assert/strict";
import { ServerFiles } from "../src/editor/lspSync.ts";

// 出来事の並び。workspace.ts の connected・disconnected・openFile・closeFile にあたる
type Step = ["connected", string[]] | ["disconnected"] | ["open", string] | ["close", string];

// 並びのとおりに動かし、didClose を送ったファイルを順に返す
function closesSent(steps: Step[]): string[] {
  const files = new ServerFiles();
  const sent: string[] = [];
  for (const step of steps) {
    if (step[0] === "connected") {
      files.connected(step[1]);
    } else if (step[0] === "disconnected") {
      files.disconnected();
    } else if (step[0] === "open") {
      files.markOpened(step[1]);
    } else if (files.takeForClose(step[1])) {
      sent.push(step[1]);
    }
  }
  return sent;
}

test("今つながっているサーバーに開いたファイルを閉じるときだけ、didClose を送る", () => {
  const cases: { name: string; steps: Step[]; sent: string[] }[] = [
    {
      name: "つながっているときに開いて閉じる",
      steps: [["connected", []], ["open", "a"], ["close", "a"]],
      sent: ["a"],
    },
    {
      name: "⑥ 再起動：切ったあとで拡張を外した closeFile では送らず、次のサーバーで開き直したものは送る",
      steps: [["connected", []], ["open", "a"], ["disconnected"], ["close", "a"], ["connected", []], ["open", "a"], ["close", "a"]],
      sent: ["a"],
    },
    {
      name: "サーバーが自分で終わって切れたあとに、タブを替えて閉じても送らない",
      steps: [["connected", []], ["open", "a"], ["disconnected"], ["close", "a"]],
      sent: [],
    },
    {
      name: "つないだときに開いていたファイル（connected で didOpen したもの）は送る",
      steps: [["connected", ["a"]], ["close", "a"]],
      sent: ["a"],
    },
    {
      name: "今のサーバーに開いていないファイルと、2 回目の close には送らない",
      steps: [["connected", []], ["close", "b"], ["open", "a"], ["close", "a"], ["close", "a"]],
      sent: ["a"],
    },
    {
      name: "切れているあいだに開いたものは、次につないだときの connected で数える",
      steps: [["disconnected"], ["open", "a"], ["connected", ["a"]], ["close", "a"]],
      sent: ["a"],
    },
  ];
  // 合わないものを全部集めてから比べる
  const wrong = cases
    .map(({ name, steps, sent }) => ({ name, sent, actual: closesSent(steps) }))
    .filter(({ sent, actual }) => JSON.stringify(sent) !== JSON.stringify(actual))
    .map(({ name, sent, actual }) => `${name}：期待 ${JSON.stringify(sent)}、実際 ${JSON.stringify(actual)}`);
  assert.deepEqual(wrong, [], `合わない：\n${wrong.join("\n")}`);
});
