// 利用者のビルドとプログラムの起動・停止・上限（Phase 2 作業 3。D-095・D-096）
//
// - 新しいプロセスグループで起動し、止めるときはグループ全体に SIGKILL を送る
// - SIGKILL は、直接の子（グループの長）を回収する前に送る。回収した後は killpg を呼ばない
//   （グループが空になると、番号が再利用されて別のグループに届くおそれがあるため。推測）
// - 直接の子が自然に終わったときも、回収する前にグループの残りを止める。終わったことは
//   waitid の WNOWAIT で、回収せずに知る（標準ライブラリの Child::wait は回収まで行うため）
// - 標準出力と標準エラーは、別々のスレッドで並行して読む（片方が詰まらないように。05 T-03）
//
// 限界：setsid などでグループから抜けた子孫は止められない（D-095、AC2-13）。
// そうした子孫がパイプを持ち続けると、出力の読み取りは終わらないので、回収した後は
// 出力が READ_GRACE の間届かなくなるか、回収から DRAIN_LIMIT が過ぎたら、読み取りを打ち切る
// （結果の output_left_open が true になる）。
// macOS（Unix）だけを対象にする。Windows は将来、Job Object で別に作る（05 P-05-3）
//
// Windows（移植の段階 1。docs/windows-port-plan.md 7 章 C3）は仮の実装：Unix の関数と同じ名前・同じ形の関数を
// `#[cfg(windows)]` で置き、中身は win.rs に任せる。限界：止めるのは直接の子だけで子孫は残り、使用メモリは測らない
// （段階 2 で Job Object にする）。Unix の経路のコードは変えない

use std::fs::File;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

// 実行時間の上限（D-096）。通常実行・テストの 1 ケース。ビルドには設けない
pub const RUN_TIME_LIMIT: Duration = Duration::from_secs(10);
// 標準出力・標準エラーそれぞれの上限（D-096）
pub const OUTPUT_LIMIT: usize = 16 * 1024 * 1024;

// 停止の操作を確かめる間隔
const POLL_INTERVAL: Duration = Duration::from_millis(20);
// 回収した後、残りの出力を読み切るとき、出力が届かないまま待つ時間（届くたびに待ち直す）
const READ_GRACE: Duration = Duration::from_millis(500);
// 回収した後、残りの出力を読み切るのに使う時間の上限（抜けた子孫が出力し続けても返るように）。
// ためておける最大 1 MiB を読み切れる長さにした。値は作業 4 で画面につないだ後に見直す
const DRAIN_LIMIT: Duration = Duration::from_secs(5);
// 読み取りのスレッドから受け取る出力の、ためておける数（1 つは最大 64 KiB）。
// いっぱいになると読み取りが待つので、大量に出力しても IDE のメモリが増え続けない
const CHANNEL_CAPACITY: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    // 終了コードで終わった
    Exited(i32),
    // シグナルで終わった（panic の abort、Segmentation fault など）
    Signaled(i32),
    // 時間の上限で止めた
    TimedOut,
    // 出力の上限で止めた（どちらの出力か）
    OutputLimit(Stream),
    // 利用者の操作で止めた
    Stopped,
}

pub struct Limits {
    pub time: Option<Duration>,
    pub output: usize,
    // 実行中のメモリの上限（直接の子の今の RSS。バイト）。超えたらグループを止める（Phase 10 作業 18。点検の 12。D-175）。
    // 止めた理由は結果に残さず、終わってから受け取る最大 RSS で、呼び出し側が MLE と判定する（D-141）
    pub memory: Option<u64>,
}

// 外部のプログラムを起動する Command を作る。Unix では Command::new と同じ。
// Windows では、窓だけで端末を持たない IDE からコンソールのプログラム（cargo・g++・解答など）を起動すると、そのたびに
// コンソールの窓が出る（推測。未確認）ので、CREATE_NO_WINDOW を付ける（Windows 移植の段階 1 C6。docs/windows-port-plan.md 4.6）。
// creation_flags は上書きなので、段階 2 で CREATE_SUSPENDED などを足すときも、ここで組み合わせる
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[cfg_attr(unix, allow(unused_mut))]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    command
}

// プロセスの番号の型（Unix は pid_t。Windows の番号は u32 だが、Unix と同じ i32 で持ち、使うときに u32 に戻す）
#[cfg(unix)]
pub type Pid = libc::pid_t;
#[cfg(windows)]
pub type Pid = i32;

// Windows には、プロセスグループもシグナルもない。Unix の CommandExt::process_group と ExitStatusExt::signal と
// 同じ名前のものを置き、Unix と同じ書き方のまま動くようにする（何もしない・いつも None）。段階 2 で Job Object に置き換える
#[cfg(windows)]
pub(crate) trait ProcessGroupShim {
    fn process_group(&mut self, pgroup: i32) -> &mut Self;
}

#[cfg(windows)]
impl ProcessGroupShim for Command {
    fn process_group(&mut self, _pgroup: i32) -> &mut Self {
        self
    }
}

#[cfg(windows)]
trait SignalShim {
    fn signal(&self) -> Option<i32>;
}

#[cfg(windows)]
impl SignalShim for std::process::ExitStatus {
    fn signal(&self) -> Option<i32> {
        None
    }
}

// 動いているプロセスの今の使用メモリ（RSS。バイト）。読めなければ None（終わったあとなど）
#[cfg(unix)]
fn current_rss(pid: libc::pid_t) -> Option<u64> {
    // SAFETY: rusage_info_v0 はゼロで初期化してよい C の構造体で、proc_pid_rusage に書き込み先として渡すだけ
    let mut info: libc::rusage_info_v0 = unsafe { std::mem::zeroed() };
    let buffer = (&mut info as *mut libc::rusage_info_v0).cast::<libc::rusage_info_t>();
    let result = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V0, buffer) };
    (result == 0).then_some(info.ri_resident_size)
}

// Windows：段階 1 では測らない（メモリの上限で止めない。段階 2）
#[cfg(windows)]
fn current_rss(_pid: Pid) -> Option<u64> {
    None
}

