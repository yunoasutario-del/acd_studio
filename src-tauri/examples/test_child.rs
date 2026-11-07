// テスト用の子のプログラム（Windows 移植の段階 2 S2。docs/windows-port-plan.md 7 章・9.1 の追加 8）
//
// process.rs などのテストが起動する。/bin/sh などの OS のコマンドに頼らずに、同じ動きを Windows と Unix の両方で作るためのもの。
// テストは、このファイルを rustc で直接コンパイルして使う（process.rs の test_child_path）ので、std だけを使う。
// examples に置くのは、cargo clippy --all-targets の対象にするため
// 引数を 1 つずつ順に実行する（`out:a err:b exit:3` なら、a を標準出力に、b を標準エラーに書いて 3 で終わる）。
// 書いたものはすぐに flush する。改行は `\n` だけ（Rust は CRLF にしない）。書けなくなったら（相手が閉じた）121 で終わる
//
// - out:文字列・err:文字列 … 1 行書く
// - bytes:N・errbytes:N … `y` を N バイト書く（改行なし）
// - yes・yeserr … 書けなくなるまで `y\n` を書き続ける
// - cat … 標準入力を EOF まで標準出力に写す
// - discard … 標準入力を EOF まで読み捨てる
// - read … 1 行読む（EOF でも続ける）
// - read-or-exit:N … 1 行読む。EOF なら N で終わる
// - expect:文字列 … 1 行読み、違えば（EOF も）1 で終わる
// - double … 1 行読み、数の 2 倍を書く
// - echo-lines … EOF まで、読んだ行をそのまま書く
// - ask:N … 0 から N-1 まで「書いて 1 行読む」を繰り返す
// - close-stdout … 標準出力を閉じる（相手には EOF が届く。このあとは標準出力に書かない）
// - touch:パス … パスに空のファイルを作る（ほかのプロセスに順番を知らせる印。テストが片付ける）
// - wait-file:パス … パスのファイルができるまで待つ（最大 5 秒。できなければ 120 で終わる）
// - sleep:ミリ秒・spin … 待つ／回り続ける
// - hold:パス … パスのファイルを作って開いたままにする（Windows では、ほかから開けない・消せないように排他で開き、
//   閉じたら消える指定にする）。終わるまで持つので、テストはそのファイルが消せるか・消えたかで、このプロセスが
//   終わったかを知る（pid を使わない。再利用されうるため）
// - grandchild:パス … 自分をもう 1 つ `hold:パス sleep:30000` で起動し（孫）、パスのファイルができるまで（最大 5 秒）待つ。
//   孫は待たずに残す。孫の標準入出力は空。孫は 30 秒で自分で終わる（止める仕組みが働かなくても残り続けない）
// - alloc:MiB … MiB だけ確保して書き込む（使用メモリを増やす）。確保したものは終わるまで持つ
// - crash:access-violation・crash:stack-overflow・crash:abort・crash:panic・crash:divide-by-zero（x86_64 だけ）
// - exit:N
// - stdio-kinds:パス … 標準入力・標準出力・標準エラーが null か（Windows だけ）を、パスのファイルに `null other null` の形で書く
// - record:パス … 最初の手順にする。残りの引数を 1 行に 1 つ、空行、標準入力の全部の順にパスのファイルに書いて 0 で終わる
//   （残りの引数は手順として読まない。curl の代わりに起動して、渡されたものを見るため。段階 3 T6）
//
// 知らない手順・読めない数は、標準エラーに書いて 120 で終わる

