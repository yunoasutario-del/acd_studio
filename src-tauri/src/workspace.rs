// 作業領域（利用者が選んだフォルダ）の中のパスの検証（05 T-08、AC1-13）
//
// フロントエンドからは、作業領域のルートからの相対パスだけを受け取る。
// シンボリックリンクを解決した実際のパスが、作業領域の中にあることを確かめてから使う。
//
// 限界：検証してから開くまでの間にリンクが差し替えられる場合（TOCTOU）は防げない。
// ハードリンクは別の場所のファイルと区別できない。

use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

#[derive(Debug, PartialEq, Eq)]
pub enum PathError {
    // 絶対パス、`..`、空の要素、NUL などを含む
    Invalid,
    // 実際のパスが作業領域の外にある
    OutsideWorkspace,
    // ファイル・フォルダが見つからない、またはリンクがループしている
    NotFound,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            PathError::Invalid => "使えないパスです",
            PathError::OutsideWorkspace => "作業領域の外のパスは使えません",
            PathError::NotFound => "ファイルまたはフォルダが見つかりません",
        };
        f.write_str(message)
    }
}

// 相対パスの形を確かめる。空の文字列は作業領域のルートを表す
fn check_relative(relative: &str) -> Result<&Path, PathError> {
    if relative.contains('\0') {
        return Err(PathError::Invalid);
    }
    if !relative.is_empty() && relative.split('/').any(|part| part.is_empty()) {
        // 先頭・末尾の `/` や `a//b` のような空の要素
        return Err(PathError::Invalid);
    }
    let path = Path::new(relative);
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            // 絶対パス（RootDir・Prefix）と `..`
            _ => return Err(PathError::Invalid),
        }
    }
    Ok(path)
}

// 作業領域のルート（canonicalize 済み）と相対パスから、実際のパスを求める
pub fn resolve(root: &Path, relative: &str) -> Result<PathBuf, PathError> {
    let path = check_relative(relative)?;
    let real = root
        .join(path)
        .canonicalize()
        .map_err(|_| PathError::NotFound)?;
    // Path::starts_with はパスの要素単位で比べる（`/ws` と `/ws2` を区別する）
    if real.starts_with(root) {
        Ok(real)
    } else {
        Err(PathError::OutsideWorkspace)
    }
}

// 読み込めるファイルの大きさの上限（1 MB）。超えるファイルは開かない
pub const MAX_FILE_SIZE: u64 = 1024 * 1024;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct DirEntry {
    pub name: String,
    // 作業領域のルートからの相対パス（区切りは `/`）
    pub path: String,
    pub is_dir: bool,
}

