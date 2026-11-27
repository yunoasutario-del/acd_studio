import { useCallback, useEffect, useRef, useState } from "react";
import "./styles/tokens.css";
import "./App.css";
import Splitter from "./layout/Splitter";
import {
  ACTIVITYBAR_WIDTH,
  BOTTOM_PANEL_RATIO,
  BOTTOM_RIGHT_PANEL_RATIO,
  CENTER_MIN,
  PANEL_MIN_HEIGHT,
  RIGHT_COLUMN_MIN,
  RIGHT_COLUMN_WIDTH,
  SIDEBAR_MIN,
  SIDEBAR_WIDTH,
  clamp,
  fitColumns,
  shouldCollapse,
} from "./layout/sizes";
import { findCommand, setKeyBindings } from "./keybindings";
import { listen } from "@tauri-apps/api/event";
import {
  cancelClose,
  finishClose,
  loadAppState,
  openSettingsFile,
  reloadSettings,
  saveAppState,
  DEFAULT_PROBLEM_SETTINGS,
  DEFAULT_PYTHON_SETTINGS,
  type Appearance,
  type ProblemSettings,
  type PythonSettings,
  type SettingsView,
  type SubmitSettings,
} from "./ipc/state";
import SettingsDialog from "./settings/SettingsDialog";
import PaneBoundary from "./layout/PaneBoundary";
import type { Command } from "./keybindings";
import { applyAppearance } from "./appearance";
import Editor, {
  remeasureEditor,
  toggleBreakpointAtCursor,
  type Reveal,
  type StoppedLine,
} from "./editor/Editor";
import {
  allBreakpoints,
  clearAllBreakpoints,
  removeBreakpoint,
  renameBreakpoints,
  setBreakpoints,
  subscribeBreakpoints,
} from "./editor/breakpoints";
import FileTree from "./sidebar/FileTree";
import ContestCreator from "./sidebar/ContestCreator";
import SearchView from "./sidebar/SearchView";
import { importVscodeSnippets, openSnippetsFile, reloadSnippets, snippetsFile } from "./ipc/snippets";
import type { SnippetsViews } from "./settings/SettingsDialog";
import { setSnippets } from "./editor/snippets";
import TabBar from "./editor/TabBar";
import Breadcrumbs from "./editor/Breadcrumbs";
import ContestView from "./sidebar/ContestView";
import { useTabs } from "./editor/useTabs";
import {
  askDiscardChanges,
  fileStamps,
  locateSource,
  trustWorkspace,
  openWorkspace,
  resolveTarget,
  saveExpected,
  type Language,
  type Target,
} from "./ipc/workspace";
import BottomPanel, { type PanelTab } from "./panels/BottomPanel";
import ProblemPane from "./panels/ProblemPane";
import ProblemSelector from "./panels/ProblemSelector";
import {
  problemChoice,
  packageProblem,
  setProblemChoice,
  type ProblemId,
} from "./ipc/problem";
import { appendLog, describeFinish, type LogEntry } from "./panels/runLog";
import { mergeCase, summarizeCases, summarizeInteractive } from "./panels/testCases";
import { isReadOnlyKey } from "./editor/readOnlyKey";
import {
  findAllReferences,
  formatActiveDocument,
  formatBeforeSave,
  goToDefinition,
  restartLsp,
  setFileOpener,
  subscribeLsp,
  type LspStatus,
} from "./editor/lsp";
import type { TestState } from "./panels/TestResults";
import {
  startInteractive,
  startRandom,
  startRun,
  stopRun,
  type CaseResult,
  type Diagnostic,
  type RunEvent,
} from "./ipc/run";
import { clearHistory, loadHistory, type HistoryEntry } from "./ipc/history";
import { startDebug, type FrameView, type StepAction } from "./ipc/debug";
import RunButtons from "./panels/RunButtons";
import {
  caseRunBlock,
  debugButtons,
  runButtons,
  submitButton,
  type RunState,
} from "./panels/runState";
import DebugPanel from "./panels/DebugPanel";
import { useDebug } from "./panels/useDebug";
import { debugEvaluate } from "./ipc/debug";
import { appendConsole, consoleKindOf, exitText, type ConsoleEntry, type ConsoleKind } from "./panels/debugView";
import BottomRightPanel from "./panels/BottomRightPanel";
import SubmissionPanel, { upsertSubmission, type SubmitNotice } from "./panels/SubmissionPanel";
import { copyText } from "./panels/copyText";
import { readFile } from "./ipc/workspace";
import {
  atcoderLogin,
  atcoderLogout,
  atcoderSession,
  atcoderSubmissions,
  atcoderSubmit,
  onSession,
  onSubmission,
  openSubmissionPage,
  openSubmitPage,
  refreshSubmission,
  type SessionView,
  type SubmissionView,
} from "./ipc/submit";

// ファイルのアイコン（アイコンのライブラリは使わない。D-031）
function FilesIcon() {
  return (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <path d="M8 3h7l4 4v11H8z" />
      <path d="M15 3v4h4" />
      <path d="M5 7v14h11" />
    </svg>
  );
}

// アクティビティバーの「検索」（Phase 12 作業 1。D-182）。虫眼鏡
function SearchIcon() {
  return (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <circle cx="10.5" cy="10.5" r="6" />
      <path d="M15 15l5.5 5.5" />
    </svg>
  );
}

// アクティビティバーの「実行とデバッグ」（Phase 12 作業 1。D-182）。再生の三角と虫
function DebugIcon() {
  return (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <path d="M5 4l9 6-9 6z" />
      <ellipse cx="17.5" cy="16.5" rx="2.8" ry="3.5" />
      <path d="M14 14.5h-1.5M21 14.5h1.5M14 18.5h-1.5M21 18.5h1.5M17.5 13v7" />
    </svg>
  );
}

// アクティビティバーの「コンテスト」（Phase 18 作業 4。D-218）。旗
function ContestIcon() {
  return (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <path d="M6 21V3" />
      <path d="M6 4h12l-3 4.5 3 4.5H6" />
    </svg>
  );
}

// アクティビティバーの下端の「設定」（Phase 8 作業 6。D-159）。歯車
function SettingsIcon() {
  return (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="3" />
      <path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M5.6 18.4l2.1-2.1M16.3 7.7l2.1-2.1" />
      <circle cx="12" cy="12" r="6.5" />
    </svg>
  );
}

// settings.json の中身（キー割り当て・見た目）を使う（D-052・D-159）。読めなかったものがあれば、知らせの文を返す
function applySettings(settings: SettingsView): string | null {
  const { unknown, duplicates } = setKeyBindings(settings.keyBindings);
  const badBindings = [...settings.invalidKeyBindings, ...unknown];
  const badAppearance = [...settings.appearanceProblems, ...applyAppearance(settings.appearance)];
  // 文字の大きさが変わったときに、エディタの行の高さを測り直す
  remeasureEditor();
  const messages: string[] = [];
  if (badBindings.length > 0) {
    messages.push(
      `settings.json のキー割り当てのうち、読めないものがあったので、初期の割り当てを使います：${badBindings.join("、")}`,
    );
  }
  if (duplicates.length > 0) {
    messages.push(
      `settings.json で、同じキーが 2 つのコマンドに割り当てられています（先のものを使います）：${duplicates.join("、")}`,
    );
  }
  if (badAppearance.length > 0) {
    messages.push(`settings.json の次の項目を読めないので、既定の値を使います：${badAppearance.join("、")}`);
  }
  return messages.length > 0 ? messages.join("\n") : null;
}

const LSP_STATUS_TEXT: Record<Exclude<LspStatus, "none">, string> = {
  starting: "読み込み中",
  ready: "準備完了",
  stopped: "停止",
  failed: "起動に失敗",
};

// 横幅を変えられる列（ファイル一覧・右列）の状態
type ColumnState = { width: number; visible: boolean };
// サイドバーの中身（Phase 12 作業 1。D-182）。エクスプローラー（ファイル一覧）・検索・実行とデバッグ・
// コンテスト（Phase 18 作業 4。D-218）
type SidebarView = "files" | "search" | "debug" | "contest";
const SIDEBAR_VIEWS: { view: SidebarView; label: string }[] = [
  { view: "files", label: "エクスプローラー" },
  { view: "search", label: "検索" },
  { view: "debug", label: "実行とデバッグ" },
  { view: "contest", label: "コンテスト" },
];
function SidebarIcon({ view }: { view: SidebarView }) {
  switch (view) {
    case "files":
      return <FilesIcon />;
    case "search":
      return <SearchIcon />;
    case "debug":
      return <DebugIcon />;
    case "contest":
      return <ContestIcon />;
  }
}
function sidebarViewOf(value: string): SidebarView {
  return SIDEBAR_VIEWS.find((item) => item.view === value)?.view ?? "files";
}
// 上下に分けた列の下側のペイン（下部パネル・右下パネル）の状態。ratio は列の高さに対する割合
type PanelState = { ratio: number; visible: boolean };

