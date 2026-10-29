// 設定の画面（Phase 8 作業 6b。D-159）。キー・色・表示（入切と文字の大きさ）・提出（Phase 9 作業 2。D-162）・
// Python の処理系（Phase 19 作業 3。D-221）を変え、
// 「保存」で settings.json に書く。「スニペット」（Phase 13 作業 2。D-187）・「テンプレート」（Phase 13 作業 3。D-188）は
// 一覧とボタンだけで、保存とは関係しない。
//
// - 色・表示は、変えるとすぐに画面に出す（試し見）。「キャンセル」・Esc で、開く前の見た目に戻す
// - キーは「変更」を押してから押したキーを割り当てる。ぶつかるキー・修飾キーのないキー・決まったキーは断る
// - 画面を開いている間に settings.json が外で変わったら（App が読み直したら resetSeq が進む）、中身を読み直して知らせる
import { useEffect, useMemo, useState } from "react";
import { applyAppearance, COLOR_GROUPS, defaultColor, joinColor, splitColor } from "../appearance";
import {
  INTERPRETERS,
  LANGUAGES,
  PROBLEM_BUTTONS,
  type Appearance,
  type Interpreter,
  type LoadedKeyBinding,
  type ProblemSettings,
  type PythonSettings,
  type SubmitSettings,
} from "../ipc/state";
import { snippetsFile, type LoadedSnippets } from "../ipc/snippets";
import type { Language } from "../ipc/workspace";
import { openTemplatesFolder, templateFiles, type TemplateFiles } from "../ipc/scaffold";
import { saveSettings } from "../ipc/state";
import {
  bindingFromEvent,
  COMMAND_LABELS,
  COMMANDS,
  DEFAULT_KEY_BINDINGS,
  getKeyBindings,
  IS_MAC,
  keyLabel,
  setKeyBindings,
  type Command,
  type KeyBinding,
} from "../keybindings";
import { remeasureEditor } from "../editor/Editor";

// 言語ごとの、今読めているスニペットと、ファイルを読めなかったときの理由（D-187・D-213）
export type SnippetsViews = Record<Language, { loaded: LoadedSnippets | null; error: string | null }>;

type Keys = Omit<KeyBinding, "command">;
// コマンドごとの割り当て。null は「なし」
type Assigned = Record<Command, Keys | null>;

type Section = "keys" | "colors" | "view" | "problem" | "submit" | "python" | "snippets" | "templates";

const MIN_FONT_SIZE = 8;
const MAX_FONT_SIZE = 32;

// macOS とメニューの決まったキー（割り当てると、そちらが先に動く・使えなくなる）。押しても届かないものもある。
// Windows（macOS 以外）は、Windows と編集・終了のメニューの決まったキー（Windows 移植の段階 1 C9）
const RESERVED = IS_MAC
  ? new Set(["⌘Q", "⌘H", "⌥⌘H", "⌘M", "⌘C", "⌘V", "⌘X", "⌘Z", "⇧⌘Z", "⌘A", "⌃⌘F"])
  : new Set(["Ctrl+Q", "Ctrl+C", "Ctrl+V", "Ctrl+X", "Ctrl+Z", "Ctrl+Y", "Ctrl+Shift+Z", "Ctrl+A", "Alt+F4"]);
const OS_NAME = IS_MAC ? "macOS" : "Windows";
const MODIFIER_NAMES = IS_MAC ? "⌘・⌃・⌥" : "Ctrl・Alt";

// 今の割り当てを、コマンドごとの最初の 1 つにする（settings.json に同じコマンドが 2 つ以上あれば、先のもの）
function currentAssigned(): Assigned {
  const assigned = {} as Assigned;
  for (const command of COMMANDS) {
    const binding = getKeyBindings().find((item) => item.command === command);
    assigned[command] = binding && binding.key !== "" ? toKeys(binding) : null;
  }
  return assigned;
}

function toKeys(binding: KeyBinding): Keys {
  return { key: binding.key, meta: binding.meta, ctrl: binding.ctrl, alt: binding.alt, shift: binding.shift };
}

function defaultKeys(command: Command): Keys | null {
  const binding = DEFAULT_KEY_BINDINGS.find((item) => item.command === command);
  return binding ? toKeys(binding) : null;
}