// 作業領域の中のフォルダの中身を返す。フォルダが先、次に名前の順（大文字・小文字を区別しない）。
// 外を指すシンボリックリンクも一覧には出す（開くときに resolve で拒否される）。
// フォルダが見つからない（外で消えた）・フォルダでなくなったときは None（ファイル一覧は、そのフォルダを閉じる。D-156）
pub fn list_dir(root: &Path, relative: &str) -> Result<Option<Vec<DirEntry>>, String> {
    let dir = match resolve(root, relative) {
        Ok(dir) => dir,
        Err(PathError::NotFound) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !dir.is_dir() {
        return Ok(None);
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|_| "フォルダを読めません".to_string())? {
        let entry = entry.map_err(|_| "フォルダを読めません".to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // リンクの先がフォルダなら、フォルダとして表示する
        let is_dir = entry.path().is_dir();
        let path = if relative.is_empty() {
            name.clone()
        } else {
            format!("{relative}/{name}")
        };
        entries.push(DirEntry { name, path, is_dir });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(Some(entries))
}

// ファイルの更新日時と大きさ（作業 5。D-038 の外部での変更の検出に使う）。
// 更新日時はナノ秒。JavaScript の数では精度が足りないので、文字列で渡す
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Stamp {
    pub modified: String,
    pub size: u64,
}

pub fn stamp_of(path: &Path) -> Result<Stamp, String> {
    let metadata = fs::metadata(path).map_err(|_| "ファイルを読めません".to_string())?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().to_string())
        .unwrap_or_default();
    Ok(Stamp {
        modified,
        size: metadata.len(),
    })
}

// ファイルの今の更新日時と大きさ。ファイルがなければ None
pub fn current_stamp(root: &Path, relative: &str) -> Option<Stamp> {
    let file = resolve(root, relative).ok()?;
    if !file.is_file() {
        return None;
    }
    stamp_of(&file).ok()
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ReadResult {
    pub content: String,
    pub stamp: Stamp,
}

// ファイルを UTF-8 の文字列として、更新日時と大きさと一緒に返す。
// UTF-8 として読めないファイルと、上限を超えるファイルは拒否する（D-068）
pub fn read_text(root: &Path, relative: &str) -> Result<ReadResult, String> {
    let file = resolve(root, relative).map_err(|error| error.to_string())?;
    if !file.is_file() {
        return Err("ファイルではありません".to_string());
    }
    // 上限より 1 バイトだけ多く読み、超えたかどうかを確かめる
    let mut bytes = Vec::new();
    fs::File::open(&file)
        .and_then(|f| f.take(MAX_FILE_SIZE + 1).read_to_end(&mut bytes))
        .map_err(|_| "ファイルを読めません".to_string())?;
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err("1 MB を超えるファイルは開けません".to_string());
    }
    let content =
        String::from_utf8(bytes).map_err(|_| "UTF-8 ではないファイルは開けません".to_string())?;
    let stamp = stamp_of(&file)?;
    Ok(ReadResult { content, stamp })
}

// 書き込みの結果。外部で変更されていたら書かずに Conflict を返す（D-038）
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WriteResult {
    Saved { stamp: Stamp },
    Conflict,
}

// 既存のファイルを上書きする。新しいファイルは作らない。
// expected を渡すと、今の更新日時と大きさが違うとき（外部で変更されたとき）は書かずに Conflict を返す。
// 書き方は W2：そのファイルに直接書き、sync_all で確定させる。ファイルの実体（inode）を変えないので、
// 権限・拡張属性・ハードリンクが保たれる（SV-05）。書き込みの途中でアプリが落ちると、ファイルが途中までになる可能性がある
pub fn write_existing(
    root: &Path,
    relative: &str,
    content: &str,
    expected: Option<&Stamp>,
) -> Result<WriteResult, String> {
    let file = resolve(root, relative).map_err(|error| error.to_string())?;
    if !file.is_file() {
        return Err("ファイルではありません".to_string());
    }
    if let Some(expected) = expected
        && &stamp_of(&file)? != expected
    {
        return Ok(WriteResult::Conflict);
    }
    let mut handle = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&file)
        .map_err(|_| "ファイルに書き込めません".to_string())?;
    handle
        .write_all(content.as_bytes())
        .and_then(|_| handle.sync_all())
        .map_err(|_| "ファイルに書き込めません".to_string())?;
    Ok(WriteResult::Saved {
        stamp: stamp_of(&file)?,
    })
}

// 期待出力の保存の結果（Phase 4a 作業 3。D-137）
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ExpectedSaved {
    // 書いた（新しく作った、または上書きした）
    Written,
    // すでに同じ内容だったので、書かなかった
    Unchanged,
    // 上書きの確認で断られた
    Cancelled,
}

pub type CaseKey = (u8, u64);

// テストケースの名前（`sample-<数字>`、またはランダムテストで保存した `random-<数字>`。D-090・D-143）の並び順の鍵
// （種類、番号）。サンプルが先で、ランダムテストのものがあと。ケースの名前でなければ None
pub fn case_key(stem: &str) -> Option<CaseKey> {
    let (kind, number) = match stem.strip_prefix("sample-") {
        Some(number) => (0, number),
        None => (1, stem.strip_prefix("random-")?),
    };
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((kind, number.parse().ok()?))
}

// テストケースの期待出力のファイル名（`sample-<数字>.out`・`random-<数字>.out`）か。対応する入力は同じ名前の `.in`（D-090・D-143）
fn expected_name_stem(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".out")?;
    case_key(stem).map(|_| stem)
}

// テスト結果の実際の出力を、期待出力（`sample-N.out`・`random-N.out`）として保存する。
// 書けるのは、作業領域の中の `sample-N.out`・`random-N.out` で、同じフォルダに対応する `.in` があるときだけ。
// すでに `.out` があって内容が違うときは confirm_overwrite で確かめ、断られたら書かない。
// 新しいファイルは「なければ作る」を 1 回の操作で行い、既存のファイルは write_existing と同じ書き方で上書きする。
// 限界：確かめてから書くまでの間に、ファイルが差し替えられる場合（TOCTOU）は防げない
pub fn write_expected(
    root: &Path,
    relative: &str,
    content: &str,
    confirm_overwrite: impl FnOnce() -> bool,
) -> Result<ExpectedSaved, String> {
    if content.len() as u64 > MAX_FILE_SIZE {
        return Err("1 MB を超える内容は保存できません".to_string());
    }
    let (parent, name) = relative.rsplit_once('/').unwrap_or(("", relative));
    let stem = expected_name_stem(name).ok_or_else(|| {
        "期待出力として保存できるのは sample-N.out と random-N.out だけです".to_string()
    })?;
    let folder = resolve(root, parent).map_err(|error| error.to_string())?;
    if !folder.is_dir() {
        return Err("フォルダではありません".to_string());
    }
    if !folder.join(format!("{stem}.in")).is_file() {
        return Err(format!("対応する {stem}.in がありません"));
    }
    let path = folder.join(name);
    // リンクも含め、何かあれば既存として扱う（ないときだけ新しく作る）
    if fs::symlink_metadata(&path).is_err() {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| format!("{name} を書けません"))?;
        file.write_all(content.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|_| format!("{name} を書けません"))?;
        return Ok(ExpectedSaved::Written);
    }
    let real = resolve(root, relative).map_err(|error| error.to_string())?;
    if !real.is_file() {
        return Err("ファイルではありません".to_string());
    }
    let existing = fs::read(&real).map_err(|_| format!("{name} を読めません"))?;
    if existing == content.as_bytes() {
        return Ok(ExpectedSaved::Unchanged);
    }
    if !confirm_overwrite() {
        return Ok(ExpectedSaved::Cancelled);
    }
    write_existing(root, relative, content, None)?;
    Ok(ExpectedSaved::Written)
}

