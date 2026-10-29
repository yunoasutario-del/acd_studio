// 対話テスト（インタラクティブ問題。Phase 4c 作業 1。D-144）：解答とジャッジ（問題の相手役のプログラム）を同時に動かし、
// 互いの標準入出力をつなぐ
//
// 手順：ジャッジ（example）を探す → 解答と一緒にビルドする（D-089）→ ケースごとに、解答とジャッジを起動してつなぐ。
// ケースは、テストのフォルダの `interactive-N.in`（N の数値の順）で、そのパスをジャッジの 1 つ目の引数で渡す。
// 1 つもなければ、引数なしで 1 回だけ動かす（ケースの名前は `interactive`）。
// 判定は、時間の制限を対話の全体にかけて TLE、解答の最大使用メモリで MLE、出力の上限で OLE、
// そのほかは、こちらが止める前に先に異常に終わった側で決める（解答なら RE、ジャッジなら WA）。両方が 0 で終われば AC。
// 片方が異常に終わったら、もう片方には短い猶予（process::PAIR_GRACE）だけ待ち、終わらなければ止める

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use tauri::ipc::Channel;

use crate::compare::{self, TestSettings};
use crate::language::LangSettings;
use crate::process::{self, Limits, OUTPUT_LIMIT, Outcome, PairResult, Side};
use crate::run::{
    CaseResult, DISPLAY_LIMIT, Finish, Interaction, NOT_CHECKED, Program, RunEvent, Turn, Verdict,
    info, millis, outcome_of, prepare, relative, send, shown,
};
use crate::toolchain::{self, Target};

// ジャッジの example の名前を決める（D-144 の 2）。自分のバイナリ用の `<bin>_judge` があればそれを使い、
// なければ、パッケージのバイナリが 1 つのときに限り `judge`
// extension は、案内に出すファイルの拡張子（rs・cpp。D-212）
pub(crate) fn pick_judge(
    extension: &str,
    bins: &[String],
    examples: &[String],
    bin: &str,
) -> Result<String, String> {
    let own = format!("{bin}_judge");
    if examples.contains(&own) {
        return Ok(own);
    }
    if bins.len() == 1 && examples.iter().any(|name| name == "judge") {
        return Ok("judge".to_string());
    }
    Err(format!(
        "対話テストには、ジャッジ（問題の相手役のプログラム）の example が要ります。examples/{own}.{extension} を作ってください（バイナリが 1 つなら、examples/judge.{extension} でも使えます）"
    ))
}

// `interactive-<数字>` の数字
fn interactive_number(stem: &str) -> Option<u64> {
    let number = stem.strip_prefix("interactive-")?;
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    number.parse().ok()
}

// 対話テストのケース：名前と、ジャッジに渡す入力のファイル。テストのフォルダの `interactive-N.in` を、N の数値の順に並べる。
// 1 つもなければ、入力のファイルなしのケース `interactive` が 1 つ
pub(crate) fn interactive_cases(test_dir: Option<&Path>) -> Vec<(String, Option<PathBuf>)> {
    let mut found: Vec<(u64, PathBuf)> = test_dir
        .and_then(|dir| fs::read_dir(dir).ok())
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let number = interactive_number(name.strip_suffix(".in")?)?;
                    let path = entry.path();
                    path.is_file().then_some((number, path))
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    if found.is_empty() {
        return vec![("interactive".to_string(), None)];
    }
    found
        .into_iter()
        .map(|(number, path)| (format!("interactive-{number}"), Some(path)))
        .collect()
}

// 実行するケースを名前で選ぶ（1 ケースだけ・失敗したケースだけの再実行。D-137）。ないケースがあれば、実行しない
pub(crate) fn select_interactive(
    all: Vec<(String, Option<PathBuf>)>,
    wanted: Option<&[String]>,
) -> Result<Vec<(String, Option<PathBuf>)>, String> {
    let Some(wanted) = wanted else {
        return Ok(all);
    };
    if wanted.is_empty() {
        return Err("実行するケースが指定されていません".to_string());
    }
    let missing: Vec<&str> = wanted
        .iter()
        .filter(|name| !all.iter().any(|(case, _)| case == *name))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "指定したケースがありません：{}（テストケースのフォルダを確かめてください）",
            missing.join("、")
        ));
    }
    Ok(all
        .into_iter()
        .filter(|(case, _)| wanted.contains(case))
        .collect())
}

// 対話の判定（D-144 の 4）。止めた → 中断、時間の制限 → TLE、出力の上限 → OLE、解答のメモリの制限 → MLE、
// 先に異常に終わった側（解答 → RE、ジャッジ → WA）、出力の全体を得られなかった → 比較しない、そうでなければ AC
pub(crate) fn pair_verdict(result: &PairResult, settings: &TestSettings) -> Verdict {
    let memory_exceeded = result
        .solution
        .max_rss_bytes
        .is_some_and(|bytes| bytes > settings.memory_limit_mib * 1024 * 1024);
    if result.stopped {
        Verdict::Interrupted
    } else if result.timed_out {
        Verdict::TimedOut
    } else if result.output_limit.is_some() {
        Verdict::OutputLimit
    } else if memory_exceeded {
        Verdict::MemoryLimit
    } else {
        match result.first_failure {
            Some(Side::Solution) => Verdict::RuntimeError {
                outcome: outcome_of(result.solution.outcome),
            },
            Some(Side::Judge) => Verdict::WrongAnswer,
            None if result.output_left_open => Verdict::OutputIncomplete,
            None => Verdict::Accepted,
        }
    }
}

