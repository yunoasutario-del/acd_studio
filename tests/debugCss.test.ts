// デバッグの変数の欄の CSS（src/App.css）の形を見るテスト（windows-debug-ui-fixes の件 8）。
// 見た目そのものは node では確かめられないので、規則の形を見る（見た目は画面で確かめる）。
// 動かし方は logLinks.test.ts と同じ（node --test。依存は足さない）
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const css = readFileSync(new URL("../src/App.css", import.meta.url), "utf8");

// 行がちょうど「<selector> {」のところから、次の「}」までを、名前と値の組にする。見つからなければ null
function rule(selector: string): Map<string, string> | null {
  const lines = css.split(/\r?\n/);
  const start = lines.findIndex((line) => line.trimEnd() === `${selector} {`);
  if (start < 0) {
    return null;
  }
  const declarations = new Map<string, string>();
  for (const line of lines.slice(start + 1)) {
    if (line.trim() === "}") {
      return declarations;
    }
    const match = /^\s*([a-z-]+)\s*:\s*(.+?)\s*;\s*$/.exec(line);
    if (match) {
      declarations.set(match[1], match[2]);
    }
  }
  // 閉じる「}」がない
  return null;
}

// 縮む比率（flex-shrink）。読むのは、flex-shrink だけ・flex: none・flex: <伸び> <縮み> <基準> の 3 つの値・どちらも書いていない
// （CSS の初めの値の 1）の形だけ。ほかの書き方（flex: 1・flex: auto など）や、両方を書いた形は、推し量らずに読めない理由を返す
function shrinkOf(declarations: Map<string, string>): number | string {
  const shrink = declarations.get("flex-shrink");
  const flex = declarations.get("flex");
  if (shrink !== undefined && flex !== undefined) {
    return "flex と flex-shrink の両方がある";
  }
  if (shrink !== undefined) {
    return /^\d+(\.\d+)?$/.test(shrink) ? Number(shrink) : `flex-shrink: ${shrink} は読まない形`;
  }
  if (flex === undefined) {
    return 1;
  }
  if (flex === "none") {
    return 0;
  }
  const parts = flex.split(/\s+/);
  if (parts.length === 3 && /^\d+(\.\d+)?$/.test(parts[1])) {
    return Number(parts[1]);
  }
  return `flex: ${flex} は読まない形`;
}

test("件 8：縮む比率の読み方は、決めた形だけを読み、ほかの書き方は読めない理由を返す", () => {
  const cases: [[string, string][], number | string][] = [
    [[["flex", "none"]], 0],
    [[["flex", "0 100 auto"]], 100],
    [[["flex", "0 1 auto"]], 1],
    [[["flex-shrink", "5"]], 5],
    [[], 1],
    [[["flex", "1"]], "flex: 1 は読まない形"],
    [[["flex", "auto"]], "flex: auto は読まない形"],
    [[["flex", "0 1"]], "flex: 0 1 は読まない形"],
    [[["flex-shrink", "1"], ["flex", "none"]], "flex と flex-shrink の両方がある"],
  ];
  const wrong: string[] = [];
  for (const [declarations, want] of cases) {
    const got = shrinkOf(new Map(declarations));
    if (got !== want) {
      wrong.push(`${JSON.stringify(declarations)} → ${got}`);
    }
  }
  assert.deepEqual(wrong, [], `合わない：\n${wrong.join("\n")}`);
});

test("件 8：変数の欄では、型の名前の欄が値の欄より先に縮み、「…」で切れる（App.css）", () => {
  // 合わないものを全部集めてから比べる
  const wrong: string[] = [];
  const type = rule(".debug-variable-type");
  const value = rule(".debug-variable-value");
  if (type === null) {
    wrong.push(".debug-variable-type の規則がない");
  }
  if (value === null) {
    wrong.push(".debug-variable-value の規則がない");
  }
  if (type !== null && value !== null) {
    const typeShrink = shrinkOf(type);
    const valueShrink = shrinkOf(value);
    if (typeof typeShrink === "string") {
      wrong.push(`.debug-variable-type：${typeShrink}`);
    }
    if (typeof valueShrink === "string") {
      wrong.push(`.debug-variable-value：${valueShrink}`);
    }
    if (typeof typeShrink === "number" && typeof valueShrink === "number" && !(typeShrink > valueShrink)) {
      wrong.push(`型の欄の縮む比率 ${typeShrink} が、値の欄の ${valueShrink} より大きくない`);
    }
  }
  // 型の欄が縮めて「…」で切れること。値の欄の同じ形は、今と同じく残っていること
  const needed: [string, string][] = [
    ["min-width", "0"],
    ["overflow", "hidden"],
    ["text-overflow", "ellipsis"],
  ];
  const rules: [string, Map<string, string> | null][] = [
    [".debug-variable-type", type],
    [".debug-variable-value", value],
  ];
  for (const [selector, declarations] of rules) {
    if (declarations === null) {
      continue;
    }
    for (const [name, want] of needed) {
      const got = declarations.get(name);
      if (got !== want) {
        wrong.push(`${selector} の ${name} が ${got ?? "ない"}（${want} のはず）`);
      }
    }
  }
  assert.deepEqual(wrong, [], `合わない：\n${wrong.join("\n")}`);
});
