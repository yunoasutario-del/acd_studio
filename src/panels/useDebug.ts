// デバッグ中の画面の状態（Phase 5 作業 2。D-146 2.5〜2.7・2.9）。
// 止まったらスタック・ローカル変数・ウォッチ式を読み、選んだフレームの場所をエディタに出す。
// デバッグ中にブレークポイントを付け外ししたら、そのファイルの分をアダプターに送り直す
import { useEffect, useRef, useState } from "react";
import {
  debugEvaluate,
  debugScopes,
  debugSetBreakpoints,
  debugStack,
  debugStep,
  debugVariables,
  type BreakpointView,
  type DebugEvent,
  type FrameView,
  type StepAction,
  type VariableView,
} from "../ipc/debug";
import {
  breakpointsOf,
  clearVerified,
  setBreakpoints,
  subscribeBreakpoints,
} from "../editor/breakpoints";
import type { DebugPhase } from "./runState";
import { entriesFromViews, failureLog, pickFrame } from "./debugView";

// ウォッチ式の値（評価できなければ error）
export type WatchValue = { result: string; typeName: string | null } | { error: string };

export type DebugView = {
  // デバッグしていなければ null
  phase: DebugPhase | null;
  // 最後に止まったときのスタック（動いている間も、直前のものを薄く出す）
  frames: FrameView[];
  selectedFrameId: number | null;
  locals: VariableView[];
  // 最後に止まった理由（breakpoint・step・panic など）
  stopReason: string | null;
  // 止まったときの情報を読めなかった理由
  error: string | null;
  // 止まった回数。変数の木の開き方は、止まるたびに戻す（参照の番号は止まるたびに変わるため）
  stopSeq: number;
};

const IDLE: DebugView = {
  phase: null,
  frames: [],
  selectedFrameId: null,
  locals: [],
  stopReason: null,
  error: null,
  stopSeq: 0,
};

type Options = {
  // 選んだフレームの場所をエディタに出す
  onOpenFrame: (frame: FrameView) => void;
  // 実行ログに足す（操作の失敗など）
  onLog: (text: string) => void;
};

