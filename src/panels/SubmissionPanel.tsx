// 右下パネルの「提出結果」（原典 §4.8、D-043。Phase 6 作業 3。D-148。Phase 12 作業 1 でタブをなくした。D-182）。
// 上の段にログインの状態、その下に送らなかったときの知らせ（期間の外など）、
// その下に、このアプリを起動してから IDE で出した提出を新しい順に並べる（保存しない。履歴は「履歴」タブ）
import type { ProblemId } from "../ipc/problem";
import type { SessionView, SubmissionView, SubmitOutcome } from "../ipc/submit";
import { IS_WINDOWS } from "./logLinks";
import { logoutTitle } from "./sessionNote";

// 送らなかったときの知らせ。problem は「コードをコピーして公式の提出ページを開く」に使う
export type SubmitNotice =
  | { kind: "outcome"; outcome: Exclude<SubmitOutcome, { kind: "sent" }>; problem: ProblemId }
  | { kind: "error"; message: string };

type Props = {
  // null なら、まだ確かめていない
  session: SessionView | null;
  submissions: SubmissionView[];
  notice: SubmitNotice | null;
  // 「コピーして開く」の結果（例：「コードをコピーしました…」）
  copyNote: string | null;
  onLogin: () => void;
  onLogout: () => void;
  onRefresh: (key: number) => void;
  onOpenSubmission: (contest: string, submissionId: number | null) => void;
  onCopyAndOpen: (file: string, problem: ProblemId) => void;
};

// 一覧の提出を、届いた内容で入れ替える（なければ加える）。新しい順（key の大きい順）に並べる
export function upsertSubmission(list: SubmissionView[], view: SubmissionView): SubmissionView[] {
  return [...list.filter((item) => item.key !== view.key), view].sort((a, b) => b.key - a.key);
}

// 提出の前の検査の注意に添える出典（src-tauri/src/submit.rs の JUDGE_CRATES_NOTE と同じ）
const JUDGE_CRATES_NOTE = "ジャッジのクレートは、AtCoder の 2025/10 版の言語の一覧（2026-10-14 に確認）のものです";