use std::io::{BufRead, Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

const MIB: usize = 1024 * 1024;

fn main() {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    // alloc で確保したもの・hold で開いたファイル（終わるまで持つ）
    let mut kept: Vec<Vec<u8>> = Vec::new();
    let mut held: Vec<std::fs::File> = Vec::new();
    for step in std::env::args().skip(1) {
        let (name, value) = step.split_once(':').unwrap_or((step.as_str(), ""));
        match name {
            "out" => write(&mut std::io::stdout(), format!("{value}\n").as_bytes()),
            "err" => write(&mut std::io::stderr(), format!("{value}\n").as_bytes()),
            "bytes" => write_bytes(&mut std::io::stdout(), number(value)),
            "errbytes" => write_bytes(&mut std::io::stderr(), number(value)),
            "yes" => write_forever(&mut std::io::stdout()),
            "yeserr" => write_forever(&mut std::io::stderr()),
            "cat" => {
                let mut buffer = vec![0u8; 64 * 1024];
                loop {
                    match input.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => write(&mut std::io::stdout(), &buffer[..n]),
                    }
                }
            }
            "discard" => {
                let _ = std::io::copy(&mut input, &mut std::io::sink());
            }
            "read" => {
                read_line(&mut input);
            }
            "read-or-exit" => {
                if read_line(&mut input).is_none() {
                    std::process::exit(number(value));
                }
            }
            "expect" => {
                if read_line(&mut input).as_deref() != Some(value) {
                    std::process::exit(1);
                }
            }
            "double" => {
                let line = read_line(&mut input).unwrap_or_default();
                let n: i64 = number(line.trim());
                write(&mut std::io::stdout(), format!("{}\n", n * 2).as_bytes());
            }
            "echo-lines" => {
                while let Some(line) = read_line(&mut input) {
                    write(&mut std::io::stdout(), format!("{line}\n").as_bytes());
                }
            }
            "ask" => {
                for i in 0..number::<u64>(value) {
                    write(&mut std::io::stdout(), format!("{i}\n").as_bytes());
                    read_line(&mut input);
                }
            }
            "close-stdout" => close_stdout(),
            "touch" => {
                std::fs::write(value, b"")
                    .unwrap_or_else(|error| fail(&format!("{value} を作れません：{error}")));
            }
            "wait-file" => {
                if !wait_for_file(std::path::Path::new(value)) {
                    fail(&format!("{value} ができません"));
                }
            }
            "sleep" => std::thread::sleep(Duration::from_millis(number(value))),
            "spin" => loop {
                std::hint::spin_loop();
            },
            "hold" => held.push(hold(value)),
            "grandchild" => {
                let exe = std::env::current_exe().unwrap_or_else(|error| fail(&error.to_string()));
                // 孫は待たずに残す（止められるかをテストで確かめるため）
                #[allow(clippy::zombie_processes)]
                Command::new(exe)
                    .args([format!("hold:{value}"), "sleep:30000".to_string()])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap_or_else(|error| fail(&error.to_string()));
                if !wait_for_file(std::path::Path::new(value)) {
                    fail(&format!("孫が {value} を作りません"));
                }
            }
            "alloc" => {
                let mut block = vec![0u8; number::<usize>(value) * MIB];
                // 確保しただけではページが割り当てられない（WorkingSet・RSS に入らない）ので、ページごとに書き込む。
                // 先頭がページの区切りからずれていると、4096 バイトおきでは最後のページに届かないことがあるので、最後のバイトにも書く
                for i in (0..block.len()).step_by(4096) {
                    block[i] = 1;
                }
                if let Some(last) = block.last_mut() {
                    *last = 1;
                }
                kept.push(std::hint::black_box(block));
            }
            "crash" => crash(value),
            "exit" => std::process::exit(number(value)),
            #[cfg(windows)]
            "stdio-kinds" => stdio_kinds(value),
            "record" => record(value, &mut input),
            other => fail(&format!("知らない手順 {other}")),
        }
    }
}

// 標準入出力が null か（段階 3 T3）。NUL は文字デバイスで、コンソールではないものとして見分ける。std には見る手段がないので API を宣言する
#[cfg(windows)]
fn stdio_kinds(path: &str) {
    use std::ffi::c_void;
    const FILE_TYPE_CHAR: u32 = 2;
    // 順に STD_INPUT_HANDLE・STD_OUTPUT_HANDLE・STD_ERROR_HANDLE
    const STREAMS: [u32; 3] = [-10i32 as u32, -11i32 as u32, -12i32 as u32];
    unsafe extern "system" {
        fn GetStdHandle(id: u32) -> *mut c_void;
        fn GetFileType(handle: *mut c_void) -> u32;
        fn GetConsoleMode(handle: *mut c_void, mode: *mut u32) -> i32;
    }
    let kinds: Vec<&str> = STREAMS
        .iter()
        .map(|&id| {
            let mut mode = 0;
            // SAFETY: 値を渡すだけ。ハンドルは閉じない。mode は書き込める u32
            let is_null = unsafe {
                let handle = GetStdHandle(id);
                GetFileType(handle) == FILE_TYPE_CHAR && GetConsoleMode(handle, &mut mode) == 0
            };
            if is_null { "null" } else { "other" }
        })
        .collect();
    std::fs::write(path, kinds.join(" "))
        .unwrap_or_else(|error| fail(&format!("{path} を作れません：{error}")));
}

// 残りの引数と標準入力をファイルに書いて終わる（段階 3 T6）
fn record(path: &str, input: &mut impl Read) -> ! {
    let mut text = String::new();
    for arg in std::env::args().skip(2) {
        text.push_str(&arg);
        text.push('\n');
    }
    text.push('\n');
    if let Err(error) = input.read_to_string(&mut text) {
        fail(&format!("標準入力を読めません：{error}"));
    }
    std::fs::write(path, text)
        .unwrap_or_else(|error| fail(&format!("{path} を作れません：{error}")));
    std::process::exit(0);
}

