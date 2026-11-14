// LSP クライアントが開いているファイルを管理する場所（Phase 3b-2 作業 4。D-133）。
//
// ライブラリの既定（DefaultWorkspace）は公開されておらず、開いているエディタのファイルしか扱えない。
// そのため、次の 2 つが要る。
// - 定義ジャンプで別のファイルへ移る（displayFile）：そのファイルをタブで開き、LSP の拡張が付くまで待つ。
// - 参照の一覧に、今のタブ以外のファイルの参照を出す：ライブラリは、要求の時点で files にないファイルの位置を
//   写せない（mapPosition が例外になる）ので、一覧を出す前に、ディスクの内容の「仮のファイル」を files に入れておく。
//   仮のファイルはビューを持たず、rust-analyzer には didOpen を送らない（実際に開いたときに初めて送る）。
//
// files・syncFiles・openFile・closeFile は、ライブラリの既定の実装（@codemirror/lsp-client 6.3.0）を写したもの。
import { LSPPlugin, Workspace, type WorkspaceFile } from "@codemirror/lsp-client";
import type { Text } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import { IS_WINDOWS } from "../panels/logLinks";
import { canonicalUri } from "./lspUri";

class IdeFile implements WorkspaceFile {
  constructor(
    readonly uri: string,
    readonly languageId: string,
    public version: number,
    public doc: Text,
    private readonly view: EditorView | null,
  ) {}

  getView() {
    return this.view;
  }
}

type FileUpdate = ReturnType<Workspace["syncFiles"]>[number];

// ワークスペースが画面に頼むこと
export type WorkspaceHooks = {
  // uri のファイルをタブで開く。作業領域の中なら "lsp"（LSP の拡張が付く）、作業領域の外の読み取り専用のソースなら
  // "plain"（LSP につながない）。開けなければ null
  open(uri: string): Promise<"lsp" | "plain" | null>;
  // key のタブがエディタに出ていれば、そのエディタ（読み取り専用のタブを待つ）
  shownView(key: string): EditorView | null;
  // 実行ログに 1 行足す
  log(text: string): void;
};

// 開いてから LSP の拡張が付くまで待つ上限（タブの切り替えから起動の確かめまでの待ち時間を含む）
const DISPLAY_TIMEOUT_MS = 10_000;
const POLL_MS = 50;

// URI の比べ方：パーセントエンコードの違い（%e3 と %E3 など）で別のファイルと見なさない。
// Windows では、ドライブ名の大文字・小文字も見分けない（lspUri.ts。Windows 移植の段階 4 T8）
function canonical(uri: string): string {
  return canonicalUri(uri, IS_WINDOWS);
}

export class IdeWorkspace extends Workspace {
  files: IdeFile[] = [];
  private fileVersions: Record<string, number> = Object.create(null);

  constructor(
    client: ConstructorParameters<typeof Workspace>[0],
    private readonly hooks: WorkspaceHooks,
  ) {
    super(client);
  }

  private nextFileVersion(uri: string) {
    return (this.fileVersions[uri] = (this.fileVersions[uri] ?? -1) + 1);
  }

  getFile(uri: string): IdeFile | null {
    const wanted = canonical(uri);
    return this.files.find((file) => file.uri === uri || canonical(file.uri) === wanted) ?? null;
  }

  syncFiles() {
    const result: FileUpdate[] = [];
    for (const file of this.files) {
      const view = file.getView();
      const plugin = view && LSPPlugin.get(view);
      if (!view || !plugin) {
        continue;
      }
      const changes = plugin.unsyncedChanges;
      if (!changes.empty) {
        result.push({ changes, file, prevDoc: file.doc });
        file.doc = view.state.doc;
        file.version = this.nextFileVersion(file.uri);
        plugin.clear();
      }
    }
    return result;
  }

  openFile(uri: string, languageId: string, view: EditorView) {
    // 同じファイルの仮のファイルがあれば捨てる（実際に開くので）
    const known = this.getFile(uri);
    if (known?.getView()) {
      throw new Error("同じファイルを 2 つのエディタで開くことはできません");
    }
    this.files = this.files.filter((file) => file !== known);
    const file = new IdeFile(uri, languageId, this.nextFileVersion(uri), view.state.doc, view);
    this.files.push(file);
    this.client.didOpen(file);
  }

  closeFile(uri: string) {
    const file = this.getFile(uri);
    if (file?.getView()) {
      this.files = this.files.filter((other) => other !== file);
      this.client.didClose(uri);
    }
  }

  // 接続したら、開いているファイル（ビューがあるもの）だけを rust-analyzer に知らせる
  connected() {
    for (const file of this.files) {
      if (file.getView()) {
        this.client.didOpen(file);
      }
    }
  }

  // 仮のファイル（ディスクの内容）を足す。すでにあれば何もしない
  addPreview(uri: string, doc: Text) {
    if (!this.getFile(uri)) {
      this.files.push(new IdeFile(uri, "rust", 0, doc, null));
    }
  }

  clearPreviews() {
    this.files = this.files.filter((file) => file.getView());
  }

  // 別のファイルを見せる（定義ジャンプ、参照の一覧から選んだとき）。開いていなければタブで開き、
  // LSP の拡張が付いて、そのファイルが開かれる（openFile）まで待つ。開けなければ null
  async displayFile(uri: string): Promise<EditorView | null> {
    const open = this.getFile(uri)?.getView();
    if (open) {
      return open;
    }
    const opened = await this.hooks.open(uri);
    if (opened === null) {
      return null;
    }
    // LSP のファイルなら openFile が呼ばれるまで、読み取り専用のタブならそのタブが出るまで待つ
    const find = () =>
      opened === "lsp" ? (this.getFile(uri)?.getView() ?? null) : this.hooks.shownView(uri);
    const until = Date.now() + DISPLAY_TIMEOUT_MS;
    while (Date.now() < until) {
      const view = find();
      if (view) {
        return view;
      }
      await new Promise((resolve) => setTimeout(resolve, POLL_MS));
    }
    this.hooks.log("ファイルを開きましたが、エディタに出るまで待てなかったので、移動できません");
    return null;
  }
}
