// Windows の API の小さな包み（Windows 移植の段階 1。docs/windows-port-plan.md 7 章 C3）
//
// - プロセス：pid でプロセスを開き、終わるのを待つ・終わったかを確かめる・止める。標準ライブラリの Child がハンドルを
//   持っている間は、pid は別のプロセスに再利用されない（推測。未確認）
// - Job：子を Job に入れ、止めるときは子孫ごと止める（段階 2 S3。下の「Job」の節）。使用メモリを読む（S4）
// - ファイル：2 つのパスが同じもの（同じボリュームの同じファイルの番号）か。Unix の dev・ino にあたる。
//   止めた直後のビルドの前に、exe が書けるようになるまで待つ（S3b）
// - 時刻：UNIX 時刻（ミリ秒）を手元の時刻の月・日・時・分にする（Unix の localtime_r にあたる）

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::process::Child;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, DuplicateHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, STILL_ACTIVE, SYSTEMTIME,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, GetFileInformationByHandle, OPEN_EXISTING,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcess, OpenThread,
    PROCESS_ACCESS_RIGHTS, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, PROCESS_VM_READ, ResumeThread, THREAD_SUSPEND_RESUME, TerminateProcess,
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

// 止める（直接の子だけ。子孫は残る）。終了コードは 1 にする。Job に入れていないプロセス（デバッグの対象など）に使う
pub fn terminate(pid: u32) {
    if let Some(handle) = open_process(pid, PROCESS_TERMINATE) {
        // SAFETY: 開いたハンドルを渡す。すでに終わっていれば失敗するだけ
        unsafe {
            TerminateProcess(handle.0, 1);
        }
    }
}

// ---- Job（段階 2 S3。docs/windows-port-plan.md 9.1 の 1） ----
//
// IDE が起動する子（解答・ビルド・言語サーバーなど。process.rs で process_group(0) を付けたもの）は、CREATE_SUSPENDED で
// 止めたまま起動し、start_in_job で Job に入れてから主スレッドを再開する（子が孫を作る前に Job に入る）。
// Job には KILL_ON_JOB_CLOSE を付けるので、IDE が異常に終わって Job のハンドルが閉じても、子と孫は止まる。
// 子孫は Job から抜けられない（JOB_OBJECT_LIMIT_BREAKAWAY_OK を付けない）。
// Job は pid ごとに覚えておき、kill_job（process.rs の kill_group）で TerminateJobObject し、Job が空になるまで待ってから閉じる
// （止めた直後に exe を上書きできるように。4.7）。Unix の作り（回収する前に必ずグループを止める）と同じく、kill_job は
// 子を回収する前に必ず呼ばれる（run・run_pair・lsp の stop・debug の shutdown）。子が自然に終わっていても、残った孫は
// ここで止まる。2 回目からは何もしない。覚えた pid は release_job（process.rs の reap）で忘れる。lsp・debug は reap を
// 通らないので、閉じた印だけが残る（同じ pid が再利用されたら上書きする）。
// 記録には、起動したときのプロセスのハンドルの写しも持ち、使用メモリ（今の WorkingSet と、終わったあとの最大）はそれで読む
// （pid で開き直すと、pid が使い回されたときに別のプロセスを読むおそれがあるため。段階 2 S4）

// IDE が止めたときの終了コード（9.1 の追加 10）。上位の E は「エラー」と「利用者が定める値」の印で、NTSTATUS と重ならない
pub const KILLED_EXIT_CODE: u32 = 0xE0AC_0001;
// 止めたあと、Job の中のプロセスがすべて終わるのを待つ時間の上限
const JOB_EMPTY_WAIT: Duration = Duration::from_secs(5);

struct Job {
    // Job のハンドル。kill_job で止めて閉じたら None
    job: Option<Handle>,
    // 起動したときのプロセスのハンドルの写し（使用メモリを読む）
    process: Handle,
}

// SAFETY: ハンドルはプロセスの中のどのスレッドからでも使え、閉じられる
unsafe impl Send for Handle {}

static JOBS: LazyLock<Mutex<HashMap<u32, Job>>> = LazyLock::new(Default::default);

fn jobs() -> MutexGuard<'static, HashMap<u32, Job>> {
    JOBS.lock().unwrap_or_else(PoisonError::into_inner)
}

// Job に入れる段階（テストで、それぞれの段階の失敗を起こすのに使う）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobStep {
    Create,
    Assign,
    Resume,
}

