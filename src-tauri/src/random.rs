// ランダムテスト（Phase 4b 作業 2。D-138・D-143）：生成器・愚直解・解答を繰り返し動かし、解答が愚直解と合わなくなる入力を探す
//
// 手順：生成器（example）と愚直解（example）を探す → 解答と一緒にビルドする（D-089）→
// 各回で、生成器に seed を 1 つ目の引数で渡して入力を作り、愚直解の出力を期待出力として解答を判定する（サンプルテストと同じ判定。D-140・D-141）。
// 解答が AC でなかったら、そこで止めて、入力と期待出力をテストのフォルダに `random-N.in`・`.out` として保存する。
// 生成器・愚直解の失敗は、解答の失敗とは数えず、止める（保存しない）

use std::fs::{self, File};
use std::io::Write;
#[cfg(unix)]
use std::os::fd::OwnedFd;
// Windows では、パイプの読み口はハンドル。Unix と同じ書き方のまま File にする
#[cfg(windows)]
use std::os::windows::io::OwnedHandle as OwnedFd;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::ipc::Channel;

use crate::compare::{self, Mode, TestSettings};
use crate::language::LangSettings;
use crate::process::{self, Limits, OUTPUT_LIMIT, Outcome, RUN_TIME_LIMIT};
use crate::run::{
    CaseResult, Finish, NOT_CHECKED, Program, RandomFailure, RunEvent, Verdict, info, millis,
    prepare, relative, send, shown, verdict_of,
};
use crate::toolchain::{self, Target};
use crate::workspace::{MAX_FILE_SIZE, case_key};

// 生成器と愚直解の example の名前を決める（D-143 の 1）。
// 自分のバイナリ用の `<bin>_gen`・`<bin>_naive` があればそれを使い、なければ、パッケージのバイナリが 1 つのときに限り `gen`・`naive`。
// 見つからないときは、置くべきファイルの名前を理由に返す
// extension は、案内に出すファイルの拡張子（rs・cpp。D-212）
pub(crate) fn pick_programs(
    extension: &str,
    bins: &[String],
    examples: &[String],
    bin: &str,
) -> Result<(String, String), String> {
    let pick = |suffix: &str| -> Option<String> {
        let own = format!("{bin}_{suffix}");
        if examples.contains(&own) {
            return Some(own);
        }
        (bins.len() == 1 && examples.iter().any(|name| name == suffix)).then(|| suffix.to_string())
    };
    let generator = pick("gen");
    let naive = pick("naive");
    match (generator, naive) {
        (Some(generator), Some(naive)) => Ok((generator, naive)),
        (generator, naive) => {
            let mut missing = Vec::new();
            if generator.is_none() {
                missing.push(format!("生成器：examples/{bin}_gen.{extension}"));
            }
            if naive.is_none() {
                missing.push(format!("愚直解：examples/{bin}_naive.{extension}"));
            }
            Err(format!(
                "ランダムテストには、生成器と愚直解（正解の出力を出す遅い・単純な解法）の example が要ります。{} を作ってください（バイナリが 1 つなら、examples/gen.{extension}・examples/naive.{extension} でも使えます）",
                missing.join("、")
            ))
        }
    }
}

// 標準入力に渡す入力。パイプに書く別のスレッドを立てる（一時ファイルを作らない）。
// 読まずに終わるプログラムでは、書き込みが途中で失敗するが、無視してよい
fn pipe_input(bytes: &[u8]) -> Result<File, String> {
    let (reader, mut writer) =
        std::io::pipe().map_err(|error| format!("入力を渡せません：{error}"))?;
    let data = bytes.to_vec();
    std::thread::spawn(move || {
        let _ = writer.write_all(&data);
    });
    Ok(File::from(OwnedFd::from(reader)))
}

fn command(executable: &Program, dir: &Path) -> Command {
    let mut command = executable.command();
    command.current_dir(dir);
    toolchain::clear_ide_env(&mut command);
    command
}

// 生成器・愚直解を 1 回動かした結果
enum Helper {
    Done(Vec<u8>),
    Stopped,
    Failed(String),
}

// 生成器（seed を引数で渡す）か愚直解（入力を標準入力で渡す）を、通常実行と同じ上限（D-096）で 1 回動かす。
// 0 で終わり、出力の全体を得られたときだけ成功。失敗の理由と標準エラーは実行ログに出す
fn run_helper(
    what: &str,
    executable: &Program,
    dir: &Path,
    seed: u64,
    input: Option<&[u8]>,
    cancel: &Arc<AtomicBool>,
    emit: &mut dyn FnMut(RunEvent),
) -> Helper {
    let mut program = command(executable, dir);
    let stdin = match input {
        Some(bytes) => match pipe_input(bytes) {
            Ok(file) => Some(file),
            Err(message) => return Helper::Failed(message),
        },
        None => {
            program.arg(seed.to_string());
            None
        }
    };
    let result = process::run(
        program,
        stdin,
        &Limits {
            time: Some(RUN_TIME_LIMIT),
            output: OUTPUT_LIMIT,
            memory: None,
        },
        cancel,
        |_, _| {},
    );
    match result {
        Ok(result) => helper_of(what, seed, result, cancel, emit),
        Err(message) => Helper::Failed(format!("{what}を起動できません：{message}")),
    }
}

// process::run は、子が動いている間だけ止める印を Stopped にし、子が終わったあとの残りの出力の読み取りは、止める印があれば
// すぐやめる。そのため、止める印と子の終わりが重なると、Stopped ではなく、出力が途中の結果（Exited(0) などで output_left_open）になる。
// 出力が途中なのは止めたためなので、ランダムテストでは止めたことにする（失敗として数えない・保存しない）
fn stopped_while_reading(result: &process::RunResult, cancel: &AtomicBool) -> bool {
    result.output_left_open && cancel.load(Ordering::SeqCst)
}

