// キー割り当ての表（D-052）。起動したときと、窓に戻って settings.json を読み直したときに、その内容で置き換える（作業 6・D-159）
export type Command =
  | "toggleSidebar"
  | "save"
  | "closeTab"
  | "openFolder"
  | "run"
  | "test"
  | "stop"
  | "goToDefinition"
  | "findReferences"
  | "formatDocument"
  | "debug"
  | "toggleBreakpoint"
  | "stepOver"
  | "stepIn"
  | "stepOut"
  // 提出（Phase 6 作業 3。D-148 §2.2）。誤って送るのを防ぐため、初期の割り当てはない。
  // settings.json に割り当てを書けば使える（押しても、ボタンと同じく、送る前の確かめ〔D-162〕を通る）
  | "submit"
  // 設定（Phase 8 作業 6。D-159）
  | "openSettings"
  // サイドバーの中身（Phase 12 作業 1。D-182）
  | "showExplorer"
  | "showSearch"
  | "showDebug";

export const COMMANDS: Command[] = [
  "toggleSidebar",
  "save",
  "closeTab",
  "openFolder",
  "run",
  "test",
  "stop",
  "goToDefinition",
  "findReferences",
  "formatDocument",
  "debug",
  "toggleBreakpoint",
  "stepOver",
  "stepIn",
  "stepOut",
  "submit",
  "openSettings",
  "showExplorer",
  "showSearch",
  "showDebug",
];

// コマンドの名前（知らせ・設定の画面で使う。docs/usage.md の「キーの割り当て」の表と同じ）
export const COMMAND_LABELS: Record<Command, string> = {
  toggleSidebar: "サイドバー",
  save: "保存",
  closeTab: "タブを閉じる",
  openFolder: "フォルダを開く",
  run: "実行",
  test: "サンプルテスト",
  stop: "停止",
  goToDefinition: "定義へジャンプ",
  findReferences: "参照検索",
  formatDocument: "整形",
  debug: "デバッグ・続行",
  toggleBreakpoint: "ブレークポイント",
  stepOver: "ステップオーバー",
  stepIn: "ステップイン",
  stepOut: "ステップアウト",
  submit: "提出",
  openSettings: "設定",
  showExplorer: "エクスプローラー",
  showSearch: "検索",
  showDebug: "実行とデバッグ",
};

export type KeyBinding = {
  command: Command;
  key: string; // KeyboardEvent.key（小文字）。空なら「割り当てなし」（D-159）
  meta: boolean; // ⌘
  ctrl: boolean;
  alt: boolean; // ⌥
  shift: boolean;
};

// macOS で動いているか（WebView の userAgent。macOS の WKWebView は Macintosh、Windows の WebView2 は Windows NT を含む）。
// Windows 移植の段階 1 C9（docs/windows-port-plan.md 9.1 の 5）
export const IS_MAC = /Macintosh|Mac OS X/.test(navigator.userAgent);

// macOS 以外では、⌘ を使う割り当てを Ctrl に置き換える（src-tauri/src/storage.rs の default_key_bindings と同じ規則）
function forPlatform(bindings: KeyBinding[]): KeyBinding[] {
  return IS_MAC ? bindings : bindings.map((b) => (b.meta ? { ...b, meta: false, ctrl: true } : b));
}

