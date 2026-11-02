// 実行履歴（Phase 4a 作業 4。D-137）
//
// 通常実行・テストの結果と、AtCoder への提出（Phase 6）だけを、アプリのフォルダの history.json に残す（ソースの写しは残さない）。
// 書き方・権限・読めないときの退避は、ほかの作業データと同じ（D-034〜D-036・D-039・D-041）。
// すべての作業領域の分を 1 つのファイルに入れ、合わせて HISTORY_LIMIT 件を超えたら古いものから消す。
// 画面には、今の作業領域の分だけを、作業領域のパスを除いて渡す（パスは WebView に渡さない）

use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::run::Finish;
use crate::storage::{self, Loaded, VERSION};

pub const HISTORY_FILE: &str = "history.json";
// 残す件数の上限（すべての作業領域の合計。ユーザーの判断、2026-10-23）
pub const HISTORY_LIMIT: usize = 500;

// 読んでから書くまでを 1 つずつ行う（記録と消去が重ならないように）
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    // 通常実行
    Run,
    // サンプルテスト（全ケース）
    Test,
    // サンプルテスト（一部のケースだけ。Phase 4a 作業 2）
    PartialTest,
    // ランダムテスト（Phase 4b 作業 2。D-143）
    RandomTest,
    // 対話テスト（Phase 4c 作業 1。D-144）
    InteractiveTest,
    // AtCoder への提出（Phase 6 作業 2。D-148）。ソースの写し・セッションは残さない
    Submit,
}

// ファイルに残す 1 件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    // 始めた時刻（UNIX 時間のミリ秒）
    pub time_ms: u64,
    // 作業領域のルート（canonicalize 済み）
    pub workspace: String,
    pub package: String,
    pub bin: String,
    pub kind: Kind,
    pub finish: Finish,
}

// 画面に渡す 1 件（作業領域のパスを除く）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryView {
    pub time_ms: u64,
    pub package: String,
    pub bin: String,
    pub kind: Kind,
    pub finish: Finish,
}

fn parse(value: Value) -> Option<Vec<Entry>> {
    // 読めない項目（形が変わった古い項目など）は捨てる
    Some(
        value
            .get("entries")?
            .as_array()?
            .iter()
            .filter_map(|item| serde_json::from_value(item.clone()).ok())
            .collect(),
    )
}

// 読めなかったので退避した先（画面に知らせるまで覚える。Phase 10 作業 15。D-173）
static BROKEN: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