fn exit_text(outcome: Outcome) -> String {
    match outcome {
        Outcome::Exited(code) => format!("終了コード {code}"),
        Outcome::Signaled(signal) => format!("シグナル {signal}"),
        Outcome::TimedOut => "時間切れ".to_string(),
        Outcome::OutputLimit(_) => "出力の上限".to_string(),
        Outcome::Stopped => "停止".to_string(),
    }
}

// 判定の補足（画面に出す）。ジャッジの終わり方、時間切れの注意など
fn note_of(result: &PairResult, verdict: &Verdict, settings: &TestSettings) -> Option<String> {
    match verdict {
        Verdict::TimedOut => Some(format!(
            "対話の全体が時間の制限（{} ms）を超えたので、解答とジャッジを止めました。解答が出力のあとに flush していないと、ジャッジが待ち続けて止まることがあります。",
            settings.time_limit_ms
        )),
        Verdict::OutputLimit => Some(format!(
            "{}の出力が上限（{} MiB）を超えたので、解答とジャッジを止めました。",
            match result.output_limit {
                Some(Side::Judge) => "ジャッジ",
                _ => "解答",
            },
            OUTPUT_LIMIT / 1024 / 1024
        )),
        Verdict::WrongAnswer => Some(format!(
            "ジャッジが{}で終わりました。ジャッジの終了コードが 0 以外のときは WA です。",
            exit_text(result.judge.outcome)
        )),
        Verdict::RuntimeError { .. } => Some(format!(
            "ジャッジは{}で終わりました。",
            exit_text(result.judge.outcome)
        )),
        _ => None,
    }
}

// やり取りを、画面に出す量（合わせて DISPLAY_LIMIT）まで切り出す。省略したかも返す
fn turns_of(transcript: &[(Side, Vec<u8>)]) -> (Vec<Turn>, bool) {
    let mut turns = Vec::new();
    let mut room = DISPLAY_LIMIT;
    for (side, bytes) in transcript {
        if room == 0 {
            return (turns, true);
        }
        let part = &bytes[..bytes.len().min(room)];
        room -= part.len();
        turns.push(Turn {
            from: *side,
            text: String::from_utf8_lossy(part).into_owned(),
        });
        if part.len() < bytes.len() {
            return (turns, true);
        }
    }
    (turns, false)
}

// 1 つのケースを、解答とジャッジをつないで動かして判定する
fn run_case(
    plan: &Plan,
    name: &str,
    case: Option<&Path>,
    cancel: &Arc<AtomicBool>,
) -> Result<CaseResult, String> {
    let input_bytes = match case {
        Some(path) => fs::read(path).map_err(|error| format!("{name}.in を読めません：{error}"))?,
        None => Vec::new(),
    };
    let command = |executable: &Program| {
        let mut command = executable.command();
        command.current_dir(plan.dir);
        toolchain::clear_ide_env(&mut command);
        command
    };
    let solution = command(plan.solution);
    let mut judge = command(plan.judge);
    if let Some(path) = case {
        judge.arg(path);
    }
    let result = process::run_pair(
        solution,
        judge,
        &Limits {
            time: Some(Duration::from_millis(plan.settings.time_limit_ms)),
            output: OUTPUT_LIMIT,
            // 解答のメモリの制限を、実行中にも見る（D-175）
            memory: Some(plan.settings.memory_limit_mib * 1024 * 1024),
        },
        cancel,
    )?;
    let verdict = pair_verdict(&result, plan.settings);
    let (turns, turns_omitted) = turns_of(&result.transcript);
    let note = note_of(&result, &verdict, plan.settings);
    Ok(CaseResult {
        name: name.to_string(),
        elapsed_ms: Some(millis(result.elapsed)),
        input: shown(&input_bytes),
        expected: None,
        actual: String::new(),
        stderr: shown(&result.solution.stderr),
        out_file: String::new(),
        save_blocked: Some("対話テストの結果は、期待出力として保存できません".to_string()),
        memory_kib: result
            .solution
            .max_rss_bytes
            .map(|bytes| bytes.div_ceil(1024)),
        interaction: Some(Interaction {
            turns,
            turns_omitted,
            judge_stderr: shown(&result.judge.stderr),
            note,
        }),
        verdict,
    })
}

// 止めたので実行しなかったケース（未実行。D-099）
fn not_run(name: &str) -> CaseResult {
    CaseResult {
        name: name.to_string(),
        verdict: Verdict::NotRun,
        elapsed_ms: None,
        input: String::new(),
        expected: None,
        actual: String::new(),
        stderr: String::new(),
        out_file: String::new(),
        save_blocked: Some("実行していないので保存できません".to_string()),
        memory_kib: None,
        interaction: None,
    }
}

// 対話テストの 1 回分の材料
pub(crate) struct Plan<'a> {
    // パッケージのディレクトリ（プログラムの作業ディレクトリ）
    pub dir: &'a Path,
    pub solution: &'a Program,
    pub judge: &'a Program,
    pub settings: &'a TestSettings,
}