// 生成器・愚直解の 1 回の結果を、成功・止めた・失敗に分ける
fn helper_of(
    what: &str,
    seed: u64,
    result: process::RunResult,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(RunEvent),
) -> Helper {
    if stopped_while_reading(&result, cancel) {
        return Helper::Stopped;
    }
    let reason = match result.outcome {
        Outcome::Stopped => return Helper::Stopped,
        Outcome::Exited(0) if !result.output_left_open => return Helper::Done(result.stdout),
        Outcome::Exited(0) => "出力の全体を得られませんでした".to_string(),
        Outcome::Exited(code) => format!("終了コード {code}"),
        Outcome::Signaled(signal) => format!("シグナル {signal}"),
        Outcome::TimedOut => format!("{} 秒の上限を超えました", RUN_TIME_LIMIT.as_secs()),
        Outcome::OutputLimit(_) => format!(
            "出力が上限（{} MiB）を超えました",
            OUTPUT_LIMIT / 1024 / 1024
        ),
    };
    let stderr = shown(&result.stderr);
    if !stderr.is_empty() {
        emit(RunEvent::Log {
            kind: crate::run::LogKind::Stderr,
            text: format!("[{what}の標準エラー]\n{stderr}\n"),
        });
    }
    Helper::Failed(format!("{what}が失敗しました（seed {seed}）：{reason}"))
}

// 保存の結果
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Saved {
    pub name: String,
    // 同じ入力がすでにあったので、新しくは書かなかった
    pub existing: bool,
}

// folder（またはその親のうち、いちばん近い今ある場所）が root の中か。リンクで外へ出ていないことを確かめる
fn ensure_inside(root: &Path, folder: &Path) -> Result<(), String> {
    let mut nearest = folder;
    while !nearest.exists() {
        nearest = nearest
            .parent()
            .ok_or_else(|| "テストのフォルダの場所を確かめられません".to_string())?;
    }
    let real = nearest
        .canonicalize()
        .map_err(|error| format!("テストのフォルダの場所を確かめられません：{error}"))?;
    if real.starts_with(root) {
        Ok(())
    } else {
        Err("テストのフォルダが作業領域の外にあるので保存できません".to_string())
    }
}

// 失敗したケースを、テストのフォルダに `random-N.in`・`random-N.out` として保存する（D-143 の 5）。
// 同じ入力の `random-*.in` がすでにあれば書かない。N は今ある最大の番号 + 1。既存のファイルは上書きしない
// （`create_new`。`.out` を先に作り、`.in` で完成させる）。
// 限界：確かめてから書くまでの間に、フォルダが差し替えられる場合（TOCTOU）は防げない
pub(crate) fn save_case(
    root: &Path,
    folder: &Path,
    input: &[u8],
    expected: &[u8],
) -> Result<Saved, String> {
    if input.len() as u64 > MAX_FILE_SIZE || expected.len() as u64 > MAX_FILE_SIZE {
        return Err("入力か期待出力が 1 MB を超えるので保存できません".to_string());
    }
    ensure_inside(root, folder)?;
    fs::create_dir_all(folder).map_err(|error| format!("テストのフォルダを作れません：{error}"))?;
    ensure_inside(root, folder)?;
    let mut max_number = 0;
    let entries =
        fs::read_dir(folder).map_err(|error| format!("テストのフォルダを読めません：{error}"))?;
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let Some((stem, extension)) = file_name.rsplit_once('.') else {
            continue;
        };
        let Some((1, number)) = case_key(stem) else {
            continue;
        };
        if extension == "in" {
            let same = entry
                .metadata()
                .is_ok_and(|meta| meta.is_file() && meta.len() == input.len() as u64)
                && fs::read(entry.path()).is_ok_and(|bytes| bytes == input);
            if same {
                return Ok(Saved {
                    name: stem.to_string(),
                    existing: true,
                });
            }
        }
        if extension == "in" || extension == "out" {
            max_number = max_number.max(number);
        }
    }
    for number in max_number + 1..max_number + 6 {
        let name = format!("random-{number}");
        let create = |extension: &str, bytes: &[u8]| -> std::io::Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(folder.join(format!("{name}.{extension}")))?;
            file.write_all(bytes)
        };
        match create("out", expected) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("{name}.out を保存できません：{error}")),
        }
        if let Err(error) = create("in", input) {
            let _ = fs::remove_file(folder.join(format!("{name}.out")));
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                continue;
            }
            return Err(format!("{name}.in を保存できません：{error}"));
        }
        return Ok(Saved {
            name,
            existing: false,
        });
    }
    Err("保存する名前（random-N）を決められません".to_string())
}

fn verdict_code(verdict: &Verdict) -> &'static str {
    match verdict {
        Verdict::Accepted => "AC",
        Verdict::WrongAnswer => "WA",
        Verdict::Unjudged => "目視確認",
        Verdict::RuntimeError { .. } => "RE",
        Verdict::TimedOut => "TLE",
        Verdict::MemoryLimit => "MLE",
        Verdict::OutputLimit => "OLE",
        Verdict::OutputIncomplete => "出力を得られませんでした",
        Verdict::Interrupted => "中断",
        Verdict::NotRun => "未実行",
    }
}

// ランダムテストの 1 回分の材料
pub(crate) struct Plan<'a> {
    // パッケージのディレクトリ（プログラムの作業ディレクトリ）
    pub dir: &'a Path,
    // 作業領域のルート（canonicalize 済み。保存先の検査に使う）
    pub root: &'a Path,
    // 失敗したケースを保存するテストのフォルダ（まだなくてもよい）
    pub test_dir: &'a Path,
    pub solution: &'a Program,
    pub generator: &'a Program,
    pub naive: &'a Program,
    pub settings: &'a TestSettings,
    pub start_seed: u64,
}