pub struct RunResult {
    pub outcome: Outcome,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    // 起動から、終わった（または止めた）ことが分かるまでの実時間
    pub elapsed: Duration,
    // 回収した後も出力のパイプが閉じなかった（グループから抜けた子孫が持っている可能性。推測）
    pub output_left_open: bool,
    // 直接の子の最大使用メモリ（最大 RSS。バイト）。OS から受け取れなかったら None（D-141）。
    // 子孫のプロセスの分は含まない
    pub max_rss_bytes: Option<u64>,
}

enum Event {
    Output(Stream, Vec<u8>),
    Closed,
    Exited,
}

fn spawn_reader(mut source: impl Read + Send + 'static, stream: Stream, events: SyncSender<Event>) {
    thread::spawn(move || {
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            match source.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if events
                        .send(Event::Output(stream, buffer[..n].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
        let _ = events.send(Event::Closed);
    });
}

// 直接の子が終わるのを、回収せずに待つ（waitid の WNOWAIT）。終わった時刻を exited に記録する
// （出力がたまっていても、実行時間が遅れて記録されないように）。
// 限界：waitid が EINTR 以外で失敗したときも、終わったものとして扱う
#[cfg(unix)]
fn spawn_exit_watcher(pid: libc::pid_t, exited: Arc<OnceLock<Instant>>, events: SyncSender<Event>) {
    thread::spawn(move || {
        loop {
            // SAFETY: siginfo_t はゼロで初期化してよい C の構造体で、waitid に書き込み先として渡すだけ
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            // SAFETY: 自分が起動した子の pid を渡す。WNOWAIT なので子は回収されない
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOWAIT,
                )
            };
            if result == 0
                || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
            {
                break;
            }
        }
        let _ = exited.set(Instant::now());
        let _ = events.send(Event::Exited);
    });
}

// Windows：終わるのを WaitForSingleObject で待つ（ハンドルを待っても、Unix の回収にあたることは起きない）
#[cfg(windows)]
fn spawn_exit_watcher(pid: Pid, exited: Arc<OnceLock<Instant>>, events: SyncSender<Event>) {
    thread::spawn(move || {
        let _ = crate::win::wait_for_exit(pid as u32);
        let _ = exited.set(Instant::now());
        let _ = events.send(Event::Exited);
    });
}

// 直接の子を回収し、終わり方と最大使用メモリ（最大 RSS。バイト）を受け取る（wait4。D-141）。
// 標準ライブラリの Child::wait は使用メモリを返さないため、自分で回収する
#[cfg(unix)]
fn reap(pid: libc::pid_t) -> Result<(std::process::ExitStatus, Option<u64>), String> {
    loop {
        let mut status: libc::c_int = 0;
        // SAFETY: rusage はゼロで初期化してよい C の構造体で、wait4 に書き込み先として渡すだけ
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: 自分が起動し、まだ回収していない子の pid を渡す
        let result = unsafe { libc::wait4(pid, &mut status, 0, &mut usage) };
        if result == pid {
            let max_rss = u64::try_from(usage.ru_maxrss).ok();
            // ru_maxrss の単位は、macOS はバイト、Linux はキロバイト
            #[cfg(not(target_os = "macos"))]
            let max_rss = max_rss.map(|kib| kib * 1024);
            return Ok((std::process::ExitStatus::from_raw(status), max_rss));
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(format!("終了を確かめられません：{error}"));
        }
    }
}

// Windows：終わるのを待って終了コードを受け取る。最大使用メモリは段階 1 では測らない（None）
#[cfg(windows)]
fn reap(pid: Pid) -> Result<(std::process::ExitStatus, Option<u64>), String> {
    use std::os::windows::process::ExitStatusExt;
    let code = crate::win::wait_for_exit(pid as u32)
        .ok_or_else(|| "終了を確かめられません".to_string())?;
    Ok((std::process::ExitStatus::from_raw(code), None))
}

// グループ全体に SIGKILL を送る。直接の子を回収する前にだけ呼ぶ
#[cfg(unix)]
pub(crate) fn kill_group(pgid: libc::pid_t) {
    // SAFETY: 自分が起動し、まだ回収していない子の pid（＝グループの番号）を渡す
    unsafe {
        libc::killpg(pgid, libc::SIGKILL);
    }
}

// Windows：直接の子だけを止める（子孫は残る。段階 2 で Job Object の TerminateJobObject にする）
#[cfg(windows)]
pub(crate) fn kill_group(pid: Pid) {
    crate::win::terminate(pid as u32);
}

// 直接の子が終わったかを、回収せずに確かめる（waitid の WNOHANG・WNOWAIT）。rust-analyzer の停止で使う（Phase 3b-1）。
// 限界：waitid が失敗したときも、終わったものとして扱う
#[cfg(unix)]
pub(crate) fn exited_without_reaping(pid: libc::pid_t) -> bool {
    // SAFETY: siginfo_t はゼロで初期化してよい C の構造体で、waitid に書き込み先として渡すだけ
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: 自分が起動し、まだ回収していない子の pid を渡す。WNOWAIT なので子は回収されない
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    // WNOHANG で、まだ終わっていなければ si_pid が 0 のまま
    result != 0 || info.si_pid != 0
}

// Windows：待たずに確かめる。開けないときも、終わったものとして扱う
#[cfg(windows)]
pub(crate) fn exited_without_reaping(pid: Pid) -> bool {
    crate::win::has_exited(pid as u32)
}

// 直接の子が終わるまで、回収せずに待つ（waitid の WNOWAIT）。終了コードで終わったならその値を返す
// （シグナルで終わった・waitid が失敗したときは None）。rust-analyzer が自分で終わったことを知るのに使う（Phase 3b-1）
#[cfg(unix)]
pub(crate) fn wait_without_reaping(pid: libc::pid_t) -> Option<i32> {
    loop {
        // SAFETY: siginfo_t はゼロで初期化してよい C の構造体で、waitid に書き込み先として渡すだけ
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: 自分が起動した子の pid を渡す。WNOWAIT なので子は回収されない
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if result == 0 {
            return (info.si_code == libc::CLD_EXITED).then_some(info.si_status);
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return None;
        }
    }
}

// Windows：終わるまで待ち、終了コードを返す（Windows の終了コードは u32 なので、i32 として読む。例外の 0xC0000005 は負になる）
#[cfg(windows)]
pub(crate) fn wait_without_reaping(pid: Pid) -> Option<i32> {
    crate::win::wait_for_exit(pid as u32).map(|code| code as i32)
}

