// 実行ログの中の「ファイル名:行:列」（例：panic の `src/main.rs:5:10`、コンパイルエラーの `--> src/main.rs:3:5`、
// C++ のコンパイルエラーの `main.cpp:4:5:`。Phase 16 作業 1）を
// 見つけて、押して移動できるようにする（Phase 4a 作業 1。D-137）

export type Location = { file: string; line: number; column: number | null };

// 文字列の一部。location があれば、押して移動できる場所
export type Segment = { text: string; location?: Location };

// ファイル名の文字（相対パスの要素）。絶対パス（先頭が `/`）や `..` は押せる場所にしない
const LOCATION = /((?:[A-Za-z0-9_.-]+\/)*[A-Za-z0-9_.-]+\.(?:rs|cpp|cc|cxx|hpp|hh|h)):(\d+)(?::(\d+))?/g;
// 直前がこれなら、パスの途中なので場所とみなさない（`/rustc/…/core/src/lib.rs:1:1`、`foo_bar.rs` の途中など）
const PATH_CHAR = /[A-Za-z0-9_./-]/;

// Windows（段階 2 S10）：rustc は場所を `src\main.rs:3:5` のように `\` 区切りで出すので、`\` も区切りとして読む。
// 直前が `\` でもパスの途中とみなす（`C:\x\main.rs`、`/rustc/…/library\std\src\io\mod.rs` の途中など）
const LOCATION_WINDOWS =
  /((?:[A-Za-z0-9_.-]+[\\/])*[A-Za-z0-9_.-]+\.(?:rs|cpp|cc|cxx|hpp|hh|h)):(\d+)(?::(\d+))?/g;
const PATH_CHAR_WINDOWS = /[A-Za-z0-9_.\\/-]/;

// IDE が Windows で動いているか（WebView2 の User-Agent。keybindings.ts の IS_MAC と同じ読み方）
export const IS_WINDOWS = /Windows/.test(navigator.userAgent);

// windows は、Windows の区切り（`\`）も読むか（テストでは引数で渡す）。場所のファイル名は `/` 区切りにして返す
export function splitLocations(text: string, windows: boolean = IS_WINDOWS): Segment[] {
  const pattern = windows ? LOCATION_WINDOWS : LOCATION;
  const pathChar = windows ? PATH_CHAR_WINDOWS : PATH_CHAR;
  const segments: Segment[] = [];
  let last = 0;
  for (const match of text.matchAll(pattern)) {
    const start = match.index;
    if (start > 0 && pathChar.test(text[start - 1])) {
      continue;
    }
    const file = windows ? match[1].replaceAll("\\", "/") : match[1];
    if (file.split("/").includes("..")) {
      continue;
    }
    const line = Number(match[2]);
    if (line < 1) {
      continue;
    }
    if (start > last) {
      segments.push({ text: text.slice(last, start) });
    }
    segments.push({
      text: match[0],
      location: { file, line, column: match[3] === undefined ? null : Number(match[3]) },
    });
    last = start + match[0].length;
  }
  if (last < text.length) {
    segments.push({ text: text.slice(last) });
  }
  return segments;
}
