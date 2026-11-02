import { useState } from "react";
import { exitLabel, type CaseResult, type Interaction, type Verdict } from "../ipc/run";
import { failedNames } from "./testCases";
import { isDebuggableCase } from "./debugView";

// テスト結果のタブ（Phase 2 作業 5）。ローカルの比較であることを常に示す（D-094）

export type TestState = {
  cases: CaseResult[];
  // 実行中のケース
  running: string | null;
  // 全体の結果（終わったら入る）
  summary: string | null;
  // コンパイルエラーで終わった（CE。D-140）
  compileError?: boolean;
  // この結果を出した処理：サンプルテスト、ランダムテスト（Phase 4b 作業 3。D-143）、対話テスト（Phase 4c 作業 2。D-144）
  mode: "sample" | "random" | "interactive";
  // ランダムテストの今の回（実行中だけ入る）
  progress: { index: number; total: number; seed: number } | null;
};

// 判定の表示（D-140。ユーザーの判断、2026-10-23）。AC は緑、WA・RE・TLE・MLE・OLE・CE は黄の略語で示し、
// 補足を後ろに付ける。目視確認・出力の全体を得られなかった・中断・未実行は、これまでの文言のまま
type VerdictView = { code: string | null; tone: "ac" | "ng" | null; text: string };

function verdictView(verdict: Verdict): VerdictView {
  switch (verdict.kind) {
    case "accepted":
      return { code: "AC", tone: "ac", text: "" };
    case "wrongAnswer":
      return { code: "WA", tone: "ng", text: "" };
    case "unjudged":
      return { code: null, tone: null, text: "比較不可・目視確認" };
    case "runtimeError": {
      const outcome = verdict.outcome;
      const text =
        outcome.kind === "exited"
          ? outcome.code === 101
            ? "終了コード 101（panic）"
            : exitLabel(outcome)
          : outcome.kind === "signaled"
            ? `シグナル ${outcome.signal}`
            : "";
      return { code: "RE", tone: "ng", text };
    }
    case "timedOut":
      return { code: "TLE", tone: "ng", text: "時間の制限を超えたので止めました" };
    case "memoryLimit":
      return { code: "MLE", tone: "ng", text: "メモリの制限を超えました" };
    case "outputLimit":
      return { code: "OLE", tone: "ng", text: "出力が上限を超えた（比較していません）" };
    case "outputIncomplete":
      return { code: null, tone: null, text: "出力の全体を得られなかった（比較していません）" };
    case "interrupted":
      return { code: null, tone: null, text: "中断" };
    case "notRun":
      return { code: null, tone: null, text: "未実行" };
  }
}

export function VerdictBadge({ code, tone }: { code: string; tone: "ac" | "ng" }) {
  return <span className={`verdict-badge verdict-${tone}`}>{code}</span>;
}

function VerdictText({ verdict }: { verdict: Verdict }) {
  const view = verdictView(verdict);
  return (
    <span className="case-verdict">
      {view.code && view.tone && <VerdictBadge code={view.code} tone={view.tone} />}
      {view.text && <span className="case-verdict-text">{view.text}</span>}
    </span>
  );
}

// 実行時間と最大使用メモリ（ローカルの計測）
function measured(result: CaseResult): string | null {
  if (result.elapsedMs === null) {
    return null;
  }
  const memory =
    result.memoryKib === null ? "" : `・${(result.memoryKib / 1024).toFixed(1)} MiB`;
  return `${result.elapsedMs} ms${memory}（ローカルの計測）`;
}

// 名前を指定して実行できるケースか（テストのフォルダの `sample-N`・`random-N`・`interactive-N`、
// 対話テストの入力のファイルがないときの `interactive`）。
// ランダムテストで保存できなかったケース（`seed-N`）は、フォルダにないので実行できない
function isRunnableCase(name: string): boolean {
  return /^(sample|random|interactive)-\d+$/.test(name) || name === "interactive";
}

// 対話テストのケースの中身：ジャッジに渡した入力、解答とジャッジのやり取り（時系列、向きの印付き）、補足、標準エラー（D-144）
function InteractionView({ result, interaction }: { result: CaseResult; interaction: Interaction }) {
  return (
    <div className="interaction">
      {interaction.note && <div className="test-note">{interaction.note}</div>}
      <div className="case-label">ジャッジに渡した入力（interactive-N.in）</div>
      {result.name === "interactive" ? (
        <div className="case-none">なし（interactive-N.in がないので、引数なしで動かしました）</div>
      ) : result.input === "" ? (
        <div className="case-none">（空のファイル）</div>
      ) : (
        <pre className="case-text">{result.input}</pre>
      )}
      <div className="case-label">解答とジャッジのやり取り（時系列）</div>
      {interaction.turns.length === 0 ? (
        <div className="case-none">やり取りはありませんでした</div>
      ) : (
        <div className="exchange">
          {interaction.turns.map((turn, index) => (
            <div key={index} className={`exchange-turn exchange-${turn.from}`}>
              <span className="exchange-arrow">
                {turn.from === "judge" ? "ジャッジ → 解答" : "解答 → ジャッジ"}
              </span>
              <pre className="case-text">{turn.text}</pre>
            </div>
          ))}
          {interaction.turnsOmitted && (
            <div className="case-none">（やり取りが長いので、先頭 256 KiB だけを表示しています）</div>
          )}
        </div>
      )}
      {result.stderr && (
        <>
          <div className="case-label">解答の標準エラー</div>
          <pre className="case-text">{result.stderr}</pre>
        </>
      )}
      {interaction.judgeStderr && (
        <>
          <div className="case-label">ジャッジの標準エラー</div>
          <pre className="case-text">{interaction.judgeStderr}</pre>
        </>
      )}
    </div>
  );
}

