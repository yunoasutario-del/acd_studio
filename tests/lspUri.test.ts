// LSP の文書の URI の比べ方（src/editor/lspUri.ts。Windows 移植の段階 4 T8）のテスト。
// 動かし方は logLinks.test.ts と同じ（node --test）
import { test } from "node:test";
import assert from "node:assert/strict";
import { canonicalUri } from "../src/editor/lspUri.ts";

const same = (a: string, b: string, windows: boolean) => canonicalUri(a, windows) === canonicalUri(b, windows);

// こちらの側の URI：Rust の file_uri（段階 4 T2）が作る rootUri に、画面が相対パスを足した形
// （ドライブ名は大文字、: は符号にしない、空白は %20、日本語は大文字の %E…）
const ours = "file:///C:/T/ws/%E6%97%A5%E6%9C%AC%20%E8%AA%9E.rs";

test("Windows：本物のサーバーが返した 3 つの形（clangd・rust-analyzer・basedpyright）が、こちらの URI と合う", () => {
  // 合わないものを全部集めてから比べる（どの形が合わないかが、全部分かるように）
  const mismatched = [

    // clangd（段階 4 T6 の本物のテストで file:///C:）
    "file:///C:/T/ws/%E6%97%A5%E6%9C%AC%20%E8%AA%9E.rs",
    // rust-analyzer（段階 4 T5 で file:///c:）
    "file:///c:/T/ws/%E6%97%A5%E6%9C%AC%20%E8%AA%9E.rs",
    // basedpyright（段階 4 T7 で file:///c%3A）。16 進の小文字も
    "file:///c%3A/T/ws/%E6%97%A5%E6%9C%AC%20%E8%AA%9E.rs",
    "file:///c%3a/T/ws/%E6%97%A5%E6%9C%AC%20%E8%AA%9E.rs",
    // 日本語の %XX の 16 進が小文字のもの
    "file:///c%3A/T/ws/%e6%97%a5%e6%9c%ac%20%e8%aa%9e.rs",
  ].filter((server) => !same(ours, server, true));
  assert.deepEqual(mismatched, [], `合わない：${mismatched.join("、")}`);
});

test("Windows：違うファイルは合わない（ドライブ名だけを直し、フォルダの大文字・小文字は区別する）", () => {
  for (const other of [
    "file:///C:/T/ws/%E6%97%A5%E6%9C%AC%20%E8%AA%9E2.rs",
    "file:///c:/T/WS/%E6%97%A5%E6%9C%AC%20%E8%AA%9E.rs",
    "file:///D:/T/ws/%E6%97%A5%E6%9C%AC%20%E8%AA%9E.rs",
    "file:///d%3A/T/ws/%E6%97%A5%E6%9C%AC%20%E8%AA%9E.rs",
  ]) {
    assert.ok(!same(ours, other, true), `合ってしまう：${other}`);
  }
});

test("macOS：今までどおり。%XX は戻すが、ドライブ名のような部分の大文字・小文字は区別する", () => {
  assert.ok(same("file:///a/b%20c/%E3%81%82.rs", "file:///a/b c/%e3%81%82.rs", false));
  assert.ok(!same("file:///c:/x.rs", "file:///C:/x.rs", false));
  assert.ok(same("file:///c%3A/x.rs", "file:///c:/x.rs", false));
});

test("読めない %XX は、元の文字のまま比べる", () => {
  assert.equal(canonicalUri("file:///C:/a%ZZ.rs", true), "file:///C:/a%ZZ.rs");
  assert.equal(canonicalUri("file:///a%ZZ.rs", false), "file:///a%ZZ.rs");
});
