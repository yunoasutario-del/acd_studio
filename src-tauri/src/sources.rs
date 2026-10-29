// 作業領域の外のソースを読み取り専用で読む（Phase 3b-2 作業 5。D-133）。
//
// 定義ジャンプの行き先が標準ライブラリ・依存クレートのソースのとき、画面は読み取り専用のタブで開く。
// 読むのは、次の 2 か所の中の `.rs` だけ。ファイル操作の境界（作業領域の外は読み書きしない。AC1-13）は変えない。
// - `$RUSTUP_HOME/toolchains`（なければ `~/.rustup/toolchains`）：標準ライブラリのソース（rust-src）
// - `$CARGO_HOME/registry/src`（なければ `~/.cargo/registry/src`）：crates.io の依存クレートのソース
// パスは URI から戻し、リンクと `..` を `canonicalize` で解決してから、許可した場所の下かを確かめる。
// 書き込みの経路はない。
// C++ のヘッダー（GCC の標準ヘッダー・SDK の usr/include・AC Library。cpp::header_dirs）も読む（Phase 17 作業 5。D-210）。
// こちらは拡張子のないもの（`vector` など）があるので、拡張子は見ない
// Python（Phase 20 作業 5。D-225）は、basedpyright の typeshed と、設定の処理系の標準ライブラリ・site-packages の
// 中の `.py`・`.pyi` を読む

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::language::Language;
use crate::lsp::path_of_uri;
use crate::workspace::MAX_FILE_SIZE;

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadOnlySource {
    pub content: String,
    // 画面に出す場所（ホームの中なら ~ で始める）
    pub location: String,
    // 色分けに使う言語
    pub language: Language,
}

const NOT_ALLOWED: &str = "作業領域の外のファイルです。読み取り専用で開けるのは、Rust の標準ライブラリと依存クレートのソース（~/.rustup/toolchains と ~/.cargo/registry/src の中の .rs）と、C++ の標準ヘッダー（使う g++ と SDK のもの）・AC Library と、Python の型の情報（basedpyright の typeshed）・標準ライブラリ・site-packages（設定の処理系のもの）の .py・.pyi だけです";

// 許可する場所（canonicalize 済み。なければ載せない）。rustup_home・cargo_home は環境変数 RUSTUP_HOME・CARGO_HOME の値
fn allowed_roots(
    home: Option<&Path>,
    rustup_home: Option<&Path>,
    cargo_home: Option<&Path>,
) -> Vec<PathBuf> {
    let rustup = rustup_home
        .map(Path::to_path_buf)
        .or_else(|| home.map(|home| home.join(".rustup")));
    let cargo = cargo_home
        .map(Path::to_path_buf)
        .or_else(|| home.map(|home| home.join(".cargo")));
    [
        rustup.map(|dir| dir.join("toolchains")),
        cargo.map(|dir| dir.join("registry/src")),
    ]
    .into_iter()
    .flatten()
    .filter_map(|dir| dir.canonicalize().ok())
    .collect()
}

fn display_location(real: &Path, home: Option<&Path>) -> String {
    let home = home.and_then(|home| home.canonicalize().ok());
    match home
        .as_deref()
        .and_then(|home| real.strip_prefix(home).ok())
    {
        Some(rest) => format!("~/{}", rest.to_string_lossy()),
        None => real.to_string_lossy().into_owned(),
    }
}

// roots は Rust の場所（.rs だけ）、cpp_roots は C++ のヘッダーの場所、python_roots は Python の場所
// （.py・.pyi だけ）。どれも canonicalize 済み
fn read_from(
    roots: &[PathBuf],
    cpp_roots: &[PathBuf],
    python_roots: &[PathBuf],
    home: Option<&Path>,
    uri: &str,
) -> Result<ReadOnlySource, String> {
    let path = path_of_uri(uri).ok_or_else(|| format!("file: の URI ではありません：{uri}"))?;
    // リンクと .. を解決した実際のパスで確かめる。ないファイルも、許可の外も、同じ文言で断る
    let real = path.canonicalize().map_err(|_| NOT_ALLOWED.to_string())?;
    let language = if roots.iter().any(|root| real.starts_with(root)) {
        if real.extension().is_none_or(|extension| extension != "rs") {
            return Err("読み取り専用で開けるのは .rs のファイルだけです".to_string());
        }
        Language::Rust
    } else if cpp_roots.iter().any(|root| real.starts_with(root)) {
        Language::Cpp
    } else if python_roots.iter().any(|root| real.starts_with(root)) {
        if real
            .extension()
            .is_none_or(|extension| extension != "py" && extension != "pyi")
        {
            return Err("読み取り専用で開けるのは .py・.pyi のファイルだけです".to_string());
        }
        Language::Python
    } else {
        return Err(NOT_ALLOWED.to_string());
    };
    if !real.is_file() {
        return Err("ファイルではありません".to_string());
    }
    // 上限より 1 バイトだけ多く読み、超えたかどうかを確かめる（作業領域のファイルと同じ上限）
    let mut bytes = Vec::new();
    fs::File::open(&real)
        .and_then(|file| file.take(MAX_FILE_SIZE + 1).read_to_end(&mut bytes))
        .map_err(|_| "ファイルを読めません".to_string())?;
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err("1 MB を超えるファイルは開けません".to_string());
    }
    let content =
        String::from_utf8(bytes).map_err(|_| "UTF-8 ではないファイルは開けません".to_string())?;
    Ok(ReadOnlySource {
        content,
        location: display_location(&real, home),
        language,
    })
}