type RunProps = {
  // 押せない理由。押せるなら null（D-023）
  blocked: string | null;
  // 名前（`sample-N`）のケースだけ実行する（Phase 4a 作業 2。D-137）
  onRunCases: (names: string[]) => void;
  // そのケースの .in を標準入力にしてデバッグする（Phase 5 作業 3。D-146 2.4）
  onDebugCase: (name: string) => void;
  // このケースの実際の出力を期待出力として保存する（Phase 4a 作業 3。D-137）
  onSaveExpected: (result: CaseResult) => void;
};

// 期待出力と実際の出力を行ごとに並べる。違う行に印を付ける（AC2-03 の差分の表示）。
// 行の比較は表示のためだけで、判定は比較の方式による（D-092・D-093）
function DiffView({ expected, actual }: { expected: string; actual: string }) {
  const expectedLines = expected.split("\n");
  const actualLines = actual.split("\n");
  const count = Math.max(expectedLines.length, actualLines.length);
  const rows = [];
  for (let i = 0; i < count; i++) {
    const e = expectedLines[i];
    const a = actualLines[i];
    const differs = (e ?? "").trimEnd() !== (a ?? "").trimEnd();
    rows.push(
      <tr key={i} className={differs ? "diff-row differs" : "diff-row"}>
        <td className="diff-line">{i + 1}</td>
        <td className="diff-text">{e ?? ""}</td>
        <td className="diff-text">{a ?? ""}</td>
      </tr>,
    );
  }
  return (
    <table className="diff">
      <thead>
        <tr>
          <th />
          <th>期待する出力</th>
          <th>実際の出力</th>
        </tr>
      </thead>
      <tbody>{rows}</tbody>
    </table>
  );
}

// 入力・期待する出力・実際の出力を並べる（Phase 4a 作業 2。D-137。D-025 の案 2 の趣旨）。
// 期待する出力があって正解でないときは、期待と実際を行ごとに並べて違う行に印を付ける。幅が足りないときは折り返す
function CaseColumns({ result }: { result: CaseResult }) {
  const diff = result.expected !== null && result.verdict.kind !== "accepted";
  return (
    <div className="case-columns">
      <div className="case-col">
        <div className="case-label">入力</div>
        <pre className="case-text">{result.input}</pre>
      </div>
      {diff ? (
        <div className="case-col case-col-wide">
          <DiffView expected={result.expected ?? ""} actual={result.actual} />
        </div>
      ) : (
        <>
          <div className="case-col">
            <div className="case-label">期待する出力</div>
            {result.expected !== null ? (
              <pre className="case-text">{result.expected}</pre>
            ) : (
              <div className="case-none">なし（.out がありません）</div>
            )}
          </div>
          <div className="case-col">
            <div className="case-label">実際の出力</div>
            <pre className="case-text">{result.actual}</pre>
          </div>
        </>
      )}
    </div>
  );
}

