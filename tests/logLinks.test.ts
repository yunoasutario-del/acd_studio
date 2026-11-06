// 実行ログの「ファイル名:行:列」のリンク（src/panels/logLinks.ts）のテスト（Windows 移植の段階 2 S10）。
// Node の組み込みのテスト（node --test）で動かす。Node は TypeScript の型を取り除いてそのまま動かすので、依存を足さない。
// src の外に置くのは、tsc（npm run build）の対象にしないため（Node の型の定義を足さずに済ませる）
import { test } from "node:test";
import assert from "node:assert/strict";
import { splitLocations } from "../src/panels/logLinks.ts";

// リンクになった部分だけを、文字と場所の組にする
function links(text: string, windows: boolean) {
  return splitLocations(text, windows)
    .filter((segment) => segment.location !== undefined)
    .map((segment) => ({ text: segment.text, ...segment.location }));
}

test("Windows：rustc のコンパイルエラーの `\\` 区切りの場所を、`/` に直して読む", () => {
  assert.deepEqual(links("error[E0308]: mismatched types\n --> src\\main.rs:3:5\n", true), [
    { text: "src\\main.rs:3:5", file: "src/main.rs", line: 3, column: 5 },
  ]);
});

test("Windows：panic の場所と example の場所も読む", () => {
  assert.deepEqual(links("thread 'main' panicked at src\\main.rs:5:10:\nexplicit panic\n", true), [
    { text: "src\\main.rs:5:10", file: "src/main.rs", line: 5, column: 10 },
  ]);
  assert.deepEqual(links(" --> examples\\gen.rs:1:5\n", true), [
    { text: "examples\\gen.rs:1:5", file: "examples/gen.rs", line: 1, column: 5 },
  ]);
});

test("Windows：`/` 区切りの場所も、今までどおり読む", () => {
  assert.deepEqual(links(" --> src/main.rs:3:5\n", true), [
    { text: "src/main.rs:3:5", file: "src/main.rs", line: 3, column: 5 },
  ]);
});

test("Windows：C++ の `main.cpp:4:5:` は、今までどおりリンクになる", () => {
  assert.deepEqual(links("main.cpp:4:5: error: 'x' was not declared in this scope\n", true), [
    { text: "main.cpp:4:5", file: "main.cpp", line: 4, column: 5 },
  ]);
});

test("Windows：絶対パスと `..` を含むものは、リンクにしない", () => {
  assert.deepEqual(links("C:\\x\\main.rs:3:1\n", true), []);
  assert.deepEqual(links("..\\a.rs:1:1\n", true), []);
  assert.deepEqual(links("src\\..\\..\\a.rs:1:1\n", true), []);
});

test("Windows：rustc が出した、`/` と `\\` が混ざった標準ライブラリの場所は、どの部分もリンクにしない", () => {
  const text =
    " --> /rustc/48a229ceaefd4985c50990b14116b6d856af0985/library\\std\\src\\io\\mod.rs:978:7\n";
  assert.deepEqual(links(text, true), []);
});

test("Windows 以外：今までどおり `/` 区切りだけを読む", () => {
  assert.deepEqual(links(" --> src/main.rs:3:5\n", false), [
    { text: "src/main.rs:3:5", file: "src/main.rs", line: 3, column: 5 },
  ]);
  assert.deepEqual(links("main.cpp:4:5: error: x\n", false), [
    { text: "main.cpp:4:5", file: "main.cpp", line: 4, column: 5 },
  ]);
  assert.deepEqual(links(" --> /rustc/abc/library/core/src/lib.rs:1:1\n", false), []);
  assert.deepEqual(links("../a.rs:1:1\n", false), []);
});