// ケースを順に動かす。events には、始めたケースと結果を渡す。止めたら残りは未実行にする（D-099）
pub(crate) fn run_cases(
    plan: &Plan,
    cases: &[(String, Option<PathBuf>)],
    cancel: &Arc<AtomicBool>,
    emit: &mut dyn FnMut(RunEvent),
) -> Finish {
    let (mut accepted, mut failed) = (0, 0);
    let mut interrupted = false;
    let mut max_elapsed_ms: Option<u64> = None;
    for (name, case) in cases {
        if interrupted {
            emit(RunEvent::Case {
                result: not_run(name),
            });
            continue;
        }
        emit(RunEvent::CaseStarted { name: name.clone() });
        let result = match run_case(plan, name, case.as_deref(), cancel) {
            Ok(result) => result,
            Err(message) => return Finish::Failed { message },
        };
        match result.verdict {
            Verdict::Accepted => accepted += 1,
            Verdict::Interrupted => interrupted = true,
            _ => failed += 1,
        }
        if !matches!(result.verdict, Verdict::Interrupted) {
            max_elapsed_ms = max_elapsed_ms.max(result.elapsed_ms);
        }
        emit(RunEvent::Case { result });
    }
    Finish::InteractiveTested {
        total: cases.len(),
        accepted,
        failed,
        interrupted,
        max_elapsed_ms,
    }
}