// ランダムテストの本体。events には、進み具合・実行ログ・失敗したケースを順に渡す
pub(crate) fn run_random(
    plan: &Plan,
    cancel: &Arc<AtomicBool>,
    emit: &mut dyn FnMut(RunEvent),
) -> Finish {
    let total = plan.settings.random_count as usize;
    let mut done = 0;
    let mut failure = None;
    let mut interrupted = false;
    let mut max_elapsed_ms: Option<u64> = None;
    for index in 0..total {
        if cancel.load(Ordering::SeqCst) {
            interrupted = true;
            break;
        }
        let seed = plan.start_seed + index as u64;
        emit(RunEvent::RandomProgress {
            index: index + 1,
            total,
            seed,
        });
        let input = match run_helper("生成器", plan.generator, plan.dir, seed, None, cancel, emit)
        {
            Helper::Done(bytes) => bytes,
            Helper::Stopped => {
                interrupted = true;
                break;
            }
            Helper::Failed(message) => return Finish::Failed { message },
        };
        let expected = match run_helper(
            "愚直解",
            plan.naive,
            plan.dir,
            seed,
            Some(&input),
            cancel,
            emit,
        ) {
            Helper::Done(bytes) => bytes,
            Helper::Stopped => {
                interrupted = true;
                break;
            }
            Helper::Failed(message) => return Finish::Failed { message },
        };
        let stdin = match pipe_input(&input) {
            Ok(file) => file,
            Err(message) => return Finish::Failed { message },
        };
        let result = match process::run(
            command(plan.solution, plan.dir),
            Some(stdin),
            &Limits {
                time: Some(std::time::Duration::from_millis(
                    plan.settings.time_limit_ms,
                )),
                output: OUTPUT_LIMIT,
                // 実行中もメモリの制限を見て、超えたら止める（D-175）
                memory: Some(plan.settings.memory_limit_mib * 1024 * 1024),
            },
            cancel,
            |_, _| {},
        ) {
            Ok(result) => result,
            Err(message) => return Finish::Failed { message },
        };
        let verdict = verdict_of(&result, Some(&expected), plan.settings);
        if matches!(verdict, Verdict::Interrupted) || stopped_while_reading(&result, cancel) {
            interrupted = true;
            break;
        }
        done += 1;
        let elapsed_ms = millis(result.elapsed);
        max_elapsed_ms = max_elapsed_ms.max(Some(elapsed_ms));
        emit(RunEvent::Log {
            kind: crate::run::LogKind::Info,
            text: format!(
                "#{} seed {seed}：{}（{elapsed_ms} ms）\n",
                index + 1,
                verdict_code(&verdict)
            ),
        });
        if matches!(verdict, Verdict::Accepted) {
            continue;
        }
        failure = Some(report_failure(
            plan, seed, &input, &expected, &result, verdict, emit,
        ));
        break;
    }
    Finish::RandomTested {
        total,
        done,
        start_seed: plan.start_seed,
        failure,
        interrupted,
        max_elapsed_ms,
    }
}

// 解答が AC でなかった回：保存して、サンプルテストのケースと同じ形で結果を送る
fn report_failure(
    plan: &Plan,
    seed: u64,
    input: &[u8],
    expected: &[u8],
    result: &process::RunResult,
    verdict: Verdict,
    emit: &mut dyn FnMut(RunEvent),
) -> RandomFailure {
    let code = verdict_code(&verdict);
    let saved = save_case(plan.root, plan.test_dir, input, expected);
    let (name, already_saved, blocked) = match &saved {
        Ok(saved) => (
            saved.name.clone(),
            saved.existing,
            "愚直解の出力を期待出力として保存しています".to_string(),
        ),
        Err(message) => (
            format!("seed-{seed}"),
            false,
            format!("保存できなかったケースです（{message}）"),
        ),
    };
    match &saved {
        Ok(saved) if saved.existing => emit(RunEvent::Log {
            kind: crate::run::LogKind::Info,
            text: format!(
                "\n同じ入力が {}.in に保存されているので、新しくは保存しませんでした\n",
                saved.name
            ),
        }),
        Ok(saved) => emit(RunEvent::Log {
            kind: crate::run::LogKind::Info,
            text: format!(
                "\nseed {seed} の入力と愚直解の出力を {}/{}.in・.out に保存しました\n",
                relative(plan.dir, plan.test_dir),
                saved.name
            ),
        }),
        Err(message) => emit(RunEvent::Log {
            kind: crate::run::LogKind::Info,
            text: format!("\n失敗したケースを保存できませんでした：{message}\n"),
        }),
    }
    emit(RunEvent::Case {
        result: CaseResult {
            out_file: relative(plan.dir, &plan.test_dir.join(format!("{name}.out"))),
            name,
            verdict,
            elapsed_ms: Some(millis(result.elapsed)),
            input: shown(input),
            expected: Some(shown(expected)),
            actual: shown(&result.stdout),
            stderr: shown(&result.stderr),
            save_blocked: Some(blocked),
            memory_kib: result.max_rss_bytes.map(|bytes| bytes.div_ceil(1024)),
            interaction: None,
        },
    });
    RandomFailure {
        seed,
        verdict: code.to_string(),
        case: saved.ok().map(|saved| saved.name),
        already_saved,
    }
}