// CREATE_SUSPENDED で起動した子を Job に入れ、主スレッドを再開する。失敗したら子を止めて（止まったまま残さない）エラーにする
pub fn start_in_job(child: &Child) -> Result<(), String> {
    start_in_job_with(child, |_| false)
}

// fail が true を返した段階で失敗したことにする（テスト用。本番は常に false）
pub(crate) fn start_in_job_with(
    child: &Child,
    fail: impl Fn(JobStep) -> bool,
) -> Result<(), String> {
    let process = child.as_raw_handle() as HANDLE;
    // 失敗したら子を止める。Job を閉じる前に止める（先に Job を閉じると KILL_ON_JOB_CLOSE で止まり、終了コードが 0 になる）
    let stop = |message: String| {
        // SAFETY: Child が持つ、まだ開いているプロセスのハンドルを渡す
        unsafe {
            TerminateProcess(process, KILLED_EXIT_CODE);
        }
        Err(message)
    };
    let job = match create_job(&fail) {
        Ok(job) => job,
        Err(message) => return stop(message),
    };
    if let Err(message) = enter_job(&job, process, child.id(), &fail) {
        // job はこのあと（子を止めたあと）閉じる
        return stop(message);
    }
    let copy = match duplicate_process(process) {
        Ok(copy) => copy,
        Err(message) => return stop(message),
    };
    jobs().insert(
        child.id(),
        Job {
            job: Some(job),
            process: copy,
        },
    );
    Ok(())
}

fn last_error(what: &str) -> String {
    format!("{what}（{}）", std::io::Error::last_os_error())
}

// Child が持つプロセスのハンドルの写しを作る（使用メモリを読む権限だけ）。Child のハンドルとは別に閉じる
fn duplicate_process(process: HANDLE) -> Result<Handle, String> {
    let mut copy: HANDLE = std::ptr::null_mut();
    // SAFETY: 自分のプロセスの中で、Child が持つ開いているハンドルの写しを作る。書き込み先を渡す
    let copied = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            process,
            GetCurrentProcess(),
            &mut copy,
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
            0,
            0,
        )
    };
    if copied == 0 {
        return Err(last_error("プロセスのハンドルを写せません"));
    }
    Ok(Handle(copy))
}

// プロセスの使用メモリの数（PROCESS_MEMORY_COUNTERS）。読めなければ None
fn memory_counters(process: &Handle) -> Option<PROCESS_MEMORY_COUNTERS> {
    // SAFETY: PROCESS_MEMORY_COUNTERS はゼロで初期化してよい C の構造体。cb に大きさを入れてから渡す
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: 開いているプロセスのハンドルと、書き込み先とその大きさを渡す
    (unsafe { K32GetProcessMemoryInfo(process.0, &mut counters, counters.cb) } != 0)
        .then_some(counters)
}

// 動いている子の今の使用メモリ（WorkingSetSize。バイト）。起動したときのハンドルの写しで読む（段階 2 S4）。
// Job に入れていない・読めないなら None
pub fn working_set(pid: u32) -> Option<u64> {
    let jobs = jobs();
    memory_counters(&jobs.get(&pid)?.process).map(|counters| counters.WorkingSetSize as u64)
}

// KILL_ON_JOB_CLOSE を付けた Job を作る
fn create_job(fail: &impl Fn(JobStep) -> bool) -> Result<Handle, String> {
    // SAFETY: 名前もセキュリティ属性もない Job を作る。失敗すれば null が返る
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw.is_null() {
        return Err(last_error("Job を作れません"));
    }
    let job = Handle(raw);
    if fail(JobStep::Create) {
        return Err("Job を作れません（テスト）".to_string());
    }
    // SAFETY: JOBOBJECT_EXTENDED_LIMIT_INFORMATION はゼロで初期化してよい C の構造体
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: 作った Job と、読む構造体とその大きさを渡す
    let set = unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if set == 0 {
        return Err(last_error("Job を設定できません"));
    }
    Ok(job)
}

// 子を Job に入れ、主スレッドを再開する
fn enter_job(
    job: &Handle,
    process: HANDLE,
    pid: u32,
    fail: &impl Fn(JobStep) -> bool,
) -> Result<(), String> {
    // SAFETY: 作った Job と、Child が持つプロセスのハンドルを渡す
    if unsafe { AssignProcessToJobObject(job.0, process) } == 0 {
        return Err(last_error("Job に入れられません"));
    }
    if fail(JobStep::Assign) {
        return Err("Job に入れられません（テスト）".to_string());
    }
    if fail(JobStep::Resume) {
        return Err("主スレッドを再開できません（テスト）".to_string());
    }
    resume_threads(pid)
}