function CaseView({
  result,
  running,
  blocked,
  onRunCases,
  onDebugCase,
  onSaveExpected,
}: { result: CaseResult; running: boolean } & RunProps) {
  const failed = !["accepted", "unjudged", "notRun"].includes(result.verdict.kind);
  const [open, setOpen] = useState(failed || result.verdict.kind === "unjudged");
  const canOpen = result.verdict.kind !== "notRun" && !running;
  // 保存できない理由：実行できない状態の理由が先、なければこのケースの出力の理由
  const saveBlocked = blocked ?? result.saveBlocked;
  const runBlocked =
    blocked ?? (isRunnableCase(result.name) ? null : "保存していないケースなので実行できません");
  return (
    <li className={`case case-${running ? "running" : result.verdict.kind}`}>
      <div className="case-row">
        <button
          type="button"
          className="case-head"
          aria-expanded={open && canOpen}
          disabled={!canOpen}
          onClick={() => setOpen(!open)}
        >
          <span className="case-name">{result.name}</span>
          {running ? (
            <span className="case-verdict">実行中…</span>
          ) : (
            <VerdictText verdict={result.verdict} />
          )}
          {!running && measured(result) && <span className="case-time">{measured(result)}</span>}
        </button>
        <button
          type="button"
          className="case-action"
          aria-disabled={runBlocked !== null}
          title={runBlocked ?? "このケースだけ実行します"}
          onClick={() => {
            if (runBlocked === null) {
              onRunCases([result.name]);
            }
          }}
        >
          このケースだけ実行
        </button>
        {/* sample-N・random-N だけ（対話テストのケースは、ジャッジとつなげないので置かない。D-146 2.4） */}
        {isDebuggableCase(result.name) && (
          <button
            type="button"
            className="case-action"
            aria-disabled={blocked !== null}
            title={blocked ?? "このケースの入力でデバッグします（F5 と同じ。ブレークポイントで止まります）"}
            onClick={() => {
              if (blocked === null) {
                onDebugCase(result.name);
              }
            }}
          >
            このケースをデバッグ
          </button>
        )}
      </div>
      {open && canOpen && (
        <div className="case-body">
          {result.interaction ? (
            // 対話テストには、期待出力がない（保存のボタンは出さない）
            <InteractionView result={result} interaction={result.interaction} />
          ) : (
            <>
              <div className="case-save">
                <button
                  type="button"
                  className="case-action"
                  aria-disabled={saveBlocked !== null}
                  title={saveBlocked ?? "実際の出力を期待出力（.out）として保存します"}
                  onClick={() => {
                    if (saveBlocked === null) {
                      onSaveExpected(result);
                    }
                  }}
                >
                  出力を期待出力として保存
                </button>
                {saveBlocked && <span className="test-note">{saveBlocked}</span>}
              </div>
              <CaseColumns result={result} />
              {result.stderr && (
                <>
                  <div className="case-label">標準エラー</div>
                  <pre className="case-text">{result.stderr}</pre>
                </>
              )}
            </>
          )}
        </div>
      )}
    </li>
  );
}

function TestResults({
  state,
  blocked,
  onRunCases,
  onDebugCase,
  onSaveExpected,
  notice,
}: { state: TestState; notice: string | null } & RunProps) {
  const failed = failedNames(state.cases);
  const random = state.mode === "random";
  // 押せない理由：実行できない状態の理由が先、なければ失敗したケースがないこと
  const retryBlocked = blocked ?? (failed.length === 0 ? "失敗したケースがありません" : null);
  return (
    <div className="test-results">
      <div className="test-note">
        {random
          ? "愚直解の出力と比べた、ローカルの結果です。AtCoder の判定と同じとは限りません。"
          : state.mode === "interactive"
            ? "解答とジャッジをつないだ、ローカルの結果です。AtCoder の判定と同じとは限りません。"
            : "ローカルの比較の結果です。AtCoder の判定と同じとは限りません。"}
      </div>
      {/* ビルドの直後の 1 回目の起動は、遅くなることがあるため（原因は推測。P2-05-6） */}
      <div className="test-note">
        ビルドの直後の 1 件目は、OS の検査などで実行時間が長く出ることがあります。
      </div>
      {state.compileError ? (
        <div className="test-summary">
          <VerdictBadge code="CE" tone="ng" />
          <span className="case-verdict-text">コンパイルエラー（実行していません。エラー一覧を見てください）</span>
        </div>
      ) : (
        state.summary && <div className="test-summary">{state.summary}</div>
      )}
      {state.progress && (
        <div className="test-summary">
          {`${state.progress.index} / ${state.progress.total} 回目（seed ${state.progress.seed}）を実行中…`}
        </div>
      )}
      {notice && <div className="test-notice">{notice}</div>}
      {/* ランダムテストでは、失敗は 1 件だけなので、そのケースの「このケースだけ実行」で足りる */}
      {!random && (
        <div className="test-actions">
          <button
            type="button"
            className="case-action"
            aria-disabled={retryBlocked !== null}
            title={retryBlocked ?? "不正解・エラーなどのケースだけを、もう一度実行します"}
            onClick={() => {
              if (retryBlocked === null) {
                onRunCases(failed);
              }
            }}
          >
            失敗したケースだけ再実行
          </button>
          {retryBlocked && <span className="test-note">{retryBlocked}</span>}
        </div>
      )}
      <ul className="cases">
        {state.cases.map((result) => (
          <CaseView
            // 入れ替わったケースは、開閉の状態を引き継がず、新しい結果で開き直す
            key={`${result.name}:${result.verdict.kind}:${result.elapsedMs}`}
            result={result}
            running={state.running === result.name}
            blocked={blocked}
            onRunCases={onRunCases}
            onDebugCase={onDebugCase}
            onSaveExpected={onSaveExpected}
          />
        ))}
        {state.running && !state.cases.some((result) => result.name === state.running) && (
          <li className="case">
            <div className="case-head">
              <span className="case-name">{state.running}</span>
              <span className="case-verdict">実行中…</span>
            </div>
          </li>
        )}
      </ul>
    </div>
  );
}

export default TestResults;