// command を新しいプロセスグループで起動し、終わるまで待つ。
// stdin が None なら、標準入力は空にする。cancel が true になったら止める。
// on_output には、読んだ出力を順に渡す（実行ログの表示に使う。作業 4）
pub fn run(
    mut command: Command,
    stdin: Option<File>,
    limits: &Limits,
    cancel: &Arc<AtomicBool>,
    mut on_output: impl FnMut(Stream, &[u8]),
) -> Result<RunResult, String> {
    command
        .process_group(0)
        .stdin(stdin.map_or_else(Stdio::null, Stdio::from))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let started = Instant::now();
    let mut child: Child = command
        .spawn()
        .map_err(|error| format!("起動できません：{error}"))?;
    let pid = child.id() as Pid;

    let (sender, events) = mpsc::sync_channel(CHANNEL_CAPACITY);
    let exited = Arc::new(OnceLock::new());
    spawn_reader(
        child.stdout.take().expect("piped"),
        Stream::Stdout,
        sender.clone(),
    );
    spawn_reader(
        child.stderr.take().expect("piped"),
        Stream::Stderr,
        sender.clone(),
    );
    spawn_exit_watcher(pid, exited.clone(), sender);

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut open = 2;
    let mut stopped: Option<Outcome> = None;

    // 出力を受け取る。上限を超えた分は捨て、上限を超えたら true を返す
    let mut accept = |stream: Stream, bytes: &[u8]| {
        let buffer = match stream {
            Stream::Stdout => &mut stdout,
            Stream::Stderr => &mut stderr,
        };
        let room = limits.output.saturating_sub(buffer.len());
        let kept = &bytes[..bytes.len().min(room)];
        buffer.extend_from_slice(kept);
        on_output(stream, kept);
        bytes.len() > room
    };

    // メモリの上限で止めたか（止めるのは 1 回だけ）
    let mut memory_killed = false;
    // 直接の子が終わるか、止めるまで待つ
    let exit_at = loop {
        match events.recv_timeout(POLL_INTERVAL) {
            Ok(Event::Output(stream, bytes)) => {
                if accept(stream, &bytes) && stopped.is_none() {
                    stopped = Some(Outcome::OutputLimit(stream));
                    kill_group(pid);
                }
            }
            Ok(Event::Closed) => open -= 1,
            Ok(Event::Exited) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break Instant::now(),
        }
        // 時間の上限は、終わった時刻（終わっていなければ今）で判定する
        let now = exited.get().copied().unwrap_or_else(Instant::now);
        if stopped.is_none() {
            if limits.time.is_some_and(|limit| now - started > limit) {
                stopped = Some(Outcome::TimedOut);
                kill_group(pid);
            } else if exited.get().is_none() && cancel.load(Ordering::SeqCst) {
                stopped = Some(Outcome::Stopped);
                kill_group(pid);
            } else if !memory_killed
                && exited.get().is_none()
                && limits
                    .memory
                    .is_some_and(|limit| current_rss(pid).is_some_and(|rss| rss > limit))
            {
                memory_killed = true;
                kill_group(pid);
            }
        }
        if let Some(at) = exited.get() {
            break *at;
        }
    };

    // 回収する前に、グループの残り（直接の子が残した子孫）を止める
    kill_group(pid);
    let (status, max_rss_bytes) = reap(pid)?;
    // 回収は済んだので、Child は待たずに捨てる（Child を捨てても、プロセスは待たない・止めない）
    drop(child);

    // 残りの出力を読み切る。ためておいた出力が多くても、届いている間は待ち続ける。
    // 出力が READ_GRACE の間届かないか、DRAIN_LIMIT が過ぎるか、出力の上限を超えるか、
    // 停止の操作があれば打ち切る
    let drain_until = Instant::now() + DRAIN_LIMIT;
    while open > 0 && !cancel.load(Ordering::SeqCst) {
        let Some(left) = drain_until.checked_duration_since(Instant::now()) else {
            break;
        };
        match events.recv_timeout(left.min(READ_GRACE)) {
            Ok(Event::Output(stream, bytes)) => {
                if accept(stream, &bytes) {
                    if stopped.is_none() {
                        stopped = Some(Outcome::OutputLimit(stream));
                    }
                    break;
                }
            }
            Ok(Event::Closed) => open -= 1,
            Ok(Event::Exited) => {}
            Err(_) => break,
        }
    }

    let outcome = stopped.unwrap_or_else(|| match (status.code(), status.signal()) {
        (Some(code), _) => Outcome::Exited(code),
        (None, Some(signal)) => Outcome::Signaled(signal),
        (None, None) => Outcome::Exited(-1),
    });
    Ok(RunResult {
        outcome,
        stdout,
        stderr,
        elapsed: exit_at - started,
        output_left_open: open > 0,
        max_rss_bytes,
    })
}

// ---- 2 つのプロセスを標準入出力でつなぐ（Phase 4c。D-144）----
//
// 解答の標準出力をジャッジの標準入力へ、ジャッジの標準出力を解答の標準入力へ中継する（インタラクティブ問題）。
// 2 つとも新しいプロセスグループで起動し、止めるときは両方のグループに SIGKILL を送る（回収する前に）。
// 中継したやり取りは、届いた順に記録する（解答の出力はジャッジに渡す前に記録するので、ジャッジの返事より先になる）。
// 限界：グループから抜けた子孫は止められない（run と同じ）。やり取りの順は、中継のスレッドがイベントを送った順で、
// OS が実際に書き込んだ順とは、わずかにずれることがある【推測】

// どちらの側か
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    Solution,
    Judge,
}

impl Side {
    fn index(self) -> usize {
        match self {
            Side::Solution => 0,
            Side::Judge => 1,
        }
    }
}

// 片方が異常に終わった後、もう片方に終わるのを待つ時間（その間にジャッジの標準エラーなどを集める）。過ぎたら止める
const PAIR_GRACE: Duration = Duration::from_millis(500);