// 対話テストを、ビルドから始めて行う。dir はパッケージのディレクトリ（canonicalize 済み）。
// wanted は、実行するケースの名前（None なら全部）
pub(crate) fn interactive_target(
    dir: &Path,
    target: &Target,
    langs: &LangSettings,
    wanted: Option<&[String]>,
    cancel: &Arc<AtomicBool>,
    channel: &Channel<RunEvent>,
) -> Finish {
    // ジャッジの名前、ケース、設定は、ビルドの前に確かめる（誤りならビルドしない）
    let extension = target.language.extension();
    let (prepared, (judge, cases, total, folder, settings)) = match prepare(
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
            let judge = pick_judge(extension, bins, examples, &target.bin)
                .map_err(|message| Finish::Failed { message })?;
            let settings = tests
                .dir
                .as_deref()
                .map(compare::read_settings)
                .unwrap_or_else(|| Ok(TestSettings::default()))
                .map_err(|message| Finish::Failed { message })?;
            let folder = tests
                .dir
                .as_ref()
                .map(|test| relative(dir, test))
                .unwrap_or_else(|| {
                    if bins.len() == 1 {
                        "test".to_string()
                    } else {
                        format!("test/{}", target.bin)
                    }
                });
            let all = interactive_cases(tests.dir.as_deref());
            let total = all.len();
            let cases =
                select_interactive(all, wanted).map_err(|message| Finish::Failed { message })?;
            let examples = vec![judge.clone()];
            Ok(((judge, cases, total, folder, settings), examples))
        },
    ) {
        Ok(prepared) => prepared,
        Err(finish) => return finish,
    };
    let executable = prepared.executable.clone();
    let judge_exe = match prepared.example(&judge, "ジャッジ") {
        Ok(path) => path,
        Err(finish) => return finish,
    };
    let count = if cases.len() == total {
        format!("{} 件", cases.len())
    } else {
        format!("全 {total} 件のうち {} 件", cases.len())
    };
    let without_files = cases.iter().all(|(_, path)| path.is_none());
    info(
        channel,
        format!(
            "\n対話テスト：ジャッジ examples/{judge}.{extension}、{folder} の {count}{}（時間の制限 {} ms、メモリの制限 {} MiB。ローカルの判定）\nジャッジには、ケースの入力のファイル（interactive-N.in）のパスを 1 つ目の引数で渡します。\n\n",
            if without_files {
                "（interactive-N.in がないので、引数なしで 1 回だけ動かします）"
            } else {
                ""
            },
            settings.time_limit_ms,
            settings.memory_limit_mib
        ),
    );
    let plan = Plan {
        dir,
        solution: &executable,
        judge: &judge_exe,
        settings: &settings,
    };
    let finish = run_cases(&plan, &cases, cancel, &mut |event| send(channel, event));
    if let Finish::InteractiveTested { interrupted, .. } = &finish
        && *interrupted
    {
        info(
            channel,
            format!("\n対話テストを止めました。{NOT_CHECKED}\n"),
        );
    }
    finish
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{PairMember, PairResult};
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;
    use tauri::ipc::InvokeResponseBody;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn picks_judge_by_name() {
        let one = names(&["solution"]);
        let two = names(&["a", "b"]);
        assert_eq!(
            pick_judge("rs", &one, &names(&["judge"]), "solution"),
            Ok("judge".to_string())
        );
        // 自分用の名前が先
        assert_eq!(
            pick_judge("rs", &one, &names(&["judge", "solution_judge"]), "solution"),
            Ok("solution_judge".to_string())
        );
        // バイナリが複数なら、judge は使わない
        assert_eq!(
            pick_judge("rs", &two, &names(&["a_judge", "judge"]), "a"),
            Ok("a_judge".to_string())
        );
        let error = pick_judge("rs", &two, &names(&["judge"]), "a").unwrap_err();
        assert!(error.contains("examples/a_judge.rs"), "{error}");
        // 似た名前は使わない
        assert!(
            pick_judge(
                "rs",
                &one,
                &names(&["judge2", "gen", "solution_judge_2"]),
                "solution"
            )
            .is_err()
        );
    }

    struct Dir(PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn workspace(name: &str) -> Dir {
        let path = std::env::temp_dir().join(format!(
            "atcoder-studio-interactive-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Dir(path.canonicalize().unwrap())
    }

    #[test]
    fn lists_interactive_cases_in_number_order() {
        let dir = workspace("cases");
        // ファイルがなければ、入力なしのケースが 1 つ
        assert_eq!(
            interactive_cases(Some(&dir.0)),
            vec![("interactive".to_string(), None)]
        );
        assert_eq!(
            interactive_cases(None),
            vec![("interactive".to_string(), None)]
        );
        for name in [
            "interactive-10.in",
            "interactive-2.in",
            "interactive-1.in",
            "interactive-1.out",
            "interactive-x.in",
            "interactive-.in",
            "interactive.in",
            "sample-1.in",
            "random-1.in",
            "input.txt",
        ] {
            fs::write(dir.0.join(name), "").unwrap();
        }
        // フォルダは、名前が合ってもケースにしない
        fs::create_dir(dir.0.join("interactive-3.in")).unwrap();
        let listed = interactive_cases(Some(&dir.0));
        let listed_names: Vec<&str> = listed.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            listed_names,
            vec!["interactive-1", "interactive-2", "interactive-10"]
        );
        assert_eq!(listed[0].1, Some(dir.0.join("interactive-1.in")));
    }

    #[test]
    fn selects_interactive_cases_by_name() {
        let all = vec![
            (
                "interactive-1".to_string(),
                Some(PathBuf::from("/t/interactive-1.in")),
            ),
            (
                "interactive-2".to_string(),
                Some(PathBuf::from("/t/interactive-2.in")),
            ),
            (
                "interactive-10".to_string(),
                Some(PathBuf::from("/t/interactive-10.in")),
            ),
        ];
        let picked = |wanted: &[&str]| {
            select_interactive(all.clone(), Some(&names(wanted)))
                .map(|cases| cases.into_iter().map(|(name, _)| name).collect::<Vec<_>>())
        };
        assert_eq!(select_interactive(all.clone(), None).unwrap().len(), 3);
        // 指定した名前だけ。順番はケースの順のまま
        assert_eq!(
            picked(&["interactive-10", "interactive-1"]),
            Ok(names(&["interactive-1", "interactive-10"]))
        );
        // ない名前、パスのような名前、空、ほかの種類の名前は実行しない
        for wanted in [
            vec!["interactive-3"],
            vec!["interactive-1", "interactive-9"],
            vec!["../interactive-1"],
            vec!["sample-1"],
            vec![],
        ] {
            assert!(picked(&wanted).is_err(), "{wanted:?}");
        }
        assert!(
            picked(&["interactive-3"])
                .unwrap_err()
                .contains("interactive-3")
        );
    }

    fn member(outcome: Outcome, rss: Option<u64>) -> PairMember {
        PairMember {
            outcome,
            stderr: Vec::new(),
            max_rss_bytes: rss,
        }
    }

    fn result(
        solution: Outcome,
        judge: Outcome,
        first_failure: Option<Side>,
        rss: Option<u64>,
    ) -> PairResult {
        PairResult {
            solution: member(solution, rss),
            judge: member(judge, None),
            first_failure,
            timed_out: false,
            stopped: false,
            output_limit: None,
            elapsed: Duration::from_millis(5),
            transcript: Vec::new(),
            output_left_open: false,
        }
    }

    #[test]
    fn decides_verdicts_in_order() {
        let settings = TestSettings {
            memory_limit_mib: 10,
            ..TestSettings::default()
        };
        let ok = || result(Outcome::Exited(0), Outcome::Exited(0), None, Some(1024));
        assert!(matches!(pair_verdict(&ok(), &settings), Verdict::Accepted));
        // 解答が先に異常 → RE、ジャッジが先 → WA
        let mut re = result(
            Outcome::Exited(101),
            Outcome::Exited(1),
            Some(Side::Solution),
            None,
        );
        assert!(matches!(
            pair_verdict(&re, &settings),
            Verdict::RuntimeError {
                outcome: crate::run::RunOutcome::Exited { code: 101 }
            }
        ));
        let wa = result(
            Outcome::Exited(0),
            Outcome::Exited(1),
            Some(Side::Judge),
            None,
        );
        assert!(matches!(pair_verdict(&wa, &settings), Verdict::WrongAnswer));
        // 制限ちょうどは超えていない。1 バイト超えは MLE（RE より先）
        let mut limit = ok();
        limit.solution.max_rss_bytes = Some(10 * 1024 * 1024);
        assert!(matches!(pair_verdict(&limit, &settings), Verdict::Accepted));
        limit.solution.max_rss_bytes = Some(10 * 1024 * 1024 + 1);
        assert!(matches!(
            pair_verdict(&limit, &settings),
            Verdict::MemoryLimit
        ));
        re.solution.max_rss_bytes = Some(10 * 1024 * 1024 + 1);
        assert!(matches!(pair_verdict(&re, &settings), Verdict::MemoryLimit));
        // メモリが取れないときは MLE にしない
        let mut unknown = ok();
        unknown.solution.max_rss_bytes = None;
        assert!(matches!(
            pair_verdict(&unknown, &settings),
            Verdict::Accepted
        ));
        // OLE は MLE より、TLE は OLE より、中断は TLE より先
        let mut stacked = re;
        stacked.output_limit = Some(Side::Judge);
        assert!(matches!(
            pair_verdict(&stacked, &settings),
            Verdict::OutputLimit
        ));
        stacked.timed_out = true;
        assert!(matches!(
            pair_verdict(&stacked, &settings),
            Verdict::TimedOut
        ));
        stacked.stopped = true;
        assert!(matches!(
            pair_verdict(&stacked, &settings),
            Verdict::Interrupted
        ));
        // 出力の全体を得られなかった
        let mut open = ok();
        open.output_left_open = true;
        assert!(matches!(
            pair_verdict(&open, &settings),
            Verdict::OutputIncomplete
        ));
        // 補足
        let mut wa_note = wa;
        wa_note.judge.outcome = Outcome::Exited(7);
        let note = note_of(&wa_note, &Verdict::WrongAnswer, &settings).unwrap();
        assert!(note.contains("終了コード 7"), "{note}");
        let note = note_of(&ok(), &Verdict::TimedOut, &settings).unwrap();
        assert!(note.contains("flush"), "{note}");
        assert!(note_of(&ok(), &Verdict::Accepted, &settings).is_none());
    }

    #[test]
    fn cuts_the_exchange_at_the_display_limit() {
        let big = vec![b'a'; DISPLAY_LIMIT + 10];
        let (turns, omitted) = turns_of(&[
            (Side::Judge, b"1 100\n".to_vec()),
            (Side::Solution, b"? 50\n".to_vec()),
        ]);
        assert!(!omitted);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].text, "1 100\n");
        assert_eq!(turns[1].from, Side::Solution);
        let (turns, omitted) = turns_of(&[(Side::Solution, big), (Side::Judge, b"x".to_vec())]);
        assert!(omitted);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].text.len(), DISPLAY_LIMIT);
        // ちょうど収まる
        let (turns, omitted) = turns_of(&[(Side::Solution, vec![b'a'; DISPLAY_LIMIT])]);
        assert!(!omitted);
        assert_eq!(turns[0].text.len(), DISPLAY_LIMIT);
        let _ = Ordering::SeqCst;
    }

    // ---- 実際に cargo でビルドして動かす確認 ----

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

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    // 数当て（1〜100）。ジャッジはケースのファイルの「秘密の数 モード」を読み、最初に「1 100 モード」を出す。
    // 解答はモードで動きを変える：ok（二分探索）、wrong（最後に違う答え）、silent（答えずに終わる）、panic、hang（眠る）、
    // noflush（flush しない）、mle（300 MiB 確保）、ole（出力し続ける）。ジャッジの judgepanic は panic する
    const JUDGE: &str = r#"
