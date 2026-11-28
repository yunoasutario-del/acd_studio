// デバッグ情報（原典 §4.8、Phase 5 作業 2。D-146 2.7）。サイドバーの「実行とデバッグ」に置く（Phase 12 作業 1。D-182）。
// 上から「変数」「ウォッチ式」「コールスタック」「ブレークポイント」。止まっているときに中身を出し、
// 動いている間は直前のものを薄くする
import { useState } from "react";
import type { FrameView, VariableView } from "../ipc/debug";
import type { BreakpointEntry } from "../editor/breakpoints";
import type { DebugView, WatchValue } from "./useDebug";
import { failureStatus, failureStop, fileName } from "./debugView";

type Props = {
  view: DebugView;
  watches: string[];
  watchValues: Record<string, WatchValue>;
  breakpoints: { path: string; entries: BreakpointEntry[] }[];
  loadChildren: (reference: number) => Promise<VariableView[]>;
  onSelectFrame: (frame: FrameView) => void;
  onAddWatch: (expression: string) => void;
  onRemoveWatch: (expression: string) => void;
  onOpenBreakpoint: (path: string, line: number) => void;
  onRemoveBreakpoint: (path: string, line: number) => void;
};

// 1 つの変数。中を見られるものは、開いたときに取りに行く
function VariableRow({
  variable,
  depth,
  loadChildren,
}: {
  variable: VariableView;
  depth: number;
  loadChildren: (reference: number) => Promise<VariableView[]>;
}) {
  const [open, setOpen] = useState(false);
  const [children, setChildren] = useState<VariableView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const expandable = variable.variablesReference !== 0;

  async function toggle() {
    if (!expandable) {
      return;
    }
    if (!open && children === null) {
      try {
        setChildren(await loadChildren(variable.variablesReference));
      } catch (e) {
        setError(String(e));
      }
    }
    setOpen(!open);
  }

  return (
    <li>
      <div
        className={`debug-variable ${expandable ? "expandable" : ""}`}
        style={{ paddingLeft: `${depth * 14 + 4}px` }}
        title={variable.typeName ?? undefined}
        onClick={() => void toggle()}
      >
        <span className="debug-twisty" aria-hidden="true">
          {expandable ? (open ? "▾" : "▸") : ""}
        </span>
        <span className="debug-variable-name">{variable.name}</span>
        <span className="debug-variable-value">{variable.value}</span>
        {variable.typeName && <span className="debug-variable-type">{variable.typeName}</span>}
      </div>
      {open && error && (
        <div className="debug-message" style={{ paddingLeft: `${(depth + 1) * 14 + 4}px` }}>
          {error}
        </div>
      )}
      {open && children && (
        <ul className="debug-list">
          {children.map((child, i) => (
            <VariableRow key={`${child.name}-${i}`} variable={child} depth={depth + 1} loadChildren={loadChildren} />
          ))}
        </ul>
      )}
    </li>
  );
}