enum PairEvent {
    // 中継した標準出力
    Chunk(Side, Vec<u8>),
    Stderr(Side, Vec<u8>),
    // 読み取り（中継または標準エラー）が終わった
    Closed,
    // 直接の子が終わった（回収はしていない）。終わった時刻つき
    Exited(Side, ExitKind, Instant),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitKind {
    Code(i32),
    // Windows にはシグナルがないので作られない
    #[cfg_attr(windows, allow(dead_code))]
    Signal(i32),
    Unknown,
}

impl ExitKind {
    // 0 以外の終了コード、またはシグナルでの終わり
    fn is_abnormal(self) -> bool {
        match self {
            ExitKind::Code(code) => code != 0,
            ExitKind::Signal(_) => true,
            ExitKind::Unknown => false,
        }
    }
}

// 片方のプロセスの結果
pub struct PairMember {
    pub outcome: Outcome,
    pub stderr: Vec<u8>,
    // 直接の子の最大使用メモリ（最大 RSS。バイト）。受け取れなかったら None
    pub max_rss_bytes: Option<u64>,
}

pub struct PairResult {
    pub solution: PairMember,
    pub judge: PairMember,
    // こちらが止める前に、自然に異常な終わり方（0 以外の終了コード・シグナル）をした、最初の側
    pub first_failure: Option<Side>,
    pub timed_out: bool,
    pub stopped: bool,
    // 出力が上限を超えた側（超えたら両方を止める）
    pub output_limit: Option<Side>,
    // 始めてから、両方が終わった（または止めた）ことが分かるまでの実時間
    pub elapsed: Duration,
    // やり取り。届いた順で、続けて同じ側が出力したものは 1 つにまとめる
    pub transcript: Vec<(Side, Vec<u8>)>,
    // 回収した後も、読み取りが終わらなかった（出力の全体を得られなかった可能性がある）
    pub output_left_open: bool,
}

// source の出力を、sink（相手の標準入力）へ中継する。届いたら、中継の前にイベントとして送る
fn spawn_relay(
    mut source: ChildStdout,
    mut sink: ChildStdin,
    side: Side,
    events: SyncSender<PairEvent>,
) {
    thread::spawn(move || {
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            match source.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if events
                        .send(PairEvent::Chunk(side, buffer[..n].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                    // 相手が閉じていたら（相手が終わった）、中継をやめる
                    if sink
                        .write_all(&buffer[..n])
                        .and_then(|()| sink.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
        // 相手の標準入力を閉じて、EOF を伝える
        drop(sink);
        let _ = events.send(PairEvent::Closed);
    });
}

fn spawn_stderr_reader(mut source: ChildStderr, side: Side, events: SyncSender<PairEvent>) {
    thread::spawn(move || {
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            match source.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if events
                        .send(PairEvent::Stderr(side, buffer[..n].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
        let _ = events.send(PairEvent::Closed);
    });
}

// 直接の子が終わるのを、回収せずに待ち、終わり方と時刻を知らせる（spawn_exit_watcher と同じく waitid の WNOWAIT）
#[cfg(unix)]
fn spawn_pair_exit_watcher(pid: libc::pid_t, side: Side, events: SyncSender<PairEvent>) {
    thread::spawn(move || {
        let kind = loop {
            // SAFETY: siginfo_t はゼロで初期化してよい C の構造体で、waitid に書き込み先として渡すだけ
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            // SAFETY: 自分が起動した子の pid を渡す。WNOWAIT なので子は回収されない
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOWAIT,
                )
            };
            if result == 0 {
                // SAFETY: waitid が成功したので、si_status は読んでよい
                let status = unsafe { info.si_status() };
                break if info.si_code == libc::CLD_EXITED {
                    ExitKind::Code(status)
                } else {
                    ExitKind::Signal(status)
                };
            }
            if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                break ExitKind::Unknown;
            }
        };
        let _ = events.send(PairEvent::Exited(side, kind, Instant::now()));
    });
}

// Windows：終わるのを待ち、終了コードを知らせる（シグナルはないので ExitKind::Signal にはならない）
#[cfg(windows)]
fn spawn_pair_exit_watcher(pid: Pid, side: Side, events: SyncSender<PairEvent>) {
    thread::spawn(move || {
        let kind = match crate::win::wait_for_exit(pid as u32) {
            Some(code) => ExitKind::Code(code as i32),
            None => ExitKind::Unknown,
        };
        let _ = events.send(PairEvent::Exited(side, kind, Instant::now()));
    });
}

fn outcome_of_status(status: std::process::ExitStatus) -> Outcome {
    match (status.code(), status.signal()) {
        (Some(code), _) => Outcome::Exited(code),
        (None, Some(signal)) => Outcome::Signaled(signal),
        (None, None) => Outcome::Exited(-1),
    }
}

// solution と judge を起動して、互いの標準入出力をつなぎ、両方が終わるまで（または止めるまで）待つ。
// limits.time は対話の全体の時間の上限、limits.output はそれぞれの側の標準出力の上限
// （標準エラーの上限もこの値。超えた分は捨てる）。cancel が true になったら両方を止める
pub fn run_pair(
    mut solution: Command,
    mut judge: Command,
    limits: &Limits,
    cancel: &Arc<AtomicBool>,
) -> Result<PairResult, String> {
    for command in [&mut solution, &mut judge] {
        command
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    }
    let started = Instant::now();
    let mut solution_child: Child = solution
        .spawn()
        .map_err(|error| format!("解答を起動できません：{error}"))?;
    let solution_pid = solution_child.id() as Pid;
    let mut judge_child: Child = match judge.spawn() {
        Ok(child) => child,
        Err(error) => {
            kill_group(solution_pid);
            let _ = reap(solution_pid);
            return Err(format!("ジャッジを起動できません：{error}"));
        }
    };
    let judge_pid = judge_child.id() as Pid;
    let pids = [solution_pid, judge_pid];

    let (sender, events) = mpsc::sync_channel(CHANNEL_CAPACITY);
    {
        let solution_in = solution_child.stdin.take().expect("piped");
        let solution_out = solution_child.stdout.take().expect("piped");
        let solution_err = solution_child.stderr.take().expect("piped");
        let judge_in = judge_child.stdin.take().expect("piped");
        let judge_out = judge_child.stdout.take().expect("piped");
        let judge_err = judge_child.stderr.take().expect("piped");
        spawn_relay(solution_out, judge_in, Side::Solution, sender.clone());
        spawn_relay(judge_out, solution_in, Side::Judge, sender.clone());
        spawn_stderr_reader(solution_err, Side::Solution, sender.clone());
        spawn_stderr_reader(judge_err, Side::Judge, sender.clone());
    }
    spawn_pair_exit_watcher(solution_pid, Side::Solution, sender.clone());
    spawn_pair_exit_watcher(judge_pid, Side::Judge, sender);

    let mut transcript: Vec<(Side, Vec<u8>)> = Vec::new();
    let mut output_total = [0usize; 2];
    let mut stderr = [Vec::new(), Vec::new()];
    let mut exits: [Option<(ExitKind, Instant)>; 2] = [None, None];
    let mut first_failure: Option<Side> = None;
    let mut grace_until: Option<Instant> = None;
    let (mut timed_out, mut stopped) = (false, false);
    let mut output_limit: Option<Side> = None;
    // 止めた時刻（止めていなければ None）
    let mut killed_at: Option<Instant> = None;
    // 読み取りの数：中継 2 つと標準エラー 2 つ
    let mut open = 4;

    let kill_both = || {
        for pid in pids {
            kill_group(pid);
        }
    };
    // 届いたものを記録する。出力の上限を超えたら、その側を返す
    let mut accept = |event: PairEvent,
                      open: &mut i32,
                      exits: &mut [Option<(ExitKind, Instant)>; 2]|
     -> Option<Side> {
        match event {
            PairEvent::Chunk(side, bytes) => {
                output_total[side.index()] += bytes.len();
                let over = output_total[side.index()] > limits.output;
                match transcript.last_mut() {
                    Some((last, data)) if *last == side => data.extend_from_slice(&bytes),
                    _ => transcript.push((side, bytes)),
                }
                over.then_some(side)
            }
            PairEvent::Stderr(side, bytes) => {
                let buffer = &mut stderr[side.index()];
                let room = limits.output.saturating_sub(buffer.len());
                buffer.extend_from_slice(&bytes[..bytes.len().min(room)]);
                None
            }
            PairEvent::Closed => {
                *open -= 1;
                None
            }
            PairEvent::Exited(side, kind, at) => {
                exits[side.index()] = Some((kind, at));
                None
            }
        }
    };

    loop {
        let event = events.recv_timeout(POLL_INTERVAL);
        match event {
            Ok(event) => {
                let exited = matches!(&event, PairEvent::Exited(..));
                let over = accept(event, &mut open, &mut exits);
                if let Some(side) = over
                    && killed_at.is_none()
                {
                    output_limit = Some(side);
                    killed_at = Some(Instant::now());
                    kill_both();
                }
                // こちらが止める前に、異常に終わった最初の側
                if exited && first_failure.is_none() && killed_at.is_none() {
                    for side in [Side::Solution, Side::Judge] {
                        if exits[side.index()].is_some_and(|(kind, _)| kind.is_abnormal()) {
                            first_failure = Some(side);
                            grace_until = Some(Instant::now() + PAIR_GRACE);
                            break;
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if exits.iter().all(Option::is_some) {
            break;
        }
        if killed_at.is_none() {
            let now = Instant::now();
            if limits.time.is_some_and(|limit| now - started > limit) {
                timed_out = true;
            } else if cancel.load(Ordering::SeqCst) {
                stopped = true;
            }
            // 解答のメモリの上限（D-175）。止めた理由は残さず、終わってから最大 RSS で MLE と判定する
            let memory_over = exits[Side::Solution.index()].is_none()
                && limits
                    .memory
                    .is_some_and(|limit| current_rss(solution_pid).is_some_and(|rss| rss > limit));
            if timed_out || stopped || memory_over || grace_until.is_some_and(|until| now >= until)
            {
                killed_at = Some(now);
                kill_both();
            }
        }
    }

    // 両方が終わった。時間の上限は、終わった時刻で判定する
    let finished_at = killed_at.unwrap_or_else(|| {
        exits
            .iter()
            .flatten()
            .map(|(_, at)| *at)
            .max()
            .unwrap_or_else(Instant::now)
    });
    if killed_at.is_none()
        && limits
            .time
            .is_some_and(|limit| finished_at - started > limit)
    {
        timed_out = true;
    }

    // 回収する前に、グループの残り（直接の子が残した子孫）を止める
    kill_both();
    let (solution_status, solution_rss) = reap(solution_pid)?;
    let (judge_status, judge_rss) = reap(judge_pid)?;
    drop((solution_child, judge_child));

    // 残りの出力を読み切る（run と同じ。届かなくなるか、DRAIN_LIMIT が過ぎるか、停止の操作で打ち切る）
    let drain_until = Instant::now() + DRAIN_LIMIT;
    while open > 0 && !cancel.load(Ordering::SeqCst) {
        let Some(left) = drain_until.checked_duration_since(Instant::now()) else {
            break;
        };
        match events.recv_timeout(left.min(READ_GRACE)) {
            Ok(event) => {
                if let Some(side) = accept(event, &mut open, &mut exits) {
                    output_limit.get_or_insert(side);
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let [solution_stderr, judge_stderr] = stderr;
    Ok(PairResult {
        solution: PairMember {
            outcome: outcome_of_status(solution_status),
            stderr: solution_stderr,
            max_rss_bytes: solution_rss,
        },
        judge: PairMember {
            outcome: outcome_of_status(judge_status),
            stderr: judge_stderr,
            max_rss_bytes: judge_rss,
        },
        first_failure,
        timed_out,
        stopped,
        output_limit,
        elapsed: finished_at - started,
        transcript,
        output_left_open: open > 0,
    })
}

// /bin/sh・perl・kill を使うので Unix だけ。Windows の仮の実装のテストは下の windows_tests
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    fn limits(time_ms: u64, output: usize) -> Limits {
        Limits {
            memory: None,
            time: Some(Duration::from_millis(time_ms)),
            output,
        }
    }

    fn run_sh(script: &str, limits: &Limits) -> RunResult {
        run(
            sh(script),
            None,
            limits,
            &Arc::new(AtomicBool::new(false)),
            |_, _| {},
        )
        .unwrap()
    }

    // pid のプロセスがもういないか（kill(pid, 0) が ESRCH）。止めた子孫は launchd に引き取られて
    // 回収されるまでゾンビとして残り、その間は kill(pid, 0) が成功するので、1 秒まで待つ
    fn gone(pid: libc::pid_t) -> bool {
        (0..100).any(|_| {
            // SAFETY: シグナル 0 は送らず、存在だけを確かめる
            let result = unsafe { libc::kill(pid, 0) };
            let gone =
                result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            if !gone {
                thread::sleep(Duration::from_millis(10));
            }
            gone
        })
    }

    #[test]
    fn captures_output_and_exit_code() {
        let result = run_sh("echo out; echo err >&2; exit 3", &limits(5000, 1024));
        assert_eq!(result.outcome, Outcome::Exited(3));
        assert_eq!(result.stdout, b"out\n");
        assert_eq!(result.stderr, b"err\n");
        assert!(!result.output_left_open);
    }

    #[test]
    fn reports_max_memory_of_the_child() {
        // sh が 50 MB の文字列を変数に持つと、sh 自身の最大使用メモリが増える
        let small = run_sh("true", &limits(5000, 1024));
        let large = run_sh(
            "x=$(head -c 50000000 /dev/zero | tr '\\0' a); echo ${#x}",
            &limits(10000, 1024),
        );
        let small = small.max_rss_bytes.unwrap();
        let large = large.max_rss_bytes.unwrap();
        assert!(small > 0);
        assert!(large > 40 * 1024 * 1024, "{large}");
        assert!(large > small);
    }

    #[test]
    fn reports_signals() {
        let result = run_sh("kill -SEGV $$", &limits(5000, 1024));
        assert_eq!(result.outcome, Outcome::Signaled(libc::SIGSEGV));
    }

    #[test]
    fn passes_stdin_from_file() {
        let path =
            std::env::temp_dir().join(format!("atcoder-studio-stdin-{}", std::process::id()));
        std::fs::write(&path, "1 2\n").unwrap();
        let result = run(
            sh("cat"),
            Some(File::open(&path).unwrap()),
            &limits(5000, 1024),
            &Arc::new(AtomicBool::new(false)),
            |_, _| {},
        )
        .unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(result.stdout, b"1 2\n");
    }

    #[test]
    fn stops_at_time_limit_with_descendants() {
        // 子（sleep）の pid を出してから、待ち続ける
        let result = run_sh("sleep 100 & echo $!; wait", &limits(300, 1024));
        assert_eq!(result.outcome, Outcome::TimedOut);
        assert!(result.elapsed < Duration::from_secs(3));
        let child: libc::pid_t = String::from_utf8_lossy(&result.stdout)
            .trim()
            .parse()
            .unwrap();
        assert!(gone(child), "子孫のプロセスが残っている");
    }

    #[test]
    fn stops_descendants_left_after_exit() {
        // 直接の子はすぐ終わるが、sleep が残る
        let result = run_sh("sleep 100 & echo $!", &limits(5000, 1024));
        assert_eq!(result.outcome, Outcome::Exited(0));
        let child: libc::pid_t = String::from_utf8_lossy(&result.stdout)
            .trim()
            .parse()
            .unwrap();
        assert!(gone(child), "子孫のプロセスが残っている");
    }

    #[test]
    fn stops_at_output_limit() {
        let result = run_sh("yes", &limits(5000, 1024 * 1024));
        assert_eq!(result.outcome, Outcome::OutputLimit(Stream::Stdout));
        assert_eq!(result.stdout.len(), 1024 * 1024);
        let result = run_sh("yes >&2", &limits(5000, 1024 * 1024));
        assert_eq!(result.outcome, Outcome::OutputLimit(Stream::Stderr));
    }

    #[test]
    fn reads_both_streams_without_blocking() {
        // 標準エラーに大量に書いてから、標準出力に書く（片方だけを読むと詰まる）
        let result = run_sh(
            "head -c 2000000 /dev/zero >&2; echo done",
            &limits(5000, 4 * 1024 * 1024),
        );
        assert_eq!(result.outcome, Outcome::Exited(0));
        assert_eq!(result.stderr.len(), 2_000_000);
        assert_eq!(result.stdout, b"done\n");
    }

    #[test]
    fn stops_when_cancelled() {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::SeqCst);
        });
        let result = run(
            sh("sleep 100"),
            None,
            &limits(5000, 1024),
            &cancel,
            |_, _| {},
        )
        .unwrap();
        assert_eq!(result.outcome, Outcome::Stopped);
        assert!(result.elapsed < Duration::from_secs(3));
    }

    #[test]
    fn returns_even_if_escaped_descendant_keeps_writing() {
        // setsid でグループから抜けた子孫が、出力を 0.1 秒おきに書き続ける（AC2-13 の状況）。
        // 親は、子孫がグループから抜けた（印のファイルを作った）のを確かめてから終わる。確かめずに終わると、
        // 子孫が抜ける前にグループごと止められることがあり、テストが時々失敗した（Phase 10 作業 17）
        let mark = std::env::temp_dir().join(format!(
            "atcoder-studio-escaped-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_file(&mark);
        let script = format!(
            "perl -MPOSIX -e 'my $f = shift; if (fork() == 0) {{ POSIX::setsid(); open(my $h, \">\", $f); close($h); $| = 1; for (1..200) {{ print \"x\"; select(undef, undef, undef, 0.1) }} exit }} for (1..500) {{ last if -e $f; select(undef, undef, undef, 0.01) }} print \"$$\\n\"' {}",
            mark.display()
        );
        let started = Instant::now();
        let result = run_sh(&script, &limits(5000, 1024));
        let _ = std::fs::remove_file(&mark);
        assert!(started.elapsed() < Duration::from_secs(8));
        assert!(result.output_left_open);
        // 後片付け：抜けた子孫を止める（グループ外なので、pid を探して止める）
        let _ = Command::new("pkill").args(["-f", "POSIX::setsid"]).status();
    }

    #[test]
    fn stops_a_program_over_the_memory_limit_while_running() {
        // 10 MiB ずつ、最大 400 MiB まで使ってから 3 秒眠る。上限 50 MiB なら、使っている途中で止まる（D-175）
        let script = "perl -e '$s = \"x\" x (10 * 1024 * 1024); @a = (); for (1..40) { push @a, $s . \"\" } sleep 3'";
        let mut limits = limits(5000, 1024);
        limits.memory = Some(50 * 1024 * 1024);
        let started = Instant::now();
        let result = run_sh(script, &limits);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        assert!(
            matches!(result.outcome, Outcome::Signaled(_)),
            "{:?}",
            result.outcome
        );
        assert!(
            result
                .max_rss_bytes
                .is_some_and(|bytes| bytes > 50 * 1024 * 1024)
        );
        // 上限より少なければ止めない（1 MiB だけ使い、そのまま終わる）
        let quiet = run_sh(
            "perl -e '$s = \"x\" x (1024 * 1024); print length($s)'",
            &limits,
        );
        assert_eq!(quiet.outcome, Outcome::Exited(0));
    }

    #[test]
    fn keeps_exact_limit_and_stops_one_byte_over() {
        let result = run_sh("head -c 1024 /dev/zero", &limits(5000, 1024));
        assert_eq!(result.outcome, Outcome::Exited(0));
        assert_eq!(result.stdout.len(), 1024);
        let result = run_sh("head -c 1025 /dev/zero", &limits(5000, 1024));
        assert_eq!(result.outcome, Outcome::OutputLimit(Stream::Stdout));
    }

    #[test]
    fn reads_all_output_with_slow_callback() {
        // 1 MiB 以上を書いてすぐ終わる。受け取り側が遅くても、末尾まで読む
        let result = run(
            sh("head -c 3000000 /dev/zero"),
            None,
            &limits(10000, 4 * 1024 * 1024),
            &Arc::new(AtomicBool::new(false)),
            |_, _| thread::sleep(Duration::from_millis(2)),
        )
        .unwrap();
        assert_eq!(result.outcome, Outcome::Exited(0));
        assert_eq!(result.stdout.len(), 3_000_000);
    }

    #[test]
    fn passes_output_to_callback() {
        let mut seen = Vec::new();
        run(
            sh("echo a; echo b >&2"),
            None,
            &limits(5000, 1024),
            &Arc::new(AtomicBool::new(false)),
            |stream, bytes| seen.push((stream, bytes.to_vec())),
        )
        .unwrap();
        assert!(seen.contains(&(Stream::Stdout, b"a\n".to_vec())));
        assert!(seen.contains(&(Stream::Stderr, b"b\n".to_vec())));
    }

    // ---- run_pair ----

    fn pair(solution: &str, judge: &str, limits: &Limits) -> PairResult {
        run_pair(
            sh(solution),
            sh(judge),
            limits,
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
    }

    fn text(result: &PairResult) -> Vec<(Side, String)> {
        result
            .transcript
            .iter()
            .map(|(side, bytes)| (*side, String::from_utf8_lossy(bytes).into_owned()))
            .collect()
    }

    #[test]
    fn relays_between_two_processes_in_order() {
        // 解答が 5 を出し、ジャッジが 2 倍を返し、解答が確かめる
        let result = pair(
            "echo 5; read r; [ \"$r\" = 10 ]",
            "read q; echo $((q * 2))",
            &limits(5000, 1024),
        );
        assert_eq!(result.solution.outcome, Outcome::Exited(0));
        assert_eq!(result.judge.outcome, Outcome::Exited(0));
        assert_eq!(result.first_failure, None);
        assert!(!result.timed_out && !result.stopped && result.output_limit.is_none());
        assert!(!result.output_left_open);
        assert_eq!(
            text(&result),
            vec![
                (Side::Solution, "5\n".to_string()),
                (Side::Judge, "10\n".to_string())
            ]
        );
        assert!(result.solution.max_rss_bytes.is_some());
    }

    #[test]
    fn collects_stderr_of_each_side() {
        let result = pair(
            "echo from-solution >&2",
            "echo from-judge >&2",
            &limits(5000, 1024),
        );
        assert_eq!(result.solution.stderr, b"from-solution\n");
        assert_eq!(result.judge.stderr, b"from-judge\n");
    }

    #[test]
    fn finds_who_failed_first() {
        // 解答が出力せずに 3 で終わる。ジャッジは EOF を読んで 2 で終わる → 解答が先
        let result = pair("exit 3", "read q || exit 2", &limits(5000, 1024));
        assert_eq!(result.first_failure, Some(Side::Solution));
        assert_eq!(result.solution.outcome, Outcome::Exited(3));
        assert_eq!(result.judge.outcome, Outcome::Exited(2));
        // ジャッジが先に 1 で終わる。解答は読み取りが EOF になって 0 で終わる → ジャッジが先
        let result = pair("read x; exit 0", "exit 1", &limits(5000, 1024));
        assert_eq!(result.first_failure, Some(Side::Judge));
        assert_eq!(result.judge.outcome, Outcome::Exited(1));
        // 両方が 0 で終われば、失敗はない。解答が先に終わっても、ジャッジを待つ
        let result = pair(
            "echo done",
            "read x; sleep 0.3; exit 0",
            &limits(5000, 1024),
        );
        assert_eq!(result.first_failure, None);
        assert_eq!(result.judge.outcome, Outcome::Exited(0));
        assert!(result.elapsed >= Duration::from_millis(250));
        // 解答がシグナルで終わる
        let result = pair("kill -SEGV $$", "read x; exit 1", &limits(5000, 1024));
        assert_eq!(result.first_failure, Some(Side::Solution));
        assert_eq!(result.solution.outcome, Outcome::Signaled(libc::SIGSEGV));
    }

    #[test]
    fn stops_the_other_side_after_a_grace_period() {
        // 解答が 2 で終わり、ジャッジは何も読まず眠り続ける。猶予のあとで止める
        let started = Instant::now();
        let result = pair("exit 2", "sleep 30", &limits(10000, 1024));
        assert_eq!(result.first_failure, Some(Side::Solution));
        assert!(!result.timed_out);
        assert_eq!(result.judge.outcome, Outcome::Signaled(libc::SIGKILL));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn times_out_and_stops_both() {
        // 解答もジャッジも待ち続ける（出力のあとの flush 忘れなどで起きる）
        let started = Instant::now();
        let result = pair("sleep 30", "sleep 30", &limits(300, 1024));
        assert!(result.timed_out);
        assert_eq!(result.first_failure, None);
        assert_eq!(result.solution.outcome, Outcome::Signaled(libc::SIGKILL));
        assert_eq!(result.judge.outcome, Outcome::Signaled(libc::SIGKILL));
        assert!(result.elapsed < Duration::from_secs(3));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn stops_both_on_request() {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let result = run_pair(
            sh("sleep 30"),
            sh("sleep 30"),
            &limits(20000, 1024),
            &cancel,
        )
        .unwrap();
        assert!(result.stopped);
        assert!(!result.timed_out);
        assert_eq!(result.first_failure, None);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn limits_output_of_each_side() {
        // 解答が出力し続ける。ジャッジは読み捨てる。上限（4 KiB）を超えたら両方を止める
        let result = pair("yes", "cat > /dev/null", &limits(10000, 4096));
        assert_eq!(result.output_limit, Some(Side::Solution));
        assert!(!result.timed_out);
        assert!(result.elapsed < Duration::from_secs(5));
        // 標準エラーは、上限を超えた分を捨てるだけで止めない
        let result = pair("head -c 100000 /dev/zero >&2", "true", &limits(10000, 4096));
        assert_eq!(result.output_limit, None);
        assert_eq!(result.solution.stderr.len(), 4096);
    }

    #[test]
    fn keeps_a_long_exchange_in_order() {
        // 1000 往復。順序と、往復の数が保たれる
        let result = pair(
            "i=0; while [ $i -lt 1000 ]; do echo $i; read r; i=$((i + 1)); done",
            "while read q; do echo $q; done",
            &limits(20000, 1024 * 1024),
        );
        assert_eq!(result.solution.outcome, Outcome::Exited(0));
        // 解答の出力と、ジャッジの返事が交互に並び、まとめられていない（往復ごとに側が変わる）
        assert_eq!(result.transcript.len(), 2000);
        assert_eq!(result.transcript[0], (Side::Solution, b"0\n".to_vec()));
        assert_eq!(result.transcript[1], (Side::Judge, b"0\n".to_vec()));
        assert_eq!(result.transcript[1998], (Side::Solution, b"999\n".to_vec()));
    }
}

// Windows の仮の実装（段階 1）のテスト。cmd.exe の組み込みのコマンドだけで動かす（子孫を作らないもの）。
// 子孫を止めること・使用メモリは、段階 2 の Job Object で確かめる
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::os::windows::process::CommandExt;

    // cmd /c に、引用の加工をせずにそのまま渡す
    fn cmd(script: &str) -> Command {
        let mut command = Command::new("cmd");
        command.arg("/c").raw_arg(script);
        command
    }

    fn limits(time_ms: u64, output: usize) -> Limits {
        Limits {
            memory: None,
            time: Some(Duration::from_millis(time_ms)),
            output,
        }
    }

    fn run_cmd(script: &str, limits: &Limits) -> RunResult {
        run(
            cmd(script),
            None,
            limits,
            &Arc::new(AtomicBool::new(false)),
            |_, _| {},
        )
        .unwrap()
    }

    #[test]
    fn captures_output_and_exit_code() {
        let result = run_cmd("echo out& 1>&2 echo err& exit /b 3", &limits(5000, 1024));
        assert_eq!(result.outcome, Outcome::Exited(3));
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "out");
        assert_eq!(String::from_utf8_lossy(&result.stderr).trim(), "err");
        assert!(!result.output_left_open);
        // 使用メモリは段階 1 では測らない
        assert_eq!(result.max_rss_bytes, None);
    }

    #[test]
    fn passes_stdin_from_file() {
        let path =
            std::env::temp_dir().join(format!("atcoder-studio-win-stdin-{}", std::process::id()));
        std::fs::write(&path, "1 2\n").unwrap();
        let mut findstr = Command::new("findstr");
        findstr.arg("^");
        let result = run(
            findstr,
            Some(File::open(&path).unwrap()),
            &limits(5000, 1024),
            &Arc::new(AtomicBool::new(false)),
            |_, _| {},
        )
        .unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "1 2");
    }

    #[test]
    fn stops_at_time_limit() {
        let result = run_cmd("for /l %i in () do @rem", &limits(300, 1024));
        assert_eq!(result.outcome, Outcome::TimedOut);
        assert!(result.elapsed < Duration::from_secs(3));
    }

    #[test]
    fn stops_when_cancelled() {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::SeqCst);
        });
        let result = run(
            cmd("for /l %i in () do @rem"),
            None,
            &limits(5000, 1024),
            &cancel,
            |_, _| {},
        )
        .unwrap();
        assert_eq!(result.outcome, Outcome::Stopped);
        assert!(result.elapsed < Duration::from_secs(3));
    }

    #[test]
    fn stops_at_output_limit() {
        let result = run_cmd(
            "for /l %i in () do @echo yyyyyyyyyyyyyyyy",
            &limits(10000, 64 * 1024),
        );
        assert_eq!(result.outcome, Outcome::OutputLimit(Stream::Stdout));
        assert_eq!(result.stdout.len(), 64 * 1024);
    }

    #[test]
    fn reads_both_streams_without_blocking() {
        // 標準エラーに、パイプの容量より多く書いてから、標準出力に書く（片方だけを読むと詰まる）
        let result = run_cmd(
            "(for /l %i in (1,1,3000) do @(1>&2 echo xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx))& echo done",
            &limits(20000, 4 * 1024 * 1024),
        );
        assert_eq!(result.outcome, Outcome::Exited(0));
        assert!(result.stderr.len() > 100_000, "{}", result.stderr.len());
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "done");
    }

    #[test]
    fn relays_between_two_processes() {
        // 解答が 5 を出して終わる。ジャッジ（findstr）は届いた行を返し、EOF で終わる
        let result = run_pair(
            cmd("echo 5"),
            {
                let mut judge = Command::new("findstr");
                judge.arg("^");
                judge
            },
            &limits(5000, 1024),
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(result.solution.outcome, Outcome::Exited(0));
        assert_eq!(result.judge.outcome, Outcome::Exited(0));
        assert!(!result.timed_out && !result.stopped);
        let text: Vec<(Side, String)> = result
            .transcript
            .iter()
            .map(|(side, bytes)| (*side, String::from_utf8_lossy(bytes).trim().to_string()))
            .collect();
        assert_eq!(
            text,
            vec![
                (Side::Solution, "5".to_string()),
                (Side::Judge, "5".to_string())
            ]
        );
    }

    #[test]
    fn times_out_and_stops_both() {
        let result = run_pair(
            cmd("for /l %i in () do @rem"),
            cmd("for /l %i in () do @rem"),
            &limits(300, 1024),
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(result.timed_out);
        assert!(result.elapsed < Duration::from_secs(3));
    }
}
