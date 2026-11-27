// デバッグの画面のための、小さな純粋な関数（Phase 5 作業 2。D-146 2.6・2.7）
import type { BreakpointView, FrameView } from "../ipc/debug";
import type { BreakpointEntry } from "../editor/breakpoints";

// panic（Rust）・例外を投げた（C++ の "throw"）・abort（C++）・捕まえていない例外（Python の debugpy の "exception"）で
// 止まったときの、画面に出す文の頭（「panic で」など。D-146 2.9・D-211・D-226）。それ以外の理由なら null
export function failureStop(reason: string | null): string | null {
  switch (reason) {
    case "panic":
      return "panic で";
    case "throw":
      return "例外で";
    case "abort":
      return "abort で";
    case "exception":
      return "例外で";
    default:
      return null;
  }
}

// 止まっているときの状態の文（DebugPanel）。panic・例外・abort 以外の理由なら null
// （failureStop の値が「で」まで含むので、ここでは付けない。件 1）
export function failureStatus(reason: string | null): string | null {
  const failure = failureStop(reason);
  return failure === null ? null : `${failure}止まっています`;
}

// 止まったときに実行ログに出す文（useDebug）。panic・例外・abort 以外の理由なら null
export function failureLog(reason: string | null): string | null {
  const failure = failureStop(reason);
  return failure === null ? null : `${failure}止まりました。`;
}

// コールスタックとブレークポイントの一覧に出す、ファイルの名前（DebugPanel.tsx から移した）。Windows の「\」でも区切る（件 5）
export function fileName(path: string): string {
  return path.slice(Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\")) + 1);
}

// 止まったときに選ぶフレーム。panic・例外・abort のときは、作業領域のソースのいちばん上（D-146 2.7。
// 一番上は rust_panic・__cxa_throw・abort などの作業領域の外の関数）。ほかは一番上（作業領域の外でも）
export function pickFrame(frames: FrameView[], reason: string | null): FrameView | null {
  if (failureStop(reason) !== null) {
    return frames.find((frame) => frame.workspacePath !== null) ?? frames[0] ?? null;
  }
  return frames[0] ?? null;
}

// アダプターの答え（送った行の順）を、保存の場所の形にする。動いた行はその行に、置けなかったものは verified: false
export function entriesFromViews(lines: number[], views: BreakpointView[]): BreakpointEntry[] {
  return lines.map((line, i) => ({
    line: views[i]?.line ?? line,
    verified: views[i]?.verified ?? false,
  }));
}

// 「このケースをデバッグ」を置けるケース：テストのフォルダの `sample-N`・`random-N`（D-146 2.4）。
// 対話テストのケース（ジャッジとつなげない）と、保存できなかったケースには置かない
export function isDebuggableCase(name: string): boolean {
  return /^(sample|random)-\d+$/.test(name);
}

// デバッグコンソールの行の種類（D-146 2.8）：プログラムの出力（標準出力と標準エラーが混ざる）、LLDB の警告、
// LLDB の知らせ、打った式、その結果、評価できなかった理由、IDE からの知らせ
export type ConsoleKind = "output" | "important" | "console" | "input" | "result" | "error" | "info";

export type ConsoleEntry = { kind: ConsoleKind; text: string };

// アダプターの出力の種類（DAP の category）を、コンソールの行の種類にする
export function consoleKindOf(category: string): ConsoleKind {
  if (category === "stdout" || category === "stderr") {
    return "output";
  }
  return category === "important" ? "important" : "console";
}

// 続けて届いた同じ種類の出力は 1 つにまとめる（実行ログの appendLog と同じ）。打った式と結果はまとめない
export function appendConsole(entries: ConsoleEntry[], kind: ConsoleKind, text: string): ConsoleEntry[] {
  const last = entries[entries.length - 1];
  if (last && last.kind === kind && kind !== "input" && kind !== "result" && kind !== "error") {
    return [...entries.slice(0, -1), { kind, text: last.text + text }];
  }
  return [...entries, { kind, text }];
}
