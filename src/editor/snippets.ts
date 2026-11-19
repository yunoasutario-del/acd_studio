// スニペットの補完（Phase 13 作業 1。D-186）。.rs・C++・.py のタブに、その言語のスニペットだけを付ける
// （createEditorState の snippets。言語ごとは Phase 17 作業 9。D-213。Python は Phase 20 作業 8。D-227）。
// 中身は App が言語のファイルを読むたびに setSnippets で入れ替える（タブの状態は作り直さない）。
// rust-analyzer の候補と並んで出る。確定で展開し、Tab・⇧Tab で欄を移る（@codemirror/autocomplete の snippet）
import { snippet, type Completion, type CompletionContext, type CompletionResult } from "@codemirror/autocomplete";
import { EditorState } from "@codemirror/state";
import type { Snippet } from "../ipc/snippets";
import type { Language } from "../ipc/workspace";

// 呼び出す文字として打っている言葉（英数字・日本語などの文字・_・!）
const WORD = /[\p{L}\p{N}_!]+$/u;
const VALID = /^[\p{L}\p{N}_!]*$/u;

const options: Record<Language, Completion[]> = { rust: [], cpp: [], python: [] };

// 候補の説明に出す本文（欄の書き方を外し、既定の文字だけにする）
function preview(template: string): string {
  return template.replace(/[$#]\{(?:\d+:?)?([^{}]*)\}/g, "$1").replace(/\\([{}])/g, "$1");
}

export function setSnippets(language: Language, snippets: Snippet[]) {
  options[language] = snippets.flatMap((item) =>
    item.prefixes.map((prefix) => ({
      label: prefix,
      detail: `${item.description ?? item.name}（スニペット）`,
      info: preview(item.template),
      type: "snippet",
      apply: snippet(item.template),
    })),
  );
}

function snippetSource(language: Language) {
  return (context: CompletionContext): CompletionResult | null => {
    const list = options[language];
    if (list.length === 0) {
      return null;
    }
    const word = context.matchBefore(WORD);
    if (!word && !context.explicit) {
      return null;
    }
    return { from: word ? word.from : context.pos, options: list, validFor: VALID };
  };
}

// 出どころは言語ごとに 1 度だけ作り、毎回同じものを返す。@codemirror/autocomplete は、取引のたびに読み直した
// 出どころを前のものと同じ関数かどうかで突き合わせるため、読むたびに作り直すと結果が捨てられて候補に出ない
function languageDataFor(language: Language) {
  const data = [{ autocomplete: snippetSource(language) }];
  return EditorState.languageData.of(() => data);
}

const completions: Record<Language, ReturnType<typeof EditorState.languageData.of>> = {
  rust: languageDataFor("rust"),
  cpp: languageDataFor("cpp"),
  python: languageDataFor("python"),
};

export function snippetCompletions(language: Language) {
  return completions[language];
}