// ランダムテストを、ビルドから始めて行う。dir はパッケージのディレクトリ、root は作業領域のルート
pub(crate) fn random_target(
    dir: &Path,
    root: &Path,
    target: &Target,
    langs: &LangSettings,
    start_seed: u64,
    cancel: &Arc<AtomicBool>,
    channel: &Channel<RunEvent>,
) -> Finish {
    // 生成器・愚直解の名前、保存先、設定は、ビルドの前に確かめる（誤りならビルドしない）
    let extension = target.language.extension();
    let (prepared, (generator, naive, test_dir, settings)) = match prepare(
        dir,
        target,
        true,
        langs,
        cancel,
        channel,
        |bins, examples| {
            let tests = toolchain::test_dir(dir, bins, &target.bin);
            if let Some(notice) = &tests.notice {
                info(channel, format!("{notice}\n"));
            }
            let (generator, naive) = pick_programs(extension, bins, examples, &target.bin)
                .map_err(|message| Finish::Failed { message })?;
            let settings = tests
                .dir
                .as_deref()
                .map(compare::read_settings)
                .unwrap_or_else(|| Ok(TestSettings::default()))
                .map_err(|message| Finish::Failed { message })?;
            if matches!(settings.mode, Mode::None) {
                return Err(Finish::Failed {
                    message: "compare.json の mode が none のときは、ランダムテストできません（愚直解の出力と比べられません）".to_string(),
                });
            }
            let test_dir = tests.dir.unwrap_or_else(|| {
                if bins.len() == 1 {
                    dir.join("test")
                } else {
                    dir.join("test").join(&target.bin)
                }
            });
            let examples = vec![generator.clone(), naive.clone()];
            Ok(((generator, naive, test_dir, settings), examples))
        },
    ) {
        Ok(prepared) => prepared,
        Err(finish) => return finish,
    };
    let executable = prepared.executable.clone();
    let find = |name: &str, what: &str| prepared.example(name, what);
    let generator_exe = match find(&generator, "生成器") {
        Ok(path) => path,
        Err(finish) => return finish,
    };
    let naive_exe = match find(&naive, "愚直解") {
        Ok(path) => path,
        Err(finish) => return finish,
    };
    info(
        channel,
        format!(
            "\nランダムテスト：生成器 examples/{generator}.{extension}、愚直解 examples/{naive}.{extension}、最大 {} 回、開始の seed {}（生成器には seed を 1 つ目の引数で渡し、回ごとに 1 ずつ増やします）\n比較の方式：{}、時間の制限 {} ms、メモリの制限 {} MiB。失敗したケースの保存先：{}\n\n",
            settings.random_count,
            start_seed,
            mode_label(&settings.mode),
            settings.time_limit_ms,
            settings.memory_limit_mib,
            relative(dir, &test_dir)
        ),
    );
    let plan = Plan {
        dir,
        root,
        test_dir: &test_dir,
        solution: &executable,
        generator: &generator_exe,
        naive: &naive_exe,
        settings: &settings,
        start_seed,
    };
    let finish = run_random(&plan, cancel, &mut |event| send(channel, event));
    if let Finish::RandomTested {
        total,
        done,
        failure,
        interrupted,
        ..
    } = &finish
    {
        let text = if *interrupted {
            format!("\nランダムテストを止めました（{done} 回を判定済み）。{NOT_CHECKED}\n")
        } else if let Some(failure) = failure {
            format!(
                "\n{done} 回目（seed {}）で、解答が愚直解と合いませんでした。\n",
                failure.seed
            )
        } else {
            format!("\n{total} 回すべて、解答が愚直解と一致しました。\n")
        };
        info(channel, text);
    }
    finish
}