fn fail(message: &str) -> ! {
    eprintln!("test_child: {message}");
    std::process::exit(120);
}

fn number<T: std::str::FromStr>(value: &str) -> T {
    value
        .parse()
        .unwrap_or_else(|_| fail(&format!("数ではありません：{value}")))
}

fn write(stream: &mut impl Write, bytes: &[u8]) {
    if stream
        .write_all(bytes)
        .and_then(|()| stream.flush())
        .is_err()
    {
        std::process::exit(121);
    }
}

fn write_bytes(stream: &mut impl Write, mut left: usize) {
    let chunk = vec![b'y'; 64 * 1024];
    while left > 0 {
        let n = left.min(chunk.len());
        write(stream, &chunk[..n]);
        left -= n;
    }
}

fn write_forever(stream: &mut impl Write) -> ! {
    let chunk = b"y\n".repeat(2048);
    loop {
        write(stream, &chunk);
    }
}

// パスのファイルができるまで待つ（10 ms おきに見て、最大 5 秒）。できたら true
fn wait_for_file(path: &std::path::Path) -> bool {
    (0..500).any(|_| {
        let made = path.exists();
        if !made {
            std::thread::sleep(Duration::from_millis(10));
        }
        made
    })
}

// 標準出力を閉じる。std だけで閉じるため、標準出力のハンドル（Unix ではファイル記述子 1）を File として受け取って捨てる
fn close_stdout() {
    let _ = std::io::stdout().flush();
    #[cfg(windows)]
    {
        use std::os::windows::io::{AsRawHandle, FromRawHandle};
        let handle = std::io::stdout().as_raw_handle();
        // SAFETY: 標準出力のハンドルを 1 回だけ閉じる。このあと標準出力には書かない
        drop(unsafe { std::fs::File::from_raw_handle(handle) });
    }
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;
        // SAFETY: ファイル記述子 1（標準出力）を 1 回だけ閉じる。このあと標準出力には書かない
        drop(unsafe { std::fs::File::from_raw_fd(1) });
    }
}

// パスのファイルを作って開く。Windows では共有を許さず（ほかから開けない・消せない）、閉じたら消える指定で開く
// （このプロセスが自然に終わっても、止められても、印のファイルが残らない）
fn hold(path: &str) -> std::fs::File {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // GENERIC_WRITE と DELETE（閉じたら消える指定には DELETE が要る）、FILE_FLAG_DELETE_ON_CLOSE
        // （std だけで作るので、windows-sys を使わずに値を書く）
        const GENERIC_WRITE: u32 = 0x4000_0000;
        const DELETE: u32 = 0x0001_0000;
        const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
        options
            .access_mode(GENERIC_WRITE | DELETE)
            .share_mode(0)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE);
    }
    options
        .open(path)
        .unwrap_or_else(|error| fail(&format!("{path} を開けません：{error}")))
}

// 1 行読み、行末の改行（`\r\n` も）を除いて返す。EOF・読めないときは None
fn read_line(input: &mut impl BufRead) -> Option<String> {
    let mut line = String::new();
    match input.read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim_end_matches(['\r', '\n']).to_string()),
    }
}

fn crash(kind: &str) {
    match kind {
        // null でない、割り当てられていない番地に書く（null だと、debug のビルドでは書く前の検査で abort になる）
        "access-violation" => {
            let address: usize = std::hint::black_box(16);
            // SAFETY: わざと不正な番地に書いて落とす（テスト用）
            unsafe { (address as *mut u64).write_volatile(1) };
        }
        "stack-overflow" => {
            recurse(0);
        }
        "abort" => std::process::abort(),
        "panic" => panic!("test_child: crash:panic"),
        "divide-by-zero" => divide_by_zero(),
        other => fail(&format!("知らない落ち方 {other}")),
    }
}

// 止まらない再帰。フレームごとに大きな配列を持ち、最適化で消されないようにする
#[allow(unconditional_recursion)]
fn recurse(depth: u64) -> u64 {
    let frame = std::hint::black_box([depth; 1024]);
    recurse(depth + 1) + frame[0]
}

// 整数の 0 除算（CPU の例外）。Rust の `/` は 0 除算で panic になるので、div 命令を直接使う
#[cfg(target_arch = "x86_64")]
fn divide_by_zero() {
    let divisor: u64 = std::hint::black_box(0);
    // SAFETY: わざと 0 で割って落とす（テスト用）。触るのはレジスタだけ
    unsafe {
        std::arch::asm!(
            "div {0}",
            in(reg) divisor,
            inout("rax") 1u64 => _,
            inout("rdx") 0u64 => _,
            options(nomem, nostack),
        );
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn divide_by_zero() {
    fail("crash:divide-by-zero は x86_64 だけ");
}