// ファイルの名前の検証（作業 4c）。1 つの要素だけで、`/`・NUL・`.`・`..`・空の名前は使えない
fn check_name(name: &str) -> Result<&str, String> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err("使えない名前です".to_string());
    }
    Ok(name)
}

// 作業領域の中の実際のパスを、ルートからの相対パス（区切りは `/`）にする
fn to_relative(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| PathError::OutsideWorkspace.to_string())?;
    Ok(relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/"))
}

// コンパイルエラーや panic のメッセージに出るファイル名（例：`src/main.rs`）を、作業領域の中のファイルの
// 相対パスにする（エラー一覧・実行ログの場所から移動する。Phase 4a 作業 1。D-137）。
// ファイル名は cargo・rustc の実行の場所からの相対パスで、パッケージの中か、cargo の作業領域の根からかは
// 場合による【未確認】ので、実行の対象のパッケージのフォルダから作業領域のルートまで順に探す。
// 作業領域の外・ない・フォルダ・リンクで外を指すものは None。ファイルは読まない
pub fn locate_in_package(root: &Path, package: &str, file: &str) -> Option<String> {
    if file.is_empty() || file.contains('\0') {
        return None;
    }
    let file = file.strip_prefix("./").unwrap_or(file);
    let file_path = Path::new(file);
    let found = |path: &Path| -> Option<String> {
        let real = path.canonicalize().ok()?;
        if !real.is_file() || !real.starts_with(root) {
            return None;
        }
        to_relative(root, &real).ok()
    };
    if file_path.is_absolute() {
        return found(file_path);
    }
    let mut base = resolve(root, package).ok()?;
    loop {
        if let Some(relative) = found(&base.join(file_path)) {
            return Some(relative);
        }
        if base == root || !base.pop() || !base.starts_with(root) {
            return None;
        }
    }
}

// 一覧の項目そのもの（リンクならリンク自体）のパスを返す。
// 親のフォルダを resolve で検証し、名前の部分はリンクを解決しない（名前の変更・削除はリンク自体に対して行う）
pub fn entry_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let (parent, name) = match relative.rsplit_once('/') {
        Some((parent, name)) => (parent, name),
        None => ("", relative),
    };
    let name = check_name(name)?;
    let parent = resolve(root, parent).map_err(|error| error.to_string())?;
    let path = parent.join(name);
    // フォルダも扱う（D-157。D-073 を置き換え）。作業領域のルートは、名前が空なので check_name で断る
    fs::symlink_metadata(&path).map_err(|_| PathError::NotFound.to_string())?;
    Ok(path)
}

// 一覧の項目がフォルダか（リンクは、先がフォルダでもフォルダとしない。D-157）
pub fn is_folder(root: &Path, relative: &str) -> Result<bool, String> {
    let path = entry_path(root, relative)?;
    let metadata = fs::symlink_metadata(&path).map_err(|_| PathError::NotFound.to_string())?;
    Ok(metadata.is_dir())
}

// 作業領域の中のフォルダに、新しいフォルダを作る。同じ名前があれば作らない（D-157）。戻り値は作ったフォルダの相対パス
pub fn create_folder(root: &Path, dir: &str, name: &str) -> Result<String, String> {
    let name = check_name(name)?;
    let dir = resolve(root, dir).map_err(|error| error.to_string())?;
    if !dir.is_dir() {
        return Err("フォルダではありません".to_string());
    }
    let path = dir.join(name);
    fs::create_dir(&path).map_err(|error| match error.kind() {
        std::io::ErrorKind::AlreadyExists => {
            "同じ名前のファイルまたはフォルダがあります".to_string()
        }
        _ => "フォルダを作れません".to_string(),
    })?;
    to_relative(root, &path)
}

// フォルダの中のファイルの数（削除の確認に出す。D-157）。リンクはたどらず 1 つと数える。limit まで数えて止める
pub fn count_files(dir: &Path, limit: usize) -> usize {
    let mut count = 0;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                pending.push(entry.path());
            } else {
                count += 1;
                if count >= limit {
                    return limit;
                }
            }
        }
    }
    count
}

// 作業領域の中のフォルダに、空のファイルを作る。同じ名前があれば作らない（上書きしない）。
// 戻り値は、作ったファイルの相対パス
pub fn create_file(root: &Path, dir: &str, name: &str) -> Result<String, String> {
    let name = check_name(name)?;
    let dir = resolve(root, dir).map_err(|error| error.to_string())?;
    if !dir.is_dir() {
        return Err("フォルダではありません".to_string());
    }
    let path = dir.join(name);
    // 「なければ作る」を 1 回の操作で行う
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::AlreadyExists => {
                "同じ名前のファイルまたはフォルダがあります".to_string()
            }
            _ => "ファイルを作れません".to_string(),
        })?;
    to_relative(root, &path)
}