export function useDebug({ onOpenFrame, onLog }: Options) {
  const [view, setView] = useState<DebugView>(IDLE);
  const [watches, setWatches] = useState<string[]>([]);
  const [watchValues, setWatchValues] = useState<Record<string, WatchValue>>({});
  const activeRef = useRef(false);
  const stopSeqRef = useRef(0);
  // ブレークポイントの id → 今の場所（アダプターが後から動かしたときに合わせる）
  const idsRef = useRef(new Map<number, { path: string; line: number }>());
  const watchesRef = useRef(watches);
  watchesRef.current = watches;
  const optionsRef = useRef({ onOpenFrame, onLog });
  optionsRef.current = { onOpenFrame, onLog };
  const frameRef = useRef<number | null>(null);

  // アダプターの答えを保存の場所に入れる
  function applyViews(path: string, lines: number[], views: BreakpointView[]) {
    const entries = entriesFromViews(lines, views);
    views.forEach((item, i) => {
      if (item.id !== null) {
        idsRef.current.set(item.id, { path, line: entries[i].line });
      }
    });
    setBreakpoints(path, entries, "adapter");
  }

  // デバッグ中にエディタで付け外ししたら、そのファイルの分を送り直す（少し待って、まとめて送る）
  useEffect(() => {
    const timers = new Map<string, ReturnType<typeof setTimeout>>();
    const unsubscribe = subscribeBreakpoints((path, origin) => {
      if (!activeRef.current || origin !== "user") {
        return;
      }
      clearTimeout(timers.get(path));
      timers.set(
        path,
        setTimeout(() => {
          timers.delete(path);
          const lines = breakpointsOf(path).map((entry) => entry.line);
          debugSetBreakpoints(path, lines)
            .then((views) => {
              if (activeRef.current) {
                applyViews(path, lines, views);
              }
            })
            .catch((e) => optionsRef.current.onLog(`ブレークポイントを置けません（${path}）：${String(e)}\n`));
        }, 200),
      );
    });
    return () => {
      unsubscribe();
      for (const timer of timers.values()) {
        clearTimeout(timer);
      }
    };
  }, []);

  // ウォッチ式を、選んでいるフレームで評価する
  async function evaluateWatches(frameId: number | null, seq: number, list = watchesRef.current) {
    const values: Record<string, WatchValue> = {};
    for (const expression of list) {
      try {
        const answer = await debugEvaluate(expression, frameId, "watch");
        values[expression] = { result: answer.result, typeName: answer.typeName };
      } catch (e) {
        values[expression] = { error: String(e) };
      }
    }
    if (seq === stopSeqRef.current) {
      setWatchValues(values);
    }
  }

  // フレームのローカル変数（Locals の範囲）を読む
  async function loadLocals(frameId: number): Promise<VariableView[]> {
    const scopes = await debugScopes(frameId);
    const locals = scopes.find((scope) => scope.name === "Locals") ?? scopes[0];
    return locals ? debugVariables(locals.variablesReference) : [];
  }

  async function showFrame(frame: FrameView, seq: number) {
    frameRef.current = frame.id;
    setView((current) => ({ ...current, selectedFrameId: frame.id }));
    optionsRef.current.onOpenFrame(frame);
    try {
      const locals = await loadLocals(frame.id);
      if (seq === stopSeqRef.current) {
        setView((current) => ({ ...current, locals, error: null }));
      }
    } catch (e) {
      if (seq === stopSeqRef.current) {
        setView((current) => ({ ...current, locals: [], error: String(e) }));
      }
    }
    await evaluateWatches(frame.id, seq);
  }

  async function onStopped(reason: string) {
    stopSeqRef.current += 1;
    const seq = stopSeqRef.current;
    setView((current) => ({
      ...current,
      phase: "stopped",
      stopReason: reason,
      stopSeq: seq,
      error: null,
    }));
    const log = failureLog(reason);
    if (log !== null) {
      optionsRef.current.onLog(`\n${log}\n`);
    }
    try {
      const frames = await debugStack();
      if (seq !== stopSeqRef.current) {
        return;
      }
      setView((current) => ({ ...current, frames }));
      const frame = pickFrame(frames, reason);
      if (frame) {
        await showFrame(frame, seq);
      }
    } catch (e) {
      if (seq === stopSeqRef.current) {
        setView((current) => ({ ...current, error: String(e) }));
      }
    }
  }

  // Rust 側から届くデバッグの知らせ
  function onEvent(event: DebugEvent) {
    switch (event.kind) {
      case "stopped":
        void onStopped(event.reason);
        break;
      case "continued":
        // 動いている間は、直前の情報を薄くして残す（D-146 2.7）
        stopSeqRef.current += 1;
        setView((current) => ({ ...current, phase: "running" }));
        break;
      case "breakpoints":
        applyViews(
          event.path,
          breakpointsOf(event.path).map((entry) => entry.line),
          event.breakpoints,
        );
        break;
      case "breakpointChanged": {
        const id = event.breakpoint.id;
        const known = id === null ? undefined : idsRef.current.get(id);
        if (!known || event.reason === "new") {
          break;
        }
        const entries = breakpointsOf(known.path);
        const moved = event.reason === "removed"
          ? entries.filter((entry) => entry.line !== known.line)
          : entries.map((entry) =>
              entry.line === known.line
                ? { line: event.breakpoint.line ?? entry.line, verified: event.breakpoint.verified }
                : entry,
            );
        if (id !== null && event.reason !== "removed") {
          idsRef.current.set(id, { path: known.path, line: event.breakpoint.line ?? known.line });
        }
        setBreakpoints(known.path, moved, "adapter");
        break;
      }
      case "output":
      case "exited":
        // 出力と終了コードは、呼び出し側が実行ログに出す（デバッグコンソールは作業 3）
        break;
    }
  }

  function begin() {
    activeRef.current = true;
    stopSeqRef.current += 1;
    idsRef.current.clear();
    frameRef.current = null;
    setWatchValues({});
    setView({ ...IDLE, phase: "starting", stopSeq: stopSeqRef.current });
  }

  function end() {
    activeRef.current = false;
    stopSeqRef.current += 1;
    idsRef.current.clear();
    frameRef.current = null;
    clearVerified();
    setView(IDLE);
  }

  function step(action: StepAction) {
    const before = stopSeqRef.current;
    debugStep(action)
      .then(() => {
        // 続行・ステップは、次に止まるまで「動いている」にする（continued が届かないこともあるため）。
        // 答えより先に、次に止まった知らせが届いていたら（止まった回数が変わっていたら）何もしない
        if (action !== "pause" && activeRef.current && stopSeqRef.current === before) {
          stopSeqRef.current += 1;
          setView((current) => (current.phase === "stopped" ? { ...current, phase: "running" } : current));
        }
      })
      .catch((e) => optionsRef.current.onLog(`\n操作できません：${String(e)}\n`));
  }

  // コールスタックのフレームを選ぶ（エディタと変数を切り替える）
  function selectFrame(frame: FrameView) {
    if (view.phase !== "stopped") {
      return;
    }
    void showFrame(frame, stopSeqRef.current);
  }

  function addWatch(expression: string) {
    const text = expression.trim();
    if (text === "" || watches.includes(text)) {
      return;
    }
    const next = [...watches, text];
    setWatches(next);
    if (view.phase === "stopped") {
      void evaluateWatches(frameRef.current, stopSeqRef.current, next);
    }
  }

  function removeWatch(expression: string) {
    setWatches((current) => current.filter((item) => item !== expression));
  }

  return {
    view,
    watches,
    watchValues,
    onEvent,
    begin,
    end,
    step,
    selectFrame,
    addWatch,
    removeWatch,
    loadChildren: debugVariables,
  };
}