// 例：20:12:05（手元の時刻）
function formatClock(timeMs: number): string {
  const date = new Date(timeMs);
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

function sessionText(session: SessionView | null): string {
  if (!session) {
    return "ログインの状態を確かめています…";
  }
  if (!session.loggedIn) {
    return "ログインしていません";
  }
  return session.userName
    ? `${session.userName} でログイン中`
    : "ログインしています（ユーザー名は、提出のときに確かめます）";
}

// 判定の印。確定した AC は緑、確定したそれ以外は黄（D-140 と同じ）、判定中は灰
function JudgeBadge({ submission }: { submission: SubmissionView }) {
  const judge = submission.judge;
  if (!judge) {
    return null;
  }
  const tone = !judge.waiting ? (judge.label === "AC" ? "ac" : "ng") : "wait";
  return (
    <span className={`verdict-badge verdict-${tone}`} title={judge.title ?? undefined}>
      {judge.label}
    </span>
  );
}

function stateText(submission: SubmissionView): string {
  const stopped = submission.tracking ? "" : "（読むのを止めました）";
  switch (submission.state) {
    case "rejected":
      return "受け付けられませんでした";
    case "unknownId":
      return "送りました（提出の ID が分かりません）";
    case "waiting":
      return `判定待ち${stopped}`;
    case "judging":
      return `判定中${stopped}`;
    case "done":
      return submission.judge?.title ?? "";
    case "unreadable":
      return "取得できませんでした";
  }
}

function SubmissionItem({
  submission,
  onRefresh,
  onOpenSubmission,
}: {
  submission: SubmissionView;
  onRefresh: Props["onRefresh"];
  onOpenSubmission: Props["onOpenSubmission"];
}) {
  const judge = submission.judge;
  const canRefresh =
    submission.submissionId !== null && !submission.tracking && submission.state !== "done";
  return (
    <li className={`submit-item submit-${submission.state}`}>
      <div className="submit-head">
        <span className="submit-task">
          {submission.task}「{submission.taskTitle}」
        </span>
        <span className="submit-time">{formatClock(submission.timeMs)}</span>
      </div>
      <div className="submit-state">
        <JudgeBadge submission={submission} />
        <span>{stateText(submission)}</span>
      </div>
      {judge && submission.state === "done" && (
        <div className="submit-numbers">
          得点 {judge.score ?? "—"} ・ 実行時間 {judge.timeMs === null ? "—" : `${judge.timeMs} ms`} ・ メモリ{" "}
          {judge.memoryKib === null ? "—" : `${judge.memoryKib} KiB`}
        </div>
      )}
      {submission.message && <div className="submit-message">{submission.message}</div>}
      <div className="submit-meta">
        {submission.language} ・ {submission.file}
      </div>
      {submission.submissionId !== null && (
        <div className="submit-meta">ケースごとの結果：取得できません（公式の詳細ページで見られます）</div>
      )}
      <div className="submit-actions">
        {submission.submissionId !== null && (
          <button
            type="button"
            className="case-action"
            title="公式の提出の詳細をブラウザで開きます"
            onClick={() => onOpenSubmission(submission.contest, submission.submissionId)}
          >
            詳細
          </button>
        )}
        <button
          type="button"
          className="case-action"
          title="公式の「自分の提出」をブラウザで開きます"
          onClick={() => onOpenSubmission(submission.contest, null)}
        >
          自分の提出
        </button>
        {canRefresh && (
          <button
            type="button"
            className="case-action"
            title="判定を読み直します"
            onClick={() => onRefresh(submission.key)}
          >
            更新
          </button>
        )}
      </div>
    </li>
  );
}

function NoticeView({
  notice,
  copyNote,
  onLogin,
  onCopyAndOpen,
}: {
  notice: SubmitNotice;
  copyNote: string | null;
  onLogin: () => void;
  onCopyAndOpen: Props["onCopyAndOpen"];
}) {
  if (notice.kind === "error") {
    return <div className="submit-notice failed">提出できませんでした：{notice.message}</div>;
  }
  const outcome = notice.outcome;
  if (outcome.kind === "cancelled") {
    return <div className="submit-notice">キャンセルしました（何も送っていません）</div>;
  }
  const text =
    outcome.kind === "outOfPeriod"
      ? `開催期間の外なので、IDE からは提出できません（期間：${outcome.start} 〜 ${outcome.end}）`
      : outcome.kind === "notLoggedIn"
        ? outcome.message
        : `送りませんでした：${outcome.reason}`;
  return (
    <div className="submit-notice failed">
      <div>{text}</div>
      {outcome.warnings.length > 0 && (
        <>
          <div className="submit-warnings-title">提出の前の検査の注意：</div>
          <ul className="submit-warnings">
            {outcome.warnings.map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
          <div className="submit-meta">（{JUDGE_CRATES_NOTE}）</div>
        </>
      )}
      <div className="submit-actions">
        {outcome.kind === "notLoggedIn" && (
          <button type="button" className="case-action" onClick={onLogin}>
            ログイン
          </button>
        )}
        <button
          type="button"
          className="case-action"
          title={`${outcome.file} をコピーし、${outcome.submitUrl} を開きます`}
          onClick={() => onCopyAndOpen(outcome.file, notice.problem)}
        >
          コードをコピーして公式の提出ページを開く
        </button>
        {copyNote && <span className="test-note">{copyNote}</span>}
      </div>
    </div>
  );
}

function SubmissionPanel({
  session,
  submissions,
  notice,
  copyNote,
  onLogin,
  onLogout,
  onRefresh,
  onOpenSubmission,
  onCopyAndOpen,
}: Props) {
  return (
    <div className="submit-body">
      <div className="submit-session">
        <span>{sessionText(session)}</span>
        {session && (
          <button
            type="button"
            className="case-action"
            title={
              session.loggedIn
                ? logoutTitle(IS_WINDOWS)
                : "AtCoder のログインの窓を開きます（パスワードは IDE に渡りません）"
            }
            onClick={session.loggedIn ? onLogout : onLogin}
          >
            {session.loggedIn ? "ログアウト" : "ログイン"}
          </button>
        )}
      </div>
      {session?.error && <div className="submit-message">{session.error}</div>}
      {notice && (
        <NoticeView notice={notice} copyNote={copyNote} onLogin={onLogin} onCopyAndOpen={onCopyAndOpen} />
      )}
      {submissions.length === 0 ? (
        <div className="panel-empty">まだ提出していません</div>
      ) : (
        <ul className="submit-list">
          {submissions.map((submission) => (
            <SubmissionItem
              key={submission.key}
              submission={submission}
              onRefresh={onRefresh}
              onOpenSubmission={onOpenSubmission}
            />
          ))}
        </ul>
      )}
      <div className="submit-footnote">
        得点・実行時間・メモリは AtCoder のジャッジの値で、手元の実行（サンプルテスト）の値とは違います。
        この一覧は、このアプリを起動してから IDE で出した提出だけです（保存しません。「履歴」タブに残ります）。
      </div>
    </div>
  );
}

export default SubmissionPanel;
