// Windows の API の小さな包み（Windows 移植の段階 1。docs/windows-port-plan.md 7 章 C3）
//
// - プロセス：pid でプロセスを開き、終わるのを待つ・終わったかを確かめる・止める。標準ライブラリの Child がハンドルを
//   持っている間は、pid は別のプロセスに再利用されない（推測。未確認）
// - ファイル：2 つのパスが同じもの（同じボリュームの同じファイルの番号）か。Unix の dev・ino にあたる
// - 時刻：UNIX 時刻（ミリ秒）を手元の時刻の月・日・時・分にする（Unix の localtime_r にあたる）
//
// 限界（段階 1 の仮の実装）：止めるのは直接の子だけで、子孫は残る。使用メモリは測らない。
// 段階 2 で Job Object（CREATE_SUSPENDED で起動して入れる。9.1 の 1）に置き換える

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{
    CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, STILL_ACTIVE, SYSTEMTIME, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, GetFileInformationByHandle, OPEN_EXISTING,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, INFINITE, OpenProcess, PROCESS_ACCESS_RIGHTS,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess,
    WaitForSingleObject,
};
use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

// 開いたハンドル。捨てるときに閉じる
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: OpenProcess・CreateFileW が返した、まだ閉じていないハンドルを 1 回だけ閉じる
        unsafe {
            CloseHandle(self.0);
        }
    }
}

// ---- プロセス ----

fn open_process(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Option<Handle> {
    // SAFETY: 引数は値だけ。失敗すれば null が返る
    let handle = unsafe { OpenProcess(access, 0, pid) };
    (!handle.is_null()).then_some(Handle(handle))
}

fn exit_code(handle: &Handle) -> Option<u32> {
    let mut code = 0u32;
    // SAFETY: 開いたハンドルと、書き込み先の u32 を渡す
    (unsafe { GetExitCodeProcess(handle.0, &mut code) } != 0).then_some(code)
}

// 終わるまで待ち、終了コードを返す。開けない・読めないときは None
pub fn wait_for_exit(pid: u32) -> Option<u32> {
    let handle = open_process(pid, PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION)?;
    // SAFETY: 開いたハンドルを渡す
    if unsafe { WaitForSingleObject(handle.0, INFINITE) } != WAIT_OBJECT_0 {
        return None;
    }
    exit_code(&handle)
}

// 終わったか（待たない）。開けないときも、終わったものとして扱う
pub fn has_exited(pid: u32) -> bool {
    match open_process(pid, PROCESS_SYNCHRONIZE) {
        // SAFETY: 開いたハンドルを渡す。0 ミリ秒なので待たない
        Some(handle) => (unsafe { WaitForSingleObject(handle.0, 0) }) == WAIT_OBJECT_0,
        None => true,
    }
}

// まだ動いているか（終了コードが STILL_ACTIVE）
pub fn is_alive(pid: u32) -> bool {
    open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)
        .and_then(|handle| exit_code(&handle))
        .is_some_and(|code| code == STILL_ACTIVE as u32)
}

// 止める（直接の子だけ。子孫は残る）。終了コードは 1 にする
pub fn terminate(pid: u32) {
    if let Some(handle) = open_process(pid, PROCESS_TERMINATE) {
        // SAFETY: 開いたハンドルを渡す。すでに終わっていれば失敗するだけ
        unsafe {
            TerminateProcess(handle.0, 1);
        }
    }
}

// ---- ファイル ----