use std::io::{BufRead, Write};
fn main() {
    let (secret, mode) = match std::env::args().nth(1) {
        Some(path) => {
            let text = std::fs::read_to_string(path).unwrap();
            let mut words = text.split_whitespace();
            let secret: i64 = words.next().unwrap().parse().unwrap();
            (secret, words.next().unwrap_or("ok").to_string())
        }
        None => (7, "ok".to_string()),
    };
    let mut out = std::io::stdout();
    writeln!(out, "1 100 {mode}").unwrap();
    out.flush().unwrap();
    if mode == "judgepanic" {
        panic!("judge panic");
    }
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    if mode == "ole" {
        let mut line = String::new();
        while input.read_line(&mut line).unwrap() > 0 {
            line.clear();
        }
        return;
    }
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).unwrap() == 0 {
            eprintln!("judge: EOF before the answer");
            std::process::exit(2);
        }
        let mut words = line.split_whitespace();
        let kind = words.next().unwrap();
        let x: i64 = words.next().unwrap().parse().unwrap();
        if kind == "!" {
            std::process::exit(if x == secret { 0 } else { 1 });
        }
        let reply = if x < secret { "HIGHER" } else if x > secret { "LOWER" } else { "EQUAL" };
        writeln!(out, "{reply}").unwrap();
        out.flush().unwrap();
    }
}
"#;
    const SOLUTION: &str = r#"
