// スニペットの補完（src/editor/snippets.ts。D-186・D-213・D-227）のテスト。
// 動かし方は logLinks.test.ts と同じ（node --test）。スニペットの名前や本文は偽のもの
import { test } from "node:test";
import assert from "node:assert/strict";
import { EditorState } from "@codemirror/state";
import { CompletionContext, type CompletionSource } from "@codemirror/autocomplete";
import { setSnippets, snippetCompletions } from "../src/editor/snippets.ts";
import type { Language } from "../src/ipc/workspace.ts";

const languages: Language[] = ["rust", "cpp", "python"];

// @codemirror/autocomplete は、取引のたびに languageData から出どころを読み直し、前の出どころと
// 同じ関数かどうかで問い合わせの結果を結び付ける。毎回違う関数になると、結果が捨てられて候補に出ない
for (const language of languages) {
  test(`${language}: 補完の出どころは、読み直しても、文字を打ったあとも同じ関数`, () => {
    const state = EditorState.create({ doc: "tem", extensions: [snippetCompletions(language)] });
    const first = state.languageDataAt<CompletionSource>("autocomplete", 3);
    const again = state.languageDataAt<CompletionSource>("autocomplete", 3);
    const typed = state.update({ changes: { from: 3, insert: "p" } }).state;
    const after = typed.languageDataAt<CompletionSource>("autocomplete", 4);
    assert.equal(first.length, 1);
    assert.equal(again[0], first[0], "同じ状態で 2 回読んだ出どころが、違う関数になっている");
    assert.equal(after[0], first[0], "1 文字打ったあとの出どころが、違う関数になっている");
  });
}

// 出どころそのものは、打った言葉に合うスニペットを返す
for (const language of languages) {
  test(`${language}: temp と打つと、スニペットの temp が候補に入る`, () => {
    setSnippets(language, [
      { name: "偽のスニペット", prefixes: ["temp"], template: "template<${1}>", description: null },
    ]);
    const state = EditorState.create({ doc: "temp", extensions: [snippetCompletions(language)] });
    const [source] = state.languageDataAt<CompletionSource>("autocomplete", 4);
    const result = source(new CompletionContext(state, 4, false));
    assert.ok(result !== null && !(result instanceof Promise), "候補がすぐに返っていない");
    assert.equal(result.from, 0);
    assert.ok(
      result.options.some((option) => option.label === "temp" && option.type === "snippet"),
      "スニペットの temp が候補にない",
    );
  });
}