// ファイルの名前を変える（同じフォルダの中だけ）。同じ名前があれば変えない。
// 戻り値は、変えた後の相対パス。
// 限界：名前があるかを確かめてから変えるまでの間に、同じ名前のファイルが作られると上書きする
pub fn rename_file(root: &Path, relative: &str, new_name: &str) -> Result<String, String> {
    let new_name = check_name(new_name)?;
    let from = entry_path(root, relative)?;
    let to = from
        .parent()
        .ok_or_else(|| "使えないパスです".to_string())?
        .join(new_name);
    if let Ok(existing) = fs::symlink_metadata(&to) {
        // 大文字・小文字だけを変えるとき（macOS の既定のディスクは区別しないので、新しい名前が「ある」ことになる）は、
        // 同じもの（同じ i ノード）なら変えてよい。ハードリンク（別の名前の同じもの）は変えない（Phase 10 作業 12）
        let old_name = from
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let case_only = old_name != new_name && old_name.to_lowercase() == new_name.to_lowercase();
        #[cfg(unix)]
        let same = fs::symlink_metadata(&from).is_ok_and(|current| {
            use std::os::unix::fs::MetadataExt;
            current.dev() == existing.dev() && current.ino() == existing.ino()
        });
        // Windows：リンク自体のボリュームのシリアル番号とファイルの番号で比べる（Unix の dev・ino にあたる）
        #[cfg(windows)]
        let same = {
            let _ = &existing;
            crate::win::same_entry(&from, &to)
        };
        if !(case_only && same) {
            return Err("同じ名前のファイルまたはフォルダがあります".to_string());
        }
    }
    fs::rename(&from, &to).map_err(|_| "名前を変えられません".to_string())?;
    to_relative(root, &to)
}

// 一覧の項目がシンボリックリンクか（削除の確認の文言を変えるため。D-102）
pub fn is_link(root: &Path, relative: &str) -> Result<bool, String> {
    let path = entry_path(root, relative)?;
    let metadata = fs::symlink_metadata(&path).map_err(|_| PathError::NotFound.to_string())?;
    Ok(metadata.file_type().is_symlink())
}