// 初期の割り当て（D-022・D-051）。src-tauri/src/storage.rs の default_key_bindings と同じ内容。
// 提出（submit）は、初期の割り当てを置かない（D-148 §2.2）
export const DEFAULT_KEY_BINDINGS: KeyBinding[] = forPlatform([
  // ⌘B：ファイル一覧の表示を切り替える
  { command: "toggleSidebar", key: "b", meta: true, ctrl: false, alt: false, shift: false },
  // ⌘S：開いているファイルをすぐ保存する
  { command: "save", key: "s", meta: true, ctrl: false, alt: false, shift: false },
  // ⌘W：開いているタブを閉じる（作業 7b。D-083）
  { command: "closeTab", key: "w", meta: true, ctrl: false, alt: false, shift: false },
  // ⌘O：フォルダを開く（作業 7b。D-083）
  { command: "openFolder", key: "o", meta: true, ctrl: false, alt: false, shift: false },
  // ⌘R：実行、⇧⌘R：サンプルテスト、⌘.：停止（Phase 2 作業 6。D-101）
  { command: "run", key: "r", meta: true, ctrl: false, alt: false, shift: false },
  { command: "test", key: "r", meta: true, ctrl: false, alt: false, shift: true },
  { command: "stop", key: ".", meta: true, ctrl: false, alt: false, shift: false },
  // F12：定義へジャンプ、⇧F12：参照検索（Phase 3b-2 作業 4。D-133）。エディタのコマンドなので、LSP につながっていなければ何もしない
  { command: "goToDefinition", key: "f12", meta: false, ctrl: false, alt: false, shift: false },
  { command: "findReferences", key: "f12", meta: false, ctrl: false, alt: false, shift: true },
  // ⇧⌥F：整形（Phase 3b-2 作業 6。D-133）。rust-analyzer が rustfmt を呼ぶので、LSP につながっていなければ何もしない
  { command: "formatDocument", key: "f", meta: false, ctrl: false, alt: true, shift: true },
  // F5：デバッグの開始・続行（止まっていなければ一時停止）、F9：カーソルの行のブレークポイント、
  // F10：ステップオーバー、F11：ステップイン、⇧F11：ステップアウト（Phase 5 作業 2。D-146 2.5）。停止は ⌘.
  { command: "debug", key: "f5", meta: false, ctrl: false, alt: false, shift: false },
  { command: "toggleBreakpoint", key: "f9", meta: false, ctrl: false, alt: false, shift: false },
  { command: "stepOver", key: "f10", meta: false, ctrl: false, alt: false, shift: false },
  { command: "stepIn", key: "f11", meta: false, ctrl: false, alt: false, shift: false },
  { command: "stepOut", key: "f11", meta: false, ctrl: false, alt: false, shift: true },
  // ⌘,：設定（Phase 8 作業 6。D-159。macOS の「設定…」の決まり）
  { command: "openSettings", key: ",", meta: true, ctrl: false, alt: false, shift: false },
  // ⇧⌘E・⇧⌘F・⇧⌘D：サイドバーの中身（Phase 12 作業 1。D-182。VS Code と同じ割り当て）
  { command: "showExplorer", key: "e", meta: true, ctrl: false, alt: false, shift: true },
  { command: "showSearch", key: "f", meta: true, ctrl: false, alt: false, shift: true },
  { command: "showDebug", key: "d", meta: true, ctrl: false, alt: false, shift: true },
]);

let keyBindings: KeyBinding[] = DEFAULT_KEY_BINDINGS;

type LoadedKeyBinding = Omit<KeyBinding, "command"> & { command: string };

// 名前で見せるキー（KeyboardEvent.key の小文字）
const KEY_NAMES: Record<string, string> = {
  " ": "Space",
  enter: "Enter",
  escape: "Esc",
  tab: "Tab",
  backspace: "Delete",
  delete: "⌦",
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
  home: "Home",
  end: "End",
  pageup: "PageUp",
  pagedown: "PageDown",
};

// Windows の、名前で見せるキーのうち macOS と違うもの
const WINDOWS_KEY_NAMES: Record<string, string> = {
  backspace: "Backspace",
  delete: "Delete",
};

// キーの見せ方（例：⇧⌘R、F12）。修飾キーは macOS の順（⌃⌥⇧⌘）。
// macOS 以外では Ctrl+Alt+Shift+R の形（⌘ にあたる Windows キーは Win）
export function keyLabel(binding: Omit<KeyBinding, "command">): string {
  if (!IS_MAC) {
    const parts = [
      ...(binding.ctrl ? ["Ctrl"] : []),
      ...(binding.alt ? ["Alt"] : []),
      ...(binding.shift ? ["Shift"] : []),
      ...(binding.meta ? ["Win"] : []),
    ];
    const name =
      WINDOWS_KEY_NAMES[binding.key] ??
      KEY_NAMES[binding.key] ??
      (binding.key.length === 1 ? binding.key.toUpperCase() : binding.key.replace(/^f(\d+)$/, "F$1"));
    return [...parts, name].join("+");
  }
  const modifiers =
    (binding.ctrl ? "⌃" : "") + (binding.alt ? "⌥" : "") + (binding.shift ? "⇧" : "") + (binding.meta ? "⌘" : "");
  const key =
    KEY_NAMES[binding.key] ??
    (binding.key.length === 1 ? binding.key.toUpperCase() : binding.key.replace(/^f(\d+)$/, "F$1"));
  return modifiers + key;
}

