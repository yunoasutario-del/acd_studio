// 言語サーバーとの接続（Phase 3b-1 作業 3。D-122〜D-126）
//
// - サーバーは、.rs なら rust-analyzer、C++ のファイルなら clangd（Phase 17 作業 5。D-210）、.py なら basedpyright
//   （Phase 20 作業 5。D-225）。どれも同じ流れでつなぐ。clangd・basedpyright は読み込みの終わりを知らせないので、
//   initialize の応答で使える状態とする。basedpyright は整形をしないので、.py では整形を送らない
// - LSP クライアント（@codemirror/lsp-client）は画面に 1 つだけ置き、サーバーを起動し直したら接続し直す
// - LSP に開くのは、エディタで開いているタブの .rs・C++ のファイル・.py だけ。タブの状態には Compartment を入れ、
//   タブに切り替えたときに LSP の拡張を外し（Editor.tsx）、起動を確かめてから付け直す
// - 開いているファイルの最寄りの Cargo.toml のパッケージで起動する（D-124）。決めるのは Rust 側で、
//   画面はタブを切り替えるたびに、少し待ってから lsp_start を呼ぶ
// - 起動 1 回ごとに Tauri のチャンネルを 1 つ使う。止めたサーバーの知らせは古いチャンネルに届くので、
//   新しいサーバーの知らせと取り違えない
// - 状態・ログ・失敗は開発者ツールの console に出し、あわせて画面（下部パネルの状態の文字と実行ログ）に知らせる
//   （Phase 3b-2 作業 3。D-130）。画面への知らせは subscribeLsp で受ける
import { Channel } from "@tauri-apps/api/core";
import { getIndentUnit } from "@codemirror/language";
import { Compartment, Text, type EditorState } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import {
  LSPClient,
  LSPPlugin,
  closeReferencePanel,
  findReferences,
  formatDocument,
  hoverTooltips,
  jumpToDefinition,
  serverCompletion,
  serverDiagnostics,
  type LSPClientExtension,
  type Transport,
} from "@codemirror/lsp-client";
import { lspLocate, lspSend, lspStart, lspStop, type LspEvent } from "../ipc/lsp";
import { readFile } from "../ipc/workspace";
import { fromFileContent } from "./fileText";
import { IdeWorkspace } from "./workspace";
import { inlayHints, QUIESCENT_EVENT } from "./inlayHints";
import { sanitizeDocHTML } from "./sanitize";
import { isCppPath, isPythonPath } from "./language";
import { toOutline, type OutlineNode } from "./breadcrumbTrail";
import { exitReport } from "./lspExit";

// 開いているファイルを言語サーバーにつないだ合図。パンくずリストが記号を聞き直す（Phase 18 作業 2。D-216）
export const ATTACHED_EVENT = "lsp-attached";

// タブを切り替えてから lsp_start を呼ぶまでの待ち時間（D-124 の「少し待つ」。値は試作で決める）
const SWITCH_DELAY_MS = 500;

// 言語サーバーの状態（D-130）。none は、まだ起動していない（表示しない）
export type LspStatus = "none" | "starting" | "ready" | "stopped" | "failed";

// 画面に知らせるもの：状態の変化（server はサーバーの名前）と、実行ログに足す行（行頭の [rust-analyzer] などと改行を含む）
export type LspUiEvent = { kind: "status"; status: LspStatus; server: string } | { kind: "log"; text: string };

const listeners = new Set<(event: LspUiEvent) => void>();

