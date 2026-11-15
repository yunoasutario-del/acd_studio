// 言語サーバーに didClose を送ってよいかを決める（実行ログの直し ⑥。docs/windows-port-plan.md）。
// ライブラリ（@codemirror/lsp-client 6.3.0）の disconnect() は transport を消さず、initializing を新しい待ちの Promise にするだけで、
// notification() は transport があれば initializing を待ってから送る。そのため、切れているあいだに didClose を頼むと、
// 次の connect のあと、新しいサーバーに届いてしまう（新しいサーバーはそのファイルを開いていないので、rust-analyzer が
// 「orphan DidCloseTextDocument」を出す）。ここでは、今つながっているサーバーに開いたファイルだけを覚え、
// そのファイルを閉じるときだけ didClose を送る。IdeWorkspace（workspace.ts）が使う
export class ServerFiles {
  // 今サーバーにつながっているか
  #open = false;
  // 今のサーバーに didOpen を送ったファイルの URI
  #opened = new Set<string>();

  // つないだ（workspace.connected）。uris は、そのときに didOpen を送ったファイル（ビューのあるもの）
  connected(uris: Iterable<string>): void {
    this.#open = true;
    this.#opened = new Set(uris);
  }

  // 切れた（workspace.disconnected。ライブラリの disconnect() の最後に呼ばれる）。前のサーバーに開いたものは忘れる
  disconnected(): void {
    this.#open = false;
    this.#opened.clear();
  }

  // ファイルを開いて didOpen を送った（workspace.openFile）。切れているあいだのものは数えない
  // （そのファイルは、次につないだときの connected で数える）
  markOpened(uri: string): void {
    if (this.#open) {
      this.#opened.add(uri);
    }
  }

  // ファイルを閉じる（workspace.closeFile）。今のサーバーに開いたものなら true（didClose を送る）。2 回目からは false
  takeForClose(uri: string): boolean {
    return this.#opened.delete(uri);
  }
}