function sameKeys(a: Keys | null, b: Keys | null): boolean {
  return a === null || b === null ? a === b : keyLabel(a) === keyLabel(b);
}

// 割り当てられない理由。割り当てられるなら null
function refusal(keys: Keys, command: Command, assigned: Assigned): string | null {
  const label = keyLabel(keys);
  if (RESERVED.has(label)) {
    return `${label} は ${OS_NAME} やメニューで使うキーなので、割り当てられません`;
  }
  const functionKey = /^f([1-9]|1[0-9]|20)$/.test(keys.key);
  if (!keys.meta && !keys.ctrl && !keys.alt && !functionKey) {
    return `${MODIFIER_NAMES} のどれかと一緒に押してください（F1〜F20 は、そのままでも使えます）`;
  }
  for (const other of COMMANDS) {
    if (other !== command && sameKeys(assigned[other], keys)) {
      return `${label} は「${COMMAND_LABELS[other]}」に割り当てられています`;
    }
  }
  return null;
}

type Props = {
  // 今使っている見た目（開いたとき・キャンセルしたときに戻すもの）
  appearance: Appearance;
  // 今使っている提出の設定（D-162）
  submit: SubmitSettings;
  // 今使っている問題文の欄の設定（D-196・D-197）
  problem: ProblemSettings;
  // 今の Python の設定（D-221）。画面で変えるのは処理系だけ
  python: PythonSettings;
  // App が settings.json を読み直したら進む。進んだら、中身を読み直す
  resetSeq: number;
  onClose: () => void;
  // 保存できた。App は見た目と提出の設定を覚える
  onSaved: (appearance: Appearance, submit: SubmitSettings, problem: ProblemSettings, interpreter: Interpreter) => void;
  // 外で変わっていて保存できなかった。App が読み直す（resetSeq が進む）
  onReload: () => Promise<void>;
  onOpenJson: () => void;
  // 言語ごとの、今読めているスニペットと、ファイルを読めなかったときの理由（D-187・D-213）
  snippets: SnippetsViews;
  onOpenSnippets: (language: Language) => Promise<void>;
  // 取り込んで読み直す。出す知らせを返す（キャンセルなら null）
  onImportSnippets: (language: Language) => Promise<string | null>;
};

