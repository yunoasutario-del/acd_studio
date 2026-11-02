// 実行ログの中身と、終わり方の説明（Phase 2 作業 4）
import { exitLabel, type Finish, type LogKind } from "../ipc/run";

// lsp は rust-analyzer の状態・失敗の理由・標準エラー（画面で足す行。Phase 3b-2 作業 3。D-130）
export type LogEntryKind = LogKind | "lsp";

export type LogEntry = { kind: LogEntryKind; text: string };

// 続けて届いた同じ種類の出力は 1 つにまとめる（画面の要素を増やしすぎないように）
export function appendLog(log: LogEntry[], kind: LogEntryKind, text: string): LogEntry[] {
  const last = log[log.length - 1];
  if (last && last.kind === kind) {
    return [...log.slice(0, -1), { kind, text: last.text + text }];
  }
  return [...log, { kind, text }];
}

const SIGNAL_NAMES: Record<number, string> = {
  4: "SIGILL",
  6: "SIGABRT",
  8: "SIGFPE",
  9: "SIGKILL",
  10: "SIGBUS",
  11: "SIGSEGV",
};

// 終わり方を 1 行で説明する（AC2-06・AC2-07・AC2-08・AC2-14）
export function describeFinish(finish: Finish): string {
  switch (finish.kind) {
    case "failed":
      return `実行できませんでした：${finish.message}`;
    case "compileError":
      return "CE：コンパイルエラー（実行していません）";
    case "buildStopped":
      return "ビルドを止めました";
    case "tested": {
      // AC 以外は WA・RE・TLE・MLE・OLE など（D-140）
      const counts = `全 ${finish.total} 件中、AC ${finish.accepted} 件、AC 以外 ${finish.failed} 件、目視確認 ${finish.unjudged} 件`;
      if (finish.interrupted) {
        return `サンプルテストを中断しました（${counts}）`;
      }
      if (finish.failed > 0) {
        return `サンプルテスト：失敗（${counts}）`;
      }
      if (finish.unjudged > 0) {
        return `サンプルテスト：不正解はありません。目視確認が必要なケースがあります（${counts}）`;
      }
      return `サンプルテスト：成功（${counts}）`;
    }
    case "interactiveTested": {
      const counts = `全 ${finish.total} 件中、AC ${finish.accepted} 件、AC 以外 ${finish.failed} 件`;
      if (finish.interrupted) {
        return `対話テストを中断しました（${counts}）`;
      }
      return finish.failed > 0 ? `対話テスト：失敗（${counts}）` : `対話テスト：成功（${counts}）`;
    }
    case "randomTested": {
      // 例：「ランダムテスト：2 回目（seed 101）で WA。random-1 に保存しました」
      if (finish.interrupted) {
        return `ランダムテストを中断しました（${finish.done} / ${finish.total} 回を判定済み）`;
      }
      const failure = finish.failure;
      if (failure) {
        const saved = failure.case
          ? failure.alreadySaved
            ? `同じ入力が ${failure.case} に保存されています`
            : `${failure.case} に保存しました`
          : "保存できませんでした（実行ログを見てください）";
        return `ランダムテスト：${finish.done} 回目（seed ${failure.seed}）で ${failure.verdict}。${saved}`;
      }
      return `ランダムテスト：成功（${finish.total} 回すべて、愚直解と一致しました）`;
    }
    case "submitted":
      // AtCoder への提出（Phase 6。D-148 §2.8）
      if (finish.error) {
        return `${finish.task}：${finish.error}`;
      }
      if (finish.verdict) {
        return `${finish.task}：${finish.verdict}${finish.score === null ? "" : `（${finish.score} 点）`}`;
      }
      return finish.submissionId === null
        ? `${finish.task}：送りました（提出の ID は分かりません）`
        : `${finish.task}：送りました（判定は記録されていません）`;
    case "debugged":
      // デバッグ（Phase 5。D-146）
      if (finish.stopped) {
        return "デバッグを止めました";
      }
      return finish.exitCode === null
        ? "デバッグが終わりました（終了コードは届きませんでした）"
        : `デバッグが終わりました（終了コード ${finish.exitCode}）`;
    case "ran": {
      const time = `実行時間 ${finish.elapsedMs} ms（ローカルの計測）`;
      const outcome = finish.outcome;
      switch (outcome.kind) {
        case "exited":
          if (outcome.code === 0) {
            return `正常に終了しました（${time}）`;
          }
          if (outcome.code === 101) {
            return `終了コード 101 で終了しました。Rust の panic で終わったときの値です（${time}）`;
          }
          return `${exitLabel(outcome)} で終了しました（${time}）`;
        case "signaled": {
          const name = SIGNAL_NAMES[outcome.signal];
          return `シグナル ${outcome.signal}${name ? `（${name}）` : ""} で終了しました（${time}）`;
        }
        case "timedOut":
          return "タイムアウトで止めました";
        case "outputLimit":
          return "出力が上限を超えたので止めました";
        case "stopped":
          return "止めました";
      }
    }
  }
}
