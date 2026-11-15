// 言語サーバーが終わったときに、実行ログに出す行と状態を決める（実行ログの直し ①②。docs/windows-port-plan.md）。
// - server は、終わった接続のサーバーの名前（今つないでいるサーバーの名前ではない。②）。行頭の [名前] に使う
// - endedByItself は、サーバーが自分で終わったか。Rust 側は、自分で終わったときだけ、Exited の直前に
//   「<名前> が終わりました」の Log を出す（lsp.rs の、終わるのを待つスレッド）。IDE が止めたとき（lsp.rs の stop）は
//   その Log を出さないので false（①）
// - ready は、読み込みが終わったことがあるか。code は終了コード（シグナルで終わったなら null）
export type ExitStatus = "stopped" | "failed";

export type ExitInfo = { ready: boolean; endedByItself: boolean; code: number | null };

export type ExitReport = { line: string; status: ExitStatus };

export function exitReport(server: string, info: ExitInfo): ExitReport {
  if (!info.endedByItself) {
    // IDE が止めた（フォルダを開いた・言語やパッケージを替えた・再起動など）。終了コードは付けない（①）
    return { line: `[${server}] 止めました`, status: "stopped" };
  }
  const code = info.code === null ? "なし" : String(info.code);
  if (info.ready) {
    return { line: `[${server}] 止まりました（終了コード ${code}）`, status: "stopped" };
  }
  return {
    line: `[${server}] 起動に失敗しました。読み込みが終わる前に止まりました（終了コード ${code}）`,
    status: "failed",
  };
}