// pid のプロセスのスレッドを再開する。CREATE_SUSPENDED で起動したばかりなので、スレッドは主スレッドの 1 つだけ（推測）。
// 標準ライブラリは起動したあと主スレッドのハンドルを閉じるので、スレッドの一覧から探す
fn resume_threads(pid: u32) -> Result<(), String> {
    // SAFETY: 引数は値だけ。失敗すれば INVALID_HANDLE_VALUE が返る
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(last_error("スレッドの一覧を取れません"));
    }
    let snapshot = Handle(snapshot);
    // SAFETY: THREADENTRY32 はゼロで初期化してよい C の構造体。dwSize に大きさを入れてから渡す
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
    let mut resumed = 0;
    // SAFETY: 取った一覧と、書き込み先を渡す
    let mut more = unsafe { Thread32First(snapshot.0, &mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: 引数は値だけ。失敗すれば null が返る
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if thread.is_null() {
                return Err(last_error("主スレッドを開けません"));
            }
            let thread = Handle(thread);
            // SAFETY: 開いたスレッドのハンドルを渡す。失敗すれば u32::MAX が返る
            if unsafe { ResumeThread(thread.0) } == u32::MAX {
                return Err(last_error("主スレッドを再開できません"));
            }
            resumed += 1;
        }
        // SAFETY: 取った一覧と、書き込み先を渡す
        more = unsafe { Thread32Next(snapshot.0, &mut entry) } != 0;
    }
    if resumed == 0 {
        return Err("主スレッドが見つかりません".to_string());
    }
    Ok(())
}

// pid の子の Job の中を、すべて止める（直接の子と孫）。Job が空になるまで待ってから閉じる。
// 2 回目からは何もしない。Job に入れていない pid なら、作りの誤りなので debug では止め、release では直接の子だけを止める
pub fn kill_job(pid: u32) {
    let job = {
        let mut jobs = jobs();
        match jobs.get_mut(&pid) {
            Some(entry) => match entry.job.take() {
                Some(job) => job,
                None => return,
            },
            None => {
                drop(jobs);
                debug_assert!(
                    false,
                    "Job に入れていないプロセス（pid {pid}）を止めようとした"
                );
                terminate(pid);
                return;
            }
        }
    };
    // SAFETY: 開いている Job のハンドルを渡す
    unsafe {
        TerminateJobObject(job.0, KILLED_EXIT_CODE);
    }
    wait_until_empty(&job);
}