use std::io::{BufRead, Write};
fn main() {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut header = String::new();
    if input.read_line(&mut header).unwrap() == 0 {
        return;
    }
    let mut words = header.split_whitespace();
    let mut lo: i64 = words.next().unwrap().parse().unwrap();
    let mut hi: i64 = words.next().unwrap().parse().unwrap();
    let mode = words.next().unwrap_or("ok").to_string();
    let mut keep = Vec::new();
    match mode.as_str() {
        "silent" => return,
        "hang" => std::thread::sleep(std::time::Duration::from_secs(30)),
        "ole" => {
            let mut out = std::io::stdout();
            let chunk = format!("{}\n", "x".repeat(65535));
            loop {
                if out.write_all(chunk.as_bytes()).is_err() {
                    return;
                }
            }
        }
        "mle" => keep = vec![1u8; 300 << 20],
        _ => {}
    }
    std::hint::black_box(&keep);
    let noflush = mode == "noflush";
    let mut out = std::io::BufWriter::new(std::io::stdout());
    loop {
        let mid = (lo + hi) / 2;
        writeln!(out, "? {mid}").unwrap();
        if !noflush {
            out.flush().unwrap();
        }
        let mut reply = String::new();
        if input.read_line(&mut reply).unwrap() == 0 {
            return;
        }
        if mode == "panic" {
            panic!("solution panic");
        }
        match reply.trim() {
            "HIGHER" => lo = mid + 1,
            "LOWER" => hi = mid - 1,
            "EQUAL" => {
                let answer = if mode == "wrong" { mid + 1 } else { mid };
                writeln!(out, "! {answer}").unwrap();
                out.flush().unwrap();
                return;
            }
            other => panic!("unexpected reply {other}"),
        }
    }
}
"#;

    struct Run {
        finish: Finish,
        events: Arc<Mutex<Vec<serde_json::Value>>>,
    }

    impl Run {
        // Case イベントを、名前の順に集める
        fn cases(&self) -> Vec<serde_json::Value> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event["type"] == "case")
                .map(|event| event["result"].clone())
                .collect()
        }
    }

    fn run_target(
        pkg: &Path,
        wanted: Option<&[String]>,
        cancel: &Arc<AtomicBool>,
        on_event: impl Fn(&serde_json::Value) + Send + Sync + 'static,
    ) -> Run {
        let target = Target {
            package: "pkg".to_string(),
            bin: "solution".to_string(),
            language: crate::language::Language::Rust,
        };
        let (channel, events) = collecting_channel(on_event);
        let finish = interactive_target(
            pkg,
            &target,
            &LangSettings::default(),
            wanted,
            cancel,
            &channel,
        );
        Run { finish, events }
    }

    // ビルドから判定・停止まで、実際のプログラムで確かめる。cargo が要る
    #[test]
    #[cfg_attr(windows, ignore = "Windows：段階 2（使用メモリの計測。Job Object）で直す。docs/windows-port-plan.md 7 章")]
    fn tests_interactively_with_real_programs() {
        let ws = workspace("e2e");
        let pkg = ws.0.join("pkg");
        write(
            &pkg.join("Cargo.toml"),
            "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"solution\"\npath = \"src/solution.rs\"\n",
        );
        write(&pkg.join("src/solution.rs"), SOLUTION);
        write(&pkg.join("examples/judge.rs"), JUDGE);
        let test_dir = pkg.join("test");
        let case = |number: u32, text: &str| {
            write(&test_dir.join(format!("interactive-{number}.in")), text)
        };
        let fresh = || {
            let _ = fs::remove_dir_all(&test_dir);
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let verdicts = |run: &Run| -> Vec<String> {
            run.cases()
                .iter()
                .map(|case| case["verdict"]["kind"].as_str().unwrap().to_string())
                .collect()
        };

        // 1. 3 件とも AC：N の数値の順（1、2、10）。やり取りは、ジャッジの最初の出力から始まり、向きが交互になる
        fresh();
        case(10, "99 ok");
        case(2, "42 ok");
        case(1, "7 ok");
        let run = run_target(&pkg, None, &cancel, |_| {});
        let Finish::InteractiveTested {
            total: 3,
            accepted: 3,
            failed: 0,
            interrupted: false,
            max_elapsed_ms: Some(_),
        } = &run.finish
        else {
            panic!("{:?}", run.finish);
        };
        let cases = run.cases();
        let order: Vec<&str> = cases.iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert_eq!(
            order,
            vec!["interactive-1", "interactive-2", "interactive-10"]
        );
        assert_eq!(verdicts(&run), vec!["accepted"; 3]);
        let turns = cases[0]["interaction"]["turns"].as_array().unwrap();
        assert_eq!(turns[0]["from"], "judge");
        assert_eq!(turns[0]["text"], "1 100 ok\n");
        assert_eq!(turns[1]["from"], "solution");
        assert_eq!(turns[1]["text"], "? 50\n");
        assert_eq!(turns[2]["from"], "judge");
        assert_eq!(turns[2]["text"], "LOWER\n");
        // 最後は、解答の答え
        let last = turns.last().unwrap();
        assert_eq!(last["from"], "solution");
        assert_eq!(last["text"], "! 7\n");
        assert_eq!(cases[0]["input"], "7 ok");
        assert!(cases[0]["elapsedMs"].is_u64());
        assert!(cases[0]["memoryKib"].is_u64());
        assert!(cases[0]["saveBlocked"].is_string());
        assert_eq!(cases[0]["interaction"]["turnsOmitted"], false);

        // 2. 1 ケースだけ選ぶ。ないケースは実行しない
        let wanted = names(&["interactive-2"]);
        let run = run_target(&pkg, Some(&wanted), &cancel, |_| {});
        assert!(matches!(
            run.finish,
            Finish::InteractiveTested {
                total: 1,
                accepted: 1,
                ..
            }
        ));
        assert_eq!(run.cases()[0]["name"], "interactive-2");
        let wanted = names(&["interactive-4"]);
        let run = run_target(&pkg, Some(&wanted), &cancel, |_| {});
        assert!(
            matches!(&run.finish, Finish::Failed { message } if message.contains("interactive-4"))
        );

        // 3. WA（違う答え）、通信の終わり（答えずに終わる）、ジャッジの panic
        fresh();
        case(1, "5 wrong");
        case(2, "5 silent");
        case(3, "5 judgepanic");
        let run = run_target(&pkg, None, &cancel, |_| {});
        assert!(matches!(
            run.finish,
            Finish::InteractiveTested {
                total: 3,
                accepted: 0,
                failed: 3,
                ..
            }
        ));
        assert_eq!(verdicts(&run), vec!["wrongAnswer"; 3]);
        let cases = run.cases();
        let note = |i: usize| {
            cases[i]["interaction"]["note"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert!(note(0).contains("終了コード 1"), "{}", note(0));
        assert!(note(1).contains("終了コード 2"), "{}", note(1));
        assert!(
            cases[1]["interaction"]["judgeStderr"]
                .as_str()
                .unwrap()
                .contains("EOF before the answer")
        );
        assert!(note(2).contains("終了コード 101"), "{}", note(2));
        assert!(
            cases[2]["interaction"]["judgeStderr"]
                .as_str()
                .unwrap()
                .contains("judge panic")
        );

        // 4. 解答の panic：RE（終了コード 101）。ジャッジのやり取りは途中まで出る
        fresh();
        case(1, "5 panic");
        let run = run_target(&pkg, None, &cancel, |_| {});
        let cases = run.cases();
        assert_eq!(cases[0]["verdict"]["kind"], "runtimeError");
        assert_eq!(cases[0]["verdict"]["outcome"]["code"], 101);
        assert!(
            cases[0]["stderr"]
                .as_str()
                .unwrap()
                .contains("solution panic")
        );
        assert_eq!(cases[0]["interaction"]["turns"][1]["text"], "? 50\n");

        // 5. 時間の制限（1000 ms）：眠る解答（hang）、flush しない解答（noflush）は TLE。flush の注意が出る
        fresh();
        write(&test_dir.join("compare.json"), r#"{"timeLimitMs": 1000}"#);
        case(1, "5 hang");
        case(2, "5 noflush");
        let run = run_target(&pkg, None, &cancel, |_| {});
        assert_eq!(verdicts(&run), vec!["timedOut"; 2]);
        for case in run.cases() {
            assert!(
                case["interaction"]["note"]
                    .as_str()
                    .unwrap()
                    .contains("flush")
            );
            assert!(case["elapsedMs"].as_u64().unwrap() < 2500);
        }
        // flush しない解答は、ジャッジの最初の出力のあと、何も届いていない
        assert_eq!(
            run.cases()[1]["interaction"]["turns"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        // 6. MLE（制限 100 MiB）と OLE
        fresh();
        write(&test_dir.join("compare.json"), r#"{"memoryLimitMiB": 100}"#);
        case(1, "5 mle");
        case(2, "5 ole");
        let run = run_target(&pkg, None, &cancel, |_| {});
        assert_eq!(verdicts(&run), vec!["memoryLimit", "outputLimit"]);
        assert!(run.cases()[0]["memoryKib"].as_u64().unwrap() > 100 * 1024);
        assert!(
            run.cases()[1]["interaction"]["note"]
                .as_str()
                .unwrap()
                .contains("解答の出力")
        );

        // 7. ケースのファイルがなければ、引数なしで 1 回（ジャッジの既定の秘密の数 7）
        fresh();
        let run = run_target(&pkg, None, &cancel, |_| {});
        assert!(matches!(
            run.finish,
            Finish::InteractiveTested {
                total: 1,
                accepted: 1,
                ..
            }
        ));
        assert_eq!(run.cases()[0]["name"], "interactive");
        assert_eq!(run.cases()[0]["input"], "");

        // 8. 停止：1 件目の眠る解答を、始めたところで止める。残りは未実行
        fresh();
        write(&test_dir.join("compare.json"), r#"{"timeLimitMs": 20000}"#);
        case(1, "5 hang");
        case(2, "5 ok");
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let started = std::time::Instant::now();
        let run = run_target(&pkg, None, &stop, move |event| {
            if event["type"] == "caseStarted" {
                let flag = flag.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(300));
                    flag.store(true, Ordering::SeqCst);
                });
            }
        });
        assert!(started.elapsed() < Duration::from_secs(8));
        assert!(matches!(
            run.finish,
            Finish::InteractiveTested {
                total: 2,
                accepted: 0,
                failed: 0,
                interrupted: true,
                ..
            }
        ));
        assert_eq!(verdicts(&run), vec!["interrupted", "notRun"]);

        // 9. 始める前の誤り：compare.json の誤り、ジャッジがない（ビルドしない）
        fresh();
        write(&test_dir.join("compare.json"), r#"{"timeLimitMs": 0}"#);
        let run = run_target(&pkg, None, &cancel, |_| {});
        assert!(
            matches!(&run.finish, Finish::Failed { message } if message.contains("timeLimitMs"))
        );
        fresh();
        fs::remove_file(pkg.join("examples/judge.rs")).unwrap();
        let run = run_target(&pkg, None, &cancel, |_| {});
        assert!(
            matches!(&run.finish, Finish::Failed { message } if message.contains("examples/solution_judge.rs"))
        );

        // 10. ジャッジにコンパイルエラー：CE。エラー一覧にジャッジの場所
        write(
            &pkg.join("examples/judge.rs"),
            "fn main() { let x: i32 = \"a\"; }\n",
        );
        let run = run_target(&pkg, None, &cancel, |_| {});
        assert!(
            matches!(run.finish, Finish::CompileError),
            "{:?}",
            run.finish
        );
        let events = run.events.lock().unwrap();
        let diagnostics = events
            .iter()
            .find(|event| event["type"] == "diagnostics")
            .unwrap();
        assert_eq!(diagnostics["items"][0]["file"], "examples/judge.rs");
    }

    // C++ の解答とジャッジを g++-15 でビルドしてつなぐ（Phase 17 作業 7。D-212）。g++-15 が要る
    const CPP_JUDGE: &str = r#"#include <bits/stdc++.h>
using namespace std;
int main(int argc, char** argv) {
    ifstream file(argv[1]);
    long long secret;
    file >> secret;
    cout << "1 100" << endl;
    string kind;
    long long x;
    while (cin >> kind >> x) {
        if (kind == "!") return x == secret ? 0 : 1;
        cout << (x < secret ? "HIGHER" : x > secret ? "LOWER" : "EQUAL") << endl;
    }
    return 2;
}
"#;
    const CPP_SOLUTION: &str = r#"#include <bits/stdc++.h>
using namespace std;
int main() {
    long long lo, hi;
    cin >> lo >> hi;
    while (true) {
        long long mid = (lo + hi) / 2;
        cout << "? " << mid << endl;
        string reply;
        cin >> reply;
        if (reply == "HIGHER") lo = mid + 1;
        else if (reply == "LOWER") hi = mid - 1;
        else { cout << "! " << (mid == 42 ? mid + 1 : mid) << endl; return 0; }
    }
}
"#;

    #[test]
    fn connects_cpp_programs() {
        let cpp = LangSettings::default();
        if crate::cpp::find_compiler(&cpp.cpp).is_err() {
            eprintln!("g++-15 がないので飛ばします");
            return;
        }
        let ws = workspace("cpp");
        let pkg = ws.0.join("pkg");
        write(&pkg.join("main.cpp"), CPP_SOLUTION);
        write(&pkg.join("examples/judge.cpp"), CPP_JUDGE);
        write(&pkg.join("test/interactive-1.in"), "7\n");
        write(&pkg.join("test/interactive-2.in"), "42\n");
        let target = Target {
            package: "pkg".to_string(),
            bin: crate::cpp::BIN.to_string(),
            language: crate::language::Language::Cpp,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let (channel, events) = collecting_channel(|_| {});
        let finish = interactive_target(&pkg, &target, &cpp, None, &cancel, &channel);
        let run = Run { finish, events };
        // 7 は当たり、42 は 1 つずれた答えで WA
        assert!(
            matches!(
                run.finish,
                Finish::InteractiveTested {
                    total: 2,
                    accepted: 1,
                    failed: 1,
                    ..
                }
            ),
            "{:?}",
            run.finish
        );
        let cases = run.cases();
        assert_eq!(cases[0]["verdict"]["kind"], "accepted");
        assert_eq!(cases[1]["verdict"]["kind"], "wrongAnswer");
        assert_eq!(cases[0]["interaction"]["turns"][0]["text"], "1 100\n");
        assert!(pkg.join("target/examples/judge").is_file());
        let log: String = run
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event["type"] == "log")
            .map(|event| event["text"].as_str().unwrap().to_string())
            .collect();
        assert!(log.contains("ジャッジ examples/judge.cpp"), "{log}");

        // ジャッジがない：C++ の名前で案内する
        fs::remove_file(pkg.join("examples/judge.cpp")).unwrap();
        let (channel, _) = collecting_channel(|_| {});
        let finish = interactive_target(&pkg, &target, &cpp, None, &cancel, &channel);
        assert!(
            matches!(&finish, Finish::Failed { message } if message.contains("examples/main_judge.cpp") && message.contains("examples/judge.cpp")),
            "{finish:?}"
        );
    }

    const PYTHON_JUDGE: &str = r#"import sys
secret = int(open(sys.argv[1]).read())
print("1 100", flush=True)
for line in sys.stdin:
    kind, x = line.split()
    x = int(x)
    if kind == "!":
        sys.exit(0 if x == secret else 1)
    print("HIGHER" if x < secret else "LOWER" if x > secret else "EQUAL", flush=True)
sys.exit(2)
"#;
    const PYTHON_SOLUTION: &str = r#"lo, hi = map(int, input().split())
while True:
    mid = (lo + hi) // 2
    print("?", mid, flush=True)
    reply = input()
    if reply == "HIGHER":
        lo = mid + 1
    elif reply == "LOWER":
        hi = mid - 1
    else:
        print("!", mid + 1 if mid == 42 else mid, flush=True)
        break
"#;

    // Python の解答とジャッジをつなぐ（Phase 19 作業 2。D-220）。手元の処理系（設定の既定：CPython 3.13、
    // 入っていれば PyPy 3.11 も）で試す。どちらもなければ飛ばす
    #[test]
    fn connects_python_programs() {
        let interpreters = [
            crate::python::Interpreter::Cpython,
            crate::python::Interpreter::Pypy,
        ];
        let mut tried = 0;
        for interpreter in interpreters {
            let mut langs = LangSettings::default();
            langs.python.interpreter = interpreter;
            if crate::python::find_interpreter(&langs.python).is_err() {
                eprintln!("{} がないので飛ばします", interpreter.label());
                continue;
            }
            tried += 1;
            let ws = workspace(&format!("python-{}", interpreter.label()));
            let pkg = ws.0.join("pkg");
            write(&pkg.join("main.py"), PYTHON_SOLUTION);
            write(&pkg.join("examples/judge.py"), PYTHON_JUDGE);
            write(&pkg.join("test/interactive-1.in"), "7\n");
            write(&pkg.join("test/interactive-2.in"), "42\n");
            let target = Target {
                package: "pkg".to_string(),
                bin: crate::python::BIN.to_string(),
                language: crate::language::Language::Python,
            };
            let cancel = Arc::new(AtomicBool::new(false));
            let (channel, events) = collecting_channel(|_| {});
            let finish = interactive_target(&pkg, &target, &langs, None, &cancel, &channel);
            let run = Run { finish, events };
            // 7 は当たり、42 は 1 つずれた答えで WA
            assert!(
                matches!(
                    run.finish,
                    Finish::InteractiveTested {
                        total: 2,
                        accepted: 1,
                        failed: 1,
                        ..
                    }
                ),
                "{:?}",
                run.finish
            );
            let cases = run.cases();
            assert_eq!(cases[0]["verdict"]["kind"], "accepted");
            assert_eq!(cases[1]["verdict"]["kind"], "wrongAnswer");
            assert_eq!(cases[0]["interaction"]["turns"][0]["text"], "1 100\n");
            let log: String = run
                .events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event["type"] == "log")
                .map(|event| event["text"].as_str().unwrap().to_string())
                .collect();
            assert!(log.contains("ジャッジ examples/judge.py"), "{log}");
            assert!(!pkg.join("__pycache__").exists());

            // ジャッジがない：Python の名前で案内する
            fs::remove_file(pkg.join("examples/judge.py")).unwrap();
            let (channel, _) = collecting_channel(|_| {});
            let finish = interactive_target(&pkg, &target, &langs, None, &cancel, &channel);
            assert!(
                matches!(&finish, Finish::Failed { message } if message.contains("examples/main_judge.py") && message.contains("examples/judge.py")),
                "{finish:?}"
            );
        }
        eprintln!("{tried} 個の処理系で試しました");
    }
}
