// LSP の文書の URI の比べ方（workspace.ts の getFile。Windows 移植の段階 4 T8）。
// パーセントエンコードの違い（%e3 と %E3、%3A と : など）で別のファイルと見なさない。
// windows は、IDE が Windows で動いているか（テストでは引数で渡す）。Windows では、ドライブ名の大文字・小文字も
// 見分けない（こちらの URI は Rust の file_uri が作る file:///C:/… の形。rust-analyzer は file:///c:/…、basedpyright は
// file:///c%3A/… の形で返した。段階 4 T5・T7 の本物のサーバーのテスト）。ドライブ名より後ろの大文字・小文字は区別する
export function canonicalUri(uri: string, windows: boolean): string {
  let text: string;
  try {
    text = decodeURIComponent(uri);
  } catch {
    text = uri;
  }
  if (windows) {
    return text.replace(/^file:\/\/\/([a-zA-Z]):/, (_, drive: string) => `file:///${drive.toUpperCase()}:`);
  }
  return text;
}