// Job の中で動いているプロセスが 0 になるまで待つ（最大 JOB_EMPTY_WAIT）。TerminateJobObject は終わるのを待たずに返る（推測）
fn wait_until_empty(job: &Handle) {
    let until = Instant::now() + JOB_EMPTY_WAIT;
    loop {
        // SAFETY: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION はゼロで初期化してよい C の構造体
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: 開いている Job と、書き込み先とその大きさを渡す
        let read = unsafe {
            QueryInformationJobObject(
                job.0,
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        };
        if read == 0 || info.ActiveProcesses == 0 || Instant::now() >= until {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

// 回収した子の記録を忘れ、最大使用メモリ（PeakWorkingSetSize。バイト）を返す。終わったあとに、起動したときのハンドルの
// 写しで読む（段階 2 S4）。読めなければ None。kill_job を呼ばずに回収した（作りの誤り）なら、debug では止め、release では
// Job を閉じる（KILL_ON_JOB_CLOSE で残りが止まる）
pub fn release_job(pid: u32) -> Option<u64> {
    let entry = jobs().remove(&pid);
    debug_assert!(
        entry.as_ref().is_some_and(|entry| entry.job.is_none()),
        "kill_job を呼ばずに回収した、または Job に入れていないプロセス（pid {pid}）"
    );
    memory_counters(&entry?.process).map(|counters| counters.PeakWorkingSetSize as u64)
}

// テスト用：止めずに Job のハンドルを閉じる（IDE が異常に終わったときにあたる）
#[cfg(test)]
pub(crate) fn close_job_without_terminating(pid: u32) {
    let entry = jobs().remove(&pid);
    drop(entry);
}

// ---- 書けるようになるまで待つ（段階 2 S3b。docs/windows-port-plan.md 4.7） ----
//
// 止めた直後の前の実行の exe は、Job が空になり、直接の子の終わりを待ったあとでも、10〜40 ms ほど書けないことがある
// （原因は未確認）。ビルドの前に、書き込みで開けるようになるまで待つ。開いてすぐ閉じるだけで、作らない・切り詰めない・
// 書かないので、中身も更新日時も変わらない

// ビルドの前に待つ時間の上限（全部の exe で合わせて）
pub const WRITABLE_WAIT: Duration = Duration::from_secs(2);
// 試し直す間隔
const WRITABLE_POLL: Duration = Duration::from_millis(10);

// path を書き込みで開けるようになるまで、最大 limit 待つ（少なくとも 1 回は試す）。
// 開けた・ない・共有違反とアクセス拒否のほかのエラー（あとのビルドに任せる）なら true。
// limit を過ぎても共有違反（32）かアクセス拒否（5）なら false
pub fn wait_until_writable(path: &Path, limit: Duration) -> bool {
    let until = Instant::now() + limit;
    loop {
        match std::fs::OpenOptions::new().write(true).open(path) {
            Ok(_) => return true,
            Err(error) if matches!(error.raw_os_error(), Some(5 | 32)) => {
                let left = until.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return false;
                }
                std::thread::sleep(left.min(WRITABLE_POLL));
            }
            Err(_) => return true,
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
// 諦めたときのエラーには、試し直した回数とかかった時間を書き足す（段階 2 S10c。同時の保存の失敗の原因を調べる）。
// 5・32 の試し直しを使い切って諦めたときは、ほかのプログラムが使っている可能性も案内する（段階 2 S10d。IDE の中の
// 同時の保存は storage::save_lock で 1 つずつにしているので、残るのは IDE の外のものが開いている場合と考える）
pub fn rename_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    const RETRIES: u32 = 100;
    const WAIT: std::time::Duration = std::time::Duration::from_millis(20);
    let started = Instant::now();
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Err(error) if matches!(error.raw_os_error(), Some(5 | 32)) && tries < RETRIES => {
                tries += 1;
                std::thread::sleep(WAIT);
            }
            Err(error) if tries > 0 => {
                let gave_up = matches!(error.raw_os_error(), Some(5 | 32));
                return Err(std::io::Error::new(
                    error.kind(),
                    format!(
                        "{error}。{tries} 回試し直した（{} ms）{}",
                        started.elapsed().as_millis(),
                        if gave_up {
                            "。ほかのプログラム（ウイルス対策など）が使っている可能性があります"
                        } else {
                            ""
                        }
                    ),
                ));
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

    // テスト用のフォルダ。捨てるときに消す
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let dir = std::env::temp_dir().join(format!(
                "atcoder-studio-writable-{}-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn does_not_wait_for_a_missing_file() {
        let dir = TempDir::new("missing");
        let started = Instant::now();
        assert!(wait_until_writable(
            &dir.0.join("none.exe"),
            Duration::from_secs(2)
        ));
        assert!(started.elapsed() < Duration::from_millis(50));
    }

    #[test]
    fn waits_while_another_handle_denies_writing() {
        // 共有を許さずに開いている間は書けない。limit まで待って false。閉じればすぐ true
        let dir = TempDir::new("held");
        let path = dir.0.join("main.exe");
        std::fs::write(&path, b"exe").unwrap();
        let held = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&path)
                .unwrap()
        };
        let started = Instant::now();
        let writable = wait_until_writable(&path, Duration::from_millis(300));
        let waited = started.elapsed();
        drop(held);
        assert!(!writable, "開いている間に書けることになった");
        assert!(waited >= Duration::from_millis(300), "{waited:?}");
        let started = Instant::now();
        assert!(wait_until_writable(&path, Duration::from_secs(2)));
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(std::fs::read(&path).unwrap(), b"exe");
    }

    #[test]
    fn waits_while_the_program_runs_without_changing_it() {
        // 動いている exe には書けない。止めれば 2 秒のうちに書けるようになる。確かめても中身と更新日時は変わらない
        let dir = TempDir::new("running");
        let path = dir.0.join("victim.exe");
        std::fs::copy(crate::process::test_child_path(), &path).unwrap();
        let before = std::fs::read(&path).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let mut child = std::process::Command::new(&path)
            .arg("sleep:30000")
            .spawn()
            .unwrap();
        let while_running = wait_until_writable(&path, Duration::from_millis(300));
        child.kill().unwrap();
        child.wait().unwrap();
        let started = Instant::now();
        let after_stop = wait_until_writable(&path, WRITABLE_WAIT);
        let waited = started.elapsed();
        assert!(!while_running, "動いている間に書けることになった");
        assert!(after_stop, "止めてから {waited:?} たっても書けない");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
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