// 許可した場所の中の `.rs` と、cpp_roots（cpp::header_dirs）の中のファイルと、python_roots の中の `.py`・`.pyi` を、
// 読み取り専用で読む
pub fn read_source(
    uri: &str,
    cpp_roots: &[PathBuf],
    python_roots: &[PathBuf],
) -> Result<ReadOnlySource, String> {
    let home = crate::toolchain::home_dir();
    let rustup_home = std::env::var_os("RUSTUP_HOME").map(PathBuf::from);
    let cargo_home = std::env::var_os("CARGO_HOME").map(PathBuf::from);
    let roots = allowed_roots(
        home.as_deref(),
        rustup_home.as_deref(),
        cargo_home.as_deref(),
    );
    let python_roots: Vec<PathBuf> = python_roots
        .iter()
        .filter_map(|dir| dir.canonicalize().ok())
        .collect();
    read_from(&roots, cpp_roots, &python_roots, home.as_deref(), uri)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::file_uri;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
                "atcoder-studio-sources-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn file(&self, relative: &str, content: &[u8]) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, content).unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // 一時フォルダを「ホーム」に見立てる
    fn read(home: &TempDir, path: &Path) -> Result<ReadOnlySource, String> {
        let roots = allowed_roots(Some(&home.0), None, None);
        read_from(&roots, &[], &[], Some(&home.0), &file_uri(path))
    }

    #[test]
    #[cfg_attr(windows, ignore = "Windows：段階 4（file: の URI と、canonicalize が付ける接頭辞）で直す。docs/windows-port-plan.md 7 章")]
    fn reads_rust_sources_in_the_allowed_places() {
        let home = TempDir::new("allowed");
        let std_src = home.file(
            ".rustup/toolchains/1.89-x/lib/rustlib/src/rust/library/alloc/src/vec/mod.rs",
            "// vec".as_bytes(),
        );
        let krate = home.file(
            ".cargo/registry/src/index.crates.io-abc/proconio-0.5.0/src/lib.rs",
            "// proconio".as_bytes(),
        );
        let found = read(&home, &std_src).unwrap();
        assert_eq!(found.content, "// vec");
        assert_eq!(
            found.location,
            "~/.rustup/toolchains/1.89-x/lib/rustlib/src/rust/library/alloc/src/vec/mod.rs"
        );
        let found = read(&home, &krate).unwrap();
        assert_eq!(found.content, "// proconio");
        assert!(found.location.starts_with("~/.cargo/registry/src/"));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_everything_else() {
        let home = TempDir::new("refused");
        home.file(".rustup/toolchains/t/lib/ok.rs", b"");
        home.file(".cargo/registry/src/i/c/src/lib.rs", b"");
        let secret = home.file("secret.rs", b"// secret");
        let other = home.file(".cargo/config.toml", b"");
        let cargo_bin = home.file(".cargo/bin/tool.rs", b"");
        let not_rs = home.file(".rustup/toolchains/t/lib/Cargo.toml", b"");
        // 許可の外
        for path in [&secret, &other, &cargo_bin] {
            let error = read(&home, path).unwrap_err();
            assert!(error.contains("作業領域の外"), "{path:?}: {error}");
        }
        // .rs 以外、フォルダ、ないファイル
        assert!(read(&home, &not_rs).unwrap_err().contains(".rs"));
        assert!(read(&home, &home.0.join(".rustup/toolchains/t/lib")).is_err());
        assert!(read(&home, &home.0.join(".rustup/toolchains/t/lib/none.rs")).is_err());
        // 許可した場所の中のリンクが、外を指す
        let link = home.0.join(".rustup/toolchains/t/lib/link.rs");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        assert!(read(&home, &link).unwrap_err().contains("作業領域の外"));
        // .. で外に出る
        let escape = format!("{}/.rustup/toolchains/t/../../secret.rs", file_uri(&home.0));
        let roots = allowed_roots(Some(&home.0), None, None);
        assert!(read_from(&roots, &[], &[], Some(&home.0), &escape).is_err());
        // file: でない
        assert!(read_from(&roots, &[], &[], None, "untitled:a.rs").is_err());
        assert!(read_from(&roots, &[], &[], None, "https://example.com/a.rs").is_err());
    }

    #[test]
    #[cfg_attr(windows, ignore = "Windows：段階 4（file: の URI と、canonicalize が付ける接頭辞）で直す。docs/windows-port-plan.md 7 章")]
    fn checks_the_size_and_the_encoding() {
        let home = TempDir::new("limits");
        let big = home.file(
            ".rustup/toolchains/t/big.rs",
            &vec![b'a'; MAX_FILE_SIZE as usize],
        );
        assert_eq!(
            read(&home, &big).unwrap().content.len() as u64,
            MAX_FILE_SIZE
        );
        let too_big = home.file(
            ".rustup/toolchains/t/too_big.rs",
            &vec![b'a'; MAX_FILE_SIZE as usize + 1],
        );
        assert!(read(&home, &too_big).unwrap_err().contains("1 MB"));
        let binary = home.file(".rustup/toolchains/t/binary.rs", &[0xff, 0xfe, 0x00]);
        assert!(read(&home, &binary).unwrap_err().contains("UTF-8"));
    }

    #[test]
    #[cfg_attr(windows, ignore = "Windows：段階 4（file: の URI と、canonicalize が付ける接頭辞）で直す。docs/windows-port-plan.md 7 章")]
    fn honors_rustup_home_and_cargo_home() {
        let dir = TempDir::new("env");
        let custom_rustup = dir.file("rustup/toolchains/t/a.rs", b"// a");
        let custom_cargo = dir.file("cargo/registry/src/i/c/b.rs", b"// b");
        // ホームの下にない場所でも、環境変数が指していれば許可する
        let roots = allowed_roots(
            None,
            Some(&dir.0.join("rustup")),
            Some(&dir.0.join("cargo")),
        );
        assert_eq!(roots.len(), 2);
        let found = read_from(&roots, &[], &[], None, &file_uri(&custom_rustup)).unwrap();
        assert_eq!(found.content, "// a");
        // ホームが分からなければ、絶対パスで示す
        assert_eq!(found.location, custom_rustup.to_string_lossy());
        assert!(read_from(&roots, &[], &[], None, &file_uri(&custom_cargo)).is_ok());
        // ない場所は載せない
        assert!(allowed_roots(None, None, None).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn reads_cpp_headers_without_extensions() {
        let dir = TempDir::new("cpp");
        let vector = dir.file("gcc/include/c++/15/vector", b"// vector");
        let tcc = dir.file("gcc/include/c++/15/bits/vector.tcc", b"// tcc");
        let dsu = dir.file("acl/atcoder/dsu.hpp", b"// dsu");
        let secret = dir.file("secret", b"// secret");
        let rust = dir.file("rustup/toolchains/t/Cargo.toml", b"");
        let roots = allowed_roots(None, Some(&dir.0.join("rustup")), None);
        let cpp_roots = vec![dir.0.join("gcc/include/c++/15"), dir.0.join("acl")];
        for (path, content) in [(&vector, "// vector"), (&tcc, "// tcc"), (&dsu, "// dsu")] {
            let found = read_from(&roots, &cpp_roots, &[], None, &file_uri(path)).unwrap();
            assert_eq!(found.content, content);
            assert_eq!(found.language, Language::Cpp);
        }
        // C++ の場所の外、リンクで外を指すもの
        assert!(
            read_from(&roots, &cpp_roots, &[], None, &file_uri(&secret))
                .unwrap_err()
                .contains("作業領域の外")
        );
        let link = dir.0.join("acl/atcoder/link.hpp");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        assert!(read_from(&roots, &cpp_roots, &[], None, &file_uri(&link)).is_err());
        // Rust の場所は今までどおり .rs だけ
        assert!(
            read_from(&roots, &cpp_roots, &[], None, &file_uri(&rust))
                .unwrap_err()
                .contains(".rs")
        );
        let rs = dir.file("rustup/toolchains/t/a.rs", b"");
        assert_eq!(
            read_from(&roots, &cpp_roots, &[], None, &file_uri(&rs))
                .unwrap()
                .language,
            Language::Rust
        );
    }

    // Python の場所の .py・.pyi（D-225）
    #[cfg(unix)]
    #[test]
    fn reads_python_sources() {
        let dir = TempDir::new("python");
        let stub = dir.file(
            "typeshed/stdlib/collections/__init__.pyi",
            b"class deque: ...",
        );
        let module = dir.file("lib/python3.13/heapq.py", b"def heappush(): ...");
        let binary = dir.file("lib/python3.13/lib-dynload/_heapq.so", b"");
        let secret = dir.file("secret.py", b"");
        let python_roots = vec![dir.0.join("typeshed"), dir.0.join("lib/python3.13")];
        for path in [&stub, &module] {
            let found = read_from(&[], &[], &python_roots, None, &file_uri(path)).unwrap();
            assert_eq!(found.language, Language::Python);
        }
        assert!(
            read_from(&[], &[], &python_roots, None, &file_uri(&binary))
                .unwrap_err()
                .contains(".py・.pyi")
        );
        assert!(
            read_from(&[], &[], &python_roots, None, &file_uri(&secret))
                .unwrap_err()
                .contains("作業領域の外")
        );
        // リンクで外を指すもの
        let link = dir.0.join("typeshed/stdlib/link.pyi");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        assert!(read_from(&[], &[], &python_roots, None, &file_uri(&link)).is_err());
    }
}