// 退避した先を 1 回だけ返す（commands.rs が知らせる）
pub fn take_broken() -> Option<std::path::PathBuf> {
    BROKEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

// ない、または読めない（退避した）ときは空から始める。退避したときは、知らせるために覚える
fn load(dir: &Path) -> Result<Vec<Entry>, String> {
    match storage::load_json(&dir.join(HISTORY_FILE), parse)? {
        Loaded::Ok(entries) => Ok(entries),
        Loaded::Missing => Ok(Vec::new()),
        Loaded::Broken(backup) => {
            *BROKEN
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(backup);
            Ok(Vec::new())
        }
    }
}

fn save(dir: &Path, entries: &[Entry]) -> Result<(), String> {
    let value = serde_json::json!({ "version": VERSION, "entries": entries });
    let bytes = serde_json::to_vec(&value).map_err(|_| "実行履歴を保存できません".to_string())?;
    storage::write_atomic(&dir.join(HISTORY_FILE), &bytes)
}

// 新しい 1 件を末尾に足し、上限を超えた分を古いもの（先頭）から消す
fn push_limited(entries: &mut Vec<Entry>, entry: Entry, limit: usize) {
    entries.push(entry);
    if entries.len() > limit {
        let excess = entries.len() - limit;
        entries.drain(..excess);
    }
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

// 1 件を残す
pub fn record(dir: &Path, entry: Entry) -> Result<(), String> {
    let _guard = lock();
    let mut entries = load(dir)?;
    push_limited(&mut entries, entry, HISTORY_LIMIT);
    save(dir, &entries)
}

// 作業領域の分を、新しい順に返す
pub fn entries_for(dir: &Path, workspace: &str) -> Result<Vec<EntryView>, String> {
    let _guard = lock();
    Ok(load(dir)?
        .into_iter()
        .rev()
        .filter(|entry| entry.workspace == workspace)
        .map(|entry| EntryView {
            time_ms: entry.time_ms,
            package: entry.package,
            bin: entry.bin,
            kind: entry.kind,
            finish: entry.finish,
        })
        .collect())
}

// 提出の判定が決まったら、その提出の行に判定と得点を書き足す。行がなければ false
pub fn set_submission_verdict(
    dir: &Path,
    workspace: &str,
    id: u64,
    verdict: &str,
    score: Option<&str>,
) -> Result<bool, String> {
    let _guard = lock();
    let mut entries = load(dir)?;
    let found = entries.iter_mut().rev().find_map(|entry| {
        if entry.workspace != workspace || entry.kind != Kind::Submit {
            return None;
        }
        match &mut entry.finish {
            Finish::Submitted(record) if record.submission_id == Some(id) => Some(record),
            _ => None,
        }
    });
    let Some(record) = found else {
        return Ok(false);
    };
    record.verdict = Some(verdict.to_string());
    record.score = score.map(str::to_string);
    save(dir, &entries)?;
    Ok(true)
}

// 作業領域の分の件数
pub fn count_for(dir: &Path, workspace: &str) -> Result<usize, String> {
    let _guard = lock();
    Ok(load(dir)?
        .iter()
        .filter(|entry| entry.workspace == workspace)
        .count())
}

// 作業領域の分だけを消す。消した件数を返す（ほかの作業領域の分は残す）
pub fn clear_for(dir: &Path, workspace: &str) -> Result<usize, String> {
    let _guard = lock();
    let mut entries = load(dir)?;
    let before = entries.len();
    entries.retain(|entry| entry.workspace != workspace);
    let removed = before - entries.len();
    if removed > 0 {
        save(dir, &entries)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::RunOutcome;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "atcoder-studio-history-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn entry(time_ms: u64, workspace: &str, finish: Finish) -> Entry {
        Entry {
            time_ms,
            workspace: workspace.to_string(),
            package: "pkg".to_string(),
            bin: "a".to_string(),
            kind: Kind::Run,
            finish,
        }
    }

    fn ran(elapsed_ms: u64) -> Finish {
        Finish::Ran {
            outcome: RunOutcome::Exited {
                code: 0,
                label: "終了コード 0".to_string(),
            },
            elapsed_ms,
            output_left_open: false,
        }
    }

    #[test]
    fn keeps_the_newest_entries_up_to_the_limit() {
        let mut entries = Vec::new();
        for time in 0..5 {
            push_limited(&mut entries, entry(time, "/ws", Finish::CompileError), 3);
        }
        let times: Vec<u64> = entries.iter().map(|entry| entry.time_ms).collect();
        assert_eq!(times, vec![2, 3, 4]);
    }

    #[test]
    fn records_lists_and_clears_per_workspace() {
        let dir = TempDir::new("records");
        record(&dir.0, entry(1, "/ws", ran(12))).unwrap();
        record(
            &dir.0,
            Entry {
                kind: Kind::PartialTest,
                ..entry(
                    2,
                    "/ws",
                    Finish::Tested {
                        total: 2,
                        accepted: 1,
                        failed: 1,
                        unjudged: 0,
                        interrupted: false,
                        max_elapsed_ms: Some(30),
                    },
                )
            },
        )
        .unwrap();
        record(&dir.0, entry(3, "/other", Finish::BuildStopped)).unwrap();
        // 新しい順、作業領域の分だけ。読み直しても形が保たれる
        let listed = entries_for(&dir.0, "/ws").unwrap();
        let times: Vec<u64> = listed.iter().map(|entry| entry.time_ms).collect();
        assert_eq!(times, vec![2, 1]);
        assert_eq!(listed[0].kind, Kind::PartialTest);
        assert!(matches!(
            listed[0].finish,
            Finish::Tested {
                max_elapsed_ms: Some(30),
                ..
            }
        ));
        assert!(matches!(
            listed[1].finish,
            Finish::Ran { elapsed_ms: 12, .. }
        ));
        // 画面に渡す形に、作業領域のパスは含まれない
        let json = serde_json::to_value(&listed[0]).unwrap();
        assert!(json.get("workspace").is_none());
        assert_eq!(json["timeMs"], 2);
        assert_eq!(json["kind"], "partialTest");
        assert_eq!(count_for(&dir.0, "/ws").unwrap(), 2);
        // 消すのは作業領域の分だけ
        assert_eq!(clear_for(&dir.0, "/ws").unwrap(), 2);
        assert!(entries_for(&dir.0, "/ws").unwrap().is_empty());
        assert_eq!(entries_for(&dir.0, "/other").unwrap().len(), 1);
        assert_eq!(clear_for(&dir.0, "/ws").unwrap(), 0);
    }

    #[test]
    fn keeps_random_test_entries_readable() {
        // ランダムテストの履歴（Phase 4b 作業 2）：種類と結果が、ファイルを経由しても保たれる
        let dir = TempDir::new("random");
        record(
            &dir.0,
            Entry {
                kind: Kind::RandomTest,
                ..entry(
                    1,
                    "/ws",
                    Finish::RandomTested {
                        total: 100,
                        done: 37,
                        start_seed: 123,
                        failure: Some(crate::run::RandomFailure {
                            seed: 159,
                            verdict: "WA".to_string(),
                            case: Some("random-2".to_string()),
                            already_saved: false,
                        }),
                        interrupted: false,
                        max_elapsed_ms: Some(8),
                    },
                )
            },
        )
        .unwrap();
        let listed = entries_for(&dir.0, "/ws").unwrap();
        assert_eq!(listed[0].kind, Kind::RandomTest);
        let json = serde_json::to_value(&listed[0]).unwrap();
        assert_eq!(json["kind"], "randomTest");
        assert_eq!(json["finish"]["kind"], "randomTested");
        assert_eq!(json["finish"]["done"], 37);
        assert_eq!(json["finish"]["startSeed"], 123);
        assert_eq!(json["finish"]["failure"]["seed"], 159);
        assert_eq!(json["finish"]["failure"]["verdict"], "WA");
        assert_eq!(json["finish"]["failure"]["case"], "random-2");
        assert_eq!(json["finish"]["failure"]["alreadySaved"], false);
    }

    #[test]
    fn keeps_interactive_test_entries_readable() {
        // 対話テストの履歴（Phase 4c 作業 1）：種類と結果が、ファイルを経由しても保たれる
        let dir = TempDir::new("interactive");
        record(
            &dir.0,
            Entry {
                kind: Kind::InteractiveTest,
                ..entry(
                    1,
                    "/ws",
                    Finish::InteractiveTested {
                        total: 3,
                        accepted: 2,
                        failed: 1,
                        interrupted: false,
                        max_elapsed_ms: Some(40),
                    },
                )
            },
        )
        .unwrap();
        let listed = entries_for(&dir.0, "/ws").unwrap();
        assert_eq!(listed[0].kind, Kind::InteractiveTest);
        let json = serde_json::to_value(&listed[0]).unwrap();
        assert_eq!(json["kind"], "interactiveTest");
        assert_eq!(json["finish"]["kind"], "interactiveTested");
        assert_eq!(json["finish"]["accepted"], 2);
        assert_eq!(json["finish"]["failed"], 1);
        assert_eq!(json["finish"]["maxElapsedMs"], 40);
    }

    #[test]
    fn records_submissions_and_fills_in_the_verdict() {
        // 提出の履歴（Phase 6 作業 2）：判定が決まったら、同じ行に書き足す
        let dir = TempDir::new("submit");
        let submitted = |id: Option<u64>| {
            Finish::Submitted(Box::new(crate::run::SubmittedRecord {
                contest: "awc0186".to_string(),
                task: "awc0186_a".to_string(),
                submission_id: id,
                verdict: None,
                score: None,
                error: None,
            }))
        };
        record(
            &dir.0,
            Entry {
                kind: Kind::Submit,
                ..entry(1, "/ws", submitted(Some(10000003)))
            },
        )
        .unwrap();
        record(
            &dir.0,
            Entry {
                kind: Kind::Submit,
                ..entry(2, "/other", submitted(Some(10000003)))
            },
        )
        .unwrap();
        assert!(set_submission_verdict(&dir.0, "/ws", 10000003, "AC", Some("400")).unwrap());
        assert!(!set_submission_verdict(&dir.0, "/ws", 1, "AC", None).unwrap());
        let listed = entries_for(&dir.0, "/ws").unwrap();
        let json = serde_json::to_value(&listed[0]).unwrap();
        assert_eq!(json["kind"], "submit");
        assert_eq!(json["finish"]["kind"], "submitted");
        assert_eq!(json["finish"]["submissionId"], 10000003);
        assert_eq!(json["finish"]["verdict"], "AC");
        assert_eq!(json["finish"]["score"], "400");
        // ほかの作業領域の行は変えない
        let other = serde_json::to_value(&entries_for(&dir.0, "/other").unwrap()[0]).unwrap();
        assert!(other["finish"]["verdict"].is_null());
    }

    #[test]
    fn starts_empty_when_the_file_is_missing_or_broken() {
        let dir = TempDir::new("broken");
        assert!(entries_for(&dir.0, "/ws").unwrap().is_empty());
        // 読めないファイルは退避して、空から始める（D-039）
        std::fs::write(dir.0.join(HISTORY_FILE), "not json").unwrap();
        assert!(entries_for(&dir.0, "/ws").unwrap().is_empty());
        assert!(!dir.0.join(HISTORY_FILE).exists());
        // 形の読めない項目だけを捨てる
        let value = serde_json::json!({
            "version": VERSION,
            "entries": [
                { "timeMs": 1, "workspace": "/ws", "package": "", "bin": "a", "kind": "run",
                  "finish": { "kind": "compileError" } },
                { "timeMs": 2, "workspace": "/ws", "kind": "unknown" },
            ],
        });
        std::fs::write(dir.0.join(HISTORY_FILE), value.to_string()).unwrap();
        let listed = entries_for(&dir.0, "/ws").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].time_ms, 1);
    }
}