function DebugPanel({
  view,
  watches,
  watchValues,
  breakpoints,
  loadChildren,
  onSelectFrame,
  onAddWatch,
  onRemoveWatch,
  onOpenBreakpoint,
  onRemoveBreakpoint,
}: Props) {
  const [draft, setDraft] = useState("");
  const stale = view.phase !== "stopped";
  const failure = failureStop(view.stopReason);
  const status =
    view.phase === null
      ? "デバッグしていません。ブレークポイントを置いて「デバッグ」（F5）を押すと、止まったところの変数とコールスタックが出ます。"
      : view.phase === "starting"
        ? "デバッグを始めています…"
        : view.phase === "running"
          ? "プログラムが動いています（下は、前に止まったときの内容です）"
          : (failureStatus(view.stopReason) ?? "止まっています");

  return (
    // タブの列は BottomRightPanel が描く（Phase 6 で「提出結果」を加えた）
    <div className="debug-panel" role="tabpanel">
      <div className="debug-body">
        <div className={`debug-status ${failure !== null && !stale ? "panic" : ""}`}>{status}</div>
        {view.error && <div className="debug-message">{view.error}</div>}

        <section className={`debug-section ${stale ? "stale" : ""}`}>
          <h3>変数</h3>
          {view.locals.length === 0 ? (
            <div className="debug-empty">{view.phase === "stopped" ? "ローカル変数はありません" : "—"}</div>
          ) : (
            <ul className="debug-list" key={`${view.stopSeq}-${view.selectedFrameId}`}>
              {view.locals.map((variable, i) => (
                <VariableRow key={`${variable.name}-${i}`} variable={variable} depth={0} loadChildren={loadChildren} />
              ))}
            </ul>
          )}
        </section>

        <section className={`debug-section ${stale ? "stale" : ""}`}>
          <h3>ウォッチ式</h3>
          <ul className="debug-list">
            {watches.map((expression) => {
              const value = watchValues[expression];
              return (
                <li key={expression} className="debug-watch">
                  <span className="debug-variable-name">{expression}</span>
                  <span className={`debug-variable-value ${value && "error" in value ? "error" : ""}`}>
                    {value === undefined ? "" : "error" in value ? value.error : value.result}
                  </span>
                  <button
                    type="button"
                    className="debug-remove"
                    aria-label={`${expression} を消す`}
                    onClick={() => onRemoveWatch(expression)}
                  >
                    ×
                  </button>
                </li>
              );
            })}
          </ul>
          <form
            className="debug-watch-form"
            onSubmit={(event) => {
              event.preventDefault();
              onAddWatch(draft);
              setDraft("");
            }}
          >
            <input
              type="text"
              value={draft}
              placeholder="式を加える（Enter）"
              aria-label="ウォッチ式を加える"
              onChange={(event) => setDraft(event.target.value)}
            />
          </form>
        </section>

        <section className={`debug-section ${stale ? "stale" : ""}`}>
          <h3>コールスタック</h3>
          {view.frames.length === 0 ? (
            <div className="debug-empty">—</div>
          ) : (
            <ul className="debug-list">
              {view.frames.map((frame) => (
                <li
                  key={frame.id}
                  className={`debug-frame ${frame.workspacePath ? "own" : "other"} ${
                    frame.id === view.selectedFrameId ? "selected" : ""
                  }`}
                  onClick={() => onSelectFrame(frame)}
                  title={frame.source ?? undefined}
                >
                  <span className="debug-frame-name">{frame.name}</span>
                  <span className="debug-frame-place">
                    {frame.source ? `${fileName(frame.source)}:${frame.line}` : ""}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="debug-section">
          <h3>ブレークポイント</h3>
          {breakpoints.length === 0 ? (
            <div className="debug-empty">エディタの行番号の左をクリックするか、F9 で置けます</div>
          ) : (
            <ul className="debug-list">
              {breakpoints.flatMap(({ path, entries }) =>
                entries.map((entry) => (
                  <li
                    key={`${path}:${entry.line}`}
                    className={`debug-breakpoint ${entry.verified === false ? "unverified" : ""}`}
                    title={entry.verified === false ? "この行には止まれません（アダプターが置けませんでした）" : path}
                  >
                    <span className="debug-breakpoint-dot" aria-hidden="true">
                      ●
                    </span>
                    <span className="debug-breakpoint-place" onClick={() => onOpenBreakpoint(path, entry.line)}>
                      {fileName(path)}:{entry.line}
                      <span className="debug-breakpoint-path">{path}</span>
                    </span>
                    <button
                      type="button"
                      className="debug-remove"
                      aria-label={`${path} の ${entry.line} 行のブレークポイントを消す`}
                      onClick={() => onRemoveBreakpoint(path, entry.line)}
                    >
                      ×
                    </button>
                  </li>
                )),
              )}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}

export default DebugPanel;
