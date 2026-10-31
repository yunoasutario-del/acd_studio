// テスト用の子のプログラム（Windows 移植の段階 2 S2。docs/windows-port-plan.md 7 章・9.1 の追加 8）
//
// process.rs などのテストが起動する。/bin/sh などの OS のコマンドに頼らずに、同じ動きを Windows と Unix の両方で作るためのもの。
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
// - sleep:ミリ秒・spin … 待つ／回り続ける
// - grandchild … 自分をもう 1 つ（sleep:600000 で）起動し、その pid を 1 行書く。待たない（孫の標準入出力は空）
// - alloc:MiB … MiB だけ確保して書き込む（使用メモリを増やす）。確保したものは終わるまで持つ
// - crash:access-violation・crash:stack-overflow・crash:abort・crash:panic・crash:divide-by-zero（x86_64 だけ）
// - exit:N
//
// 知らない手順・読めない数は、標準エラーに書いて 120 で終わる

use std::io::{BufRead, Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

const MIB: usize = 1024 * 1024;

fn main() {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    // alloc で確保したもの（終わるまで持つ）
    let mut kept: Vec<Vec<u8>> = Vec::new();
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
            "sleep" => std::thread::sleep(Duration::from_millis(number(value))),
            "spin" => loop {
                std::hint::spin_loop();
            },
            "grandchild" => {
                let exe = std::env::current_exe().unwrap_or_else(|error| fail(&error.to_string()));
                // 孫は待たずに残す（止められるかをテストで確かめるため）
                #[allow(clippy::zombie_processes)]
                let child = Command::new(exe)
                    .arg("sleep:600000")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap_or_else(|error| fail(&error.to_string()));
                write(
                    &mut std::io::stdout(),
                    format!("{}\n", child.id()).as_bytes(),
                );
            }
            "alloc" => {
                let mut block = vec![0u8; number::<usize>(value) * MIB];
                // 確保しただけではページが割り当てられないので、ページごとに書き込む
                for i in (0..block.len()).step_by(4096) {
                    block[i] = 1;
                }
                kept.push(std::hint::black_box(block));
            }
            "crash" => crash(value),
            "exit" => std::process::exit(number(value)),
            other => fail(&format!("知らない手順 {other}")),
        }
    }
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