fn mode_label(mode: &Mode) -> &'static str {
    match mode {
        Mode::Whitespace => "空白緩和",
        Mode::Exact => "完全一致",
        Mode::Float { .. } => "浮動小数点",
        Mode::None => "比較しない",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn picks_generator_and_naive_by_name() {
        let one = names(&["solution"]);
        let two = names(&["a", "b"]);
        // バイナリが 1 つなら gen・naive でもよい
        assert_eq!(
            pick_programs("rs", &one, &names(&["gen", "naive"]), "solution"),
            Ok(("gen".to_string(), "naive".to_string()))
        );
        // 自分用の名前が先
        assert_eq!(
            pick_programs(
                "rs",
                &one,
                &names(&["gen", "naive", "solution_gen", "solution_naive"]),
                "solution"
            ),
            Ok(("solution_gen".to_string(), "solution_naive".to_string()))
        );
        // 片方ずつ決める
        assert_eq!(
            pick_programs("rs", &one, &names(&["solution_gen", "naive"]), "solution"),
            Ok(("solution_gen".to_string(), "naive".to_string()))
        );
        // バイナリが複数なら、gen・naive は使わない
        assert_eq!(
            pick_programs(
                "rs",
                &two,
                &names(&["a_gen", "a_naive", "gen", "naive"]),
                "a"
            ),
            Ok(("a_gen".to_string(), "a_naive".to_string()))
        );
        let error = pick_programs("rs", &two, &names(&["gen", "naive"]), "a").unwrap_err();
        assert!(error.contains("examples/a_gen.rs"), "{error}");
        assert!(error.contains("examples/a_naive.rs"), "{error}");
        // 足りないほうだけを知らせる
        let error = pick_programs("rs", &one, &names(&["gen"]), "solution").unwrap_err();
        assert!(error.contains("examples/solution_naive.rs"), "{error}");
        assert!(!error.contains("solution_gen.rs"), "{error}");
        // 似た名前は使わない
        assert!(
            pick_programs(
                "rs",
                &one,
                &names(&["gen2", "naive_x", "solution_gen_2"]),
                "solution"
            )
            .is_err()
        );
    }

    struct Dir(std::path::PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // 作業領域のルート（canonicalize 済み）を作る。名前はテストごとに変える
    fn workspace(name: &str) -> Dir {
        let path = std::env::temp_dir().join(format!(
            "atcoder-studio-random-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Dir(path.canonicalize().unwrap())
    }

    #[test]
    fn saves_failed_cases_without_overwriting() {
        let ws = workspace("save");
        let folder = ws.0.join("pkg/test");
        // フォルダがなくても作って、random-1 から
        assert_eq!(
            save_case(&ws.0, &folder, b"5\n", b"10\n"),
            Ok(Saved {
                name: "random-1".to_string(),
                existing: false
            })
        );
        assert_eq!(fs::read(folder.join("random-1.in")).unwrap(), b"5\n");
        assert_eq!(fs::read(folder.join("random-1.out")).unwrap(), b"10\n");
        // 同じ入力は書かず、今の名前を返す
        assert_eq!(
            save_case(&ws.0, &folder, b"5\n", b"ignored\n"),
            Ok(Saved {
                name: "random-1".to_string(),
                existing: true
            })
        );
        assert_eq!(fs::read(folder.join("random-1.out")).unwrap(), b"10\n");
        // 違う入力は、最大の番号 + 1。番号が飛んでいても、sample-N や孤立した .out があっても上書きしない
        fs::write(folder.join("random-7.in"), "7\n").unwrap();
        fs::write(folder.join("random-9.out"), "orphan\n").unwrap();
        fs::write(folder.join("sample-20.in"), "x").unwrap();
        fs::write(folder.join("random-x.in"), "x").unwrap();
        assert_eq!(
            save_case(&ws.0, &folder, b"6\n", b"12\n"),
            Ok(Saved {
                name: "random-10".to_string(),
                existing: false
            })
        );
        assert_eq!(fs::read(folder.join("random-9.out")).unwrap(), b"orphan\n");
        // 1 MB を超える入力・期待出力は保存しない
        let big = vec![b'a'; MAX_FILE_SIZE as usize + 1];
        assert!(save_case(&ws.0, &folder, &big, b"1").is_err());
        assert!(save_case(&ws.0, &folder, b"1", &big).is_err());
        assert!(!folder.join("random-11.in").exists());
    }

    #[cfg(unix)]
    #[test]
    fn does_not_save_outside_the_workspace() {
        let ws = workspace("outside");
        let outside = workspace("outside-target");
        fs::create_dir_all(ws.0.join("pkg")).unwrap();
        // test がリンクで作業領域の外を指している
        std::os::unix::fs::symlink(&outside.0, ws.0.join("pkg/test")).unwrap();
        let error = save_case(&ws.0, &ws.0.join("pkg/test"), b"1", b"2").unwrap_err();
        assert!(error.contains("作業領域の外"), "{error}");
        // まだないフォルダでも、親がリンクで外を指していれば保存しない
        let error = save_case(&ws.0, &ws.0.join("pkg/test/inner"), b"1", b"2").unwrap_err();
        assert!(error.contains("作業領域の外"), "{error}");
        assert_eq!(fs::read_dir(&outside.0).unwrap().count(), 0);
        // 作業領域の外のフォルダを直接渡しても保存しない
        assert!(save_case(&ws.0, &outside.0, b"1", b"2").is_err());
        assert_eq!(fs::read_dir(&outside.0).unwrap().count(), 0);
    }

    // 止める印と子の終わりが重なり、出力が途中の結果になったときは、生成器・愚直解・解答の失敗ではなく止めたことにする。
    // 実際のプログラムでは起きるかどうかが混み具合による（CI の macos-latest で finds_differences_with_real_programs の 7 が
    // ときどき「生成器が失敗しました（seed 107）：出力の全体を得られませんでした」になった）ので、結果を作って確かめる
    #[test]
    fn treats_output_cut_by_stop_as_stopped() {
        let result = |output_left_open: bool| process::RunResult {
            outcome: Outcome::Exited(0),
            stdout: b"107\n".to_vec(),
            stderr: Vec::new(),
            elapsed: std::time::Duration::from_millis(1),
            output_left_open,
            max_rss_bytes: None,
        };
        let stopped = AtomicBool::new(true);
        let running = AtomicBool::new(false);
        let mut emit = |_: RunEvent| {};
        assert!(matches!(
            helper_of("生成器", 107, result(true), &stopped, &mut emit),
            Helper::Stopped
        ));
        // 止めていなければ、今までどおり失敗
        assert!(matches!(
            helper_of("生成器", 107, result(true), &running, &mut emit),
            Helper::Failed(message) if message.contains("生成器が失敗しました（seed 107）：出力の全体を得られませんでした")
        ));
        // 出力を読み切っていれば、止める印が立っていても成功（止めるのは次の回の前）
        assert!(matches!(
            helper_of("生成器", 107, result(false), &stopped, &mut emit),
            Helper::Done(bytes) if bytes == b"107\n"
        ));
        // 解答の結果も同じ判定で、止めたことにする
        assert!(stopped_while_reading(&result(true), &stopped));
        assert!(!stopped_while_reading(&result(true), &running));
        assert!(!stopped_while_reading(&result(false), &stopped));
    }

    // ---- 実際に cargo でビルドして動かす確認 ----

    use std::sync::Mutex;
    use tauri::ipc::InvokeResponseBody;

    // 送られたイベントを JSON のまま集めるチャンネル。on_event には、各イベントのあとに呼ぶ処理を渡せる
    fn collecting_channel(
        on_event: impl Fn(&serde_json::Value) + Send + Sync + 'static,
    ) -> (Channel<RunEvent>, Arc<Mutex<Vec<serde_json::Value>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Json(text) = body {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                on_event(&value);
                sink.lock().unwrap().push(value);
            }
            Ok(())
        });
        (channel, events)
    }

    fn logs(events: &Arc<Mutex<Vec<serde_json::Value>>>) -> String {
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event["type"] == "log")
            .map(|event| event["text"].as_str().unwrap().to_string())
            .collect()
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    // 解答は seed を n として、n % 7 == 3 なら間違った値、n % 11 == 5 なら 2 秒待ち（TLE）、n % 13 == 4 なら panic（RE）。
    // 愚直解は n == 2001 のとき panic。生成器は seed % 1000 == 999 のとき panic
    const SOLUTION: &str = r#"
use std::io::Read;
fn main() {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).unwrap();
    let n: i64 = s.trim().parse().unwrap();
    if n % 11 == 5 { std::thread::sleep(std::time::Duration::from_secs(2)); }
    if n % 13 == 4 { panic!("solution panic"); }
    println!("{}", if n % 7 == 3 { n * 2 + 1 } else { n * 2 });
}
"#;
    const NAIVE: &str = r#"
use std::io::Read;
fn main() {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).unwrap();
    let n: i64 = s.trim().parse().unwrap();
    if n == 2001 { panic!("naive panic"); }
    println!("{}", n * 2);
}
"#;
    const GENERATOR: &str = r#"