// 終わりに 0 の付いた UTF-16
fn wide(path: &Path) -> Vec<u16> {
    OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

// リンクはたどらずに、そのもの自体を開き、ボリュームのシリアル番号とファイルの番号を返す
fn file_id(path: &Path) -> Option<(u32, u32, u32)> {
    let name = wide(path);
    // SAFETY: 終わりに 0 の付いた UTF-16 の名前を渡す。読み取るのは属性だけ
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let handle = Handle(handle);
    // SAFETY: BY_HANDLE_FILE_INFORMATION はゼロで初期化してよい C の構造体で、書き込み先として渡すだけ
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: 開いたハンドルと、書き込み先を渡す
    if unsafe { GetFileInformationByHandle(handle.0, &mut info) } == 0 {
        return None;
    }
    Some((
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
    ))
}

// 2 つのパスが同じもの（リンクならリンク自体）を指すか。どちらかを開けなければ false。
// 限界：ReFS の 128 ビットのファイルの番号の上位は見ない（未確認）
pub fn same_entry(a: &Path, b: &Path) -> bool {
    match (file_id(a), file_id(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

// 名前の変更で置き換える（fs::rename）。Windows では、同じファイルをほかのスレッド・プロセスが同時に置き換えている・開いていると、
// 「アクセスが拒否されました」（5）・「共有違反」（32）で失敗する（同時に保存するテストで確認済み）。その間だけ少し待って試し直す
// （docs/windows-port-plan.md 3.3。待つのは合わせて最大で約 2 秒）
pub fn rename_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    const RETRIES: u32 = 100;
    const WAIT: std::time::Duration = std::time::Duration::from_millis(20);
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Err(error) if matches!(error.raw_os_error(), Some(5 | 32)) && tries < RETRIES => {
                tries += 1;
                std::thread::sleep(WAIT);
            }
            result => return result,
        }
    }
}

// ---- 時刻 ----

// 1601-01-01 から 1970-01-01 までの、100 ナノ秒の数
const UNIX_EPOCH_IN_FILETIME: u64 = 116_444_736_000_000_000;

// UNIX 時刻（ミリ秒）を、手元の時刻の（月, 日, 時, 分）にする。変えられなければ None。
// 限界：手元のタイムゾーンの、今の規則で変える（過去の夏時間の扱いが localtime_r と同じかは未確認）
pub fn local_month_day_time(ms: u64) -> Option<(u16, u16, u16, u16)> {
    let ticks = ms
        .checked_mul(10_000)?
        .checked_add(UNIX_EPOCH_IN_FILETIME)?;
    let file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    // SAFETY: SYSTEMTIME はゼロで初期化してよい C の構造体で、書き込み先として渡すだけ
    let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
    let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
    // SAFETY: 読む FILETIME と、書き込み先を渡す
    if unsafe { FileTimeToSystemTime(&file_time, &mut utc) } == 0 {
        return None;
    }
    // SAFETY: タイムゾーンは null（今のタイムゾーン）。読む時刻と、書き込み先を渡す
    if unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) } == 0 {
        return None;
    }
    Some((local.wMonth, local.wDay, local.wHour, local.wMinute))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_and_terminates_processes() {
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "exit 7"])
            .spawn()
            .unwrap();
        assert_eq!(wait_for_exit(child.id()), Some(7));
        assert!(has_exited(child.id()));
        assert!(!is_alive(child.id()));
        child.wait().unwrap();

        // 待ち続けるプロセス（ping は cmd を通さず直接起動する）
        let mut child = std::process::Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        assert!(is_alive(child.id()));
        assert!(!has_exited(child.id()));
        terminate(child.id());
        assert_eq!(wait_for_exit(child.id()), Some(1));
        child.wait().unwrap();
    }

    #[test]
    fn tells_the_same_file() {
        let dir = std::env::temp_dir().join(format!("atcoder-studio-win-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        // 大文字・小文字だけが違う名前は同じもの
        assert!(same_entry(&dir.join("a.txt"), &dir.join("A.TXT")));
        assert!(!same_entry(&dir.join("a.txt"), &dir.join("b.txt")));
        assert!(!same_entry(&dir.join("a.txt"), &dir.join("none.txt")));
        // ハードリンクは同じもの（Unix の dev・ino と同じ）
        std::fs::hard_link(dir.join("a.txt"), dir.join("hard.txt")).unwrap();
        assert!(same_entry(&dir.join("a.txt"), &dir.join("hard.txt")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn converts_unix_time_to_local_time() {
        // 1 分後は、分が 1 つ進む（59 分なら 0 分）
        let base = 1_761_000_000_000u64;
        let (_, _, _, minute) = local_month_day_time(base).unwrap();
        let (_, _, _, next) = local_month_day_time(base + 60_000).unwrap();
        assert_eq!(next, (minute + 1) % 60);
        let (month, day, hour, minute) = local_month_day_time(base).unwrap();
        assert!((1..=12).contains(&month) && (1..=31).contains(&day));
        assert!(hour < 24 && minute < 60);
    }
}