// 今の割り当て（設定の画面で使う。作業 6b。D-159）
export function getKeyBindings(): KeyBinding[] {
  return keyBindings;
}

// 押したキーから割り当てを作る（設定の画面の「変更」。D-159）。修飾キーだけのときは null。
// ⌥ を押しているときは、findCommand と同じく、英字・数字は物理キーの名前にする
export function bindingFromEvent(event: KeyboardEvent): Omit<KeyBinding, "command"> | null {
  if (["Meta", "Shift", "Alt", "Control", "CapsLock", "Fn", "Dead"].includes(event.key)) {
    return null;
  }
  const physical = event.altKey ? physicalKey(event.code) : undefined;
  return {
    key: physical ?? event.key.toLowerCase(),
    meta: event.metaKey,
    ctrl: event.ctrlKey,
    alt: event.altKey,
    shift: event.shiftKey,
  };
}

// settings.json から読んだ割り当てを使う。知らないコマンドの割り当ては使わずに返す。
// 割り当てがないコマンドは、初期の割り当てを使う。key が空の割り当ては「割り当てなし」（初期の割り当ても使わない。D-159）。
// 同じキーが 2 つ以上のコマンドに割り当てられていれば、そのキーを返す（先のものを使う）
export function setKeyBindings(loaded: LoadedKeyBinding[]): { unknown: string[]; duplicates: string[] } {
  const valid: KeyBinding[] = [];
  const unknown: string[] = [];
  for (const binding of loaded) {
    if ((COMMANDS as string[]).includes(binding.command)) {
      valid.push({ ...binding, command: binding.command as Command, key: binding.key.toLowerCase() });
    } else {
      unknown.push(binding.command);
    }
  }
  const missing = DEFAULT_KEY_BINDINGS.filter(
    (fallback) => !valid.some((binding) => binding.command === fallback.command),
  );
  keyBindings = [...valid, ...missing];
  const duplicates: string[] = [];
  const seen = new Map<string, Command>();
  for (const binding of keyBindings) {
    if (binding.key === "") {
      continue;
    }
    const label = keyLabel(binding);
    const first = seen.get(label);
    if (first === undefined) {
      seen.set(label, binding.command);
    } else if (first !== binding.command) {
      duplicates.push(`${label}（${COMMAND_LABELS[first]}・${COMMAND_LABELS[binding.command]}）`);
    }
  }
  return { unknown, duplicates };
}

// 物理キー（KeyboardEvent.code）の英字・数字。US 配列での名前（Key + 英字、Digit + 数字）で、位置が同じなら
// 配列が違っても同じ名前になる。英字・数字以外は undefined
function physicalKey(code: string): string | undefined {
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) {
    return letter[1].toLowerCase();
  }
  return /^Digit([0-9])$/.exec(code)?.[1];
}

export function findCommand(event: KeyboardEvent): Command | undefined {
  const key = event.key.toLowerCase();
  // ⌥ を押すと、macOS では event.key が別の文字になる（⌥⇧F は "Ï" など）。⌥ を押しているときだけ、
  // 物理キーでも照合する（⌥ を使わない割り当ては、これまでどおり event.key で照合する）
  const physical = event.altKey ? physicalKey(event.code) : undefined;
  return keyBindings.find(
    (binding) =>
      binding.key !== "" &&
      (binding.key === key || (physical !== undefined && binding.key === physical)) &&
      binding.meta === event.metaKey &&
      binding.ctrl === event.ctrlKey &&
      binding.alt === event.altKey &&
      binding.shift === event.shiftKey,
  )?.command;
}