fn main() {
    let seed: u64 = std::env::args().nth(1).unwrap().parse().unwrap();
    if seed % 1000 == 999 { panic!("generator panic"); }
    println!("{seed}");
}
"#;

    struct Outcome2 {
        finish: Finish,
        events: Arc<Mutex<Vec<serde_json::Value>>>,
    }

    fn run_scenario(
        pkg: &Path,
        root: &Path,
        start_seed: u64,
        cancel: &Arc<AtomicBool>,
        channel: &Channel<RunEvent>,
        events: Arc<Mutex<Vec<serde_json::Value>>>,
    ) -> Outcome2 {
        let target = Target {
            package: "pkg".to_string(),
            bin: "solution".to_string(),
            language: crate::language::Language::Rust,
        };
        let finish = random_target(
            pkg,
            root,
            &target,
            &LangSettings::default(),
            start_seed,
            cancel,
            channel,
        );
        Outcome2 { finish, events }
    }

    // ビルドから保存・停止まで、実際のプログラムで確かめる。cargo が要る（cargo build はネットワークを使わない）
    #[test]
    #[cfg_attr(windows, ignore = "Windows：段階 2（rustc の診断のパスの区切りが \\ になる）で直す。docs/windows-port-plan.md 7 章")]
    fn finds_differences_with_real_programs() {
        let ws = workspace("e2e");
        let pkg = ws.0.join("pkg");
        write(
            &pkg.join("Cargo.toml"),
            "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"solution\"\npath = \"src/solution.rs\"\n",
        );
        write(&pkg.join("src/solution.rs"), SOLUTION);
        write(&pkg.join("examples/gen.rs"), GENERATOR);
        write(&pkg.join("examples/naive.rs"), NAIVE);
        let test_dir = pkg.join("test");
        let fresh = || {
            let _ = fs::remove_dir_all(&test_dir);
        };
        let cancel = Arc::new(AtomicBool::new(false));

        // 1. 違いを見つける：seed 101（101 % 7 == 3）で WA。2 回目で止まり、random-1 に保存する
        fresh();
        let (channel, events) = collecting_channel(|_| {});
        let outcome = run_scenario(&pkg, &ws.0, 100, &cancel, &channel, events);
        let Finish::RandomTested {
            total,
            done,
            start_seed,
            failure: Some(failure),
            interrupted: false,
            ..
        } = &outcome.finish
        else {
            panic!("{:?}\n{}", outcome.finish, logs(&outcome.events));
        };
        assert_eq!((*total, *done, *start_seed), (100, 2, 100));
        assert_eq!(failure.seed, 101);
        assert_eq!(failure.verdict, "WA");
        assert_eq!(failure.case.as_deref(), Some("random-1"));
        assert!(!failure.already_saved);
        assert_eq!(fs::read(test_dir.join("random-1.in")).unwrap(), b"101\n");
        assert_eq!(fs::read(test_dir.join("random-1.out")).unwrap(), b"202\n");
        {
            let events = outcome.events.lock().unwrap();
            let progress: Vec<(u64, u64)> = events
                .iter()
                .filter(|event| event["type"] == "randomProgress")
                .map(|event| {
                    (
                        event["index"].as_u64().unwrap(),
                        event["seed"].as_u64().unwrap(),
                    )
                })
                .collect();
            assert_eq!(progress, vec![(1, 100), (2, 101)]);
            let case = events.iter().find(|event| event["type"] == "case").unwrap();
            assert_eq!(case["result"]["name"], "random-1");
            assert_eq!(case["result"]["verdict"]["kind"], "wrongAnswer");
            assert_eq!(case["result"]["input"], "101\n");
            assert_eq!(case["result"]["expected"], "202\n");
            assert_eq!(case["result"]["actual"], "203\n");
            assert_eq!(case["result"]["outFile"], "test/random-1.out");
            assert!(case["result"]["saveBlocked"].is_string());
        }
        let text = logs(&outcome.events);
        assert!(text.contains("seed 100：AC"), "{text}");
        assert!(text.contains("seed 101：WA"), "{text}");
        assert!(text.contains("開始の seed 100"), "{text}");

        // 2. 同じ失敗をもう一度見つけても、二重には保存しない
        let (channel, events) = collecting_channel(|_| {});
        let outcome = run_scenario(&pkg, &ws.0, 100, &cancel, &channel, events);
        let Finish::RandomTested {
            failure: Some(failure),
            ..
        } = &outcome.finish
        else {
            panic!("{:?}", outcome.finish);
        };
        assert_eq!(failure.case.as_deref(), Some("random-1"));
        assert!(failure.already_saved);
        assert!(!test_dir.join("random-2.in").exists());

        // 3. すべて一致：compare.json で回数を 3 に。何も保存しない（seed 105〜107 は、7・11・13 で割った余りが失敗の条件に当たらない）
        fresh();
        write(&test_dir.join("compare.json"), r#"{"randomCount": 3}"#);
        let (channel, events) = collecting_channel(|_| {});
        let outcome = run_scenario(&pkg, &ws.0, 105, &cancel, &channel, events);
        let Finish::RandomTested {
            total,
            done,
            failure: None,
            interrupted: false,
            max_elapsed_ms: Some(_),
            ..
        } = &outcome.finish
        else {
            panic!("{:?}\n{}", outcome.finish, logs(&outcome.events));
        };
        assert_eq!((*total, *done), (3, 3));
        assert!(!test_dir.join("random-1.in").exists());
        assert!(logs(&outcome.events).contains("3 回すべて"));

        // 4. TLE（seed 5 は 11 で割って 5。時間の制限 1000 ms）と RE（seed 4 は 13 で割って 4）
        fresh();
        write(
            &test_dir.join("compare.json"),
            r#"{"timeLimitMs": 1000, "randomCount": 5}"#,
        );
        let (channel, events) = collecting_channel(|_| {});
        let outcome = run_scenario(&pkg, &ws.0, 5, &cancel, &channel, events);
        let Finish::RandomTested {
            done: 1,
            failure: Some(failure),
            ..
        } = &outcome.finish
        else {
            panic!("{:?}\n{}", outcome.finish, logs(&outcome.events));
        };
        assert_eq!(failure.case.as_deref(), Some("random-1"));
        assert_eq!(failure.verdict, "TLE");
        assert_eq!(fs::read(test_dir.join("random-1.out")).unwrap(), b"10\n");
        {
            // 2 秒待つ解答を、制限（1000 ms）で止めている
            let events = outcome.events.lock().unwrap();
            let case = events.iter().find(|event| event["type"] == "case").unwrap();
            assert_eq!(case["result"]["verdict"]["kind"], "timedOut");
            assert!(case["result"]["elapsedMs"].as_u64().unwrap() < 1800);
        }
        let (channel, events) = collecting_channel(|_| {});
        let outcome = run_scenario(&pkg, &ws.0, 4, &cancel, &channel, events);
        assert!(matches!(
            &outcome.finish,
            Finish::RandomTested { failure: Some(failure), .. } if failure.verdict == "RE"
        ));
        let events = outcome.events.lock().unwrap();
        let case = events.iter().find(|event| event["type"] == "case").unwrap();
        assert_eq!(case["result"]["verdict"]["kind"], "runtimeError");
        assert_eq!(case["result"]["verdict"]["outcome"]["code"], 101);
        assert!(
            case["result"]["stderr"]
                .as_str()
                .unwrap()
                .contains("solution panic")
        );
        drop(events);
        assert!(test_dir.join("random-2.in").exists());

        // 5. 生成器が失敗：seed 999 で panic。解答の失敗とは数えず、保存しない
        fresh();
        let (channel, events) = collecting_channel(|_| {});
        let outcome = run_scenario(&pkg, &ws.0, 998, &cancel, &channel, events);
        let Finish::Failed { message } = &outcome.finish else {
            panic!("{:?}", outcome.finish);
        };
        assert!(
            message.contains("生成器が失敗しました（seed 999）"),
            "{message}"
        );
        assert!(logs(&outcome.events).contains("generator panic"));
        assert!(!test_dir.exists());

        // 6. 愚直解が失敗：seed 2001 で panic
        let (channel, events) = collecting_channel(|_| {});
        let outcome = run_scenario(&pkg, &ws.0, 2000, &cancel, &channel, events);
        let Finish::Failed { message } = &outcome.finish else {
            panic!("{:?}", outcome.finish);
        };
        assert!(
            message.contains("愚直解が失敗しました（seed 2001）"),
            "{message}"
        );
        assert!(!test_dir.exists());

        // 7. 停止：3 回目を始めたところで止める。止めた回は保存せず、数えない
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let (channel, events) = collecting_channel(move |event| {
            if event["type"] == "randomProgress" && event["index"] == 3 {
                flag.store(true, Ordering::SeqCst);
            }
        });
        let outcome = run_scenario(&pkg, &ws.0, 105, &stop, &channel, events);
        let Finish::RandomTested {
            done,
            failure: None,
            interrupted: true,
            ..
        } = &outcome.finish
        else {
            panic!("{:?}\n{}", outcome.finish, logs(&outcome.events));
        };
        assert!((2..=3).contains(done), "{done}");
        assert!(!test_dir.join("random-1.in").exists());
        assert!(logs(&outcome.events).contains("止めました"));

        // 8. 始める前の誤り：mode none、randomCount の範囲外、example がない（ビルドしない）
        write(&test_dir.join("compare.json"), r#"{"mode": "none"}"#);
        let (channel, events) = collecting_channel(|_| {});
        let outcome = run_scenario(&pkg, &ws.0, 100, &cancel, &channel, events);
        assert!(matches!(&outcome.finish, Finish::Failed { message } if message.contains("none")));
        write(&test_dir.join("compare.json"), r#"{"randomCount": 0}"#);
        let (channel, events) = collecting_channel(|_| {});
        let outcome = run_scenario(&pkg, &ws.0, 100, &cancel, &channel, events);
        assert!(
            matches!(&outcome.finish, Finish::Failed { message } if message.contains("randomCount"))
        );
        fs::remove_file(pkg.join("examples/naive.rs")).unwrap();
        fresh();
        let (channel, events) = collecting_channel(|_| {});
        let outcome = run_scenario(&pkg, &ws.0, 100, &cancel, &channel, events);
        assert!(
            matches!(&outcome.finish, Finish::Failed { message } if message.contains("examples/solution_naive.rs"))
        );

        // 9. 生成器にコンパイルエラー：CE。エラー一覧に場所が出る
        write(&pkg.join("examples/naive.rs"), NAIVE);
        write(
            &pkg.join("examples/gen.rs"),
            "fn main() { let x: i32 = \"a\"; }\n",
        );
        let (channel, events) = collecting_channel(|_| {});
        let outcome = run_scenario(&pkg, &ws.0, 100, &cancel, &channel, events);
        assert!(
            matches!(outcome.finish, Finish::CompileError),
            "{:?}",
            outcome.finish
        );
        let events = outcome.events.lock().unwrap();
        let diagnostics = events
            .iter()
            .find(|event| event["type"] == "diagnostics")
            .unwrap();
        assert_eq!(diagnostics["items"][0]["file"], "examples/gen.rs");
    }

    // C++ の生成器・愚直解・解答を g++-15 でビルドして試す（Phase 17 作業 7。D-212）。g++-15 が要る
    #[test]
    fn finds_differences_with_cpp_programs() {
        let cpp = LangSettings::default();
        if crate::cpp::find_compiler(&cpp.cpp).is_err() {
            eprintln!("g++-15 がないので飛ばします");
            return;
        }
        let ws = workspace("cpp");
        let pkg = ws.0.join("pkg");
        write(
            &pkg.join("main.cpp"),
            "#include <bits/stdc++.h>\nint main(){long long n;std::cin>>n;std::cout<<(n%7==3?n*2+1:n*2)<<std::endl;}\n",
        );
        write(
            &pkg.join("examples/naive.cpp"),
            "#include <bits/stdc++.h>\nint main(){long long n;std::cin>>n;std::cout<<n*2<<std::endl;}\n",
        );
        write(
            &pkg.join("examples/gen.cpp"),
            "#include <bits/stdc++.h>\nint main(int argc,char**argv){std::cout<<std::stoll(argv[1])<<std::endl;}\n",
        );
        let target = Target {
            package: "pkg".to_string(),
            bin: crate::cpp::BIN.to_string(),
            language: crate::language::Language::Cpp,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let run = || {
            let (channel, events) = collecting_channel(|_| {});
            let finish = random_target(&pkg, &ws.0, &target, &cpp, 100, &cancel, &channel);
            (finish, events)
        };

        // seed 101 で WA になり、random-1 に保存する。example は target/examples/ にビルドする
        let (finish, events) = run();
        let Finish::RandomTested {
            done,
            failure: Some(failure),
            ..
        } = &finish
        else {
            panic!("{finish:?}\n{}", logs(&events));
        };
        assert_eq!(*done, 2);
        assert_eq!((failure.seed, failure.verdict.as_str()), (101, "WA"));
        assert_eq!(failure.case.as_deref(), Some("random-1"));
        assert_eq!(fs::read(pkg.join("test/random-1.in")).unwrap(), b"101\n");
        assert_eq!(fs::read(pkg.join("test/random-1.out")).unwrap(), b"202\n");
        assert!(pkg.join("target/examples/gen").is_file());
        assert!(pkg.join("target/examples/naive").is_file());
        let text = logs(&events);
        assert!(
            text.contains("生成器 examples/gen.cpp、愚直解 examples/naive.cpp"),
            "{text}"
        );

        // 愚直解がない：C++ の名前で案内し、ビルドしない
        fs::remove_file(pkg.join("examples/naive.cpp")).unwrap();
        let (finish, _) = run();
        assert!(
            matches!(&finish, Finish::Failed { message } if message.contains("examples/main_naive.cpp") && message.contains("examples/naive.cpp")),
            "{finish:?}"
        );

        // 生成器にコンパイルエラー：CE。エラー一覧に、生成器のファイルの場所が出る
        write(
            &pkg.join("examples/naive.cpp"),
            "#include <bits/stdc++.h>\nint main(){long long n;std::cin>>n;std::cout<<n*2<<std::endl;}\n",
        );
        write(
            &pkg.join("examples/gen.cpp"),
            "int main() { int x = \"a\"; }\n",
        );
        let (finish, events) = run();
        assert!(matches!(finish, Finish::CompileError), "{finish:?}");
        let events = events.lock().unwrap();
        let diagnostics = events
            .iter()
            .rfind(|event| event["type"] == "diagnostics")
            .unwrap();
        assert_eq!(diagnostics["items"][0]["file"], "examples/gen.cpp");
        assert_eq!(diagnostics["items"][0]["line"], 1);
    }

    // Python の生成器・愚直解・解答で試す（Phase 19 作業 1。D-220〜D-222）。手元にジャッジと同じ CPython 3.13 がないので、
    // 版だけを「Python 3.13.7」と答え、ほかは Homebrew の python3 に渡す代わりの処理系で動かす（版の確かめ・構文の確かめ・
    // 命令の組み立て・example の名前を確かめる。Python の版の違いによる動きの違いは確かめない）
    #[cfg(unix)]
    #[test]
    fn finds_differences_with_python_programs() {
        let real = Path::new("/opt/homebrew/bin/python3");
        if !real.is_file() {
            eprintln!("/opt/homebrew/bin/python3 がないので飛ばします");
            return;
        }
        let ws = workspace("python");
        let stand_in = ws.0.join("bin/python3.13");
        write(
            &stand_in,
            &format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"Python 3.13.7\"; exit 0; fi\nexec {} \"$@\"\n",
                real.display()
            ),
        );
        let mut permissions = fs::metadata(&stand_in).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
        fs::set_permissions(&stand_in, permissions).unwrap();
        let mut langs = LangSettings::default();
        langs.python.cpython = stand_in.to_string_lossy().into_owned();
        let pkg = ws.0.join("pkg");
        write(
            &pkg.join("main.py"),
            "n = int(input())\nprint(n * 2 + 1 if n % 7 == 3 else n * 2)\n",
        );
        write(
            &pkg.join("examples/naive.py"),
            "n = int(input())\nprint(n * 2)\n",
        );
        write(
            &pkg.join("examples/gen.py"),
            "import sys\nprint(int(sys.argv[1]))\n",
        );
        let target = Target {
            package: "pkg".to_string(),
            bin: crate::python::BIN.to_string(),
            language: crate::language::Language::Python,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let run = |langs: &LangSettings| {
            let (channel, events) = collecting_channel(|_| {});
            let finish = random_target(&pkg, &ws.0, &target, langs, 100, &cancel, &channel);
            (finish, events)
        };

        // seed 101 で WA になり、random-1 に保存する
        let (finish, events) = run(&langs);
        let Finish::RandomTested {
            done,
            failure: Some(failure),
            ..
        } = &finish
        else {
            panic!("{finish:?}\n{}", logs(&events));
        };
        assert_eq!(*done, 2);
        assert_eq!((failure.seed, failure.verdict.as_str()), (101, "WA"));
        assert_eq!(fs::read(pkg.join("test/random-1.out")).unwrap(), b"202\n");
        let text = logs(&events);
        assert!(text.contains("ローカル：Python 3.13.7"), "{text}");
        assert!(
            text.contains("生成器 examples/gen.py、愚直解 examples/naive.py"),
            "{text}"
        );
        // __pycache__ を作らない（-B・compile だけの確かめ）
        assert!(!pkg.join("__pycache__").exists());
        assert!(!pkg.join("examples/__pycache__").exists());

        // 愚直解がない：Python の名前で案内する
        fs::rename(pkg.join("examples/naive.py"), pkg.join("naive.bak")).unwrap();
        let (finish, _) = run(&langs);
        assert!(
            matches!(&finish, Finish::Failed { message } if message.contains("examples/main_naive.py") && message.contains("examples/naive.py")),
            "{finish:?}"
        );
        fs::rename(pkg.join("naive.bak"), pkg.join("examples/naive.py")).unwrap();

        // 生成器に構文の誤り：CE。エラー一覧に、生成器のファイルの場所と行が出る
        write(
            &pkg.join("examples/gen.py"),
            "import sys\nif True\n    print(1)\n",
        );
        let (finish, events) = run(&langs);
        assert!(matches!(finish, Finish::CompileError), "{finish:?}");
        {
            let events = events.lock().unwrap();
            let diagnostics = events
                .iter()
                .rfind(|event| event["type"] == "diagnostics")
                .unwrap();
            assert_eq!(diagnostics["items"][0]["file"], "examples/gen.py");
            assert_eq!(diagnostics["items"][0]["line"], 2);
        }

        // ジャッジと違う版（Homebrew の python3 そのもの）は動かさない
        langs.python.cpython = real.to_string_lossy().into_owned();
        let (finish, _) = run(&langs);
        assert!(
            matches!(&finish, Finish::Failed { message } if message.contains("ジャッジの CPython") && message.contains("brew install python@3.13")),
            "{finish:?}"
        );
    }
}
