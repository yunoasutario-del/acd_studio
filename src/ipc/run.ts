// 通常実行とサンプルテストのコマンドの呼び出し（Rust 側は src-tauri/src/run.rs）
import { Channel, invoke } from "@tauri-apps/api/core";
import type { DebugEvent } from "./debug";
import type { Target } from "./workspace";

export type LogKind = "info" | "build" | "stdout" | "stderr";

export type Diagnostic = {
  level: string;
  message: string;
  rendered: string;
  file: string | null;
  line: number | null;
};

export type RunOutcome =
  // label は画面に出す文字（Windows の例外の名前など。段階 2 S5）。前の版の実行履歴にはない
  | { kind: "exited"; code: number; label?: string }
  | { kind: "signaled"; signal: number }
  | { kind: "timedOut" }
  | { kind: "outputLimit"; stream: LogKind }
  | { kind: "stopped" };

// 終了コードの表示。Rust が作った label があればそれ、なければ（前の版の実行履歴）今までどおりの形
export function exitLabel(outcome: { code: number; label?: string }): string {
  return outcome.label || `終了コード ${outcome.code}`;
}

export type Finish =
  | { kind: "failed"; message: string }
  | { kind: "compileError" }
  | { kind: "buildStopped" }
  | {
      kind: "tested";
      total: number;
      accepted: number;
      failed: number;
      unjudged: number;
      interrupted: boolean;
      // 実行したケースのうち最も長い実行時間（止めたケースは含めない）。実行履歴に出す（Phase 4a 作業 4）
      maxElapsedMs: number | null;
    }
  | {
      kind: "interactiveTested";
      total: number;
      accepted: number;
      // WA・RE・TLE・MLE・OLE など、AC でないもの
      failed: number;
      interrupted: boolean;
      // 実行したケースのうち最も長い実行時間（止めたケースは含めない）
      maxElapsedMs: number | null;
    }
  | {
      kind: "randomTested";
      // 予定の回数（compare.json の randomCount）
      total: number;
      // 解答を判定した回数（失敗した回を含む。止めた回は含めない）
      done: number;
      // 1 回目の seed。i 回目（0 から）の seed は開始の seed + i
      startSeed: number;
      // 解答が AC でなかった最初の回。なければ null
      failure: RandomFailure | null;
      interrupted: boolean;
      // 解答の実行時間のうち最長（止めた回は含めない）
      maxElapsedMs: number | null;
    }
  // デバッグをした（Phase 5。D-146）。exitCode はプログラムの終了コード（届かなかったら null）、stopped は停止の操作で止めた
  | { kind: "debugged"; exitCode: number | null; stopped: boolean }
  // AtCoder に提出した（Phase 6。D-148 §2.8。実行履歴だけに出る）。verdict・score は判定が決まったら入る、
  // error は送れなかった・受け付けられなかったときの理由
  | {
      kind: "submitted";
      contest: string;
      task: string;
      submissionId: number | null;
      verdict: string | null;
      score: string | null;
      error: string | null;
    }
  | { kind: "ran"; outcome: RunOutcome; elapsedMs: number; outputLeftOpen: boolean };

// ランダムテストで見つかった失敗（Phase 4b 作業 3。D-143）
export type RandomFailure = {
  seed: number;
  // 判定の略語（WA・RE・TLE・MLE・OLE など）
  verdict: string;
  // テストのフォルダに保存したケースの名前（`random-N`）。保存できなかったら null
  case: string | null;
  // 同じ入力がすでに保存されていた（case はその名前）
  alreadySaved: boolean;
};

// サンプルテストのケースの判定
export type Verdict =
  | { kind: "accepted" }
  | { kind: "wrongAnswer" }
  | { kind: "unjudged" }
  | { kind: "runtimeError"; outcome: RunOutcome }
  | { kind: "timedOut" }
  | { kind: "memoryLimit" }
  | { kind: "outputLimit" }
  | { kind: "outputIncomplete" }
  | { kind: "interrupted" }
  | { kind: "notRun" };

export type CaseResult = {
  name: string;
  verdict: Verdict;
  elapsedMs: number | null;
  input: string;
  expected: string | null;
  actual: string;
  stderr: string;
  // 期待出力のファイル（`sample-N.out`）の、パッケージのフォルダからの相対パス
  outFile: string;
  // 実際の出力を期待出力として保存できない理由。保存できるなら null（Phase 4a 作業 3。D-137）
  saveBlocked: string | null;
  // 最大使用メモリ（KiB。ローカルの計測。D-141）。受け取れなかったら null
  memoryKib: number | null;
  // 対話テストの、やり取りなど（Phase 4c。D-144）。サンプルテスト・ランダムテストでは null
  interaction: Interaction | null;
};

export type Interaction = {
  // 時系列で並べたやり取り。同じ側が続けて出力したものは 1 つ
  turns: { from: "solution" | "judge"; text: string }[];
  // やり取りが長いので、省略した
  turnsOmitted: boolean;
  // ジャッジの標準エラー（解答の標準エラーは CaseResult.stderr）
  judgeStderr: string;
  // 判定の補足（ジャッジの終わり方、時間切れの注意など）
  note: string | null;
};

export type RunEvent =
  | { type: "log"; kind: LogKind; text: string }
  | { type: "diagnostics"; items: Diagnostic[] }
  | { type: "caseStarted"; name: string }
  | { type: "case"; result: CaseResult }
  // ランダムテストの index 回目（1 から）を始めた。seed は生成器に渡す値（D-143）
  | { type: "randomProgress"; index: number; total: number; seed: number }
  // デバッグの知らせ（Phase 5。D-146）
  | { type: "debug"; event: DebugEvent }
  | { type: "finished"; finish: Finish };

// ビルドして実行する（test が true ならサンプルテスト）。終わるまで返らない。
// 経過は onEvent で受け取る（09 K-07：チャンネル）。
// cases は、サンプルテストで実行するケースの名前（`sample-N`）。渡さなければ全部（Phase 4a 作業 2。D-137）
export function startRun(
  target: Target,
  test: boolean,
  onEvent: (event: RunEvent) => void,
  cases?: string[],
): Promise<void> {
  const channel = new Channel<RunEvent>();
  channel.onmessage = onEvent;
  return invoke<void>("start_run", { target, test, cases: cases ?? null, onEvent: channel });
}

// 対話テストを始める（Phase 4c 作業 2。D-144）。終わるまで返らない。経過は onEvent で受け取る。
// cases は、実行するケースの名前（`interactive-N`）。渡さなければ全部
export function startInteractive(
  target: Target,
  onEvent: (event: RunEvent) => void,
  cases?: string[],
): Promise<void> {
  const channel = new Channel<RunEvent>();
  channel.onmessage = onEvent;
  return invoke<void>("start_interactive", { target, cases: cases ?? null, onEvent: channel });
}

// ランダムテストを始める（Phase 4b 作業 3。D-143）。終わるまで返らない。経過は onEvent で受け取る
export function startRandom(target: Target, onEvent: (event: RunEvent) => void): Promise<void> {
  const channel = new Channel<RunEvent>();
  channel.onmessage = onEvent;
  return invoke<void>("start_random", { target, onEvent: channel });
}

// 実行中の処理を止める
export function stopRun(): Promise<void> {
  return invoke<void>("stop_run");
}