// ファイルをゴミ箱に移す（確認は呼び出す側で済ませる。D-072）。
// リンクはゴミ箱に移さず、リンク自体を消す（D-102）。リンク自体を消しても、リンクの先は変わらない。
// Finder を使うゴミ箱への移動がリンクの先を移さないことは、確かめられなかったため。
// 限界：確かめてから消すまでの間に、リンクとファイルが差し替えられる場合（TOCTOU）は防げない
pub fn trash_file(root: &Path, relative: &str) -> Result<(), String> {
    let path = entry_path(root, relative)?;
    let metadata = fs::symlink_metadata(&path).map_err(|_| PathError::NotFound.to_string())?;
    if metadata.file_type().is_symlink() {
        return fs::remove_file(&path).map_err(|_| "リンクを削除できません".to_string());
    }
    trash::delete(&path).map_err(|_| "ゴミ箱に移せません".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    // テスト用の一時フォルダ。終わったら消す
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "atcoder-studio-test-{}-{}",
                name,
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempDir(path.canonicalize().unwrap())
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // base の下に、作業領域 ws と、その外の outside・ws2 を作る
    fn setup(name: &str) -> (TempDir, PathBuf) {
        let base = TempDir::new(name);
        let ws = base.0.join("ws");
        fs::create_dir_all(ws.join("src")).unwrap();
        fs::write(ws.join("src/main.rs"), "fn main() {}").unwrap();
        fs::create_dir_all(base.0.join("outside")).unwrap();
        fs::write(base.0.join("outside/secret.txt"), "secret").unwrap();
        fs::create_dir_all(base.0.join("ws2")).unwrap();
        fs::write(base.0.join("ws2/other.txt"), "other").unwrap();
        (base, ws)
    }

    #[test]
    fn writes_expected_output_only_beside_its_input() {
        let (base, ws) = setup("expected");
        fs::create_dir_all(ws.join("pkg/test")).unwrap();
        fs::write(ws.join("pkg/test/sample-1.in"), "1\n").unwrap();
        fs::write(ws.join("pkg/test/sample-2.in"), "2\n").unwrap();
        fs::write(ws.join("pkg/test/sample-2.out"), "old\n").unwrap();
        let never = || -> bool { panic!("確認は要らないはず") };
        // 新しく作る
        assert_eq!(
            write_expected(&ws, "pkg/test/sample-1.out", "2\n", never),
            Ok(ExpectedSaved::Written)
        );
        assert_eq!(
            fs::read_to_string(ws.join("pkg/test/sample-1.out")).unwrap(),
            "2\n"
        );
        // 同じ内容は書かず、確認も出さない
        assert_eq!(
            write_expected(&ws, "pkg/test/sample-1.out", "2\n", never),
            Ok(ExpectedSaved::Unchanged)
        );
        // 違う内容は確認する。断られたら書かない
        assert_eq!(
            write_expected(&ws, "pkg/test/sample-2.out", "4\n", || false),
            Ok(ExpectedSaved::Cancelled)
        );
        assert_eq!(
            fs::read_to_string(ws.join("pkg/test/sample-2.out")).unwrap(),
            "old\n"
        );
        assert_eq!(
            write_expected(&ws, "pkg/test/sample-2.out", "4\n", || true),
            Ok(ExpectedSaved::Written)
        );
        assert_eq!(
            fs::read_to_string(ws.join("pkg/test/sample-2.out")).unwrap(),
            "4\n"
        );
        // 上書きでは、元より短い内容でも残らない
        write_expected(&ws, "pkg/test/sample-2.out", "9\n", || true).unwrap();
        write_expected(&ws, "pkg/test/sample-2.out", "", || true).unwrap();
        assert_eq!(
            fs::read_to_string(ws.join("pkg/test/sample-2.out")).unwrap(),
            ""
        );
        // sample-N.out 以外の名前
        for path in [
            "pkg/test/sample-1.in",
            "pkg/test/sample-x.out",
            "pkg/test/sample-.out",
            "pkg/test/input.txt",
            "pkg/test/other.out",
            "pkg/test/sample-1.out.bak",
            "pkg/Cargo.toml",
            "sample-1.out",
        ] {
            assert!(write_expected(&ws, path, "x", never).is_err(), "{path}");
        }
        // 対応する .in がない
        let error = write_expected(&ws, "pkg/test/sample-3.out", "x", never).unwrap_err();
        assert!(error.contains("sample-3.in"), "{error}");
        assert!(!ws.join("pkg/test/sample-3.out").exists());
        // ランダムテストで保存したケース（random-N）も同じ規則
        fs::write(ws.join("pkg/test/random-1.in"), "5\n").unwrap();
        assert_eq!(
            write_expected(&ws, "pkg/test/random-1.out", "6\n", never),
            Ok(ExpectedSaved::Written)
        );
        assert_eq!(
            fs::read_to_string(ws.join("pkg/test/random-1.out")).unwrap(),
            "6\n"
        );
        assert_eq!(
            write_expected(&ws, "pkg/test/random-1.out", "7\n", || false),
            Ok(ExpectedSaved::Cancelled)
        );
        let error = write_expected(&ws, "pkg/test/random-2.out", "x", never).unwrap_err();
        assert!(error.contains("random-2.in"), "{error}");
        for path in [
            "pkg/test/random-.out",
            "pkg/test/random-x.out",
            "pkg/test/randomized-1.out",
        ] {
            assert!(write_expected(&ws, path, "x", never).is_err(), "{path}");
        }
        // 作業領域の外、ないフォルダ、..
        fs::write(base.0.join("outside/sample-1.in"), "").unwrap();
        for path in [
            "../outside/sample-1.out",
            "pkg/../../outside/sample-1.out",
            "none/sample-1.out",
            "/etc/sample-1.out",
        ] {
            assert!(write_expected(&ws, path, "x", never).is_err(), "{path}");
        }
        assert!(!base.0.join("outside/sample-1.out").exists());
        // 外を指すフォルダのリンクの先には書かない
        symlink(base.0.join("outside"), ws.join("pkg/linked")).unwrap();
        assert!(write_expected(&ws, "pkg/linked/sample-1.out", "x", never).is_err());
        assert!(!base.0.join("outside/sample-1.out").exists());
        // 外を指す .out のリンクの先には書かない
        fs::write(base.0.join("outside/target.txt"), "secret").unwrap();
        symlink(
            base.0.join("outside/target.txt"),
            ws.join("pkg/test/sample-4.out"),
        )
        .unwrap();
        fs::write(ws.join("pkg/test/sample-4.in"), "").unwrap();
        assert!(write_expected(&ws, "pkg/test/sample-4.out", "x", || true).is_err());
        assert_eq!(
            fs::read_to_string(base.0.join("outside/target.txt")).unwrap(),
            "secret"
        );
        // 大きすぎる内容
        let big = "a".repeat(MAX_FILE_SIZE as usize + 1);
        assert!(write_expected(&ws, "pkg/test/sample-1.out", &big, never).is_err());
    }

    #[test]
    fn locates_a_file_name_from_the_compiler_in_the_workspace() {
        let (base, ws) = setup("locate");
        // パッケージ pkg の中のファイル。ファイル名は、パッケージの中からでも、ルートからでもありうる
        fs::create_dir_all(ws.join("pkg/src")).unwrap();
        fs::write(ws.join("pkg/src/lib.rs"), "").unwrap();
        assert_eq!(
            locate_in_package(&ws, "pkg", "src/lib.rs"),
            Some("pkg/src/lib.rs".to_string())
        );
        assert_eq!(
            locate_in_package(&ws, "", "pkg/src/lib.rs"),
            Some("pkg/src/lib.rs".to_string())
        );
        assert_eq!(
            locate_in_package(&ws, "pkg", "pkg/src/lib.rs"),
            Some("pkg/src/lib.rs".to_string())
        );
        assert_eq!(
            locate_in_package(&ws, "pkg", "./src/lib.rs"),
            Some("pkg/src/lib.rs".to_string())
        );
        // ルートの src/main.rs は、パッケージの中になければルートで見つかる
        assert_eq!(
            locate_in_package(&ws, "pkg", "src/main.rs"),
            Some("src/main.rs".to_string())
        );
        // 絶対パスでも、作業領域の中なら見つかる
        let absolute = ws.join("pkg/src/lib.rs");
        assert_eq!(
            locate_in_package(&ws, "pkg", absolute.to_str().unwrap()),
            Some("pkg/src/lib.rs".to_string())
        );
        // ない、フォルダ、空、NUL
        for file in ["src/none.rs", "src", "", "a\0b.rs"] {
            assert_eq!(locate_in_package(&ws, "pkg", file), None, "{file:?}");
        }
        // 作業領域の外（絶対パス、.. で出る、外を指すリンク）
        let secret = base.0.join("outside/secret.txt");
        assert_eq!(
            locate_in_package(&ws, "pkg", secret.to_str().unwrap()),
            None
        );
        assert_eq!(locate_in_package(&ws, "pkg", "../outside/secret.txt"), None);
        assert_eq!(
            locate_in_package(&ws, "pkg", "../../outside/secret.txt"),
            None
        );
        assert_eq!(locate_in_package(&ws, "", "../outside/secret.txt"), None);
        symlink(&secret, ws.join("pkg/src/link.rs")).unwrap();
        assert_eq!(locate_in_package(&ws, "pkg", "src/link.rs"), None);
        // ws2 のような、名前が前方一致する別のフォルダは作業領域の中ではない
        assert_eq!(locate_in_package(&ws, "pkg", "../../ws2/other.txt"), None);
        // パッケージのパスが作業領域の外
        assert_eq!(locate_in_package(&ws, "../outside", "secret.txt"), None);
    }

    #[test]
    fn resolves_empty_path_to_root() {
        let (_base, ws) = setup("empty");
        assert_eq!(resolve(&ws, ""), Ok(ws.clone()));
    }

    #[test]
    fn allows_paths_inside() {
        let (_base, ws) = setup("inside");
        assert_eq!(resolve(&ws, "src/main.rs"), Ok(ws.join("src/main.rs")));
        assert_eq!(resolve(&ws, "src"), Ok(ws.join("src")));
        assert_eq!(resolve(&ws, ""), Ok(ws.clone()));
        assert_eq!(resolve(&ws, "./src/main.rs"), Ok(ws.join("src/main.rs")));
    }

    #[test]
    fn rejects_invalid_forms() {
        let (_base, ws) = setup("invalid");
        for path in [
            "/etc/passwd",
            "../outside/secret.txt",
            "src/../../outside",
            "src//main.rs",
            "src/",
            "/",
            "src\0main.rs",
        ] {
            assert_eq!(resolve(&ws, path), Err(PathError::Invalid), "{path:?}");
        }
    }

    #[test]
    fn rejects_sibling_with_common_prefix() {
        let (base, ws) = setup("prefix");
        // ws の中に、ws2 を指すリンクを作る。文字列の比較なら `/…/ws2` は `/…/ws` で始まる
        symlink(base.0.join("ws2"), ws.join("to-ws2")).unwrap();
        assert_eq!(
            resolve(&ws, "to-ws2/other.txt"),
            Err(PathError::OutsideWorkspace)
        );
    }

    #[test]
    fn rejects_links_to_outside() {
        let (base, ws) = setup("outside");
        symlink(base.0.join("outside/secret.txt"), ws.join("file-link")).unwrap();
        symlink(base.0.join("outside"), ws.join("dir-link")).unwrap();
        assert_eq!(resolve(&ws, "file-link"), Err(PathError::OutsideWorkspace));
        // 外を指すフォルダのリンクは、一覧しようとしても拒否される
        assert_eq!(resolve(&ws, "dir-link"), Err(PathError::OutsideWorkspace));
        assert_eq!(
            resolve(&ws, "dir-link/secret.txt"),
            Err(PathError::OutsideWorkspace)
        );
    }

    #[test]
    fn allows_links_to_inside() {
        let (_base, ws) = setup("link-inside");
        symlink(ws.join("src/main.rs"), ws.join("main-link")).unwrap();
        assert_eq!(resolve(&ws, "main-link"), Ok(ws.join("src/main.rs")));
    }

    #[test]
    fn lists_directories_first_then_names() {
        let (base, ws) = setup("list");
        fs::write(ws.join("b.txt"), "").unwrap();
        fs::write(ws.join("A.txt"), "").unwrap();
        fs::create_dir(ws.join("tests")).unwrap();
        symlink(base.0.join("outside"), ws.join("outside-link")).unwrap();
        let names: Vec<(String, bool)> = list_dir(&ws, "")
            .unwrap()
            .unwrap()
            .into_iter()
            .map(|entry| (entry.name, entry.is_dir))
            .collect();
        assert_eq!(
            names,
            vec![
                ("outside-link".to_string(), true),
                ("src".to_string(), true),
                ("tests".to_string(), true),
                ("A.txt".to_string(), false),
                ("b.txt".to_string(), false),
            ]
        );
        // 外を指すリンクは一覧に出るが、その中は一覧できない
        assert!(list_dir(&ws, "outside-link").is_err());
        let src = list_dir(&ws, "src").unwrap().unwrap();
        assert_eq!(src[0].path, "src/main.rs");
        // 外で消えたフォルダ・ファイルになったものは、誤りではなく「ない」（D-156）
        assert_eq!(list_dir(&ws, "gone"), Ok(None));
        assert_eq!(list_dir(&ws, "b.txt"), Ok(None));
        assert!(list_dir(&ws, "../x").is_err());
    }

    #[test]
    fn reads_utf8_within_limit() {
        let (_base, ws) = setup("read");
        assert_eq!(
            read_text(&ws, "src/main.rs").map(|r| r.content),
            Ok("fn main() {}".to_string())
        );
        fs::write(ws.join("exact.txt"), vec![b'a'; MAX_FILE_SIZE as usize]).unwrap();
        assert!(read_text(&ws, "exact.txt").is_ok());
    }

    #[test]
    fn rejects_large_and_non_utf8_files() {
        let (_base, ws) = setup("read-reject");
        fs::write(ws.join("large.txt"), vec![b'a'; MAX_FILE_SIZE as usize + 1]).unwrap();
        assert!(read_text(&ws, "large.txt").is_err());
        fs::write(ws.join("sjis.txt"), [0x82, 0xa0]).unwrap();
        assert!(read_text(&ws, "sjis.txt").is_err());
        assert!(read_text(&ws, "src").is_err());
    }

    #[test]
    fn writes_only_existing_files() {
        let (base, ws) = setup("write");
        write_existing(&ws, "src/main.rs", "fn main() { 1; }", None).unwrap();
        assert_eq!(
            fs::read_to_string(ws.join("src/main.rs")).unwrap(),
            "fn main() { 1; }"
        );
        assert!(write_existing(&ws, "new.rs", "", None).is_err());
        assert!(!ws.join("new.rs").exists());
        assert!(write_existing(&ws, "src", "", None).is_err());
        symlink(base.0.join("outside/secret.txt"), ws.join("secret-link")).unwrap();
        assert!(write_existing(&ws, "secret-link", "changed", None).is_err());
        assert_eq!(
            fs::read_to_string(base.0.join("outside/secret.txt")).unwrap(),
            "secret"
        );
    }

    #[test]
    fn creates_files_without_overwriting() {
        let (base, ws) = setup("create");
        assert_eq!(
            create_file(&ws, "src", "lib.rs"),
            Ok("src/lib.rs".to_string())
        );
        assert_eq!(fs::read_to_string(ws.join("src/lib.rs")).unwrap(), "");
        assert_eq!(create_file(&ws, "", "a.txt"), Ok("a.txt".to_string()));
        // 同じ名前のファイル・フォルダがあれば作らない
        assert!(create_file(&ws, "src", "main.rs").is_err());
        assert_eq!(
            fs::read_to_string(ws.join("src/main.rs")).unwrap(),
            "fn main() {}"
        );
        assert!(create_file(&ws, "", "src").is_err());
        // 名前の検証と、作業領域の外
        for name in ["", ".", "..", "a/b", "a\0b"] {
            assert!(create_file(&ws, "", name).is_err(), "{name:?}");
        }
        symlink(base.0.join("outside"), ws.join("dir-link")).unwrap();
        assert!(create_file(&ws, "dir-link", "x.txt").is_err());
        assert!(!base.0.join("outside/x.txt").exists());
    }

    #[test]
    fn renames_files_in_same_folder() {
        let (base, ws) = setup("rename");
        assert_eq!(
            rename_file(&ws, "src/main.rs", "lib.rs"),
            Ok("src/lib.rs".to_string())
        );
        assert!(ws.join("src/lib.rs").exists());
        assert!(!ws.join("src/main.rs").exists());
        // 大文字・小文字だけを変えられる（区別しないディスクでも、区別するディスクでも）
        assert_eq!(
            rename_file(&ws, "src/lib.rs", "Lib.rs"),
            Ok("src/Lib.rs".to_string())
        );
        let names: Vec<String> = fs::read_dir(ws.join("src"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"Lib.rs".to_string()), "{names:?}");
        assert!(!names.contains(&"lib.rs".to_string()), "{names:?}");
        assert_eq!(
            rename_file(&ws, "src/Lib.rs", "lib.rs"),
            Ok("src/lib.rs".to_string())
        );
        // ハードリンク（別の名前の同じもの）には変えない
        fs::hard_link(ws.join("src/lib.rs"), ws.join("src/link.rs")).unwrap();
        assert!(rename_file(&ws, "src/lib.rs", "link.rs").is_err());
        fs::remove_file(ws.join("src/link.rs")).unwrap();
        // 同じ名前があれば変えない
        fs::write(ws.join("src/other.rs"), "other").unwrap();
        assert!(rename_file(&ws, "src/lib.rs", "other.rs").is_err());
        assert_eq!(
            fs::read_to_string(ws.join("src/other.rs")).unwrap(),
            "other"
        );
        // 別のフォルダへは移さない（名前に `/` は使えない）
        assert!(rename_file(&ws, "src/lib.rs", "../lib.rs").is_err());
        // フォルダの名前も変えられる（D-157）。作業領域のルートは変えない
        assert_eq!(rename_file(&ws, "src", "source"), Ok("source".to_string()));
        assert!(ws.join("source/lib.rs").exists());
        assert!(rename_file(&ws, "", "x").is_err());
        // 外を指すリンクは、リンク自体の名前を変える（リンクの先は変わらない）
        symlink(base.0.join("outside/secret.txt"), ws.join("secret-link")).unwrap();
        assert_eq!(
            rename_file(&ws, "secret-link", "renamed-link"),
            Ok("renamed-link".to_string())
        );
        assert!(base.0.join("outside/secret.txt").exists());
    }

    // D-157：フォルダの作成・数え方・見分け方
    #[test]
    fn creates_counts_and_tells_folders() {
        let (base, ws) = setup("folder");
        assert_eq!(
            create_folder(&ws, "src", "util"),
            Ok("src/util".to_string())
        );
        assert!(ws.join("src/util").is_dir());
        // 同じ名前（ファイル・フォルダ）があれば作らない
        assert!(create_folder(&ws, "src", "util").is_err());
        assert!(create_folder(&ws, "src", "main.rs").is_err());
        // 名前・場所の確かめ
        assert!(create_folder(&ws, "src", "a/b").is_err());
        assert!(create_folder(&ws, "src", "..").is_err());
        symlink(base.0.join("outside"), ws.join("dir-link")).unwrap();
        assert!(create_folder(&ws, "dir-link", "x").is_err());
        assert!(!base.0.join("outside/x").exists());
        // 数える：中のフォルダの中も数え、リンクはたどらない。上限で止める
        fs::write(ws.join("src/util/a.rs"), "").unwrap();
        symlink(base.0.join("outside"), ws.join("src/util/out-link")).unwrap();
        assert_eq!(count_files(&ws.join("src"), 10_000), 3);
        assert_eq!(count_files(&ws.join("src"), 2), 2);
        // フォルダか：リンクは先がフォルダでもフォルダとしない。ルートは断る
        assert_eq!(is_folder(&ws, "src"), Ok(true));
        assert_eq!(is_folder(&ws, "src/main.rs"), Ok(false));
        assert_eq!(is_folder(&ws, "dir-link"), Ok(false));
        assert!(is_folder(&ws, "").is_err());
    }

    #[test]
    fn detects_external_changes_before_writing() {
        let (_base, ws) = setup("conflict");
        let read = read_text(&ws, "src/main.rs").unwrap();
        // 読み込んだときから変わっていなければ書ける。書いた後の更新日時と大きさが返る
        let WriteResult::Saved { stamp } =
            write_existing(&ws, "src/main.rs", "fn main() { 1; }", Some(&read.stamp)).unwrap()
        else {
            panic!("書けるはず");
        };
        assert_eq!(current_stamp(&ws, "src/main.rs"), Some(stamp.clone()));
        // 外部で変更されたら、古い更新日時と大きさでは書かない
        fs::write(ws.join("src/main.rs"), "changed outside").unwrap();
        assert_eq!(
            write_existing(&ws, "src/main.rs", "mine", Some(&stamp)),
            Ok(WriteResult::Conflict)
        );
        assert_eq!(
            fs::read_to_string(ws.join("src/main.rs")).unwrap(),
            "changed outside"
        );
        assert_eq!(current_stamp(&ws, "no-such-file"), None);
    }

    #[test]
    fn keeps_permissions_and_hard_links_when_writing() {
        use std::os::unix::fs::PermissionsExt;
        let (_base, ws) = setup("w2");
        let file = ws.join("src/main.rs");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        fs::hard_link(&file, ws.join("hard-link.rs")).unwrap();
        write_existing(&ws, "src/main.rs", "fn main() { 2; }", None).unwrap();
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            fs::read_to_string(ws.join("hard-link.rs")).unwrap(),
            "fn main() { 2; }"
        );
    }

    // 実際にゴミ箱に移すテストはしない（利用者のゴミ箱を汚すため）。拒否される場合だけを確かめる
    #[test]
    fn trash_rejects_root_and_outside() {
        let (_base, ws) = setup("trash");
        // 作業領域のルートは消さない。フォルダはゴミ箱に移せる（D-157）が、テストでは本物のゴミ箱に移さないため試さない
        assert!(trash_file(&ws, "").is_err());
        assert!(trash_file(&ws, "../outside/secret.txt").is_err());
        assert!(trash_file(&ws, "no-such-file").is_err());
        assert!(ws.join("src/main.rs").exists());
    }

    // リンクを削除すると、リンク自体だけが消え、リンクの先（ファイル・フォルダ）は残る（D-102）
    #[test]
    fn deletes_links_without_touching_targets() {
        let (base, ws) = setup("trash-links");
        symlink(base.0.join("outside/secret.txt"), ws.join("file-link")).unwrap();
        symlink(base.0.join("outside"), ws.join("dir-link")).unwrap();
        assert!(is_link(&ws, "file-link").unwrap());
        assert!(!is_link(&ws, "src/main.rs").unwrap());
        trash_file(&ws, "file-link").unwrap();
        trash_file(&ws, "dir-link").unwrap();
        assert!(fs::symlink_metadata(ws.join("file-link")).is_err());
        assert!(fs::symlink_metadata(ws.join("dir-link")).is_err());
        assert_eq!(
            fs::read_to_string(base.0.join("outside/secret.txt")).unwrap(),
            "secret"
        );
        assert!(base.0.join("outside").is_dir());
        // 先がないリンクも消せる
        symlink(base.0.join("no-such-file"), ws.join("broken-link")).unwrap();
        trash_file(&ws, "broken-link").unwrap();
        assert!(fs::symlink_metadata(ws.join("broken-link")).is_err());
    }

    #[test]
    fn rejects_link_loops_and_missing() {
        let (_base, ws) = setup("loop");
        symlink(ws.join("loop-b"), ws.join("loop-a")).unwrap();
        symlink(ws.join("loop-a"), ws.join("loop-b")).unwrap();
        assert_eq!(resolve(&ws, "loop-a"), Err(PathError::NotFound));
        assert_eq!(resolve(&ws, "no-such-file"), Err(PathError::NotFound));
    }
}