export function subscribeLsp(listener: (event: LspUiEvent) => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function emit(event: LspUiEvent) {
  for (const listener of listeners) {
    listener(event);
  }
}

// 今つなぐ（つないでいる）サーバーの名前。実行ログ・console の行頭と状態の表示に使う
let serverName = "rust-analyzer";

// path のファイルをつなぐサーバーの名前（Rust 側の lsp_start と同じ分け方）
function serverFor(path: string): string {
  return isCppPath(path) ? "clangd" : isPythonPath(path) ? "basedpyright" : "rust-analyzer";
}

// LSP の文書の言語の ID
function languageIdFor(path: string): string {
  return isCppPath(path) ? "cpp" : isPythonPath(path) ? "python" : "rust";
}

// 行頭の [サーバーの名前]。接続についての行（終わり・標準エラー・送れない）は、その接続の名前で出す（②）
function tagFor(server: string): string {
  return `[${server}]`;
}

// 今つなぐ（つないでいる）サーバーについての行
function tag(): string {
  return tagFor(serverName);
}

function setStatus(status: LspStatus) {
  emit({ kind: "status", status, server: serverName });
}

// 実行ログに 1 行足す（server のサーバーの行として）
function logLineFor(server: string, text: string) {
  emit({ kind: "log", text: `${tagFor(server)} ${text}\n` });
}

// 実行ログに 1 行足す（今のサーバーの行として）
function logLine(text: string) {
  logLineFor(serverName, text);
}

// 1 回の起動で実行ログに出すサーバーの標準エラーの行数。超えたら省略する
const MAX_LOG_LINES = 50;
// Rust 側が、終わったことを標準エラーと同じ「ログ」で知らせる行（「<サーバーの名前> が終わりました」）。
// 終わりは別に出すので、画面には出さない
const EXIT_LOG = " が終わりました";

// 画面のセッション。画面を読み込み直したら変わり、Rust 側はそれを見て起動し直す（新しい画面は initialize からやり直す）
const session = crypto.randomUUID();

// LSP の拡張を入れる場所。createEditorState で、すべてのタブの状態に空のまま入れる
export const lspCompartment = new Compartment();

// rust-analyzer の状態（読み込み中か、など）を知らせてもらう。設定ではなく能力の宣言（D-121 の設定は変えない）
const serverStatus: LSPClientExtension = {
  clientCapabilities: { experimental: { serverStatusNotification: true } },
  notificationHandlers: {
    "experimental/serverStatus": (_client, params) => {
      console.info(tag(), "状態", params);
      // 読み込み中（quiescent: false）か、読み込みが終わったか（true）。起動のあとの検査の間も false になる
      if (current && typeof params?.quiescent === "boolean") {
        if (params.quiescent) {
          current.ready = true;
        }
        setStatus(params.quiescent ? "ready" : "starting");
      }
      // 警告・エラーのときの説明は実行ログに出す（同じ説明は続けて出さない）
      if (typeof params?.health === "string" && params.health !== "ok" && typeof params?.message === "string") {
        const line = `状態（${params.health}）：${params.message.trim()}`;
        if (line !== lastHealthLine) {
          lastHealthLine = line;
          logLine(line);
        }
      }
      if (params?.quiescent === true) {
        // 読み込みが終わった。インレイヒントを要求し直してもらう（inlayHints.ts）
        window.dispatchEvent(new Event(QUIESCENT_EVENT));
      }
      return true;
    },
  },
};

// タブで開く関数（App が渡す）。workspace は作業領域の中のファイル（相対パス。開けたら true）、
// readOnly は作業領域の外のソース（URI。開けたら null、開けなければ理由）
type FileOpener = {
  workspace: (path: string) => Promise<boolean>;
  readOnly: (uri: string) => Promise<string | null>;
};

let fileOpener: FileOpener | null = null;

export function setFileOpener(opener: FileOpener) {
  fileOpener = opener;
}

// 別のファイルへ移るとき、そのファイルをタブで開く（IdeWorkspace.displayFile から）。
// 作業領域の中のファイルは、通常のタブ（"lsp"。LSP の拡張が付く）で、外の標準ライブラリ・依存クレートのソースは、
// 読み取り専用のタブ（"plain"。LSP につながない。Rust 側が許可した場所の .rs だけ。D-133）で開く
async function openInTab(uri: string): Promise<"lsp" | "plain" | null> {
  // 参照の一覧は、タブが入れ替わると残ってしまうので閉じる
  if (lastView) {
    closeReferencePanel(lastView);
  }
  let path: string | null;
  try {
    path = await lspLocate(uri);
  } catch (e) {
    logLine(`移動先を調べられません：${String(e)}`);
    return null;
  }
  if (!fileOpener) {
    return null;
  }
  if (path !== null) {
    return (await fileOpener.workspace(path)) ? "lsp" : null;
  }
  const error = await fileOpener.readOnly(uri);
  if (error !== null) {
    logLine(`開けません（${uri}）：${error}`);
    return null;
  }
  return "plain";
}

// 使う機能は診断・補完・ホバー・インレイヒント・定義ジャンプ・参照検索（AC3b-03〜05、AC3b2-05・07）。
// キー割り当て（F12・⇧F12）は、ライブラリのものではなく、アプリのキー割り当ての表（keybindings.ts）で扱う
const client = new LSPClient({
  workspace: (client) =>
    new IdeWorkspace(client, {
      open: openInTab,
      shownView: (key) => (shownKey === key ? lastView : null),
      log: logLine,
    }),
  sanitizeHTML: sanitizeDocHTML,
  extensions: [serverDiagnostics(), serverCompletion(), hoverTooltips(), inlayHints(), serverStatus],
});

// 起動 1 回分の接続
type Connection = {
  // サーバーの名前
  server: string;
  transport: Transport;
  // サーバーが終わったら true。終わったあとは送らない
  closed: boolean;
  // 読み込みが終わった（experimental/serverStatus の quiescent が true になった）ことがある
  ready: boolean;
  // 終わりの知らせが届いた。current にする前に届くことがある（すぐ終わったとき）ので、覚えておく
  exited: boolean;
  // サーバーが自分で終わった（Rust 側が「<名前> が終わりました」の Log を出した）。IDE が止めたときは false のまま（①）
  endedByItself: boolean;
  exitCode: number | null;
  // 実行ログに出した標準エラーの行数
  logged: number;
};

let current: Connection | null = null;
let lastHealthLine = "";
// followActiveFile で渡されたエディタ。再起動で付け直すために覚えておく
let lastView: EditorView | null = null;
// 今エディタに出しているタブのキー（読み取り専用のタブの URI を含む）
let shownKey: string | null = null;

function connectionFor(channel: Channel<LspEvent>, server: string): Connection {
  const handlers = new Set<(message: string) => void>();
  const connection: Connection = {
    server,
    closed: false,
    ready: false,
    exited: false,
    endedByItself: false,
    exitCode: null,
    logged: 0,
    transport: {
      send(message) {
        if (connection.closed) {
          return;
        }
        lspSend(message).catch((e) => console.warn(tagFor(server), "送れません", e));
      },
      subscribe(handler) {
        handlers.add(handler);
      },
      unsubscribe(handler) {
        handlers.delete(handler);
      },
    },
  };
  channel.onmessage = (event) => {
    switch (event.kind) {
      case "message":
        for (const handler of handlers) {
          handler(event.text);
        }
        break;
      case "log":
        console.log(tagFor(connection.server), event.text);
        if (event.text === `${connection.server}${EXIT_LOG}`) {
          // 自分で終わった。Rust 側は、IDE が止めたとき（stop）にはこの Log を出さない（①）。
          // 前のサーバーとして閉じたあと（closed が true）に届いても覚える。画面には出さない（今までどおり）
          connection.endedByItself = true;
        } else if (!connection.closed) {
          // 古いサーバーの標準エラーは出さない（終わったものは connection.closed）
          connection.logged += 1;
          if (connection.logged <= MAX_LOG_LINES) {
            logLineFor(connection.server, event.text);
          } else if (connection.logged === MAX_LOG_LINES + 1) {
            logLineFor(
              connection.server,
              `標準エラーは ${MAX_LOG_LINES} 行までしか出しません（以降は省略します。開発者ツールの console にはすべて出ます）`,
            );
          }
        }
        break;
      case "exited":
        connection.closed = true;
        connection.exited = true;
        connection.exitCode = event.code;
        if (current === connection) {
          // 自分で終わったか、止められた（開発者ツールの lsp_stop、別のフォルダを開いた、など）。
          // 次にタブを切り替えたときに起動し直す
          handleExit(connection);
        }
        break;
    }
  };
  return connection;
}

// 起動して使っていたサーバーが終わった。IDE が止めたなら「止めました」、自分で終わったなら、読み込みが終わる前は起動の失敗、
// あとは停止として知らせる（D-130。lspExit.ts。実行ログの直し ①）。行と状態は、終わった接続のサーバーの名前で出す（②）。
// Rust 側は、自分で終わったサーバーを、次に止めるか起動し直すまで回収しない（04 §2）ので、ここで止めて回収する
function handleExit(connection: Connection) {
  console.info(
    tagFor(connection.server),
    "止まりました。終了コード：",
    connection.exitCode,
    "自分で終わった：",
    connection.endedByItself,
  );
  const report = exitReport(connection.server, {
    ready: connection.ready,
    endedByItself: connection.endedByItself,
    code: connection.exitCode,
  });
  emit({ kind: "log", text: `${report.line}\n` });
  emit({ kind: "status", status: report.status, server: connection.server });
  if (current === connection) {
    current = null;
    client.disconnect();
  }
  reap();
}

// 終わったサーバーを Rust 側で回収する。起動などと順番を守るため queue に入れ、
// その間に新しいサーバーを使い始めていたら（current がある）、止めない
function reap() {
  queue = queue.then(async () => {
    if (current === null) {
      await lspStop().catch((e) => console.warn(tag(), "回収できません", e));
    }
  });
}

// Rust 側の file_uri と同じ規則で、パスの各部分をパーセントエンコードする
function encodePath(path: string): string {
  let out = "";
  for (const byte of new TextEncoder().encode(path)) {
    const c = String.fromCharCode(byte);
    out += /[A-Za-z0-9\-._~/]/.test(c) ? c : `%${byte.toString(16).toUpperCase().padStart(2, "0")}`;
  }
  return out;
}

// 作業領域のルートからの相対パスを、文書の URI にする（パッケージの URI に、パッケージからの相対パスを足す）
function documentUri(path: string, rootUri: string, pkg: string): string {
  const inPackage = pkg ? path.slice(pkg.length + 1) : path;
  return `${rootUri}/${encodePath(inPackage)}`;
}

// 開いているタブの .rs・C++ のファイル・.py と、その URI（保存を知らせるため）
let wanted: string | null = null;
let wantedUri: string | null = null;
let timer: ReturnType<typeof setTimeout> | undefined;
// 起動を 1 つずつ行う（initialize が別のサーバーに届かないように）
let queue: Promise<void> = Promise.resolve();

async function attach(view: EditorView, path: string) {
  if (wanted !== path) {
    return;
  }
  serverName = serverFor(path);
  const channel = new Channel<LspEvent>();
  const connection = connectionFor(channel, serverName);
  let started;
  try {
    started = await lspStart(path, session, channel);
  } catch (e) {
    console.error(tag(), "起動できません", e);
    logLine(`起動できません：${String(e)}`);
    setStatus("failed");
    return;
  }
  if (!started) {
    // パッケージ（Cargo.toml・main.cpp・main.py）の外。解析しない
    console.info(tag(), "パッケージの外なので解析しません：", path);
    return;
  }
  if (started.alreadyRunning) {
    // 渡したチャンネルは使われない
    connection.closed = true;
    if (!current) {
      console.warn(tag(), "動いているはずのサーバーとつながっていません");
      return;
    }
  } else {
    console.info(tag(), "起動しました：", started.package || "（作業領域のルート）");
    logLine(`起動しました：${started.package || "（作業領域のルート）"}`);
    lastHealthLine = "";
    setStatus("starting");
    if (current) {
      // 前のサーバーは lsp_start の中で止めた。その終了の知らせが届く前でも、ここで切る
      // （切らないと、前の initialize が済んだ状態のまま、新しいサーバーに要求を送ってしまう）
      current.closed = true;
      client.disconnect();
    }
    current = connection;
    if (connection.exited) {
      // current にする前に終わっていた（すぐ終わったとき）
      handleExit(connection);
      return;
    }
    client.connect(connection.transport);
    try {
      await client.initializing;
    } catch (e) {
      console.error(tag(), "initialize に失敗しました", e);
      // 終わったことを知らせる知らせが先に届いていれば、handleExit が知らせ済み
      if (current === connection && !connection.exited) {
        logLine(`起動に失敗しました。initialize に失敗しました：${String(e)}`);
        setStatus("failed");
        // 動いたままのサーバーを止め、つなぎ直せるようにする
        connection.closed = true;
        current = null;
        client.disconnect();
        reap();
      }
      return;
    }
    // 読み込みの終わりを知らせないサーバー（clangd・basedpyright）は、initialize が済んだら使える
    if (started.server !== "rust-analyzer" && current === connection) {
      connection.ready = true;
      setStatus("ready");
    }
  }
  if (wanted !== path) {
    return;
  }
  const uri = documentUri(path, started.rootUri, started.package);
  wantedUri = uri;
  // まだ付いていなければ、LSP の拡張を付ける（didOpen が送られる。言語の ID は clangd が見る）
  if (LSPPlugin.get(view)?.uri !== uri) {
    view.dispatch({ effects: lspCompartment.reconfigure(client.plugin(uri, languageIdFor(path))) });
  }
  window.dispatchEvent(new Event(ATTACHED_EVENT));
}

// エディタのタブが切り替わった（または同じタブの状態が入れ替わった）ときに呼ぶ。key は出しているタブのキー。
// lsp が true で key が .rs・C++ のファイル・.py なら、少し待ってから起動を確かめ、LSP の拡張を付ける。
// false（信頼していない作業領域、読み取り専用のタブ）なら、つながない
export function followActiveFile(view: EditorView, key: string | null, lsp: boolean) {
  lastView = view;
  shownKey = key;
  clearTimeout(timer);
  wanted = lsp && key !== null && (key.endsWith(".rs") || isCppPath(key) || isPythonPath(key)) ? key : null;
  wantedUri = null;
  if (wanted === null) {
    return;
  }
  const target = wanted;
  timer = setTimeout(() => {
    queue = queue.then(() => attach(view, target));
  }, SWITCH_DELAY_MS);
}

// LSP の拡張を外した状態を返す（付いていなければそのまま）。タブの状態をエディタに入れる前に使う
export function withoutLsp(state: EditorState): EditorState {
  const extension = lspCompartment.get(state);
  if (extension === undefined || (Array.isArray(extension) && extension.length === 0)) {
    return state;
  }
  return state.update({ effects: lspCompartment.reconfigure([]) }).state;
}

// ファイルを保存したときに呼ぶ。LSP に開いているファイルなら、保存を知らせる。
// rust-analyzer は保存の知らせで検査（cargo check）を行い、その診断を出す（D-121 の既定の設定）
export function notifySaved(path: string) {
  if (path !== wanted || wantedUri === null || !current) {
    return;
  }
  // 送っていない編集を先に送る（保存した内容と、サーバーが知っている内容をそろえる）
  client.sync();
  client.notification("textDocument/didSave", { textDocument: { uri: wantedUri } });
}

// 「言語サーバーを再起動」（メニュー。D-130・D-210）。止めて、開いている .rs・C++ のファイル・.py のパッケージで起動し直す。
// 終わりの知らせには頼らず、こちらで先に切る（古いサーバーの終わりを、新しいサーバーのものと取り違えないように）。
// LSP の拡張はいったん外してから付け直す（付けたままだと、新しいサーバーに didOpen が送られない。【未確認】画面で確かめる）
export function restartLsp() {
  queue = queue.then(async () => {
    logLine("再起動します");
    if (current) {
      current.closed = true;
      current = null;
      client.disconnect();
    }
    const view = lastView;
    if (view) {
      view.dispatch({ effects: lspCompartment.reconfigure([]) });
    }
    wantedUri = null;
    try {
      await lspStop();
    } catch (e) {
      logLine(`止められません：${String(e)}`);
      setStatus("failed");
      return;
    }
    if (view && wanted !== null) {
      await attach(view, wanted);
    } else {
      setStatus("stopped");
    }
  });
}

// 定義へジャンプ（F12）。開いている .rs が LSP につながっていなければ何もしない
export function goToDefinition() {
  if (lastView) {
    jumpToDefinition(lastView);
  }
}

// 参照検索（⇧F12）に出す、今のファイル以外のファイルの数の上限
const MAX_REFERENCE_FILES = 50;

type ReferenceLocation = { uri: string };

function isLocation(value: unknown): value is ReferenceLocation {
  return typeof value === "object" && value !== null && typeof (value as ReferenceLocation).uri === "string";
}

// 参照検索（⇧F12）。ライブラリの一覧は、LSP に開いているファイル（今のタブだけ）の参照しか出せないので、
// 先に参照を要求して、ほかのファイルの参照があれば、そのファイル（作業領域の中のものだけ）をディスクから読んで
// 仮のファイルにしておく。そのあと、ライブラリの findReferences が改めて要求して一覧を出す
export async function findAllReferences() {
  const view = lastView;
  const plugin = view && LSPPlugin.get(view);
  if (!view || !plugin) {
    return;
  }
  const workspace = client.workspace as IdeWorkspace;
  workspace.clearPreviews();
  try {
    client.sync();
    const response = await client.request<unknown, unknown>("textDocument/references", {
      textDocument: { uri: plugin.uri },
      position: plugin.toPosition(view.state.selection.main.head),
      context: { includeDeclaration: true },
    });
    const uris = new Set<string>();
    for (const item of Array.isArray(response) ? response : []) {
      if (isLocation(item) && !workspace.getFile(item.uri)) {
        uris.add(item.uri);
      }
    }
    for (const uri of [...uris].slice(0, MAX_REFERENCE_FILES)) {
      const path = await lspLocate(uri);
      if (path === null) {
        continue;
      }
      const { content } = await readFile(path);
      workspace.addPreview(uri, Text.of(fromFileContent(content).text.split(/\r\n?|\n/)));
    }
  } catch (e) {
    // 読めないファイルがあっても、読めたものだけで一覧を出す
    console.warn(tag(), "参照のあるほかのファイルを読めません", e);
  }
  findReferences(view);
}

// パンくずリストの記号（Phase 18 作業 2。D-216）。開いているタブ（key）が言語サーバーにつながっていれば、
// textDocument/documentSymbol を送り、聞いたときの文書の位置の形で返す。つながっていない（つなぐ前、読み取り専用のタブ、
// 信頼していないフォルダ）・待つ間にタブを替えた・失敗したときは null
export async function documentSymbols(key: string): Promise<{ doc: Text; symbols: OutlineNode[] } | null> {
  const view = lastView;
  const plugin = view && LSPPlugin.get(view);
  if (!view || !plugin || shownKey !== key || !current || current.closed) {
    return null;
  }
  const doc = view.state.doc;
  try {
    // 送っていない編集を先に送る（サーバーが知っている内容を、いまの文書にそろえる）
    client.sync();
    const response = await client.request<unknown, unknown>("textDocument/documentSymbol", {
      textDocument: { uri: plugin.uri },
    });
    if (shownKey !== key || LSPPlugin.get(view)?.uri !== plugin.uri) {
      return null;
    }
    return { doc, symbols: toOutline(response, doc) };
  } catch (e) {
    console.warn(tag(), "記号を読めません", e);
    return null;
  }
}

// 整形（⇧⌥F）。言語サーバーに `textDocument/formatting` を送る。rust-analyzer は rustfmt を呼ぶ（edition は Cargo.toml から）。
// clangd は clang-format の書き方で整形する（.clang-format があればそれ。C++ のコンテストのフォルダには IDE が置く。D-210）。
// 返ってきた変更は userEvent "format" で適用されるので、⌘Z で戻せ、自動保存の対象になる（ライブラリの動作）。
// 開いている .rs が LSP につながっていなければ（読み取り専用のタブ、信頼していないフォルダを含む）何もしない。
// .py は basedpyright が整形しないので、送らずに知らせる（D-225）
export function formatActiveDocument() {
  if (shownKey !== null && isPythonPath(shownKey)) {
    logLine("Python のファイルの整形には対応していません");
    return;
  }
  if (lastView) {
    formatDocument(lastView);
  }
}

type TextEdit = {
  range: { start: { line: number; character: number }; end: { line: number; character: number } };
  newText: string;
};

function isPosition(value: unknown): value is { line: number; character: number } {
  const position = value as { line?: unknown; character?: unknown } | null;
  return typeof position?.line === "number" && typeof position?.character === "number";
}

function isTextEdit(value: unknown): value is TextEdit {
  const edit = value as { range?: { start?: unknown; end?: unknown }; newText?: unknown } | null;
  return typeof edit?.newText === "string" && isPosition(edit.range?.start) && isPosition(edit.range?.end);
}

// ⌘S で、保存の前に整形する（D-135）。path は保存するタブ（開いているタブ）。
// rust-analyzer が使え（読み込みが一度終わっている）、整形の変更が返り、その間に文書が変わらなかったときだけ、
// 変更を適用する（userEvent "format"。⌘Z で戻せる）。それ以外（LSP につながっていない、読み込み前、読み取り専用のタブ、
// 整形に失敗、時間切れ、待つ間に編集した）は、何もせずに戻る。呼び出し側は、そのまま保存を続ける（保存は止めない）。
// 失敗の理由は実行ログに出す
export async function formatBeforeSave(path: string | null): Promise<void> {
  const view = lastView;
  const plugin = view && LSPPlugin.get(view);
  if (!view || !plugin || path === null || shownKey !== path || !current?.ready || current.closed) {
    return;
  }
  // basedpyright は整形しない（D-225）。整形せずに保存する
  if (isPythonPath(path)) {
    return;
  }
  const doc = view.state.doc;
  try {
    // 送っていない編集を先に送る（サーバーが知っている内容を、いまの文書にそろえる）
    client.sync();
    const response = await client.request<unknown, unknown>("textDocument/formatting", {
      options: { tabSize: getIndentUnit(view.state), insertSpaces: true },
      textDocument: { uri: plugin.uri },
    });
    // 待つ間にタブを切り替えた、編集した場合は、古い文書への変更なので適用しない
    if (!Array.isArray(response) || response.length === 0 || shownKey !== path || view.state.doc !== doc) {
      return;
    }
    if (!response.every(isTextEdit)) {
      logLine("整形の結果を読めなかったので、整形せずに保存します");
      return;
    }
    view.dispatch({
      changes: response.map((edit) => ({
        from: plugin.fromPosition(edit.range.start, doc),
        to: plugin.fromPosition(edit.range.end, doc),
        insert: edit.newText,
      })),
      userEvent: "format",
    });
  } catch (e) {
    logLine(`整形できなかったので、整形せずに保存します：${String(e)}`);
  }
}