function SettingsDialog({
  appearance,
  submit,
  problem,
  python,
  resetSeq,
  onClose,
  onSaved,
  onReload,
  onOpenJson,
  snippets,
  onOpenSnippets,
  onImportSnippets,
}: Props) {
  const [section, setSection] = useState<Section>("keys");
  const [assigned, setAssigned] = useState<Assigned>(currentAssigned);
  const [draft, setDraft] = useState<Appearance>(appearance);
  const [submitDraft, setSubmitDraft] = useState<SubmitSettings>(submit);
  const [problemDraft, setProblemDraft] = useState<ProblemSettings>(problem);
  const [interpreterDraft, setInterpreterDraft] = useState<Interpreter>(python.interpreter);
  const [capturing, setCapturing] = useState<Command | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [firstSeq] = useState(resetSeq);

  // 外で変わって読み直したら、画面の中身も読み直す
  useEffect(() => {
    if (resetSeq === firstSeq) {
      return;
    }
    setAssigned(currentAssigned());
    setDraft(appearance);
    setSubmitDraft(submit);
    setProblemDraft(problem);
    setInterpreterDraft(python.interpreter);
    setCapturing(null);
    setNotice("settings.json が外で変わったので、読み直しました。必要なら、もう一度変えてください");
  }, [resetSeq]);

  // 試し見
  useEffect(() => {
    applyAppearance(draft);
    remeasureEditor();
  }, [draft]);

  function cancel() {
    applyAppearance(appearance);
    remeasureEditor();
    onClose();
  }

  // キーの受け取り（「変更」を押したあと）。アプリのキー割り当てより先に受け取り、ほかへ渡さない。
  // 受け取っていないときの Esc は、キャンセル
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (capturing === null) {
        if (event.key === "Escape") {
          event.preventDefault();
          cancel();
        }
        return;
      }
      event.preventDefault();
      event.stopImmediatePropagation();
      if (event.repeat) {
        return;
      }
      if (event.key === "Escape" && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) {
        setCapturing(null);
        setNotice(null);
        return;
      }
      const keys = bindingFromEvent(event);
      if (!keys) {
        return;
      }
      const reason = refusal(keys, capturing, assigned);
      if (reason) {
        setNotice(reason);
        return;
      }
      setAssigned((current) => ({ ...current, [capturing]: keys }));
      setCapturing(null);
      setNotice(null);
    }
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [capturing, assigned, appearance]);

  function setKeys(command: Command, keys: Keys | null) {
    if (keys) {
      const reason = refusal(keys, command, assigned);
      if (reason) {
        setNotice(reason);
        return;
      }
    }
    setAssigned((current) => ({ ...current, [command]: keys }));
    setNotice(null);
  }

  function setColor(name: string, value: string | null) {
    setDraft((current) => {
      const colors = { ...current.colors };
      // 既定と同じ色は書かない（colors には変えたものだけを書く）
      const base = splitColor(defaultColor(name));
      const next = value === null ? null : splitColor(value);
      if (value === null || (base && next && base.hex === next.hex && base.alpha === next.alpha)) {
        delete colors[name];
      } else {
        colors[name] = value;
      }
      return { ...current, colors };
    });
  }

  function setFontSize(which: "editorFontSize" | "problemFontSize", size: number) {
    if (!Number.isInteger(size)) {
      return;
    }
    const clamped = Math.min(Math.max(size, MIN_FONT_SIZE), MAX_FONT_SIZE);
    setDraft((current) => ({ ...current, [which]: clamped }));
  }

  async function save() {
    const bindings: LoadedKeyBinding[] = COMMANDS.map((command) => {
      const keys = assigned[command];
      return keys
        ? { command, ...keys }
        : { command, key: "", meta: false, ctrl: false, alt: false, shift: false };
    });
    setSaving(true);
    try {
      const outcome = await saveSettings(bindings, draft, submitDraft, problemDraft, interpreterDraft);
      if (outcome === "changedOutside") {
        await onReload();
        return;
      }
      if (outcome === "cancelled") {
        setNotice("確認のダイアログを省く設定にしなかったので、保存しませんでした");
        return;
      }
      setKeyBindings(bindings);
      onSaved(draft, submitDraft, problemDraft, interpreterDraft);
      onClose();
    } catch (e) {
      setNotice(String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="settings-backdrop">
      <div className="settings-dialog" role="dialog" aria-modal="true" aria-label="設定">
        <div className="settings-head">
          <div className="settings-tabs" role="tablist">
            {(
              [
                ["keys", "キー"],
                ["colors", "色"],
                ["view", "表示"],
                ["problem", "問題文"],
                ["submit", "提出"],
                ["python", "Python"],
                ["snippets", "スニペット"],
                ["templates", "テンプレート"],
              ] as const
            ).map(([id, label]) => (
              <button
                key={id}
                type="button"
                role="tab"
                aria-selected={section === id}
                className={`settings-tab ${section === id ? "selected" : ""}`}
                onClick={() => setSection(id)}
              >
                {label}
              </button>
            ))}
          </div>
          <button type="button" className="settings-link" onClick={onOpenJson}>
            JSON で開く
          </button>
        </div>
        <div className="settings-body">
          {section === "keys" && (
            <KeysSection
              assigned={assigned}
              capturing={capturing}
              onCapture={(command) => {
                setCapturing(command);
                setNotice(null);
              }}
              onSet={setKeys}
            />
          )}
          {section === "colors" && <ColorsSection colors={draft.colors} onSet={setColor} />}
          {section === "problem" && <ProblemSection draft={problemDraft} onChange={setProblemDraft} />}
          {section === "submit" && (
            <SubmitSection
              draft={submitDraft}
              onToggle={(which, on) => setSubmitDraft((current) => ({ ...current, [which]: on }))}
            />
          )}
          {section === "snippets" && (
            <div className="settings-view">
              <p className="settings-note">
                アプリのフォルダの言語ごとのファイルに、VS Code と同じ書き方で書きます。その言語のファイル（Rust は .rs、C++ は
                .cpp・.hpp・.h など、Python は .py）で、呼び出す文字を打つと補完に出ます。書き換えたら、IDE の窓に戻ると読み直します。「VS
                Code から取り込む」は、VS Code のその言語のスニペット（rust.json・cpp.json・python.json）を写します（今のファイルは退避します）。
              </p>
              {LANGUAGES.map(({ value: language, label }) => (
                <SnippetsSection
                  key={language}
                  language={language}
                  label={label}
                  snippets={snippets[language]}
                  onOpen={() => onOpenSnippets(language).catch((e) => setNotice(String(e)))}
                  onImport={async () => {
                    setNotice(null);
                    try {
                      setNotice(await onImportSnippets(language));
                    } catch (e) {
                      setNotice(String(e));
                    }
                  }}
                />
              ))}
            </div>
          )}
          {section === "python" && (
            <PythonSection python={python} interpreter={interpreterDraft} onChange={setInterpreterDraft} />
          )}
          {section === "templates" && <TemplatesSection onError={setNotice} />}
          {section === "view" && (
            <ViewSection
              draft={draft}
              onToggle={(which, on) => setDraft((current) => ({ ...current, [which]: on }))}
              onFontSize={setFontSize}
            />
          )}
        </div>
        <div className="settings-foot">
          <div className="settings-notice" role="status">
            {notice}
          </div>
          <button type="button" className="settings-button" onClick={cancel} disabled={saving}>
            キャンセル
          </button>
          <button
            type="button"
            className="settings-button primary"
            onClick={() => void save()}
            disabled={saving || capturing !== null}
          >
            保存
          </button>
        </div>
      </div>
    </div>
  );
}

function KeysSection({
  assigned,
  capturing,
  onCapture,
  onSet,
}: {
  assigned: Assigned;
  capturing: Command | null;
  onCapture: (command: Command | null) => void;
  onSet: (command: Command, keys: Keys | null) => void;
}) {
  return (
    <table className="settings-table">
      <tbody>
        {COMMANDS.map((command) => {
          const keys = assigned[command];
          const fallback = defaultKeys(command);
          return (
            <tr key={command}>
              <td className="settings-name">{COMMAND_LABELS[command]}</td>
              <td className="settings-key">
                {capturing === command ? (
                  <span className="settings-capturing">キーを押してください（Esc でやめる）</span>
                ) : keys ? (
                  <kbd>{keyLabel(keys)}</kbd>
                ) : (
                  <span className="settings-none">なし</span>
                )}
              </td>
              <td className="settings-actions">
                <button
                  type="button"
                  className="settings-link"
                  onClick={() => onCapture(capturing === command ? null : command)}
                >
                  {capturing === command ? "やめる" : "変更"}
                </button>
                <button
                  type="button"
                  className="settings-link"
                  disabled={keys === null || capturing !== null}
                  onClick={() => onSet(command, null)}
                >
                  なし
                </button>
                <button
                  type="button"
                  className="settings-link"
                  disabled={sameKeys(keys, fallback) || capturing !== null}
                  onClick={() => onSet(command, fallback)}
                >
                  既定に戻す
                </button>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function ColorsSection({
  colors,
  onSet,
}: {
  colors: Record<string, string>;
  onSet: (name: string, value: string | null) => void;
}) {
  // 既定の色は、開いている間は変わらないので、一度だけ読む
  const defaults = useMemo(
    () =>
      Object.fromEntries(
        COLOR_GROUPS.flatMap((group) => group.items.map((item) => [item.name, defaultColor(item.name)])),
      ),
    [],
  );
  return (
    <div>
      {COLOR_GROUPS.map((group) => (
        <section key={group.title} className="settings-group">
          <h3>{group.title}</h3>
          <table className="settings-table">
            <tbody>
              {group.items.map((item) => {
                // settings.json に手で書いた読めない色は、既定の色として見せる
                const written = colors[item.name];
                const value = written !== undefined && splitColor(written) ? written : defaults[item.name];
                const parts = splitColor(value) ?? { hex: "#000000", alpha: 100 };
                return (
                  <tr key={item.name}>
                    <td className="settings-name" title={item.name}>
                      {item.label}
                    </td>
                    <td className="settings-color">
                      <span
                        className="settings-swatch"
                        style={{ backgroundImage: `linear-gradient(${value}, ${value})` }}
                      />
                      <input
                        type="color"
                        aria-label={`${item.label}の色`}
                        value={parts.hex}
                        onChange={(event) => onSet(item.name, joinColor(event.target.value, parts.alpha))}
                      />
                      <input
                        type="range"
                        min={0}
                        max={100}
                        aria-label={`${item.label}の濃さ`}
                        value={parts.alpha}
                        onChange={(event) => onSet(item.name, joinColor(parts.hex, Number(event.target.value)))}
                      />
                      <span className="settings-alpha">{parts.alpha}%</span>
                    </td>
                    <td className="settings-actions">
                      <button
                        type="button"
                        className="settings-link"
                        disabled={colors[item.name] === undefined}
                        onClick={() => onSet(item.name, null)}
                      >
                        既定に戻す
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </section>
      ))}
    </div>
  );
}

function ViewSection({
  draft,
  onToggle,
  onFontSize,
}: {
  draft: Appearance;
  onToggle: (which: "indentRainbow" | "errorLens" | "breadcrumbs", on: boolean) => void;
  onFontSize: (which: "editorFontSize" | "problemFontSize", size: number) => void;
}) {
  const sizes = [
    ["editorFontSize", "エディタの文字の大きさ"],
    ["problemFontSize", "問題文の文字の大きさ"],
  ] as const;
  return (
    <div className="settings-view">
      <label className="settings-check">
        <input
          type="checkbox"
          checked={draft.indentRainbow}
          onChange={(event) => onToggle("indentRainbow", event.target.checked)}
        />
        インデントの段に色を付ける
      </label>
      <label className="settings-check">
        <input
          type="checkbox"
          checked={draft.errorLens}
          onChange={(event) => onToggle("errorLens", event.target.checked)}
        />
        診断のある行に色とメッセージを出す
      </label>
      <label className="settings-check">
        <input
          type="checkbox"
          checked={draft.breadcrumbs}
          onChange={(event) => onToggle("breadcrumbs", event.target.checked)}
        />
        エディタの上にパンくずリスト（ファイルの場所）を出す
      </label>
      {sizes.map(([which, label]) => (
        <SizeField key={which} label={label} size={draft[which]} onChange={(size) => onFontSize(which, size)} />
      ))}
    </div>
  );
}

// 提出の設定（D-162）。確認のダイアログを省くのは、Rust 側がサンプルテストの合格を確かめたときだけ
function SubmitSection({
  draft,
  onToggle,
}: {
  draft: SubmitSettings;
  onToggle: (which: "testBeforeSubmit" | "confirmDialog", on: boolean) => void;
}) {
  return (
    <div className="settings-view">
      <label className="settings-check">
        <input
          type="checkbox"
          checked={draft.testBeforeSubmit}
          onChange={(event) => onToggle("testBeforeSubmit", event.target.checked)}
        />
        提出の前にサンプルテストを動かし、すべて AC でなければ提出しない
      </label>
      <label className="settings-check">
        <input
          type="checkbox"
          checked={draft.confirmDialog}
          onChange={(event) => onToggle("confirmDialog", event.target.checked)}
        />
        提出の確認のダイアログを出す
      </label>
      <p className="settings-note">
        確認のダイアログを出さないようにしても、省くのは、今のソースのサンプルテストがすべて AC で、提出の前の検査の注意がないときだけです。
        サンプルがない問題・目視確認のケースがある問題・注意があるときは、今までどおり確認します。出さないようにして保存するときは、OS
        の確認が出ます。
      </p>
    </div>
  );
}

// Python（Phase 19 作業 3。D-221）：手元で使う処理系。処理系のパスは settings.json で書く
function PythonSection({
  python,
  interpreter,
  onChange,
}: {
  python: PythonSettings;
  interpreter: Interpreter;
  onChange: (next: Interpreter) => void;
}) {
  const path = interpreter === "cpython" ? python.cpython : python.pypy;
  return (
    <div className="settings-view">
      <label className="settings-check">
        処理系
        <select
          className="settings-select"
          value={interpreter}
          onChange={(event) => onChange(event.target.value as Interpreter)}
        >
          {INTERPRETERS.map(({ value, label }) => (
            <option key={value} value={value}>
              {label}
            </option>
          ))}
        </select>
      </label>
      <p className="settings-note">
        main.py の実行・テストに使います。ジャッジの「Python (CPython 3.13.7)」「Python (PyPy 3.11-v7.3.20)」に合わせてください。
        {path
          ? `処理系のパス：${path}（settings.json で書いたもの）。`
          : "処理系は /opt/homebrew/bin・PATH・/usr/local/bin から探します（決まったパスを使うときは、settings.json の python.cpython・python.pypy に書きます）。"}
      </p>
    </div>
  );
}

// 問題文の欄（Phase 15 作業 3。D-196・D-197）：見出しの行に出すボタンと、main() に for まで書くか
function ProblemSection({
  draft,
  onChange,
}: {
  draft: ProblemSettings;
  onChange: (next: ProblemSettings) => void;
}) {
  function toggleButton(name: string, on: boolean) {
    const buttons = on ? [...draft.buttons, name] : draft.buttons.filter((b) => b !== name);
    // 並びは決まった順に揃えて書く
    onChange({ ...draft, buttons: PROBLEM_BUTTONS.map((b) => b.name).filter((b) => buttons.includes(b)) });
  }
  return (
    <div className="settings-view">
      <p className="settings-note">問題文の欄の見出しの行に出すボタン（出さないものは「…」の一覧に入ります）</p>
      {PROBLEM_BUTTONS.map(({ name, label }) => (
        <label key={name} className="settings-check">
          <input
            type="checkbox"
            checked={draft.buttons.includes(name)}
            onChange={(event) => toggleButton(name, event.target.checked)}
          />
          {label}
        </label>
      ))}
      <p className="settings-note">
        「この問題を作る」は、選択欄で選んだ問題のときだけ出ます。すべてを見出しの行に出すと、「…」は出ません。
      </p>
      <label className="settings-check">
        問題を作るときの言語
        <select
          className="settings-select"
          value={draft.language}
          onChange={(event) => onChange({ ...draft, language: event.target.value as Language })}
        >
          {LANGUAGES.map(({ value, label }) => (
            <option key={value} value={value}>
              {label}
            </option>
          ))}
        </select>
      </label>
      <p className="settings-note">
        「コンテストを作る」「この問題を作る」は、この言語で作ります。
      </p>
      <label className="settings-check">
        <input
          type="checkbox"
          checked={draft.fillInput}
          onChange={(event) => onChange({ ...draft, fillInput: event.target.checked })}
        />
        問題を作るときに input! を入れておく（C++・Python は入力のコード）
      </label>
      <p className="settings-note">
        「コンテストを作る」「この問題を作る」で main.rs を書くときに、テンプレートの solve() の input!
        を、問題文から作ったものにしておきます（テストケースが複数なら、下の設定に従います）。すでにある main.rs は変えません。
        C++ では main.cpp の、Python では main.py の solve()（なければ main()）に、目印のコメントで囲んで入れます。
      </p>
      <label className="settings-check">
        <input
          type="checkbox"
          checked={draft.mainLoop}
          onChange={(event) => onChange({ ...draft, mainLoop: event.target.checked })}
        />
        テストケースが複数のとき、main() に for も書く
      </label>
      <p className="settings-note">
        「solve() の input! を置き換える」で、main() の input! を t: usize にし（なければ先頭に入れ）、main() の中の solve();
        を for _ in 0..t {"{"} solve(); {"}"} にします。すでに for の中で呼んでいれば、そのままにします。C++ では、main() に T
        を読むコードを入れ、solve(); を for (int i = 0; i &lt; T; i++) solve(); にします。Python では、main() に T
        を読むコードを入れ、solve() を for _ in range(T): の中にします。
      </p>
    </div>
  );
}

// スニペット（Phase 13 作業 2。D-187）：言語ごとの、今読めているものの一覧と、ファイルを開く・VS Code から取り込む（D-213）
function SnippetsSection({
  language,
  label,
  snippets,
  onOpen,
  onImport,
}: {
  language: Language;
  label: string;
  snippets: { loaded: LoadedSnippets | null; error: string | null };
  onOpen: () => void;
  onImport: () => Promise<void>;
}) {
  const [importing, setImporting] = useState(false);
  const list = [...(snippets.loaded?.snippets ?? [])].sort((a, b) =>
    a.prefixes[0].localeCompare(b.prefixes[0], "ja"),
  );
  const problems = snippets.loaded?.problems ?? [];
  return (
    <div className="settings-snippets">
      <div className="settings-actions-row">
        <span className="settings-name">
          {label}（{snippetsFile(language)}）：{list.length} 件
        </span>
        <button type="button" className="settings-link" onClick={onOpen}>
          ファイルを開く
        </button>
        <button
          type="button"
          className="settings-link"
          disabled={importing}
          onClick={() => {
            setImporting(true);
            void onImport().finally(() => setImporting(false));
          }}
        >
          VS Code から取り込む
        </button>
      </div>
      {snippets.error && <p className="settings-warning">{snippets.error}</p>}
      {problems.length > 0 && <p className="settings-warning">読めない項目：{problems.join("、")}</p>}
      {list.length > 0 && (
        <table className="settings-table">
          <tbody>
            {list.map((item) => (
              <tr key={item.name}>
                <td className="settings-snippet-prefix">{item.prefixes.join("、")}</td>
                <td className="settings-name">{item.description ?? item.name}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

// テンプレート（Phase 13 作業 3。D-188）：雛形のファイルの説明とあるかどうか、フォルダを開く。
// Finder から戻ったときに、あるかどうかを確かめ直す
function TemplatesSection({ onError }: { onError: (message: string) => void }) {
  const [files, setFiles] = useState<TemplateFiles | null>(null);
  useEffect(() => {
    const refresh = () => {
      templateFiles()
        .then(setFiles)
        .catch((e) => onError(String(e)));
    };
    refresh();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, []);
  const state = (present: boolean | undefined) =>
    present === undefined ? "" : present ? "あり" : "なし（IDE の最小の雛形を使います）";
  return (
    <div className="settings-view">
      <div className="settings-actions-row">
        <span className="settings-name">templates/</span>
        <button
          type="button"
          className="settings-link"
          onClick={() => openTemplatesFolder().catch((e) => onError(String(e)))}
        >
          テンプレートのフォルダを開く
        </button>
      </div>
      <p className="settings-note">
        「コンテストを作る」「この問題を作る」で作る問題の中身になります。アプリのフォルダの templates/ に次のファイルを置きます（フォルダがなければ、開くときに作ります）。
      </p>
      <table className="settings-table">
        <tbody>
          <tr>
            <td className="settings-snippet-prefix">main.rs</td>
            <td className="settings-name">src/main.rs の中身</td>
            <td className="settings-name">{state(files?.mainRs)}</td>
          </tr>
          <tr>
            <td className="settings-snippet-prefix">dependencies.toml</td>
            <td className="settings-name">Cargo.toml の [dependencies] の中身</td>
            <td className="settings-name">{state(files?.dependencies)}</td>
          </tr>
          <tr>
            <td className="settings-snippet-prefix">main.cpp</td>
            <td className="settings-name">C++ の main.cpp の中身</td>
            <td className="settings-name">{state(files?.mainCpp)}</td>
          </tr>
          <tr>
            <td className="settings-snippet-prefix">main.py</td>
            <td className="settings-name">Python の main.py の中身</td>
            <td className="settings-name">{state(files?.mainPy)}</td>
          </tr>
        </tbody>
      </table>
    </div>
  );
}

// 文字の大きさの欄。打っている途中の数（範囲の外・空）は、当てずに欄にだけ残す。欄を離れたら、今の大きさに戻す
function SizeField({ label, size, onChange }: { label: string; size: number; onChange: (size: number) => void }) {
  const [text, setText] = useState(String(size));
  useEffect(() => setText(String(size)), [size]);
  return (
    <div className="settings-size">
      <span className="settings-name">{label}</span>
      <button
        type="button"
        className="settings-step"
        aria-label={`${label}を小さく`}
        disabled={size <= MIN_FONT_SIZE}
        onClick={() => onChange(size - 1)}
      >
        −
      </button>
      <input
        type="number"
        min={MIN_FONT_SIZE}
        max={MAX_FONT_SIZE}
        step={1}
        aria-label={label}
        value={text}
        onChange={(event) => {
          setText(event.target.value);
          const next = Number(event.target.value);
          if (Number.isInteger(next) && next >= MIN_FONT_SIZE && next <= MAX_FONT_SIZE) {
            onChange(next);
          }
        }}
        onBlur={() => setText(String(size))}
      />
      <button
        type="button"
        className="settings-step"
        aria-label={`${label}を大きく`}
        disabled={size >= MAX_FONT_SIZE}
        onClick={() => onChange(size + 1)}
      >
        ＋
      </button>
      <span className="settings-unit">
        px（{MIN_FONT_SIZE}〜{MAX_FONT_SIZE}）
      </span>
    </div>
  );
}

export default SettingsDialog;