// 境界線の位置から、下側のペインの高さを決める。下限の半分より小さければ折りたたむ
function panelFromDrag(column: HTMLElement, clientY: number): PanelState | null {
  const rect = column.getBoundingClientRect();
  if (rect.height <= 0) {
    return null;
  }
  const height = rect.bottom - clientY;
  if (shouldCollapse(height, PANEL_MIN_HEIGHT)) {
    return null;
  }
  const clamped = clamp(height, PANEL_MIN_HEIGHT, rect.height - PANEL_MIN_HEIGHT);
  return { ratio: clamped / rect.height, visible: true };
}

function App() {
  const [sidebar, setSidebar] = useState<ColumnState>({
    width: SIDEBAR_WIDTH,
    visible: true,
  });
  const [sidebarView, setSidebarView] = useState<SidebarView>("files");
  // 言語ごとの、今読めているスニペットと、ファイルを読めなかったときの理由（設定の画面の「スニペット」に出す。D-187・D-213）
  const [snippetsView, setSnippetsView] = useState<SnippetsViews>({
    rust: { loaded: null, error: null },
    cpp: { loaded: null, error: null },
    python: { loaded: null, error: null },
  });
  // 「検索」を出すように頼んだ回数と、そのときエディタで選んでいた文字（Phase 12 作業 2。D-183）
  const [searchRequest, setSearchRequest] = useState<{ seq: number; text: string | null }>({ seq: 0, text: null });
  const [rightColumn, setRightColumn] = useState<ColumnState>({
    width: RIGHT_COLUMN_WIDTH,
    visible: true,
  });
  const [bottomPanel, setBottomPanel] = useState<PanelState>({
    ratio: BOTTOM_PANEL_RATIO,
    visible: true,
  });
  const [bottomRightPanel, setBottomRightPanel] = useState<PanelState>({
    ratio: BOTTOM_RIGHT_PANEL_RATIO,
    visible: true,
  });

  const centerColumnRef = useRef<HTMLDivElement>(null);
  const rightColumnRef = useRef<HTMLDivElement>(null);

  // 作業領域とファイル（作業 4）、タブ（作業 4b）
  const [workspaceName, setWorkspaceName] = useState<string | null>(null);
  // フォルダを開いた回数。ファイル一覧を、同じ名前の別のフォルダでも作り直すのに使う（D-166）
  const [workspaceSeq, setWorkspaceSeq] = useState(0);
  // 作業領域を信頼しているか（D-128）。信頼していなければ、rust-analyzer・実行・サンプルテストを使わない
  // （Rust 側でも拒否する）。「信頼しない」も記録し、次からは聞かない。サイドバーの上の「信頼する…」で信頼し直せる（D-172）
  const [trusted, setTrusted] = useState(false);
  // フォルダを開いていて、信頼していない（フォルダを開く前は、信頼の問題ではない）
  const untrusted = workspaceName !== null && !trusted;
  const [error, setError] = useState<string | null>(null);
  // 今使っている見た目（settings.json。D-159）と、設定の画面（作業 6b）。settingsSeq は、
  // 窓に戻って settings.json を読み直したときに進め、開いている設定の画面に読み直させる
  const [appearance, setAppearance] = useState<Appearance | null>(null);
  // 提出の設定（D-162）。画面は「提出の前のサンプルテスト」を動かすかと、設定の画面に使う。送るかは Rust 側が決める
  const [submitSettings, setSubmitSettings] = useState<SubmitSettings>({
    testBeforeSubmit: true,
    confirmDialog: true,
  });
  // 問題文の欄の設定（Phase 15 作業 3。D-196・D-197）
  const [problemSettings, setProblemSettings] = useState<ProblemSettings>(DEFAULT_PROBLEM_SETTINGS);
  // Python の設定（Phase 19 作業 3。D-221）。設定の画面に使う。動かすときは Rust 側が読み直す
  const [pythonSettings, setPythonSettings] = useState<PythonSettings>(DEFAULT_PYTHON_SETTINGS);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsSeq, setSettingsSeq] = useState(0);
  const settingsOpenRef = useRef(false);
  settingsOpenRef.current = settingsOpen;
  // ファイルの作成・名前の変更・削除の後に、一覧を読み直すきっかけ（作業 4c）
  const [treeRefreshKey, setTreeRefreshKey] = useState(0);
  const showError = useCallback((message: string) => setError(message), []);
  const tabs = useTabs(showError);
  const activeTab = tabs.tabs.find((tab) => tab.path === tabs.activePath) ?? null;

  // 実行の状態（D-013）
  const [runState, setRunState] = useState<RunState>("idle");
  // 下部パネル（作業 4）
  const [panelTab, setPanelTab] = useState<PanelTab>("log");
  const [runLog, setRunLog] = useState<LogEntry[] | null>(null);
  const [diagnostics, setDiagnostics] = useState<Diagnostic[] | null>(null);
  const [testState, setTestState] = useState<TestState | null>(null);
  // 最後に実行した対象。エラー一覧・実行ログの場所は、この対象のパッケージから探す（Phase 4a 作業 1）
  const [runTarget, setRunTarget] = useState<Target | null>(null);
  // サンプルテストの結果を出した対象。「このケースだけ実行」などは、今開いているファイルではなく、この対象で行う
  // （Phase 4a 作業 2。D-137）
  const [testTarget, setTestTarget] = useState<Target | null>(null);
  // 「出力を期待出力として保存」の結果の知らせ（テスト結果の上に出す。次の実行で消す）
  const [testNotice, setTestNotice] = useState<string | null>(null);
  // 今の作業領域の実行履歴（新しい順。Phase 4a 作業 4。D-137）。作業領域を開いていなければ null
  const [history, setHistory] = useState<HistoryEntry[] | null>(null);
  // エラー一覧・実行ログの場所へ移る要求（Editor に渡す）
  const [reveal, setReveal] = useState<Reveal | null>(null);
  const revealSeq = useRef(0);
  // デバッグコンソール（Phase 5 作業 3。D-146 2.8）。null ならまだデバッグしていない
  const [consoleEntries, setConsoleEntries] = useState<ConsoleEntry[] | null>(null);
  const addConsole = useCallback(
    (kind: ConsoleKind, text: string) =>
      setConsoleEntries((entries) => appendConsole(entries ?? [], kind, text)),
    [],
  );
  // デバッグ（Phase 5 作業 2・3。D-146）。止まったフレームの場所をエディタに出し、panic・操作の失敗はデバッグコンソールに出す
  const debug = useDebug({
    onOpenFrame: (frame) => void openFrame(frame),
    onLog: (text) => addConsole("info", text),
  });
  // ブレークポイントの一覧（サイドバーの「実行とデバッグ」に出す。D-182）
  const [breakpointList, setBreakpointList] = useState(allBreakpoints);
  useEffect(() => subscribeBreakpoints(() => setBreakpointList(allBreakpoints())), []);
  // 言語サーバー（rust-analyzer・clangd・basedpyright）の状態（D-130・D-210・D-225）。状態の変化と、実行ログに足す行は editor/lsp.ts から届く
  const [lspStatus, setLspStatus] = useState<LspStatus>("none");
  const [lspServer, setLspServer] = useState("rust-analyzer");
  useEffect(
    () =>
      subscribeLsp((event) => {
        if (event.kind === "status") {
          setLspStatus(event.status);
          setLspServer(event.server);
        } else {
          setRunLog((log) => appendLog(log ?? [], "lsp", event.text));
        }
      }),
    [],
  );
  // 実行の対象（D-088）。開いているファイルから決まらなければ、直前の対象を使い続ける（D-088 の 3）
  const [target, setTarget] = useState<Target | null>(null);
  // 問題（D-117・D-155）：パッケージのフォルダから決まればそれ、決まらなければ選択欄で選んだもの
  const [folderProblem, setFolderProblem] = useState<ProblemId | null>(null);
  const [chosenProblem, setChosenProblem] = useState<ProblemId | null>(null);
  // フォルダから決まった問題との紐付けを外している間（D-178）。外している間に選んだ問題は、パッケージに覚えない。
  // 対象のパッケージが変わる・作業領域を開き直すと、フォルダの問題に戻る（覚えておかない）
  const [unlinked, setUnlinked] = useState(false);
  const [unlinkedProblem, setUnlinkedProblem] = useState<ProblemId | null>(null);
  const currentProblem =
    folderProblem !== null && unlinked ? (unlinkedProblem ?? folderProblem) : (folderProblem ?? chosenProblem);
  // 対象のパッケージが変わったら、フォルダから決まる問題と、そのパッケージで前に選んだ問題を読む
  // （フォルダから決まれば、そちらを使う）。読み終わるまで（読めなければそのまま）、前のパッケージの問題は使わない
  useEffect(() => {
    if (!target) {
      return;
    }
    let cancelled = false;
    setFolderProblem(null);
    setChosenProblem(null);
    setUnlinked(false);
    setUnlinkedProblem(null);
    packageProblem(target.package)
      .then((id) => {
        if (!cancelled) {
          setFolderProblem(id);
        }
      })
      .catch(() => {});
    problemChoice(target.package)
      .then((choice) => {
        if (!cancelled) {
          setChosenProblem(choice);
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [target?.package]);

  // 選択欄で問題を選んだ。対象のパッケージがあれば、そのパッケージに覚える（D-117）。
  // 紐付けを外している間は覚えない（D-178）
  function chooseProblem(id: ProblemId) {
    if (folderProblem !== null && unlinked) {
      setUnlinkedProblem(id);
      return;
    }
    setChosenProblem(id);
    if (target) {
      setProblemChoice(target.package, id).catch((e) => showError(String(e)));
    }
  }
  // サイドバーの「コンテスト」で問題を押した（Phase 18 作業 4。D-218）。フォルダから決まった問題と紐付いているときは、
  // 選択欄の「紐付けを外す」と同じく、紐付けを外してその問題を出す（D-178）
  function showProblem(id: ProblemId) {
    if (currentProblem?.contest === id.contest && currentProblem.task === id.task) {
      return;
    }
    if (folderProblem !== null && !unlinked) {
      setUnlinked(true);
      setUnlinkedProblem(id);
      return;
    }
    chooseProblem(id);
  }
  // サイドバーの「コンテスト」で問題を押した。その問題のフォルダ（<コンテスト>/<問題 ID>/）に
  // src/main.rs・main.cpp・main.py があれば、それを開く（開けば、問題はフォルダから決まる。D-117）。
  // なければ（作業領域を開いていないときも）、問題文の欄でその問題を選ぶ
  async function openContestTask(id: ProblemId) {
    const folder = `${id.contest}/${id.task}`;
    const candidates = [`${folder}/src/main.rs`, `${folder}/main.cpp`, `${folder}/main.py`];
    const stamps = await fileStamps(candidates).catch(() => null);
    const index = stamps?.findIndex((stamp) => stamp !== null) ?? -1;
    if (index >= 0) {
      await handleOpenFile(candidates[index]);
      return;
    }
    showProblem(id);
  }
  const buttons = runButtons(runState, target !== null, !untrusted);

  // 提出（Phase 6 作業 3。D-148）。ログインの状態、このアプリで出した提出、送らなかったときの知らせ
  const [session, setSession] = useState<SessionView | null>(null);
  const [submissions, setSubmissions] = useState<SubmissionView[]>([]);
  const [submitNotice, setSubmitNotice] = useState<SubmitNotice | null>(null);
  const [copyNote, setCopyNote] = useState<string | null>(null);
  // 提出を送っている間（フォームの読み取り・確認のダイアログ・送信）。押した直後に同期的に立てる
  const [sending, setSending] = useState(false);
  const sendingRef = useRef(false);
  const submitView = submitButton(runState, target !== null, !untrusted, currentProblem !== null, sending);
  // デバッグで止まって選んでいるフレームの行（エディタで強調する。D-146 2.6）
  const selectedFrame =
    debug.view.phase === "stopped"
      ? debug.view.frames.find((frame) => frame.id === debug.view.selectedFrameId)
      : undefined;
  const stoppedKey = selectedFrame ? (selectedFrame.workspacePath ?? selectedFrame.sourceUri) : null;
  const stoppedLine: StoppedLine | null =
    selectedFrame && stoppedKey ? { key: stoppedKey, line: selectedFrame.line } : null;
  // 状態の文字（D-130）。信頼していないフォルダでは言語サーバーを動かさないので、その理由を出す（D-128）
  const lspLabel: { text: string; failed: boolean } | null = untrusted
    ? { text: "言語サーバー：停止（フォルダを信頼していません）", failed: false }
    : lspStatus === "none"
      ? null
      : {
          text: `${lspServer}：${LSP_STATUS_TEXT[lspStatus]}`,
          failed: lspStatus === "failed",
        };
  // C++・Python はパッケージのフォルダの名前に「（C++）」「（Python）」を添える（ルートのパッケージなら main.cpp・main.py。
  // D-204・D-220）
  const targetLabel = target
    ? target.language === "cpp"
      ? `${target.package.split("/").pop() || "main.cpp"}（C++）`
      : target.language === "python"
        ? `${target.package.split("/").pop() || "main.py"}（Python）`
      : target.package === "" || target.package.split("/").pop() === target.bin
        ? target.bin
        : `${target.package} の ${target.bin}`
    : null;
  useEffect(() => {
    const path = tabs.activePath;
    // 信頼していないフォルダでは、cargo を動かして対象を決めない（D-128）。
    // 読み取り専用のタブ（作業領域の外のソース）からは決めない（直前の対象を使い続ける）
    if (!path || untrusted || isReadOnlyKey(path)) {
      return;
    }
    let cancelled = false;
    resolveTarget(path)
      .then((resolved) => {
        if (!cancelled && resolved) {
          setTarget(resolved);
        }
      })
      // 決められないとき（cargo が見つからない、Cargo.toml が壊れているなど）は理由を表示し、
      // 直前の対象を使い続ける
      .catch((e) => {
        if (!cancelled) {
          showError(String(e));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [tabs.activePath, untrusted, showError]);

  // 実行を始めてから終わるまで true。押した直後に同期的に立てる（二重に押しても 2 つ始めないように）
  const runningRef = useRef(false);
  // 保存している間に「停止」が押されたら true（保存の後に始めない）
  const stopRequestedRef = useRef(false);

  // 「実行」「サンプルテスト」「ランダムテスト」「対話テスト」「デバッグ」ボタン（kind がどれか）。
  // 実行中は、押したボタンが「停止」なので、止める（D-009・D-013）。
  // cases を渡すと、その名前（`sample-N`・`random-N`）のケースだけをテストし直す（結果の一部を入れ替える。Phase 4a 作業 2。D-137）。
  // ランダムテスト（Phase 4b 作業 3。D-143）は、結果を「テスト結果」のタブに出す。
  // デバッグ（Phase 5 作業 2。D-146）は、dev でビルドして lldb-dap で動かす。出力と経過は実行ログに出す
  // debugCase は、デバッグの標準入力にするケース（テスト結果の「このケースをデバッグ」。D-146 2.4）
  async function handleStart(
    kind: "run" | "test" | "random" | "interactive" | "debug",
    cases?: string[],
    debugCase?: string,
  ) {
    const test = kind === "test" || kind === "random" || kind === "interactive";
    if (runningRef.current) {
      stopRequestedRef.current = true;
      void stopRun();
      return;
    }
    // 一部のケースの再実行と、ケースを選んだデバッグは、その結果を出した対象で行う
    const runOn = cases || debugCase ? testTarget : target;
    if (runState !== "idle" || !runOn || (cases && kind !== "test" && kind !== "interactive")) {
      return;
    }
    // 結果のケースの一覧（一部のケースの再実行では、前の結果に入れ替えていく）
    let merged = cases ? (testState?.cases ?? []) : [];
    runningRef.current = true;
    stopRequestedRef.current = false;
    setRunState(
      kind === "run"
        ? "running"
        : kind === "test"
          ? "testing"
          : kind === "random"
            ? "randomTesting"
            : kind === "interactive"
              ? "interactiveTesting"
              : "debugging",
    );
    // 未保存のタブを保存してから始める（D-089）。保存できないか、その間に止められたら始めない
    if (!(await tabs.saveAll()) || stopRequestedRef.current) {
      runningRef.current = false;
      setRunState("idle");
      return;
    }
    setRunLog([]);
    setDiagnostics(null);
    setTestNotice(null);
    setRunTarget(runOn);
    if (kind === "debug") {
      debug.begin();
      setConsoleEntries([]);
      // サイドバーを「実行とデバッグ」にする（止まったところの変数を見られるように。D-182）
      setSidebarView("debug");
      setSidebar((current) => ({ ...current, visible: true }));
    }
    if (test) {
      setTestTarget(runOn);
      // 一部のケースだけなら、ほかのケースの結果は残す
      setTestState((state) => ({
        cases: cases && state ? state.cases : [],
        running: null,
        summary: null,
        mode: kind === "random" ? "random" : kind === "interactive" ? "interactive" : "sample",
        progress: null,
      }));
    }
    // デバッグを始めると、デバッグコンソールを前に出す（D-146 2.8）
    setPanelTab(test ? "results" : kind === "debug" ? "debugConsole" : "log");
    setBottomPanel((current) => ({ ...current, visible: true }));
    const onEvent = (event: RunEvent) => {
      if (event.type === "log") {
        setRunLog((log) => appendLog(log ?? [], event.kind, event.text));
      } else if (event.type === "diagnostics") {
        setDiagnostics(event.items);
      } else if (event.type === "caseStarted") {
        setTestState((state) => state && { ...state, running: event.name });
      } else if (event.type === "randomProgress") {
        const progress = { index: event.index, total: event.total, seed: event.seed };
        setTestState((state) => state && { ...state, progress });
      } else if (event.type === "debug") {
        debug.onEvent(event.event);
        // プログラムの出力（標準出力と標準エラーは混ざって届く）と終わり方は、デバッグコンソールに出す（D-146 2.8）
        const notice = event.event;
        if (notice.kind === "output") {
          addConsole(consoleKindOf(notice.category), notice.text);
        } else if (notice.kind === "exited") {
          addConsole("info", `\nプログラムが終わりました（${exitText(notice.code, notice.label) ?? "終了コード 不明"}）\n`);
        }
      } else if (event.type === "case") {
        merged = mergeCase(merged, event.result);
        const next = merged;
        setTestState((state) => state && { ...state, running: null, cases: next });
      } else {
        // 一部のケースだけ実行したときは、実行した件数ではなく、全ケースの結果から数え直す
        const finish = event.finish;
        const summary = describeFinish(
          cases && finish.kind === "tested"
            ? summarizeCases(merged)
            : cases && finish.kind === "interactiveTested"
              ? summarizeInteractive(merged)
              : finish,
        );
        setRunLog((log) => appendLog(log ?? [], "info", `\n${summary}\n`));
        if (test) {
          const compileError = finish.kind === "compileError";
          setTestState(
            (state) => state && { ...state, running: null, progress: null, summary, compileError },
          );
        }
        if (event.finish.kind === "compileError") {
          setPanelTab("errors");
        } else if (event.finish.kind === "failed" || event.finish.kind === "buildStopped") {
          // テストケースがない、ビルドに失敗したなどの理由は実行ログにある
          setPanelTab("log");
        }
      }
    };
    try {
      if (kind === "random") {
        await startRandom(runOn, onEvent);
      } else if (kind === "debug") {
        const breakpoints = allBreakpoints().map(({ path, entries }) => ({
          path,
          lines: entries.map((entry) => entry.line),
        }));
        await startDebug(runOn, breakpoints, onEvent, debugCase);
      } else if (kind === "interactive") {
        await startInteractive(runOn, onEvent, cases);
      } else {
        await startRun(runOn, test, onEvent, cases);
      }
    } catch (e) {
      setRunLog((log) => appendLog(log ?? [], "info", `\n実行できませんでした：${String(e)}\n`));
    } finally {
      runningRef.current = false;
      setRunState("idle");
      if (kind === "debug") {
        debug.end();
      }
      // 失敗などで終わりのイベントが届かなかったときも、「実行中」の表示を消す
      setTestState((state) => (state?.progress ? { ...state, progress: null } : state));
      // 結果は Rust 側が履歴に残したので、読み直す
      void refreshHistory();
    }
  }

  // 「提出結果」を出す（右列・右下パネルを閉じていれば開く。右下は提出結果だけ。D-182）
  function showSubmissions() {
    setRightColumn((current) => ({ ...current, visible: true }));
    setBottomRightPanel((current) => ({ ...current, visible: true }));
  }

  // 「提出」ボタン・ショートカット（D-148 §2.2〜§2.5）。未保存のタブを保存してから（D-089）、Rust 側に頼む。
  // 開催期間・ログイン・フォームを確かめ、確認のダイアログを出して送るのは Rust 側
  async function handleSubmit() {
    const runOn = target;
    const problem = currentProblem;
    if (sendingRef.current || runningRef.current || !runOn || !problem || submitView.disabledReason !== null) {
      return;
    }
    // 提出の前のサンプルテスト（D-162）。テスト結果のタブに出る。通ったかは Rust 側が実行履歴で確かめ、
    // 通っていなければ送らない（ケースがなければ、テストをせずに確認のダイアログを出す）
    if (submitSettings.testBeforeSubmit) {
      await handleStart("test");
      if (stopRequestedRef.current) {
        showSubmissions();
        setSubmitNotice({ kind: "error", message: "サンプルテストを止めたので、提出しませんでした" });
        return;
      }
    }
    sendingRef.current = true;
    setSending(true);
    setSubmitNotice(null);
    setCopyNote(null);
    showSubmissions();
    try {
      if (!(await tabs.saveAll())) {
        setSubmitNotice({ kind: "error", message: "保存できなかったファイルがあるので、送りませんでした" });
        return;
      }
      const outcome = await atcoderSubmit(runOn, problem);
      if (outcome.kind === "sent") {
        setSubmissions((list) => upsertSubmission(list, outcome.submission));
        if (outcome.dialogSkipped !== undefined) {
          const total = outcome.dialogSkipped;
          setRunLog((log) =>
            appendLog(log ?? [], "info", `\n確認のダイアログを省いて提出しました（サンプルテスト 全 ${total} 件 AC）\n`),
          );
        }
        // Rust 側が履歴に残したので、読み直す
        void refreshHistory();
      } else {
        setSubmitNotice({ kind: "outcome", outcome, problem });
      }
    } catch (e) {
      setSubmitNotice({ kind: "error", message: String(e) });
    } finally {
      sendingRef.current = false;
      setSending(false);
    }
  }

  // 「コードをコピーして公式の提出ページを開く」（期間の外・送らなかったとき。D-148 §2.5）。
  // 保存してから、ディスクのファイルの中身をコピーする
  async function handleCopyAndOpen(file: string, problem: { contest: string; task: string }) {
    setCopyNote(null);
    try {
      if (!(await tabs.saveAll())) {
        setCopyNote("保存できなかったファイルがあるので、コピーしませんでした");
        return;
      }
      const { content } = await readFile(file);
      if (!(await copyText(content))) {
        setCopyNote("コピーできませんでした");
        return;
      }
      await openSubmitPage(problem);
      setCopyNote("コードをコピーし、提出のページをブラウザで開きました");
    } catch (e) {
      setCopyNote(String(e));
    }
  }

  async function handleLogin() {
    try {
      await atcoderLogin();
    } catch (e) {
      showError(String(e));
    }
  }

  async function handleLogout() {
    try {
      setSession(await atcoderLogout());
    } catch (e) {
      showError(String(e));
    }
  }

  async function handleRefreshSubmission(key: number) {
    try {
      const view = await refreshSubmission(key);
      setSubmissions((list) => upsertSubmission(list, view));
    } catch (e) {
      showError(String(e));
    }
  }

  function handleOpenSubmission(contest: string, submissionId: number | null) {
    openSubmissionPage(contest, submissionId).catch((e) => showError(String(e)));
  }

  // ログインの状態と、このアプリで出した提出（画面を読み込み直しても、Rust 側が覚えている）を読む。
  // そのあとは、Rust 側からのイベントで変える。判定が決まったら、履歴を読み直す（Rust 側が判定を書き足した）
  const refreshHistoryRef = useRef(refreshHistory);
  refreshHistoryRef.current = refreshHistory;
  useEffect(() => {
    atcoderSession()
      .then(setSession)
      .catch(() => setSession({ loggedIn: false, userName: null, error: null }));
    atcoderSubmissions()
      .then((list) => setSubmissions((current) => list.reduce(upsertSubmission, current)))
      .catch(() => {});
    const stopSession = onSession(setSession);
    const stopSubmission = onSubmission((view) => {
      setSubmissions((list) => upsertSubmission(list, view));
      if (view.state === "done") {
        void refreshHistoryRef.current();
      }
    });
    return () => {
      void stopSession.then((stop) => stop());
      void stopSubmission.then((stop) => stop());
    };
  }, []);

  // 今の作業領域の実行履歴を読み直す。読めなければ、理由を表示して空にする
  async function refreshHistory() {
    try {
      setHistory(await loadHistory());
    } catch (e) {
      setHistory([]);
      showError(`実行履歴を読めません：${String(e)}`);
    }
  }

  // 「履歴を消去」。確認の画面は Rust 側で出す
  async function handleClearHistory() {
    try {
      if (await clearHistory()) {
        await refreshHistory();
      }
    } catch (e) {
      showError(String(e));
    }
  }

  // エラー一覧の項目・実行ログの場所（ファイル名:行:列）を押したとき、そのファイルをタブで開いて、場所へ移る。
  // 作業領域の外（標準ライブラリなど）や見つからないファイルは、理由を表示する（Phase 4a 作業 1。D-137）
  async function handleOpenLocation(file: string, line: number, column: number | null) {
    try {
      const path = await locateSource((runTarget ?? target)?.package ?? "", file);
      if (path === null) {
        showError(
          `作業領域の中に見つからないファイルです：${file}（作業領域の外のファイルは、ここからは開けません）`,
        );
        return;
      }
      setError(null);
      await tabs.open(path);
      if (tabs.stateOf(path) === undefined) {
        return;
      }
      revealSeq.current += 1;
      setReveal({ key: path, line, column, seq: revealSeq.current });
    } catch (e) {
      showError(String(e));
    }
  }

  // デバッグコンソールで打った式・コマンドを、選んでいるフレームで評価する（D-145 の 3・D-146 2.8）
  async function handleEvaluate(expression: string) {
    addConsole("input", expression);
    try {
      const answer = await debugEvaluate(expression, debug.view.selectedFrameId, "repl");
      addConsole("result", answer.result.endsWith("\n") ? answer.result : `${answer.result}\n`);
    } catch (e) {
      addConsole("error", `${String(e)}\n`);
    }
  }

  // デバッグで止まったフレームの場所を開く（D-146 2.6）。作業領域の中は通常のタブ、外（標準ライブラリなど）は
  // ファイルがあれば読み取り専用のタブ（D-133）。開けなければ開かない
  async function openFrame(frame: FrameView) {
    try {
      let key: string | null = null;
      if (frame.workspacePath) {
        await tabs.open(frame.workspacePath);
        key = tabs.stateOf(frame.workspacePath) ? frame.workspacePath : null;
      } else if (frame.sourceUri) {
        key = (await tabs.openReadOnly(frame.sourceUri)) === null ? frame.sourceUri : null;
      }
      if (key) {
        revealSeq.current += 1;
        setReveal({ key, line: frame.line, column: frame.column > 0 ? frame.column : null, seq: revealSeq.current });
      }
    } catch (e) {
      showError(String(e));
    }
  }

  // ブレークポイントの一覧で押した行を開く
  async function openBreakpoint(path: string, line: number) {
    setError(null);
    await tabs.open(path);
    if (tabs.stateOf(path)) {
      revealSeq.current += 1;
      setReveal({ key: path, line, column: null, seq: revealSeq.current });
    }
  }

  // テスト結果のケースの実際の出力を、期待出力（sample-N.out）として保存する（Phase 4a 作業 3。D-137）。
  // 保存先は、そのテスト結果を出した対象のパッケージの中。上書きの確認は Rust 側で出す
  async function handleSaveExpected(result: CaseResult) {
    if (!testTarget) {
      return;
    }
    const folder = testTarget.package;
    const path = folder === "" ? result.outFile : `${folder}/${result.outFile}`;
    const name = result.outFile.slice(result.outFile.lastIndexOf("/") + 1);
    try {
      const saved = await saveExpected(path, result.actual);
      setTestNotice(
        saved === "cancelled"
          ? `${name} の上書きをキャンセルしました`
          : saved === "unchanged"
            ? `${name} はすでに同じ内容です`
            : `${name} に保存しました。サンプルテストを実行すると反映されます`,
      );
      if (saved === "written") {
        setTreeRefreshKey((key) => key + 1);
      }
    } catch (e) {
      showError(String(e));
    }
  }

  // 信頼し直す（確認の画面は Rust 側。D-172）
  async function handleTrust() {
    try {
      if (await trustWorkspace()) {
        setTrusted(true);
      }
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleOpenWorkspace() {
    // 実行・テスト・デバッグ・提出の間は開き直さない（Rust 側でも、問題を作っている間を含めて断る。D-166）
    if (runningRef.current || sendingRef.current) {
      setError("実行・テスト・デバッグ・提出の間は、別のフォルダを開けません");
      return;
    }
    // 別のフォルダを開くとタブをすべて閉じる。未保存のタブは先に保存し、保存できなければ、
    // 変更を捨てるかを聞く（捨てないなら開き直さない。D-164）
    const failed = await tabs.saveAllReporting();
    if (failed.length > 0 && !(await askDiscardChanges(failed, "folder"))) {
      return;
    }
    try {
      const opened = await openWorkspace();
      if (opened !== null) {
        if (!(await tabs.closeAll(failed.length > 0))) {
          return;
        }
        setWorkspaceName(opened.name);
        setWorkspaceSeq((seq) => seq + 1);
        setTrusted(opened.trusted);
        void refreshHistory();
        // 別のフォルダでは、前のフォルダの対象・テスト結果・エラー一覧・問題・ブレークポイントを使わない
        // （「このケースだけ実行」などが、新しいフォルダの同じ相対パスに対して動かないように。D-166）
        setTarget(null);
        setRunTarget(null);
        setTestState(null);
        setTestTarget(null);
        setDiagnostics(null);
        setFolderProblem(null);
        setChosenProblem(null);
        setUnlinked(false);
        setUnlinkedProblem(null);
        clearAllBreakpoints();
        setError(null);
      }
    } catch (e) {
      setError(String(e));
    }
  }

  // 起動したときに、保存してあったレイアウト・キー割り当て・前回の作業領域とタブを戻す（作業 6）
  const [restored, setRestored] = useState(false);
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const startup = await loadAppState();
        if (cancelled) {
          return;
        }
        const settingsProblem = applySettings(startup.settings);
        setAppearance(startup.settings.appearance);
        setSubmitSettings(startup.settings.submit);
        setProblemSettings(startup.settings.problem);
        setPythonSettings(startup.settings.python);
        if (settingsProblem) {
          setError(settingsProblem);
        }
        void reloadSnippetsFile(true);
        if (startup.layout) {
          const l = startup.layout;
          setSidebar({ width: l.sidebarWidth, visible: l.sidebarVisible });
          setSidebarView(sidebarViewOf(l.sidebarView));
          setRightColumn({ width: l.rightWidth, visible: l.rightVisible });
          setBottomPanel({ ratio: l.bottomRatio, visible: l.bottomVisible });
          setBottomRightPanel({ ratio: l.bottomRightRatio, visible: l.bottomRightVisible });
        }
        if (startup.workspaceName) {
          setWorkspaceName(startup.workspaceName);
          setTrusted(startup.workspaceTrusted);
          void refreshHistory();
          // 開いていたタブを順に開き直す。なくなったファイルは開かない
          for (const path of startup.tabs) {
            await tabs.open(path, { quiet: true });
          }
          if (startup.activeTab && tabs.stateOf(startup.activeTab)) {
            tabs.setActivePath(startup.activeTab);
          }
        }
      } catch (e) {
        setError(String(e));
      } finally {
        if (!cancelled) {
          setRestored(true);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
    // 起動したときに一度だけ行う
  }, []);

  async function handleOpenFile(path: string) {
    setError(null);
    await tabs.open(path);
  }

  // フォルダの名前の変更・削除（Phase 8 作業 4。D-157）。中のファイルのタブ（作業領域の中のもの）と、ブレークポイントのパス。
  // rust-analyzer の起動し直しは、タブの付け替えを画面に描いたあと（エディタが新しいパスを持ってから）に行う
  const [lspRestartRequest, setLspRestartRequest] = useState(0);
  useEffect(() => {
    if (lspRestartRequest > 0) {
      restartRustAnalyzer();
    }
  }, [lspRestartRequest]);
  function tabsUnder(folder: string) {
    return tabs.tabs.map((tab) => tab.path).filter((path) => path.startsWith(`${folder}/`));
  }
  function breakpointsUnder(folder: string) {
    return allBreakpoints()
      .map((item) => item.path)
      .filter((path) => path.startsWith(`${folder}/`));
  }
  // 名前を変える前に、中のタブを保存する。1 つでも保存できなければ変えない
  async function prepareRenameFolder(folder: string): Promise<boolean> {
    for (const path of tabsUnder(folder)) {
      if (!(await tabs.prepareRename(path))) {
        return false;
      }
    }
    return true;
  }
  // 中のタブ・ブレークポイントを新しいパスに付け替える。タブがあれば rust-analyzer を起動し直す（新しい場所で開き直す）
  function handleRenamedFolder(from: string, to: string) {
    const moved = tabsUnder(from);
    for (const path of moved) {
      tabs.renamePath(path, to + path.slice(from.length));
    }
    for (const path of breakpointsUnder(from)) {
      renameBreakpoints(path, to + path.slice(from.length));
    }
    if (moved.length > 0) {
      setLspRestartRequest((count) => count + 1);
    }
  }
  // 中のタブを閉じ、ブレークポイントを消す。タブがあれば rust-analyzer を起動し直す
  function handleDeletedFolder(folder: string) {
    const closed = tabsUnder(folder);
    for (const path of closed) {
      tabs.discard(path);
    }
    for (const path of breakpointsUnder(folder)) {
      setBreakpoints(path, [], "user");
    }
    if (closed.length > 0) {
      setLspRestartRequest((count) => count + 1);
    }
  }

  // 「コンテストを作る」を開いている（Phase 7a。D-151）
  const [contestOpen, setContestOpen] = useState(false);
  // 問題のパッケージを作った：ファイル一覧を読み直し、その src/main.rs（C++ なら main.cpp、Python なら main.py）を開く（開けば、問題はフォルダ名から決まる。D-117）
  function handlePackagesCreated(main: string | null) {
    setTreeRefreshKey((key) => key + 1);
    if (main) {
      void handleOpenFile(main);
    }
  }

  function toggleSidebar() {
    setSidebar((current) => ({ ...current, visible: !current.visible }));
  }

  // アクティビティバー・⇧⌘E などで、サイドバーの中身を選ぶ（D-182）。いま出している中身なら、サイドバーを閉じる
  function showSidebarView(view: SidebarView) {
    if (sidebar.visible && sidebarView === view) {
      setSidebar((current) => ({ ...current, visible: false }));
      return;
    }
    setSidebarView(view);
    setSidebar((current) => ({ ...current, visible: true }));
    if (view === "search") {
      // 入力欄に移る。エディタで 1 行の中を選んでいれば、それを探す（VS Code と同じ）
      setSearchRequest((current) => ({ seq: current.seq + 1, text: selectedTextForSearch() }));
    }
  }

  // エディタで選んでいる文字（1 行の中・200 文字まで）。なければ null
  function selectedTextForSearch(): string | null {
    const state = tabs.activePath ? tabs.stateOf(tabs.activePath) : undefined;
    const selection = state?.selection.main;
    if (!state || !selection || selection.empty) {
      return null;
    }
    const text = state.sliceDoc(selection.from, selection.to);
    return text.includes("\n") || text.length > 200 ? null : text;
  }

  // 検索の当たりを押した：そのファイルを開き、その行・列へ移る（D-183）
  async function openSearchMatch(path: string, line: number, column: number) {
    setError(null);
    await tabs.open(path);
    if (tabs.stateOf(path)) {
      revealSeq.current += 1;
      setReveal({ key: path, line, column, seq: revealSeq.current });
    }
  }

  // 置換・元に戻すで書き換えたファイルのうち、開いているタブを読み直す（置換の前に保存してあるので、編集は失われない。D-184）
  async function reloadReplaced(paths: string[]) {
    for (const path of paths) {
      if (tabs.tabs.some((tab) => tab.path === path && !tab.readOnly)) {
        await tabs.reload(path);
      }
    }
  }

  // 探す前に、未保存のタブを保存する（検索はディスクの中身を見る。自動保存〔D-037〕と同じ扱い）
  async function saveBeforeSearch() {
    const failed = await tabs.saveAllReporting();
    if (failed.length > 0) {
      showError(`保存できないタブがあります（検索は保存してある中身で探します）：${failed.join("、")}`);
    }
  }

  // キー割り当て・メニューから呼ぶ操作（D-052・D-083）
  function runCommand(command: Command) {
    if (command === "toggleSidebar") {
      toggleSidebar();
    } else if (command === "save") {
      // ⌘S：整形してから、すぐ保存する（D-135）。整形できなければ、整形せずに保存する
      void (async () => {
        await formatBeforeSave(tabs.activePath);
        await tabs.saveActive();
      })();
    } else if (command === "closeTab") {
      // 開いているタブがなければ何もしない
      if (tabs.activePath) {
        setError(null);
        void tabs.close(tabs.activePath);
      }
    } else if (command === "openFolder") {
      void handleOpenWorkspace();
    } else if (command === "run" || command === "test") {
      // 待機中だけ始める。実行中は何もしない（D-101）
      if (!runningRef.current) {
        void handleStart(command === "test" ? "test" : "run");
      }
    } else if (command === "stop") {
      // 通常実行・サンプルテスト・デバッグのどれでも止める（D-101・D-146 2.5）
      if (runningRef.current) {
        stopRequestedRef.current = true;
        void stopRun();
      }
    } else if (command === "debug") {
      // F5：デバッグ中なら続行（動いていれば一時停止）、待機中なら始める（D-146 2.5）
      if (runState === "debugging") {
        if (debug.view.phase === "stopped") {
          debug.step("continue");
        } else if (debug.view.phase === "running") {
          debug.step("pause");
        }
      } else if (!runningRef.current && buttons.debug.disabledReason === null) {
        void handleStart("debug");
      }
    } else if (command === "toggleBreakpoint") {
      // F9：カーソルの行のブレークポイント（D-146 2.5）
      toggleBreakpointAtCursor();
    } else if (command === "stepOver" || command === "stepIn" || command === "stepOut") {
      // F10・F11・⇧F11：止まっているときだけ（D-146 2.5）
      if (runState === "debugging" && debug.view.phase === "stopped") {
        debug.step(command === "stepOver" ? "next" : command);
      }
    } else if (command === "submit") {
      // 提出（初期の割り当てはない。settings.json で割り当てたとき。D-148 §2.2）。ボタンと同じく、送る前の確かめ（D-162）を通る
      if (submitView.disabledReason === null) {
        void handleSubmit();
      }
    } else if (command === "goToDefinition") {
      // 定義へジャンプ（F12。D-133）。LSP につながっていなければ何もしない
      goToDefinition();
    } else if (command === "findReferences") {
      // 参照検索（⇧F12。D-133）
      void findAllReferences();
    } else if (command === "formatDocument") {
      // 整形（⇧⌥F。D-133）
      formatActiveDocument();
    } else if (command === "openSettings") {
      // 設定（⌘,。D-159）
      void openSettings();
    } else if (command === "showExplorer") {
      showSidebarView("files");
    } else if (command === "showSearch") {
      showSidebarView("search");
    } else if (command === "showDebug") {
      showSidebarView("debug");
    }
  }

  // 設定の画面を開く（作業 6b）。開く前に settings.json を読み直し、外での変更を画面に入れる
  async function openSettings() {
    await reloadSettingsOnFocus();
    setSettingsOpen(true);
  }

  // settings.json を既定のテキストエディタで開く（設定の画面の「JSON で開く」）
  function openSettingsJson() {
    openSettingsFile().catch((e) => setError(String(e)));
  }

  // メニューの「rust-analyzer を再起動」（D-130。キー割り当ては置かない）。
  // 信頼していないフォルダでは、rust-analyzer を動かさない（D-128）ので、実行ログに理由を出す
  function restartRustAnalyzer() {
    if (workspaceName === null) {
      return;
    }
    if (untrusted) {
      setRunLog((log) =>
        appendLog(log ?? [], "lsp", "[言語サーバー] フォルダを信頼していないので、再起動できません\n"),
      );
      return;
    }
    restartLsp();
  }

  // 定義ジャンプ・参照の一覧から、別のファイルをタブで開く（editor/workspace.ts）。
  // 作業領域の中は通常のタブ（開けたら true）、外の標準ライブラリ・依存クレートのソースは読み取り専用のタブ
  // （開けたら null、開けなければ理由。D-133）
  const openFileRef = useRef({
    workspace: async (_path: string): Promise<boolean> => false,
    readOnly: async (_uri: string): Promise<string | null> => "開けません",
  });
  openFileRef.current = {
    workspace: async (path) => {
      await tabs.open(path);
      return tabs.stateOf(path) !== undefined;
    },
    readOnly: (uri) => tabs.openReadOnly(uri),
  };
  useEffect(() => {
    setFileOpener({
      workspace: (path) => openFileRef.current.workspace(path),
      readOnly: (uri) => openFileRef.current.readOnly(uri),
    });
  }, []);

  // キーとメニューの処理は最初に一度だけ登録するので、最新の操作を参照で渡す
  const runCommandRef = useRef(runCommand);
  runCommandRef.current = runCommand;
  const restartRef = useRef(restartRustAnalyzer);
  restartRef.current = restartRustAnalyzer;

  // キー割り当て（D-022・D-051・D-052）
  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      // 設定の画面を開いている間は、キー割り当ての操作をしない（画面がキーを受け取る）
      if (settingsOpenRef.current) {
        return;
      }
      const command = findCommand(event);
      if (!command) {
        return;
      }
      event.preventDefault();
      // 押し続けたときの繰り返しでは実行しない
      if (event.repeat) {
        return;
      }
      runCommandRef.current(command);
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  // メニューの「フォルダを開く…」「タブを閉じる」（D-083）。Rust 側から "menu-command" のイベントで届く
  useEffect(() => {
    const unlisten = listen<string>("menu-command", (event) => {
      if (event.payload === "openFolder" || event.payload === "closeTab" || event.payload === "openSettings") {
        runCommandRef.current(event.payload);
      } else if (event.payload === "restartRustAnalyzer") {
        restartRef.current();
      }
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, []);

  // ウィンドウのフォーカスが外れたら、未保存のタブをすべて保存する（D-037）。
  // フォーカスが戻ったら、開いているファイルが外部で変更されていないかを確かめる（D-038）
  // 窓に戻ったときは、開いているファイルが外で変わったかを確かめ（D-038）、ファイル一覧も読み直す（D-156）。
  // settings.json が変わっていれば読み直して、すぐ使う（D-159）
  const refreshOnFocus = () => {
    setTreeRefreshKey((key) => key + 1);
    void reloadSettingsOnFocus();
    void reloadSnippetsFile(false);
    return tabs.checkExternalChanges();
  };

  // 言語ごとのスニペットのファイルを読み直す（Phase 13 作業 1。D-186。言語ごとは D-213、Python は D-227）。force でなければ、
  // 最後に読んだときから変わっていたときだけ。読めない項目は名前つきで知らせ（ほかは使う）、ファイル全体が読めなければ、
  // 前のスニペットのまま知らせる
  async function reloadSnippetsFile(force: boolean) {
    for (const language of ["rust", "cpp", "python"] as const) {
      try {
        const loaded = await reloadSnippets(force, language);
        if (loaded) {
          setSnippets(language, loaded.snippets);
          setSnippetsView((current) => ({ ...current, [language]: { loaded, error: null } }));
          if (loaded.problems.length > 0) {
            setError(
              `${snippetsFile(language)} の読めない項目があります（ほかは使えます）：${loaded.problems.join("、")}`,
            );
          }
        }
      } catch (e) {
        setError(String(e));
        setSnippetsView((current) => ({ ...current, [language]: { ...current[language], error: String(e) } }));
      }
    }
  }

  // VS Code の言語のスニペットを取り込み（確認のダイアログは Rust 側）、読み直す。設定の画面に出す知らせを返す（D-187）
  async function importSnippets(language: Language): Promise<string | null> {
    const done = await importVscodeSnippets(language);
    if (done.cancelled) {
      return null;
    }
    await reloadSnippetsFile(true);
    const parts = [`VS Code のスニペット ${done.count} 件を ${snippetsFile(language)} に取り込みました`];
    if (done.problems > 0) {
      parts.push(`読めない項目が ${done.problems} 個あります`);
    }
    if (done.backup) {
      parts.push(`前の ${snippetsFile(language)} は ${done.backup} に退避しました`);
    }
    return parts.join("。");
  }

  // 読めないときは、今の設定のまま知らせる（退避はしない。D-159）
  async function reloadSettingsOnFocus() {
    try {
      const settings = await reloadSettings();
      if (settings) {
        setError(applySettings(settings));
        setAppearance(settings.appearance);
        setSubmitSettings(settings.submit);
        setProblemSettings(settings.problem);
        setPythonSettings(settings.python);
        setSettingsSeq((seq) => seq + 1);
      }
    } catch (e) {
      setError(String(e));
    }
  }
  const focusHandlersRef = useRef({ blur: tabs.saveAll, focus: refreshOnFocus });
  focusHandlersRef.current = { blur: tabs.saveAll, focus: refreshOnFocus };
  useEffect(() => {
    const onBlur = () => void focusHandlersRef.current.blur();
    const onFocus = () => void focusHandlersRef.current.focus();
    window.addEventListener("blur", onBlur);
    window.addEventListener("focus", onFocus);
    return () => {
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("focus", onFocus);
    };
  }, []);

  // レイアウトと開いているタブを state.json に保存する（作業 6）
  function persistState(): Promise<void> {
    return saveAppState(
      {
        sidebarWidth: sidebar.width,
        sidebarVisible: sidebar.visible,
        rightWidth: rightColumn.width,
        rightVisible: rightColumn.visible,
        bottomRatio: bottomPanel.ratio,
        bottomVisible: bottomPanel.visible,
        bottomRightRatio: bottomRightPanel.ratio,
        bottomRightVisible: bottomRightPanel.visible,
        sidebarView,
      },
      // 読み取り専用のタブ（作業領域の外のソース）は、再起動では戻さない
      tabs.tabs.filter((tab) => !tab.readOnly).map((tab) => tab.path),
      activeTab?.readOnly ? null : tabs.activePath,
    );
  }

  // レイアウト・開いているタブが変わってから 0.5 秒後に保存する（作業 6。ドラッグの途中で書き続けないため）
  const tabPaths = tabs.tabs
    .filter((tab) => !tab.readOnly)
    .map((tab) => tab.path)
    .join("\n");
  useEffect(() => {
    if (!restored) {
      return;
    }
    const timer = setTimeout(() => {
      persistState().catch((e) => setError(String(e)));
    }, 500);
    return () => clearTimeout(timer);
    // persistState は上の値だけを使う
  }, [restored, sidebar, sidebarView, rightColumn, bottomPanel, bottomRightPanel, tabPaths, tabs.activePath, workspaceName]);

  // ウィンドウを閉じる・アプリを終える前に、未保存のタブと state.json を保存する（D-084）。
  // Rust 側が閉じるのを止めて "close-requested" を送ってくる。保存できたら閉じる。保存できないタブがあれば、
  // 変更を捨てて終了するかを聞き、捨てないなら閉じるのをやめる。state.json を書けないだけなら閉じる
  // （レイアウトが次に戻らないだけで、データは失われない。Phase 10 作業 1。D-164）
  const beforeCloseRef = useRef(async () => {});
  beforeCloseRef.current = async () => {
    const failed = await tabs.saveAllReporting();
    try {
      await persistState();
    } catch (e) {
      setError(String(e));
    }
    if (failed.length === 0 || (await askDiscardChanges(failed, "quit"))) {
      await finishClose();
    } else {
      setError("保存できなかったファイルがあるので、閉じるのをやめました");
      await cancelClose();
    }
  };
  useEffect(() => {
    const unlisten = listen("close-requested", () => void beforeCloseRef.current());
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, []);

  // ウィンドウの幅に合わせて、ファイル一覧・右列を細くする（D-017）
  const [windowWidth, setWindowWidth] = useState(window.innerWidth);
  useEffect(() => {
    const onResize = () => setWindowWidth(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  const { sidebarWidth, rightWidth } = fitColumns(windowWidth, sidebar, rightColumn);

  function dragSidebar(clientX: number) {
    const width = clientX - ACTIVITYBAR_WIDTH;
    if (shouldCollapse(width, SIDEBAR_MIN)) {
      setSidebar((current) => ({ ...current, visible: false }));
      return;
    }
    const max = window.innerWidth - ACTIVITYBAR_WIDTH - rightWidth - CENTER_MIN;
    setSidebar({ width: clamp(width, SIDEBAR_MIN, max), visible: true });
  }

  function dragRightColumn(clientX: number) {
    const width = window.innerWidth - clientX;
    if (shouldCollapse(width, RIGHT_COLUMN_MIN)) {
      setRightColumn((current) => ({ ...current, visible: false }));
      return;
    }
    const max = window.innerWidth - ACTIVITYBAR_WIDTH - sidebarWidth - CENTER_MIN;
    setRightColumn({ width: clamp(width, RIGHT_COLUMN_MIN, max), visible: true });
  }

  function dragPanel(
    column: HTMLElement | null,
    clientY: number,
    setPanel: (update: (current: PanelState) => PanelState) => void,
  ) {
    if (!column) {
      return;
    }
    const next = panelFromDrag(column, clientY);
    setPanel((current) => next ?? { ...current, visible: false });
  }

  return (
    <div
      className="app"
      style={{
        gridTemplateColumns: `${ACTIVITYBAR_WIDTH}px ${sidebarWidth}px 1px minmax(0, 1fr) 1px ${rightWidth}px`,
      }}
    >
      {/* アクティビティバー（D-042・D-045）：サイドバーの中身の「エクスプローラー」「検索」「実行とデバッグ」（D-182）・「コンテスト」（D-218）。
          いま出している中身を押すとサイドバーを閉じる。下端に「設定」（D-159） */}
      <nav className="activitybar">
        {SIDEBAR_VIEWS.map(({ view, label }) => {
          const selected = sidebar.visible && sidebarView === view;
          return (
            <button
              key={view}
              type="button"
              className={`activitybar-item ${selected ? "selected" : ""}`}
              title={label}
              aria-label={label}
              aria-pressed={selected}
              onClick={() => showSidebarView(view)}
            >
              <SidebarIcon view={view} />
            </button>
          );
        })}
        <button
          type="button"
          className="activitybar-item activitybar-bottom"
          title="設定"
          aria-label="設定"
          onClick={() => void openSettings()}
        >
          <SettingsIcon />
        </button>
      </nav>

      {/* サイドバー：中身はアクティビティバーで切り替える（D-182）。切り替えても中身（開いたフォルダ・変数など）が
          残るように、どれも置いたままにして、選んでいないものを隠す */}
      <aside className="sidebar" hidden={!sidebar.visible}>
        {untrusted && (
          <div className="trust-banner">
            <span>このフォルダは信頼していないので、言語サーバー（rust-analyzer・clangd・basedpyright）・実行・テストを使いません。</span>
            <button type="button" className="settings-button" onClick={() => void handleTrust()}>
              信頼する…
            </button>
          </div>
        )}
        <div className="sidebar-view" hidden={sidebarView !== "files"}>
        <PaneBoundary name="ファイル一覧">
        {workspaceName ? (
          <FileTree
            key={workspaceSeq}
            workspaceName={workspaceName}
            activePath={tabs.activePath}
            dirtyPaths={tabs.tabs.filter((tab) => tab.dirty).map((tab) => tab.path)}
            refreshKey={treeRefreshKey}
            onOpenFile={handleOpenFile}
            onCreated={handleOpenFile}
            onBeforeRename={tabs.prepareRename}
            onRenamed={(from, to) => {
              tabs.renamePath(from, to);
              // ブレークポイントも新しい名前に付け替える（D-146 2.6）
              renameBreakpoints(from, to);
            }}
            onDeleted={(path) => {
              tabs.discard(path);
              setBreakpoints(path, [], "user");
            }}
            onBeforeRenameFolder={prepareRenameFolder}
            onRenamedFolder={handleRenamedFolder}
            onDeletedFolder={handleDeletedFolder}
            onRefresh={() => setTreeRefreshKey((key) => key + 1)}
            onError={showError}
            onToggleContest={() => setContestOpen((open) => !open)}
            contestPanel={
              <ContestCreator
                hidden={!contestOpen}
                initialContest={currentProblem?.contest ?? ""}
                onCreated={handlePackagesCreated}
                onClose={() => setContestOpen(false)}
              />
            }
          />
        ) : (
          <>
            <div className="sidebar-title">エクスプローラー</div>
            <button type="button" className="open-folder-button" onClick={handleOpenWorkspace}>
              フォルダを開く
            </button>
          </>
        )}
        </PaneBoundary>
        </div>
        <div className="sidebar-view" hidden={sidebarView !== "search"}>
          <PaneBoundary name="検索">
            <SearchView
              key={workspaceSeq}
              workspaceName={workspaceName}
              request={searchRequest}
              onBeforeSearch={saveBeforeSearch}
              onOpenMatch={(path, line, column) => void openSearchMatch(path, line, column)}
              onReplaced={reloadReplaced}
            />
          </PaneBoundary>
        </div>
        <div className="sidebar-view" hidden={sidebarView !== "debug"}>
          <div className="sidebar-title">実行とデバッグ</div>
          <PaneBoundary name="実行とデバッグ">
            <DebugPanel
              view={debug.view}
              watches={debug.watches}
              watchValues={debug.watchValues}
              breakpoints={breakpointList}
              loadChildren={debug.loadChildren}
              onSelectFrame={debug.selectFrame}
              onAddWatch={debug.addWatch}
              onRemoveWatch={debug.removeWatch}
              onOpenBreakpoint={(path, line) => void openBreakpoint(path, line)}
              onRemoveBreakpoint={removeBreakpoint}
            />
          </PaneBoundary>
        </div>
        <div className="sidebar-view" hidden={sidebarView !== "contest"}>
          <PaneBoundary name="コンテスト">
            <ContestView
              contest={currentProblem?.contest ?? null}
              currentTask={currentProblem?.task ?? null}
              visible={sidebar.visible && sidebarView === "contest"}
              loggedIn={session?.loggedIn ?? false}
              submissions={submissions}
              onSelectTask={(id) => void openContestTask(id)}
              onLogin={() => void handleLogin()}
            />
          </PaneBoundary>
        </div>
        {error && (
          <div className="sidebar-error" role="alert">
            {error}
          </div>
        )}
      </aside>

      <Splitter
        orientation="vertical"
        className="splitter-sidebar"
        onDrag={(clientX) => dragSidebar(clientX)}
      />

      {/* ボタンゾーン：問題の選択欄（Phase 3a。D-115）と「実行」「サンプルテスト」（D-044・D-046・D-047・D-100） */}
      <div className="toolbar">
        <PaneBoundary name="ツールバー">
        <ProblemSelector
          current={currentProblem}
          fromFolder={folderProblem !== null && !unlinked}
          onUnlink={folderProblem !== null && !unlinked ? () => setUnlinked(true) : undefined}
          onRelink={
            folderProblem !== null && unlinked
              ? () => {
                  setUnlinked(false);
                  setUnlinkedProblem(null);
                }
              : undefined
          }
          onChoose={chooseProblem}
          onError={showError}
        />
        <RunButtons
          run={buttons.run}
          test={buttons.test}
          random={buttons.random}
          interactive={buttons.interactive}
          debug={buttons.debug}
          submit={submitView}
          submitLabel={currentProblem?.task ?? null}
          debugging={
            runState === "debugging" && debug.view.phase !== null ? debugButtons(debug.view.phase) : null
          }
          targetLabel={targetLabel}
          onRun={() => void handleStart("run")}
          onTest={() => void handleStart("test")}
          onRandom={() => void handleStart("random")}
          onInteractive={() => void handleStart("interactive")}
          onDebug={() => void handleStart("debug")}
          onDebugAction={(action: StepAction) => debug.step(action)}
          onDebugStop={() => {
            stopRequestedRef.current = true;
            void stopRun();
          }}
          onSubmit={() => void handleSubmit()}
        />
        </PaneBoundary>
      </div>

      <div className="center-column" ref={centerColumnRef}>
        {/* エディタ（CodeMirror 6）。ファイルの読み書きは作業 4・5 で加える */}
        <main
          className="editor"
          style={{ flexGrow: bottomPanel.visible ? 1 - bottomPanel.ratio : 1 }}
        >
          {/* サイドバーを閉じているとき（⌘B）も、知らせが見えるように、エディタの上にも出す（Phase 10 作業 7） */}
          {error && !sidebar.visible && (
            <div className="editor-error" role="alert">
              <span className="editor-error-text">{error}</span>
              <button type="button" className="editor-error-close" aria-label="知らせを消す" onClick={() => setError(null)}>
                ×
              </button>
            </div>
          )}
          <PaneBoundary name="エディタ">
          <TabBar
            tabs={tabs.tabs}
            activePath={tabs.activePath}
            onSelect={tabs.setActivePath}
            onClose={(path) => {
              setError(null);
              void tabs.close(path);
            }}
          />
          {activeTab?.readOnly && (
            <div className="readonly-banner" title={activeTab.readOnly.location}>
              読み取り専用：{activeTab.readOnly.location}
            </div>
          )}
          <Breadcrumbs tab={activeTab} onOpenFile={(path) => void handleOpenFile(path)} />
          <Editor
            activeKey={activeTab?.path ?? null}
            stateOf={tabs.stateOf}
            lspEnabled={!untrusted}
            onUpdate={tabs.update}
            reveal={reveal}
            stoppedLine={stoppedLine}
          />
          </PaneBoundary>
        </main>
        <Splitter
          orientation="horizontal"
          onDrag={(_, clientY) =>
            dragPanel(centerColumnRef.current, clientY, setBottomPanel)
          }
        />
        <section
          className="bottom-panel"
          hidden={!bottomPanel.visible}
          style={{ flexGrow: bottomPanel.ratio }}
        >
          <PaneBoundary name="下部パネル">
          <BottomPanel
            selected={panelTab}
            onSelect={setPanelTab}
            targetLabel={targetLabel}
            lspStatus={lspLabel}
            log={runLog}
            diagnostics={diagnostics}
            testState={testState}
            onOpenLocation={(file, line, column) => void handleOpenLocation(file, line, column)}
            onClearLog={() => setRunLog([])}
            caseRunBlocked={caseRunBlock(runState, testTarget !== null, trusted)}
            onRunCases={(names) =>
              void handleStart(testState?.mode === "interactive" ? "interactive" : "test", names)
            }
            onDebugCase={(name) => void handleStart("debug", undefined, name)}
            consoleEntries={consoleEntries}
            consoleBlocked={
              runState !== "debugging"
                ? "デバッグ中に、プログラムが止まっているときだけ使えます"
                : debug.view.phase === "stopped"
                  ? null
                  : "プログラムが止まっているときだけ使えます"
            }
            onEvaluate={(expression) => void handleEvaluate(expression)}
            onSaveExpected={(result) => void handleSaveExpected(result)}
            testNotice={testNotice}
            history={workspaceName === null ? null : history}
            onClearHistory={() => void handleClearHistory()}
            onOpenSubmission={handleOpenSubmission}
            historyClearBlocked={runState === "idle" ? null : "別の処理を実行中です"}
          />
          </PaneBoundary>
        </section>
      </div>

      <Splitter
        orientation="vertical"
        className="splitter-right"
        onDrag={(clientX) => dragRightColumn(clientX)}
      />

      <div className="right-column" ref={rightColumnRef} hidden={!rightColumn.visible}>
        <section
          className="problem"
          style={{ flexGrow: bottomRightPanel.visible ? 1 - bottomRightPanel.ratio : 1 }}
        >
          <PaneBoundary name="問題文">
          <ProblemPane
            id={currentProblem}
            target={target}
            untrusted={untrusted}
            onSamplesSaved={() => setTreeRefreshKey((key) => key + 1)}
            canCreate={folderProblem === null || unlinked}
            onPackageCreated={handlePackagesCreated}
            settings={problemSettings}
          />
          </PaneBoundary>
        </section>
        <Splitter
          orientation="horizontal"
          onDrag={(_, clientY) =>
            dragPanel(rightColumnRef.current, clientY, setBottomRightPanel)
          }
        />
        {/* 右下パネル：「提出結果」（Phase 6 作業 3。D-148 §2.7）だけ。デバッグ情報はサイドバーに置く（Phase 12 作業 1。D-182） */}
        <section
          className="bottom-right-panel"
          hidden={!bottomRightPanel.visible}
          style={{ flexGrow: bottomRightPanel.ratio }}
        >
          <PaneBoundary name="右下パネル">
          <BottomRightPanel judging={submissions.some((submission) => submission.tracking)}>
            <div className="right-panel-body">
              <SubmissionPanel
                session={session}
                submissions={submissions}
                notice={submitNotice}
                copyNote={copyNote}
                onLogin={() => void handleLogin()}
                onLogout={() => void handleLogout()}
                onRefresh={(key) => void handleRefreshSubmission(key)}
                onOpenSubmission={handleOpenSubmission}
                onCopyAndOpen={(file, problem) => void handleCopyAndOpen(file, problem)}
              />
            </div>
          </BottomRightPanel>
          </PaneBoundary>
        </section>
      </div>
      {settingsOpen && appearance && (
        <PaneBoundary name="設定" onClose={() => setSettingsOpen(false)}>
        <SettingsDialog
          appearance={appearance}
          submit={submitSettings}
          problem={problemSettings}
          python={pythonSettings}
          resetSeq={settingsSeq}
          onClose={() => setSettingsOpen(false)}
          onSaved={(saved, submit, problem, interpreter) => {
            setAppearance(saved);
            setSubmitSettings(submit);
            setProblemSettings(problem);
            setPythonSettings((current) => ({ ...current, interpreter }));
          }}
          onReload={reloadSettingsOnFocus}
          onOpenJson={openSettingsJson}
          snippets={snippetsView}
          onOpenSnippets={(language) => openSnippetsFile(language)}
          onImportSnippets={importSnippets}
        />
        </PaneBoundary>
      )}
    </div>
  );
}

export default App;
