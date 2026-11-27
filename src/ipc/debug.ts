// デバッグのコマンドの呼び出し（Rust 側は src-tauri/src/debug.rs。Phase 5。D-145・D-146）。
// 画面からは決まった操作だけを送る（任意の DAP の要求や LLDB の初期化コマンドは送れない。D-145 の 2）
import { Channel, invoke } from "@tauri-apps/api/core";
import type { RunEvent } from "./run";
import type { Target } from "./workspace";

// アダプターが置いたブレークポイント
export type BreakpointView = {
  id: number | null;
  verified: boolean;
  // アダプターが動かしたあとの行（空行に置いたときなど）
  line: number | null;
  // 作業領域の中のファイルなら、ルートからの相対パス
  path: string | null;
  message: string | null;
};

export type FrameView = {
  id: number;
  // 関数の名前（末尾のハッシュは外してある）
  name: string;
  // ソースの絶対パスと file: の URI（あれば）
  source: string | null;
  sourceUri: string | null;
  // 作業領域の中なら、ルートからの相対パス
  workspacePath: string | null;
  line: number;
  column: number;
};

export type ScopeView = { name: string; variablesReference: number };

export type VariableView = {
  name: string;
  value: string;
  typeName: string | null;
  // 0 でなければ、開いて中を見られる
  variablesReference: number;
};

export type EvaluateView = { result: string; typeName: string | null; variablesReference: number };

export type StepAction = "continue" | "pause" | "next" | "stepIn" | "stepOut";

// Rust 側から届くデバッグの知らせ（RunEvent の type: "debug"）
export type DebugEvent =
  // 止まった。panic で止まったときは reason が "panic"
  | { kind: "stopped"; reason: string; description: string | null; threadId: number | null }
  | { kind: "continued" }
  // プログラムの出力など（標準出力と標準エラーは混ざって stdout で届く）
  | { kind: "output"; category: string; text: string }
  | { kind: "breakpoints"; path: string; breakpoints: BreakpointView[] }
  | { kind: "breakpointChanged"; reason: string; breakpoint: BreakpointView }
  | { kind: "exited"; code: number | null; label?: string | null };

export type SourceBreakpoints = { path: string; lines: number[] };

// デバッグを始める。終わる・止めるまで返らない。止めるのは stopRun（D-146 2.10）。
// caseName は標準入力にするケース（`sample-N`・`random-N`）。渡さなければ input.txt
export function startDebug(
  target: Target,
  breakpoints: SourceBreakpoints[],
  onEvent: (event: RunEvent) => void,
  caseName?: string,
): Promise<void> {
  const channel = new Channel<RunEvent>();
  channel.onmessage = onEvent;
  return invoke<void>("start_debug", {
    target,
    case: caseName ?? null,
    breakpoints,
    onEvent: channel,
  });
}

export function debugStep(action: StepAction): Promise<void> {
  return invoke<void>("debug_step", { action });
}

export function debugSetBreakpoints(path: string, lines: number[]): Promise<BreakpointView[]> {
  return invoke<BreakpointView[]>("debug_set_breakpoints", { path, lines });
}

export function debugStack(): Promise<FrameView[]> {
  return invoke<FrameView[]>("debug_stack");
}

export function debugScopes(frameId: number): Promise<ScopeView[]> {
  return invoke<ScopeView[]>("debug_scopes", { frameId });
}

export function debugVariables(reference: number): Promise<VariableView[]> {
  return invoke<VariableView[]>("debug_variables", { reference });
}

export function debugEvaluate(
  expression: string,
  frameId: number | null,
  context: "repl" | "watch",
): Promise<EvaluateView> {
  return invoke<EvaluateView>("debug_evaluate", { expression, frameId, context });
}
