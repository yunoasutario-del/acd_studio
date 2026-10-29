# Windows 移植の調査と計画

AtCoder Studio（Tauri 2。今は macOS 専用）を Windows 11 x64 で動かすための調査結果と、段階的な移植の計画。
この文書を書いた時点では、コードは変えていない（2026-10-29、windows-port ブランチ、dc5d90c の上）。

## 確認状況の書き方

| 表記 | 意味 |
| --- | --- |
| 確認済み | コードを読んだ、またはこの環境でコマンドを実行して確かめた |
| 未確認 | 推測、または資料の記述だけで、この環境では確かめていない。確かめ方を添える |
| 不明 | 調べても分からなかった |

本文の「（推測）」は未確認の推測であることを示す。

---

## 1. 前提と環境

| 項目 | 内容 | 確認状況 |
| --- | --- | --- |
| OS | Windows 11 Home 10.0.26200、x64 | 確認済み |
| シェル | Windows PowerShell 5.1.26100（端末のコードページは 65001） | 確認済み |
| システムのコードページ | ACP 932、OEMCP 932（レジストリ） | 確認済み |
| Node.js / npm | v24.22.0（fnm）/ 11.19.1 | 確認済み |
| rustup | 1.29.1。toolchain は stable-x86_64-pc-windows-msvc（rustc 1.98.1）と、今回の cargo build で入った 1.99.0-x86_64-pc-windows-msvc | 確認済み |
| src-tauri/rust-toolchain.toml | 1.99.0（rustfmt・clippy） | 確認済み |
| MSVC | VS 18 BuildTools と VS 18 Community（vswhere） | 確認済み |
| WebView2 | 156.0.4314.50 | 確認済み |
| 既存の GCC | `C:\Program Files (x86)\mingw64`：MinGW-Builds 5.0.0 の g++ 15.2.0（x86_64-w64-mingw32、`--threads=win32 --exceptions=seh --with-default-msvcrt=ucrt`）、gdb 16.3 | 確認済み |
| curl | `C:\Windows\system32\curl.exe` 8.21.0（Schannel） | 確認済み |
| Python | `%LOCALAPPDATA%\Microsoft\WindowsApps` の 0 バイトの実行エイリアス（python.exe・python3.exe）だけ。実体は入っていない | 確認済み |
| ほかのツール | clangd・basedpyright・lldb-dap・perl・sh は PATH にない | 確認済み |
| VS Code | 入っている（code.cmd）。`%APPDATA%\Code\User\snippets` がある | 確認済み |
| Defender | リアルタイム保護は有効（Get-MpComputerStatus）。除外の一覧は管理者でないと読めない | 確認済み |
| Smart App Control | VerifiedAndReputablePolicyState = 0（無効） | 確認済み |
| git | `core.autocrlf=true`（`C:/Program Files/Git/etc/gitconfig`）。作業ツリーのテキストは CRLF（`git ls-files --eol`） | 確認済み |
| 開発者モード | レジストリの値を読めなかった（シンボリックリンクを作るテストに関係） | 不明 |

---

## 2. 実際のエラー

### 2.1 npm install

- 55 パッケージを追加、脆弱性 0、終了コード 0。package-lock.json は変わらなかった（`git status` で確認済み）
- npm の新しい版（12.2.1）の知らせが出ただけで、エラーはない

### 2.2 cargo build（src-tauri）

- rustup が rust-toolchain.toml の 1.99.0 を入れてからビルドした
- 依存クレート（tauri・wry・tao・webview2-com・trash など）はすべてコンパイルできた。失敗は atcoder-studio だけ
- **エラー 79 件**（警告は 0 件。エラーで止まったので、型検査より後の段階のエラーはまだ出ていない可能性がある）
- `cargo test --no-run` も実行した：**エラー 112 件**（上の 79 件 ＋ テストのコードの 33 件）

ビルドのエラーの分類（すべて確認済み）

| 種類 | 場所（ファイル:行） | 件数 | 確認状況 |
| --- | --- | --- | --- |
| `std::os::unix` がない | debug.rs:30、lsp.rs:30、process.rs:18、storage.rs:13、workspace.rs:457 | 5 | 確認済み（ビルドの出力） |
| `std::os::fd` がない | random.rs:10（`OwnedFd`。random.rs:74 で `std::io::pipe()` の読み口を File にする） | 1 | 確認済み（ビルドの出力） |
| libc に `pid_t` がない | debug.rs:935・1204・1458・1646、lsp.rs:539・584・607、process.rs:71・121・148・170・179・197・238・493・552・561 | 17 | 確認済み（ビルドの出力） |
| libc に `kill`・`SIGKILL` がない | debug.rs:1239・1251（kill 2、SIGKILL 1）、process.rs:173（SIGKILL） | 4 | 確認済み（ビルドの出力） |
| proc_pid_rusage 一式がない | process.rs:73・74（rusage_info_v0・rusage_info_t）・75（proc_pid_rusage・RUSAGE_INFO_V0） | 5 | 確認済み（ビルドの出力） |
| waitid 一式がない | process.rs:125・128–132・181・184–188・200・203–207・211・497・500–504・510（siginfo_t・waitid・P_PID・id_t・WEXITED・WNOWAIT・WNOHANG・CLD_EXITED） | 27 | 確認済み（ビルドの出力） |
| wait4・rusage・killpg がない | process.rs:152（rusage）・154（wait4）・173（killpg） | 3 | 確認済み（ビルドの出力） |
| localtime_r がない | submit.rs:1482 | 1 | 確認済み（ビルドの出力） |
| `Command::process_group` がない | debug.rs:858・925、lsp.rs:514、process.rs:230・543 | 5 | 確認済み（ビルドの出力） |
| `ExitStatus::from_raw`・`signal` がない | process.rs:160・340・525 | 3 | 確認済み（ビルドの出力） |
| Unix のパーミッション | storage.rs:441（DirBuilder::mode）・466・820（OpenOptions::mode）・469（Permissions::from_mode） | 4 | 確認済み（ビルドの出力） |
| `Metadata::dev`・`ino` がない | workspace.rs:458（4 件） | 4 | 確認済み（ビルドの出力） |
| 合計 | | 79 | 確認済み（ビルドの出力） |

テストのコードで増えるエラー（`cargo test --no-run`。すべて確認済み）

| 種類 | 場所（ファイル:行） | 確認状況 |
| --- | --- | --- |
| `std::os::unix`（主に `fs::symlink`・`PermissionsExt`・`ExitStatusExt`） | search.rs:541、workspace.rs:492・972、debug.rs:1736・1879、lsp.rs:1150・1236、random.rs:693・1142、run.rs:1781、scaffold.rs:1246、sources.rs:221・295・340、submit.rs:2391、trust.rs:224・225 | 確認済み（ビルドの出力） |
| libc の `pid_t`・`kill`・`SIGKILL` | debug.rs:1880、lsp.rs:1347・1360・1363、process.rs:773・776・839・851・1080・1091・1092 | 確認済み（ビルドの出力） |
| `Permissions::mode`・`from_mode` | storage.rs:882・1027・1235、workspace.rs:975・979 | 確認済み（ビルドの出力） |

コンパイルは通るが、Windows では動かないテスト（コードで確認済み。動かないことは未確認）：
process.rs の `/bin/sh`・`perl`・`pkill`・`kill -SEGV`・`head`・`yes`（:746–1146）、lsp.rs:1321 `/usr/bin/perl`、
debug.rs:1871 `/bin/sleep`・:1890 `/usr/bin/false`・:2532 `~/Library/…`・:2573 `/opt/homebrew/bin/pypy3.11`・:2761 `/usr/bin/pgrep`、
run.rs:1892・1902 `/bin/cat`、random.rs:1127 `/opt/homebrew/bin/python3`、cpp.rs・lsp.rs の `/opt/homebrew/bin/g++-15` を前提にした値の検査、
session.rs の Keychain を使うテスト、storage.rs の 0o600・0o700 の検査。

なお `libc::SIGSEGV`（process.rs:813・1070）はエラーにならなかった。Windows の libc にも定義がある（確認済み）が、Windows のプロセスがシグナルで終わることはない。

---

## 3. macOS / Unix に依存している箇所

「確認状況」は、左の 2 列（箇所と内容）をコードで確かめたかと、代替案が Windows で動くことを確かめたかを分けて書く。
代替案はこの時点ではどれも動かしていないので、ほぼすべて「代替案：未確認」になる。

### 3.1 プロセスの管理と、時間・メモリの計測

| ファイル・行 | 内容 | Windows での代替案 | 確認状況 |
| --- | --- | --- | --- |
| process.rs:18・230・543 | `CommandExt::process_group(0)` で新しいプロセスグループにし、子孫ごと止める | Job Object（CreateJobObjectW）に入れる。`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` で IDE が落ちても子が残らない | 箇所：確認済み／代替案：未確認 |
| process.rs:170–175 | `killpg(pgid, SIGKILL)`。回収する前にだけ呼ぶ | TerminateJobObject。Windows はプロセスのハンドルを持つ間は pid が再利用されないので、「回収した後は呼ばない」という制約が要らない（推測） | 箇所：確認済み／代替案：未確認 |
| process.rs:118–144・177–217・492–522 | `waitid(P_PID, …, WEXITED \| WNOWAIT)` で回収せずに終わりを知る | プロセスのハンドルに WaitForSingleObject（待っても回収にあたる処理はない）。`Child::try_wait`／`wait` でもよい（推測） | 箇所：確認済み／代替案：未確認 |
| process.rs:146–167 | `wait4` の `ru_maxrss`（最大 RSS）。macOS はバイト、Linux はキロバイト（:157–159 の cfg） | GetProcessMemoryInfo の PeakWorkingSetSize、または Job の JOBOBJECT_EXTENDED_LIMIT_INFORMATION.PeakProcessMemoryUsed（こちらはコミット量で、RSS とは意味が違う。推測）。終わったプロセスのハンドルで読めるかは未確認 | 箇所：確認済み／代替案：未確認 |
| process.rs:70–77 | `proc_pid_rusage` の `ri_resident_size`（実行中の RSS。メモリの上限で止める。D-175） | GetProcessMemoryInfo の WorkingSetSize を同じ間隔（20 ms）で読む。Job の JOB_OBJECT_LIMIT_PROCESS_MEMORY は確保が失敗する形になり、今の「超えたら止める」と挙動が違う（推測） | 箇所：確認済み／代替案：未確認 |
| process.rs:340–344・524–530 | `ExitStatusExt::signal` で Signaled を作る | Windows では常に Exited。0xC0000005（アクセス違反）・0xC00000FD（スタックオーバーフロー）・0xC0000409（`__fastfail`、abort など）を名前付きで表示する。TerminateJobObject に渡す終了コードは IDE が決める | 箇所：確認済み／代替案：未確認 |
| process.rs:160 | `ExitStatus::from_raw(status)` | 標準の `Child::wait` の ExitStatus を使う（Windows の ExitStatusExt::from_raw もある。推測） | 箇所：確認済み／代替案：未確認 |
| lsp.rs:30・514・539・584・607 | 言語サーバーをプロセスグループで起動し、`exited_without_reaping`・`kill_group` で止める | process.rs と同じ Job Object の仕組みを使う | 箇所：確認済み／代替案：未確認 |
| debug.rs:30・858・925・935・1204・1458・1646 | lldb-dap・debugpy をプロセスグループで起動 | 同上 | 箇所：確認済み／代替案：未確認 |
| debug.rs:1237–1254 | `kill(pid, 0)` で生存確認、`kill(pid, SIGKILL)` でデバッグ対象を止める | OpenProcess ＋ GetExitCodeProcess（STILL_ACTIVE）、TerminateProcess。pid の再利用の競合は Windows でも残る（推測） | 箇所：確認済み／代替案：未確認 |
| random.rs:10・67–75 | `std::io::pipe()` の読み口を `OwnedFd` で File にする | `std::os::windows::io::OwnedHandle` で同じことをする（推測） | 箇所：確認済み（ビルドのエラー）／代替案：未確認 |
| submit.rs:1477–1492 | `localtime_r` で「10/26 20:12」を作る | (a) 時刻の ms を画面に送り JS の Date で整形 (b) chrono（Cargo.lock に 0.4.45 がある）(c) windows-sys の FileTimeToSystemTime ＋ SystemTimeToTzSpecificLocalTime | 箇所：確認済み／代替案：未確認 |
| lib.rs:195–199 | 終えるときに実行・LSP を止める（子は別グループなので IDE が終わっても残るため） | Job に KILL_ON_JOB_CLOSE を付ければ、IDE が異常終了しても子が残らない（推測） | 箇所：確認済み／代替案：未確認 |

### 3.2 外部コマンドの呼び出し

| ファイル・行 | 内容 | Windows での代替案 | 確認状況 |
| --- | --- | --- | --- |
| problem.rs:558–570、submit.rs:2001–2011 | `/usr/bin/open <URL>` でブラウザを開く | ShellExecuteW（windows-sys）、tauri-plugin-opener（新しい依存）、`explorer.exe <URL>` のどれか | 箇所：確認済み／代替案：未確認 |
| commands.rs:1680–1693 | `/usr/bin/open <フォルダ>`（テンプレートのフォルダを Finder で） | ShellExecuteW、または `explorer.exe <フォルダ>`（explorer は成功しても 1 を返すことがある。推測） | 箇所：確認済み／代替案：未確認 |
| commands.rs:1695–1711・1938–1957 | `/usr/bin/open -t`（スニペット・settings.json を既定のテキストエディタで） | Windows に「既定のテキストエディタ」の決まりはない。.json の関連付けで開く／`notepad.exe`／VS Code があればそれ、から選ぶ（利用者が決める項目） | 箇所：確認済み／代替案：未確認 |
| problem.rs:156–191 | `/usr/bin/curl --proto =https --proto-redir =https --location …` で問題ページを取る | `%SystemRoot%\System32\curl.exe`（8.21.0 がある）。同じ引数で動くかは未確認 | 箇所：確認済み、curl.exe があることも確認済み／代替案：未確認 |
| session.rs:308–368 | `/usr/bin/curl --config -`（Cookie・URL・フォームを標準入力の設定で渡す） | 同上。Windows の curl.exe で `--config -` と引用符のエスケープ（session.rs:157–173）が同じに動くかは未確認 | 箇所：確認済み／代替案：未確認 |
| session.rs:33–145 | `/usr/bin/security`（Keychain）で REVEL_SESSION を保存・読み出し・削除 | 資格情報マネージャー（4.1 で詳しく） | 箇所：確認済み／代替案：未確認 |
| cpp.rs:136–148 | `/usr/bin/xcrun --show-sdk-path`（-isysroot） | 4.2 で詳しく | 箇所：確認済み／代替案：未確認 |
| lsp.rs:683–699 | `/usr/bin/xcrun --find clangd` | 4.2 で詳しく | 箇所：確認済み／代替案：未確認 |
| debug.rs:220–239・335–354 | `/usr/bin/xcrun --find lldb-dap` | 6 章（デバッグ）で詳しく | 箇所：確認済み／代替案：未確認 |
| process.rs:746–750、テストの各所 | `/bin/sh -c`、`pkill`、`pgrep`、`perl`、`/bin/sleep`、`/bin/cat`、`/usr/bin/false` | テスト用の小さな Rust のプログラム（tests の bin や examples）に置き換えるか、`cmd /c`・PowerShell。cfg で分ける | 箇所：確認済み／代替案：未確認 |

### 3.3 ツールの探索とパス

| ファイル・行 | 内容 | Windows での代替案 | 確認状況 |
| --- | --- | --- | --- |
| toolchain.rs:15–52 | cargo を PATH、`$HOME/.cargo/bin/cargo`、`/opt/homebrew/bin`・`/usr/local/bin` の順に探す。名前は `cargo` | 名前に `.exe` を付ける（`std::env::consts::EXE_SUFFIX`）。`%USERPROFILE%\.cargo\bin\cargo.exe`（CARGO_HOME があればそれ）。Homebrew の場所は探さない | 箇所：確認済み／代替案：未確認 |
| toolchain.rs:58–66 | IDE が付ける環境変数を外す（`DYLD_FALLBACK_LIBRARY_PATH` を含む） | Windows で cargo run が子に足す変数（PATH の先頭に足す target/debug/deps など）を調べ直す（未確認。`cargo run` の子で `set` を見て確かめる） | 箇所：確認済み／代替案：未確認 |
| toolchain.rs:106–112 | cargo の隣の `rustc` | `rustc.exe` | 箇所：確認済み／代替案：未確認 |
| lsp.rs:660–673 | `cargo.with_file_name("rust-analyzer")` | `rust-analyzer.exe`（rustup のプロキシ。rustup component add rust-analyzer が要る） | 箇所：確認済み／代替案：未確認 |
| cpp.rs:69–106 | `g++-15` を `/opt/homebrew/bin`、PATH、`/usr/local/bin` の順に | `g++.exe` を settings.json の cpp.compiler、PATH、`C:\msys64\ucrt64\bin` の順など（5 章）。版は `-dumpfullversion` で確かめる（cpp.rs:109–134 はそのまま使える） | 箇所：確認済み／代替案：未確認 |
| cpp.rs:26、python.rs:105–114、lsp.rs:689・753、debug.rs:77 | `brew install`・`xcode-select --install` の案内文 | Windows 向けの入れ方の文に分ける | 箇所：確認済み／代替案：未確認 |
| cpp.rs:28–31・150–154・215–218 | 出力 `target/main`・`target/main-debug`・`target/examples/<名前>` | `.exe` を付ける。MinGW の g++ は `-o main` で `main.exe` を作る（確認済み）ので、IDE が `main` のまま起動しようとすると見つからない（std の Command が `.exe` を補うかは未確認） | 箇所：確認済み、`.exe` が付くことも確認済み／代替案：未確認 |
| cpp.rs:345–364 | `g++ -E -v -x c++ /dev/null` でヘッダーの場所を得る | `NUL`、または `-` と空の標準入力 | 箇所：確認済み／代替案：未確認 |
| python.rs:40–46・152–203 | `python3.13`・`pypy3.11`・`pypy3` を Homebrew、PATH の順に | Windows の公式の CPython は `python.exe`（版の付いた名前は入らない。推測）。py ランチャー（`py -3.13`）、`%LOCALAPPDATA%\Programs\Python\Python313\python.exe`、PyPy は `pypy3.exe`（推測）。WindowsApps の 0 バイトのエイリアスは除く（実体がないことは確認済み） | 箇所：確認済み／代替案：未確認 |
| lsp.rs:751–765・802–810 | `basedpyright-langserver` を探し、リンクをたどった先の `dist/typeshed-fallback` を使う | pip 版なら `Scripts\basedpyright-langserver.exe`、npm 版なら `.cmd`（4.8）。typeshed の場所の求め方を版ごとに調べ直す（未確認） | 箇所：確認済み／代替案：未確認 |
| commands.rs:133、sources.rs:126、toolchain.rs:21、debug.rs:225・865 | `HOME` を読む | Windows では HOME は普通は設定されていない（Git Bash の中だけ。推測）→ `std::env::home_dir()`（Rust 1.86 で Windows の動きが直った。推測）か USERPROFILE | 箇所：確認済み／代替案：未確認 |
| commands.rs:1514–1519 | `app_config_dir()`（macOS は `~/Library/Application Support/<識別子>`） | Windows は `%APPDATA%\io.github.ishigaki892.acdstudio`（Roaming）になる（推測。起動して確かめる） | 箇所：確認済み／代替案：未確認 |
| commands.rs:1713–1725、snippets.rs:67・100 | VS Code のスニペット `~/Library/Application Support/Code/User/snippets/` | `%APPDATA%\Code\User\snippets\`（この環境にフォルダがあることは確認済み） | 箇所：確認済み、フォルダも確認済み／代替案：未確認 |
| lsp.rs:147–159・265–306 | `file_uri`・`path_of_uri` が `/` で始まる Unix のパスを前提にする | `file:///C:/…` の形にし、ドライブ名の大文字・小文字と `%3A` の揺れを吸収する。src/editor/lsp.ts:301–315 の documentUri も合わせる | 箇所：確認済み／代替案：未確認 |
| workspace.rs:59–71 など canonicalize の各所 | canonicalize は Windows では `\\?\C:\…` を返す（推測） | dunce（Cargo.lock に 1.0.5 がある）で普通の形にしてから、外部ツール・URI・表示に渡す | 箇所：確認済み／代替案：未確認 |
| workspace.rs:448–463 | 大文字・小文字だけの名前の変更で、`MetadataExt::dev`・`ino` で同じものかを確かめる | canonicalize した結果どうしを比べる（新しい依存なし）か、same-file（Cargo.lock にある）。ハードリンクの判定には same-file の方が確か（推測） | 箇所：確認済み／代替案：未確認 |
| workspace.rs:479–486 | リンクは remove_file で消し、ほかは trash でゴミ箱へ | フォルダへのシンボリックリンク・ジャンクションは remove_dir で消す必要がある（推測）。trash は Windows に対応（推測） | 箇所：確認済み／代替案：未確認 |
| storage.rs:13・441・466・469・820 | アプリのフォルダを 0o700、ファイルを 0o600 で作る | Windows では cfg で外す（`%APPDATA%` は利用者だけの ACL。推測）。必要なら ACL を明示する | 箇所：確認済み／代替案：未確認 |
| storage.rs:814–828 | 単一起動のロック（`File::try_lock`） | Windows でも LockFileEx で動く（推測。2 つ起動して確かめる） | 箇所：確認済み／代替案：未確認 |
| storage.rs:448–473 | 一時ファイルに書いて rename で置き換える | Windows の rename は置き換えに対応（MoveFileExW）。相手のファイルをほかのプロセス（エディタ・Defender）が開いていると失敗しうる（推測）→ 失敗したら少し待って再試行 | 箇所：確認済み／代替案：未確認 |
| src/panels/logLinks.ts:11–13 | `src/main.rs:5:10` の形だけを場所として読む（`\` を含まない） | Windows の rustc・cargo は `src\main.rs` と出す（推測）→ `\` も区切りとして受け付ける。今のままだと `main.rs:5:10` だけに一致して、別のファイルへ移動しうる | 箇所：確認済み／代替案：未確認 |
| run.rs:1457–1466、workspace.rs:310–320 | 相対パスを `/` でつなぐ | Windows でも components を `/` でつないでいるので、そのままでよい（推測） | 箇所：確認済み／代替案：未確認 |

### 3.4 画面・メニュー・設定

| ファイル・行 | 内容 | Windows での代替案 | 確認状況 |
| --- | --- | --- | --- |
| src/keybindings.ts:86–117、storage.rs:81–120 | 既定のキー割り当てが meta（⌘）。両方に同じ表がある | Windows では meta を ctrl にした表を既定にする。両方の表を OS ごとに分け、storage.rs:1508 の重なりの検査を両方で動かす | 箇所：確認済み／代替案：未確認 |
| src/keybindings.ts:141–149 | 表示が `⌃⌥⇧⌘` の記号 | Windows は `Ctrl+Shift+R` の形 | 箇所：確認済み／代替案：未確認 |
| src/keybindings.ts:157–170・216–230 | `metaKey` を照合に使う。⌥ で文字が変わる macOS の対策 | Windows キー（metaKey）は OS に取られることが多い（推測）。AltGr（Ctrl＋Alt と同じに見える）の扱い | 箇所：確認済み／代替案：未確認 |
| src/settings/SettingsDialog.tsx:51–85 | 予約のキー（⌘Q・⌘H など）と、文言「macOS やメニューで使うキー」「⌘・⌃・⌥ のどれか」 | Windows の予約（Alt+F4・Ctrl+C/V/X/Z/A、Alt 単独でメニュー）に分ける | 箇所：確認済み／代替案：未確認 |
| 既定の割り当て全体 | Ctrl+R（実行）・Ctrl+W・Ctrl+F・Ctrl+P などが WebView2 のブラウザの既定動作（再読み込みなど）と重なる | keydown の preventDefault で止まるか、WebView2 の AreBrowserAcceleratorKeysEnabled を切るかを確かめる（未確認） | 箇所：確認済み／代替案：未確認 |
| App.tsx・RunButtons.tsx:102・inputMacro.ts・lsp.ts などのコメントと表示 | 「⌘.」「⌘Z」「Finder」「Keychain」の文字（SubmissionPanel.tsx:235、ipc/scaffold.ts:104 など） | 表示する文字は OS ごとに出し分ける。コメントはそのままでよい | 箇所：確認済み／代替案：未確認 |
| menu.rs:20–113 | アプリ名のメニュー（about・services・hide・hide_others）、fullscreen、CmdOrCtrl+Q | services・hide・hide_others は macOS 向け（Windows でどう扱われるかは未確認）。Windows はメニューが窓の中に出るので、設定・終了を「ファイル」に移すなど並べ替える。Ctrl+Q は Windows の習慣ではない（Alt+F4） | 箇所：確認済み／代替案：未確認 |
| main.rs:1–2 | release だけ `windows_subsystem = "windows"` | そのままでよい。dev ではコンソールが出る | 確認済み |
| tauri.conf.json:47–61 | bundle.targets "all"、icon に icon.ico あり、macOS の署名の設定 | Windows では msi（WiX）と nsis が作られる（推測）。macOS の節は無視される（推測）。targets を Windows 用に絞るかは決める項目 | 箇所：確認済み／代替案：未確認 |
| tauri.conf.json:24–45 | CSP の `connect-src ipc: http://ipc.localhost` | Windows の Tauri の IPC も `http://ipc.localhost` を使う（推測）ので、そのまま動く見込み | 箇所：確認済み／代替案：未確認 |
| src-tauri/Cargo.toml:21 | libc を全 OS で依存にしている | `[target.'cfg(unix)'.dependencies]` に移し、Windows は windows-sys（8 章） | 確認済み |

---

## 4. Windows 特有の注意点

### 4.1 ログイン情報の保存（Keychain → 資格情報マネージャー）

今の仕組み（session.rs。確認済み）：ログインは IDE が開く別の窓（`.incognito(true)`、session.rs:464–470）で利用者が行い、
IDE は窓の Cookie から REVEL_SESSION だけを読む（session.rs:522–537）。`/usr/bin/security -i` に標準入力で値を渡して保存し、
読み直して確かめる（session.rs:65–95）。load・exists・delete（session.rs:97–144）。値はコマンドの引数・ファイル・記録・画面に出さない（session.rs:5）。
テストでは `Keychain::named`（session.rs:53–59）でサービス名を差し替える。

| 項目 | 内容 | 確認状況 | 確かめ方 |
| --- | --- | --- | --- |
| 案 A：windows-sys | CredWriteW・CredReadW・CredDeleteW（`Win32_Security_Credentials`）。CRED_TYPE_GENERIC、TargetName は `io.github.ishigaki892.acdstudio/REVEL_SESSION`、CRED_PERSIST_LOCAL_MACHINE。値は API に直接渡すので、外部コマンドも標準入力も要らない。exists は CredReadW の成否 | 未確認（API の仕様は資料の記述） | 単体テストで、テスト用の TargetName に書いて読んで消す |
| 案 B：keyring クレート | Windows・macOS・Linux を 1 つの API で扱える。新しい依存（Cargo.lock にない。確認済み）。Windows の機能を features で有効にする必要がある（推測）。macOS 側を置き換えるかは別の判断 | 未確認 | 選んだ場合に、案 A と同じ単体テストを行う |
| 値の大きさ | CredentialBlob の上限は CRED_MAX_CREDENTIAL_BLOB_SIZE（5×512 = 2560 バイト）と思われるが、valid_cookie_value（session.rs:38–44）は 4096 バイトまで許す | 上限：未確認、実際の REVEL_SESSION の長さ：不明 | ログインして値の長さを記録する（値そのものは記録しない）。上限を超える値は保存の前に断る |
| 他のプログラムからの読み出し | 同じ利用者の権限で動くプログラムは確認なしに読める（macOS の限界〔session.rs:9〕と同じ） | 推測（未確認） | 別のプロセス（PowerShell から CredRead を呼ぶ小さなスクリプト）で、確認なしに読めるかを試す |
| ログインの窓の incognito | WebView2 で InPrivate になり、閉じたあと Cookie・履歴・キャッシュ・パスワードの自動保存がディスクに残らないか。wry が WebView2 の InPrivate を使うというのは推測。パスワードの自動保存は WebView2 の IsPasswordAutosaveEnabled（既定は無効と推測）、フォームの自動入力は IsGeneralAutofillEnabled（既定は有効と推測）で、Tauri からどう設定されるかは不明 | 未確認（自動入力の設定は不明） | 下の「手順 A」 |
| パスワードが IDE に渡らない | src-tauri/src で password を含むのは security の副コマンド名だけ（session.rs:78・101・121・133。grep で確認済み）。IDE は cookies() から REVEL_SESSION だけを読む（session.rs:523–536） | コード：確認済み／Windows 版：未確認 | 下の「手順 B」 |
| ログアウト・期限切れで消える | logout（session.rs:431–435）で delete。ログインのページへ移されたら delete（session.rs:287–290）。Cookie が入れ替われば保存し直す（session.rs:281–285） | コード：確認済み／Windows 版：未確認 | 下の「手順 C」 |

手順 A（incognito の確かめ方）

1. WebView2 の利用者データのフォルダの実際の場所を確かめる（`%LOCALAPPDATA%\io.github.ishigaki892.acdstudio\EBWebView` と推測）
2. ログインの前・ログイン中・ログインの窓を閉じたあと・アプリを終えたあとに、フォルダの中のファイル一覧と更新日時を記録する
3. `Default\Network\Cookies`・`Default\Login Data`・`Default\Web Data`・`Default\History` などに、`atcoder.jp`・`REVEL_SESSION`・利用者名・パスワードの文字列が残っていないかを検索する
4. InPrivate の一時フォルダがどこに作られ、閉じたときに消えるかを見る
5. もう一度ログインの窓を開き、ログイン済みになっていないことを確かめる
6. 効いていなかったときの選択肢（利用者が決める）：ログインの窓だけ別の data_directory（一時フォルダ）にして閉じたあと消す／自動入力を切る方法を探す

手順 B（パスワードが保存されないことの確かめ方）

1. Windows 版でも「cookies() から REVEL_SESSION だけを読む」形を保ち、src-tauri/src を `password` で検索し直す
2. ログインのあと `cmdkey /list` で、項目が `io.github.ishigaki892.acdstudio…` の 1 つだけで、利用者名・パスワードの項目がないことを見る
3. CredReadW で読んだ中身が REVEL_SESSION の値だけで、入力したパスワードを含まないことを、テスト用の小さなプログラムか単体テストで確かめる
4. 手順 A の検索でもパスワードを探す。実行ログ・history.json・state.json・settings.json にも出ないことを検索する

手順 C（ログアウト・期限切れの確かめ方）

1. 「ログアウト」を押したあと、`cmdkey /list` から項目が消え、画面がログインしていない表示になる
2. 資格情報マネージャーの値を壊れた値に書き換えて（テスト用の手順）提出結果を読み込み、ログインのページへ移されて項目が消える
3. 単体テストの serve_once（session.rs:702 以降。302 で /login へ）を Windows でも動かす
4. Cookie の入れ替えで値が更新されることを、CredReadW の LastWritten で確かめる（cmdkey では見えない）

### 4.2 xcrun の呼び出し（-isysroot と clangd・lldb-dap の探索）

| 箇所 | 内容 | Windows での扱い | 確認状況 |
| --- | --- | --- | --- |
| cpp.rs:136–148 `sdk_path()` | `xcrun --show-sdk-path` の結果を OnceLock に覚える。失敗すれば None | Windows に SDK の考え方はない。`#[cfg(target_os = "macos")]` で分け、Windows では常に None にする | 箇所：確認済み／扱い：未確認 |
| cpp.rs:243–245 | `sdk_path()` があれば `-isysroot <SDK>` を付ける | Windows では付けない | 箇所：確認済み |
| cpp.rs:280–291 | clangd に渡すコンパイル命令（compilationDatabaseChanges）にも同じ `-isysroot` | 同上 | 箇所：確認済み |
| cpp.rs:345–364 `header_dirs()` | `<SDK>/usr/include` を定義ジャンプで開いてよい場所に足す | Windows では足さない。g++ の `-E -v` の場所（`/dev/null` を `NUL` に）と AC Library だけ | 箇所：確認済み／扱い：未確認 |
| 今のコードを Windows で動かしたとき | xcrun がないので起動に失敗し None になり、-isysroot は付かない（コードを読んだ限り）。ただし毎回 xcrun の起動を試みる | — | 推測（未確認。cfg で分けるので、確かめなくても問題はない） |
| lsp.rs:683–699 `find_clangd()` | `xcrun --find clangd` だけで探し、見つかった場所を OnceLock に覚える。見つからなければ「Xcode の Command Line Tools の clangd が要ります。`xcode-select --install`」（lsp.rs:689） | 探す順の案：(1) settings.json の指定（新しい項目にするかは決める項目）(2) PATH の `clangd.exe` (3) `C:\Program Files\LLVM\bin\clangd.exe` (4) MSYS2 の `C:\msys64\ucrt64\bin\clangd.exe`。案内文を Windows 向けに分ける | 箇所：確認済み／案：未確認 |
| lsp.rs:701–728 `query_driver()`・`clangd_args()` | `--query-driver=<g++ のパス>` で、clangd が g++ に標準ヘッダーの場所とターゲットを聞く | MinGW の g++ を渡すと、clangd が x86_64-w64-mingw32 のターゲットと MinGW の標準ヘッダーを使う（推測）。渡さないと MSVC のターゲットになり、bits/stdc++.h が見つからない（推測） | 未確認。確かめ方：clangd を入れ、`#include <bits/stdc++.h>` の main.cpp を開いて診断が出ないこと、`std::vector` の定義へジャンプできることを見る |
| debug.rs:220–239・335–354 `find_adapter()` | `xcrun --find lldb-dap`。見つからなければ「Xcode の Command Line Tools の lldb-dap が必要です」（debug.rs:77） | 6 章（デバッグの方式を決めてから） | 箇所：確認済み／扱い：未確認 |
| 案内文 | cpp.rs:26（`brew install gcc`）、lsp.rs:689（xcode-select）、lsp.rs:753（`brew install basedpyright`）、debug.rs:77（xcode-select）、python.rs:105–114（brew） | OS ごとの文にする | 確認済み |

### 4.3 改行コード

| 項目 | 内容 | 確認状況 |
| --- | --- | --- |
| MinGW の C++ の標準出力 | `std::cout << "a\nb\n"` をパイプで受けると `61 0D 0A 62 0D 0A`（CRLF）になった | 確認済み |
| CPython の Windows の標準出力 | `\n` を `\r\n` にする（推測） | 未確認（Python を入れて同じようにバイトを見る） |
| Rust の println! | 変えない（推測） | 未確認 |
| 比較：whitespace・float | compare.rs:150–153 の is_space が `\r` を空白として扱うので、CRLF でも WA にならない | 確認済み（コード） |
| 比較：exact | compare.rs:194 はバイトの完全一致なので、**C++・Python の解答は CRLF になって WA になる**（上の結果からの推測。比較の実行は未確認） | 未確認 |
| 期待出力の保存 | 「実際の出力を期待出力として保存」（D-137、workspace.rs:254–300）で、CRLF の `.out` ができる | 推測（未確認） |
| 入力の CRLF | AtCoder から取ったサンプルは IDE が書くので LF のまま（推測）。利用者が Windows のエディタで作った input.txt・`.in` は CRLF になりうる。C++ のテキストモードの読み込みは `\r\n` を `\n` にする（推測）。Rust の `read_line` は `\r` を残す（推測） | 未確認 |
| git の core.autocrlf | `true`（システムの gitconfig）。作業ツリーは CRLF（`git ls-files --eol`：testdata の .html は `i/lf w/crlf`、status-ac.json は `i/none w/none`） | 確認済み |
| tauri-build が作るファイル | `cargo build` のあと、src-tauri/permissions/autogenerated/*.toml の 70 個が `git status` で変更ありになった。`core.autocrlf=false` で比べると中身の差はない（`git diff` が空）。チェックアウトで CRLF になっていたファイルを、tauri-build が LF で書き直したため（`git ls-files --eol` で `i/lf w/lf`）。ビルドのたびに起きる | 確認済み |
| ソースの文字列リテラル | rustc はソースの CRLF を LF に直す（推測）ので、テストの複数行の文字列は影響を受けない | 未確認 |
| include_str! | submit.rs:2021–2023 の testdata は CRLF のまま読み込まれる（include_str! は直さない。推測）。HTML の読み取りのテストが通るかは未確認 | 未確認（`cargo test` で確かめる） |

対策の選択肢（利用者が決める）：(a) exact でも CRLF を LF に直してから比べる (b) 期待出力の保存のときに LF に直す
(c) `.gitattributes` に `* text=auto eol=lf`（または testdata だけ `-text`）を足す。tauri-build のファイルが変更ありに見える問題もこれで消える見込み（推測） (d) そのままにして注意を出す。
C++ の解答に手元だけで `_setmode` を入れる案は、ジャッジとの差が増えるので勧めない。

### 4.4 文字コード

| 項目 | 内容 | 確認状況 |
| --- | --- | --- |
| システムのコードページ | ACP・OEMCP とも 932。今の端末は 65001 | 確認済み |
| GUI から起動した子 | 端末を持たない子は既定のコードページ（932）で出力する（推測） | 未確認 |
| g++ の診断 | 本文は英語（MinGW-Builds は NLS なし。推測）。日本語を含むパスやソースの文字列は CP932 のバイトで出うる（推測）。process の出力は from_utf8_lossy などで読むので文字化けで済むが、logLinks の場所の照合が外れる | 未確認 |
| Python の `-E` | python.rs:489・504、debug.rs:283 で `-E` を付けている（確認済み）。`-E` は PYTHON で始まる環境変数を無視するので、**PYTHONUTF8・PYTHONIOENCODING を設定しても効かない**（-E の仕様は資料の記述で、未確認） | 付けていること：確認済み／効かないこと：未確認 |
| Python の標準入出力 | パイプのときはロケールの文字コード（cp932）になる（推測）。日本語を出す解答は文字化けし、cp932 で表せない文字は UnicodeEncodeError で RE になる（推測） | 未確認 |
| 案 | `-X utf8` を付ける（`-E` と両立する。ジャッジの Linux は UTF-8 なので挙動はむしろ近づく。推測）。PyPy 3.11 が `-X utf8` を受け付けるかは未確認 | 未確認 |
| cargo・rustc の出力 | UTF-8（推測） | 未確認 |
| 確かめ方 | `print("あ")` の main.py と、日本語を含むフォルダの main.cpp のエラーで、実行ログの表示を見る | 未確認（まだ行っていない） |

### 4.5 実行時間と Defender

| 項目 | 内容 | 確認状況 |
| --- | --- | --- |
| Defender の状態 | リアルタイム保護は有効（Get-MpComputerStatus）。除外の一覧は管理者でないと読めない。Smart App Control は 0（無効） | 確認済み |
| 計測の起点 | process.rs:234 の `Instant::now()` は spawn の前なので、起動にかかる時間（初回の検査を含む）が実行時間に入る | 確認済み（コード） |
| 初回の遅れ（測った値） | 54 KB の C++ の exe を 5 つ作り、それぞれ 3 回ずつ PowerShell から起動して測った：初回 33・41・46・69・33 ms、2 回目以降 5〜9 ms | 確認済み（scratchpad の %TEMP% の下。作業領域の場所・大きな exe・Rust の exe では未確認） |
| 遅れの原因 | Defender の検査と思われるが、ほかのキャッシュの影響と切り分けていない | 推測（未確認） |
| TLE への影響 | 既定の制限は 2500 ms（compare.rs:24）。上の値なら、制限すれすれの解答のときだけ判定が変わりうる | 推測（未確認） |
| 確かめ方 | 作業領域の中で、Rust と C++ の解答をビルドした直後に、サンプルテストの 1 ケース目と 2 ケース目の時間を比べる。Defender の除外に足した場合と比べる（管理者の操作が要る） | 未確認（まだ行っていない） |

対策の選択肢（利用者が決める）：(a) 作業領域を Defender の除外に足すよう案内する（安全性とのトレードオフ。IDE は自動では足さない）
(b) ビルドの直後に一度空で起動して温める (c) GetProcessTimes の作成時刻から測る（起動前の検査を除けるかは未確認）
(d) 初回の時間に注意を出す。

### 4.6 子プロセスのコンソールウィンドウ

| 項目 | 内容 | 確認状況 |
| --- | --- | --- |
| main.rs | release のときだけ `windows_subsystem = "windows"`（コンソールなし） | 確認済み |
| 起動の箇所 | src-tauri/src に `Command::new` が 36 か所、`creation_flags` は 0 か所 | 確認済み |
| 起こること | コンソールを持たない GUI のプロセスから、cargo・g++・curl・clangd・rust-analyzer・解答などのコンソールのプログラムを起動すると、そのたびに窓が出る（推測） | 未確認（release でビルドして実行・テストを押す） |
| 対策 | `std::os::windows::process::CommandExt::creation_flags(CREATE_NO_WINDOW)` を、共通の関数（例：toolchain.rs の cargo_command の隣に置く「外部コマンドを作る関数」）で必ず付ける。36 か所を 1 つの関数に寄せる | 未確認 |
| Job Object との組み合わせ | CREATE_SUSPENDED・CREATE_NO_WINDOW は同時に付けられる（推測）。IDE 自身が別の Job（端末・VS Code など）に入っていても、Windows 8 以降は入れ子にできる（推測） | 未確認 |
| dev での見え方 | dev は IDE にコンソールがあるので、子は同じコンソールを使い、窓は出ない（推測）。そのため dev だけでは気づけない | 未確認 |

### 4.7 実行中の exe の上書き

| 項目 | 内容 | 確認状況 |
| --- | --- | --- |
| 制約 | Windows では動いている exe に書き込めない・消せない（推測） | 未確認 |
| 起こること | 前の実行・デバッグ・止めたが終わりきっていない子が残ると、cargo のリンクや g++ の `-o target\main.exe` が「アクセスが拒否されました（os error 5）」などで失敗する（推測） | 未確認 |
| 止めることとの関係 | TerminateJobObject は終わるのを待たずに返る（推測）。止めたあと、Job の ActiveProcesses が 0 になる（JOBOBJECT_BASIC_ACCOUNTING_INFORMATION、または完了ポートの通知）まで待ってから、run.rs の RunControl を解放する | 未確認 |
| 抜けた子孫 | macOS の「setsid でグループから抜けた子孫」（process.rs:10–13）にあたるものは、Job に CREATE_BREAKAWAY_FROM_JOB を許さない限り作れない（推測）。Windows の方が止めやすい | 未確認 |
| ほかの原因 | Defender の検査中のロック（推測）。rust-analyzer の `cargo check` は別のファイルを作るので、普通は衝突しない（推測） | 未確認 |
| 確かめ方 | 無限ループの解答を実行し、止める→すぐビルド、を繰り返す。デバッグ中にビルドを押す（RunControl で断られることも見る） | 未確認（まだ行っていない） |
| 失敗したときの文 | 「前の実行が終わっていない可能性があります」の案内を、ビルドの失敗の文に足す | 未確認（まだ行っていない） |

### 4.8 空白や括弧を含むパス

| 項目 | 内容 | 確認状況 |
| --- | --- | --- |
| g++ の場所 | 既存の g++ は `C:\Program Files (x86)\mingw64\bin\g++.exe`。この場所から `-fsyntax-only` で bits/stdc++.h を通し、`-o main` で main.exe を作って動かした | 確認済み |
| exe への引数 | Rust の Command は、Windows では引数を MSVC の C ランタイムの規則で引用する（推測）ので、空白・括弧を含むパスを exe に渡しても問題ない見込み | 未確認 |
| .cmd・.bat | npm で入れた `basedpyright-langserver.cmd` は cmd.exe を通る。Rust 1.77.2 以降、危険な文字を含む引数を .bat・.cmd に渡すとエラーにする（推測）。括弧や `%` を含むパスで起動できるかも未確認 → basedpyright は pip 版の `.exe` を勧める | 未確認 |
| clangd の `--query-driver` | lsp.rs:701–717 は `*?[]{}\` の前に `\` を置く（glob のエスケープ）ので、Windows の区切りの `\` は `\\` になる。`,` を含むパスは断る。clangd が Windows で `\\` と括弧をどう読むかは未確認 | 未確認（4.2 の確かめ方で `Program Files (x86)` の g++ を渡す） |
| `\\?\` の付いたパス | canonicalize の結果（`\\?\C:\…`。推測）が、対話テストのジャッジの引数（interactive.rs:217–219）、g++、lldb（debug.rs:372–376 の lldb_quote）、debugpy に渡る。`\\?\` を受け付けないツールがある（推測） | 未確認 → dunce で普通の形にしてから渡す |
| 引用の自前の処理 | LLDB のコマンドの引用（debug.rs:372–376）と curl の設定の引用（session.rs:157–173）は `\` と `"` をエスケープしている。debugpy の入れ方の案内文（debug.rs:262–266）はパスを `"` で囲むだけでエスケープしない（利用者が端末に貼る文。PowerShell では先頭に `&` が要る。推測）。どれも Windows のパスで正しく読まれるかは未確認 | コード：確認済み／Windows：未確認 |
| 利用者名 | 今の利用者名は ASCII（確認済み）。CP932 で表せない文字を含む利用者名・フォルダのとき、MinGW（ANSI の API を使う部分がある。推測）や Python が扱えるかは不明 | 不明 |

### 4.9 対話テストのパイプ（process.rs:355–740、interactive.rs:199–230）

| 項目 | 内容 | 確認状況 |
| --- | --- | --- |
| 作り | 標準ライブラリの匿名パイプで、解答とジャッジの標準入出力を、別々のスレッドの同期の読み書きで中継する（spawn_relay、process.rs:437–470） | 確認済み（コード） |
| Windows で動くか | 同期の読み書きなので、そのまま動く見込み | 推測（未確認） |
| 相手が終わったあとの書き込み | BrokenPipe（ERROR_BROKEN_PIPE・ERROR_NO_DATA）になり、spawn_relay の `is_err()` で中継をやめる（推測） | 未確認 |
| ハンドルの継承 | 2 つの子を続けて起動するとき、片方のパイプの端がもう片方に継承されると EOF が届かず止まる。標準ライブラリは起動のときにロックし、子の端だけを継承させる（推測） | 未確認 |
| 終わり方 | シグナルがないので ExitKind::Signal（process.rs:393–409）は出ない。止めた側の終了コードは TerminateJobObject に渡した値になる | 推測（未確認） |
| CRLF | C++・Python のジャッジや解答は `\r\n` を出す（C++ は 4.3 で確認済み、Python は未確認）。やり取りの表示に `\r` が混じり、`\r` を考えていないジャッジ（利用者が書いたもの）が読み違えうる | 一部確認済み |
| バッファ | パイプへの出力は完全バッファ（推測）。flush を忘れたときに TLE になるのは macOS と同じ | 未確認 |
| ジャッジに渡す入力のパス | interactive.rs:217–219 で `<ケース>.in` のパスを引数で渡す。`\\?\` の形になるかは 4.8 | 未確認 |
| 確かめ方 | process.rs の run_pair のテスト（:993–1146）を、`/bin/sh` の代わりにテスト用の Rust のプログラムで書き直す。とくに keeps_a_long_exchange_in_order（1000 往復）・finds_who_failed_first・stops_the_other_side_after_a_grace_period。interactive.rs:1215 以降の Python のジャッジのテストも | 未確認（まだ行っていない） |

### 4.10 パスの安全性（作業領域の外を触らないための検証）

**この節の 1 つ目の問題は、Windows 版でほかの機能より先に直す（段階 1）。**

| 項目 | 内容 | 確認状況 |
| --- | --- | --- |
| check_name（workspace.rs:302–308） | `/`・NUL・`.`・`..`・空だけを断る。`\` と `:` は見ていない | 確認済み（コード） |
| Windows の Path の動き | `\` も区切りとして扱う。`join` に接頭辞（`C:`）や根（`\`）のあるパスを渡すと、前の部分を置き換える | 標準ライブラリの文書の記述（Windows での実行は未確認） |
| 外に出られる経路 | check_name を通った名前は canonicalize されずに `join` される。そのため次の経路で作業領域の外を触れる可能性が高い。create_file（workspace.rs:417–436）・create_folder（:377–391）：名前 `..\x`・`C:\x`・`C:x` で外にファイル・フォルダを作る（作ったあとで to_relative が失敗しても、作ったものは残る）。rename_file（:441–466）：新しい名前 `..\..\x` で外へ移す。entry_path（:356–367）を通る trash_file（:479–486）・is_link（:469–473）・is_folder（:370–374）：相対パスの最後の要素を `..\..\secret` にすると外のファイルをゴミ箱に移す（commands.rs:517 の削除） | 推測（コードからの推測。未確認） |
| 直し方 | Windows では、`\`・`:`・`<>"|?*`・制御文字・予約名・末尾の `.` と空白を check_name で断る。macOS でも `\` と `:` を断るかは決める項目（macOS ではどちらもファイル名に使える） | 未確認（まだ行っていない） |
| check_relative（workspace.rs:38–56） | `split('/')` で空の要素だけを見るが、続く `Path::components` が Windows では `\` も区切りとして扱い、`..`（ParentDir）・`C:`（Prefix）・`\`（RootDir）・UNC を断る。resolve（:58–71）は canonicalize して starts_with で確かめる。このため `\` を含む相対パスで外に出ることはない見込み | 推測（未確認） |
| `/` で分ける箇所 | workspace.rs:263（write_expected の `rsplit_once('/')`）・:357（entry_path）は最後の要素を `/` で切るだけなので、`\` を含む最後の要素がそのまま check_name・expected_name_stem に行く（上の問題につながる）。commands.rs:486・518 の `rsplit('/')` は確認の文の名前を作るだけ、problem.rs:513 はパッケージ名から問題 ID を推し量るだけ（確認済み）だが、Windows で `\` を含むと表示が変わる | 確認済み（コード） |
| 予約名 | CON・NUL・AUX・PRN・COM1〜9・LPT1〜9（`nul.txt` のような拡張子付きも）。Windows 11 でフォルダの中の `NUL` などがまだ装置を指すかは不明。canonicalize が装置のパスを返せば resolve の starts_with で断られる（推測）が、check_name の経路は canonicalize しないので、装置に書く・開くおそれがある（推測） | 不明（Windows 11 の挙動）・推測 |
| `:` を含む名前 | `ab:s` は代替データストリーム（ファイル ab の別の中身）になる（推測）。`a:s` は Path が `a:` を接頭辞と読む（推測） | 未確認 |
| 末尾の `.`・空白 | Win32 は取り除くので、`main.rs.` が `main.rs` を指し、expected_name_stem（workspace.rs:264）などの名前の検査をすり抜けうる（推測） | 未確認 |
| 8.3 の短い名前 | `PROGRA~1` など。resolve は canonicalize で長い名前にする（推測）ので starts_with は効く。名前で判断する trust.rs:35・65（`build.rs`・`target`・隠しフォルダ）は read_dir の長い名前を見るので影響は小さい（推測）。短い名前が作られるかはボリュームの設定による | 推測・設定は不明 |
| 大文字・小文字 | 信頼の記録の照合（commands.rs:91・94・145–150）は PathBuf の `==`、各所の starts_with も大文字・小文字を区別して比べる（推測）。canonicalize は実際の表記を返す（推測）ので、両方を canonicalize すれば一致する。canonicalize していない側（lsp.rs:293–298 のまだないファイルの扱い、LSP から来る `file:///c%3A/…` の小文字のドライブ名など）とは一致しない。安全側（拒否）に倒れるが、機能が壊れる | 推測（未確認） |
| `\\?\` | canonicalize した側にだけ `\\?\` が付くと、同じ場所でも starts_with が外れる（推測）→ 正規化（dunce）する場所を 1 か所に決める | 未確認 |

確かめ方（テストを Windows のパスの例で書き直す）

- workspace.rs の rejects_invalid_forms（:720–734）・rejects_sibling_with_common_prefix（:736–745）・rejects_links_to_outside（:747–759）・
  trash_rejects_root_and_outside（:989–998）・renames_files_in_same_folder（:866–915）・creates_files_without_overwriting（:841–864）に、
  `..\x`・`a\..\..\x`・`C:\x`・`C:x`・`\x`・`\\server\share`・`ab:s`・`NUL`・`nul.txt`・`x.`・`x `、`A` と `a` を足し、`#[cfg(windows)]` で動かす
- 名前を受け取るコマンド（create_new_file・create_new_folder・rename_existing_file・delete_file・save_expected）すべてについて、作業領域の外に何もできないことを確かめる
- シンボリックリンクを作るテストは開発者モードか管理者が要る（推測）。作れなければ飛ばす形にし、ジャンクション（`mklink /J`。管理者は要らない。推測）で外を指す場合も足す

---

## 5. Windows で使う外部ツールの提案

ジャッジ（AtCoder の 2025/10 版の言語一覧。コードに書かれた値：C++23 (GCC 15.2.0)、CPython 3.13.7、PyPy 3.11-v7.3.20、rustc 1.89.0）との一致を重視する。
ジャッジは Linux なので、版が同じでも OS の違いによる差は残る。

### 5.1 C++

| 候補 | 版 | 入手方法 | ジャッジとの一致 | 確認状況 |
| --- | --- | --- | --- | --- |
| 既存の MinGW-Builds（この環境に入っている） | GCC 15.2.0、ucrt、win32 スレッド、seh | 入っている。配布元は MinGW-Builds（GitHub の niXman/mingw-builds-binaries） | 版の番号は同じ。bits/stdc++.h・std::thread・std::print は `-fsyntax-only` で通った。`-o main` で作った exe は動いた | 版・ヘッダー：確認済み／std::thread・std::print のリンクと実行：未確認 |
| WinLibs（GCC ＋ MinGW-w64、POSIX スレッド、UCRT） | 版ごとの zip がある（15.2.0 の版があるかは未確認） | winlibs.com の zip、または winget の `BrechtSanders.WinLibs.POSIX.UCRT`（パッケージ名は未確認） | 版を固定しやすい。POSIX スレッド（winpthreads）はジャッジの Linux の pthread に近い（推測） | 未確認 |
| MSYS2 UCRT64 | pacman の `mingw-w64-ucrt-x86_64-gcc`（今の版は不明。ローリングリリースなので 15.2.0 に固定しにくい） | winget の `MSYS2.MSYS2`、その中で pacman | Boost・GMP などのライブラリも pacman で入る。版は動く | 未確認 |
| MSVC・clang-cl | — | — | bits/stdc++.h が使えない（GCC の拡張） | 推測（未確認） |

OS の違いによる差（ジャッジは Linux の x86_64）

| 項目 | 内容 | 確認状況 |
| --- | --- | --- |
| `long` の大きさ | MinGW では 4 バイト（`static_assert(sizeof(long)==4)` が通った）。ジャッジの Linux は 8 バイト（推測）。`long` で 64 ビットの値を扱う解答は、手元だけで桁あふれする | MinGW：確認済み／ジャッジ：推測 |
| `long double` | 16 バイト、仮数 64 ビット（x87 の 80 ビット）。Linux の x86_64 と同じ形（推測） | MinGW：確認済み |
| `__int128` | 使える | 確認済み（`-fsyntax-only`） |
| スタックの大きさ | Windows の exe の既定のスタックは小さい（MinGW の ld の既定値は不明）。ジャッジのスタックの大きさも不明。深い再帰が手元だけで落ちる（0xC00000FD）おそれ → 手元のビルドに `-Wl,--stack,<バイト>` を付けるかは決める項目 | 不明 |
| `-march=native`・`-fopenmp`・`-pthread`・`-static` | cpp.rs:22 の文のとおり、今も手元では付けていない | 確認済み（コード） |
| AC Library | ヘッダーだけなので動く見込み | 推測（未確認） |

### 5.2 Python

| 候補 | 入手方法 | ジャッジとの一致 | 確認状況 |
| --- | --- | --- | --- |
| CPython 3.13 | python.org の 3.13.7 のインストーラー（版を合わせられる。推測）、または winget の `Python.Python.3.13`（入る細かい版は未確認） | 3.13 で同じ。ジャッジのパッケージ（numpy など、python.rs:294 以降の一覧）は pip で入れる。Windows 用の wheel がないものがあるかは未確認 | 未確認 |
| PyPy 3.11 v7.3.20 | pypy.org の Windows 64 ビットの zip（v7.3.20 があるかは未確認） | 版を合わせられる見込み | 未確認 |
| 注意 | WindowsApps の 0 バイトの実行エイリアス（python.exe・python3.exe）は実体がない（確認済み）。探すときに除く。`python3.13.exe` という名前は Windows の公式版には入らない（推測） | — | 一部確認済み |

### 5.3 Rust

| 項目 | 内容 | 確認状況 |
| --- | --- | --- |
| ジャッジの版 | `rustup toolchain install 1.89.0`（x86_64-pc-windows-msvc）。MSVC のリンカーは VS 18 BuildTools で足りる見込み | 未確認 |
| 主スレッドのスタック | Windows の MSVC ターゲットの主スレッドのスタックは 1 MiB（推測。Linux は 8 MiB）。深い再帰が手元だけで落ちうる → 手元だけ `-C link-arg=/STACK:<バイト>` を付けるかは決める項目 | 未確認 |
| rust-analyzer | `rustup component add rust-analyzer`（cargo の隣に rust-analyzer.exe ができる。推測） | 未確認 |

### 5.4 言語サーバー

| 対象 | 候補と入手方法 | 確認状況 |
| --- | --- | --- |
| clangd | (1) LLVM の公式インストーラー（winget の `LLVM.LLVM`。`C:\Program Files\LLVM\bin\clangd.exe`）(2) clangd の GitHub の配布の zip (3) MSYS2 の `mingw-w64-ucrt-x86_64-clang-tools-extra`。MinGW の g++ を `--query-driver` に渡して使う（4.2） | 未確認 |
| basedpyright | (1) `pip install basedpyright`（Node.js を同梱した wheel で、Scripts に `basedpyright-langserver.exe` ができる。推測）(2) `npm install -g basedpyright`（`.cmd` になる。4.8 の制約）。(1) を勧める | 未確認 |
| rust-analyzer | 5.3 | 未確認 |

---

## 6. デバッグ

今の作り（debug.rs。確認済み）：Rust と C++ は Apple の lldb-dap を DAP で使い、`settings set target.input-path` で標準入力を渡す。
Rust は rustc に付属する LLDB の表示スクリプト（debug.rs:378–402）を読み込む。Python は debugpy を TCP でつなぐ（OS にほぼ依存しない）。

Rust（MSVC ターゲットは PDB のデバッグ情報）

| 方式 | 内容 | 難しさ | 確認状況 |
| --- | --- | --- | --- |
| A. LLVM の lldb-dap（Windows 版） | LLVM の Windows 版に lldb-dap.exe が入っているか、入っている版は未確認。LLDB の PDB の読み取りと、rustc の表示スクリプト（DWARF 向けに作られている。推測）で、Rust の enum・Vec などが正しく見えるかは未確認。LLDB の Python の版の依存がある（推測） | 高 | 未確認 |
| B. CodeLLDB の LLDB | VS Code 拡張の CodeLLDB が同梱する LLDB と、DAP の口（codelldb）。MSVC の Rust のデバッグに使われている（推測）。拡張から実行ファイルを探す、または利用者に別に入れてもらう。ライセンスは未確認 | 中 | 未確認 |
| C. Microsoft の vsdbg（cppvsdbg） | ライセンスで Visual Studio・VS Code 以外での利用が許されていない（推測）ので使えない | — | 未確認 |
| D. GNU ターゲット ＋ gdb の DAP | デバッグのときだけ `x86_64-pc-windows-gnu` でビルドし（rustup target add と MinGW のリンカーが要る）、gdb の `--interpreter=dap`（gdb 14 以降。Python で書かれている。推測）で使う。既存の gdb 16.3 は `--with-python` で作られている（`gdb --configuration` で確認済み）が、`gdb -batch -ex "python print(1)"` が 120 秒たっても終わらず、Python が使えるかは確かめられなかった。ジャッジのビルド（release、Linux）とはさらに離れる | 中〜高 | 一部確認済み・DAP は未確認 |
| E. DbgEng（WinDbg のエンジン）で自前の DAP | 自前で作る | 非常に高 | 未確認（まだ行っていない） |

C++（MinGW の g++ は DWARF）

| 方式 | 内容 | 難しさ | 確認状況 |
| --- | --- | --- | --- |
| A. gdb の DAP | 既存の gdb 16.3 か、WinLibs・MSYS2 の gdb。`-g -O0 -D_GLIBCXX_ASSERTIONS`（cpp.rs:226–228）はそのまま使える。止める関数（debug.rs:72–73 の `__cxa_throw`・`abort`）は gdb でも置ける（推測）。標準入力は DAP の launch の引数か、gdb の `run < file`（未確認） | 中 | 未確認 |
| B. LLDB（LLVM 版・CodeLLDB 版）で DWARF | MinGW の DWARF を LLDB が読めるか、libstdc++ の表示（pretty printer）が使えるかは未確認 | 中〜高 | 未確認 |

どちらの言語も、DAP の要求と答えの形がアダプターごとに少し違う（lldb-dap の拡張 `$__lldb_sourceInitFile`〔debug.rs:415〕、`settings set target.inherit-tcc`〔debug.rs:434〕など）ので、
Client の中の lldb-dap 向けの部分を、アダプターごとに分ける必要がある（推測）。

---

## 7. 移植の計画（段階ごと）

各段階で macOS の動きを壊さないよう、`#[cfg(target_os = "macos")]`・`#[cfg(windows)]`（Unix 全般に通じるものは `#[cfg(unix)]`）で分け、
段階の終わりに macOS でも `cargo test` を動かす（この環境では macOS を確かめられないので、macOS の機械での確認が要る）。

### 段階 1：ビルドが通り、`npm run tauri dev` で起動する

何を変えるか
- **最初に 4.10 のパスの検証を直す**（check_name で `\`・`:`・使えない文字・予約名・末尾の `.` と空白を断る）。Windows のテストを足す
- 2.2 の 79 件を cfg で分ける。process.rs は Windows の実装を別のモジュール（例：`process/windows.rs`）に置き、関数の形（run・run_pair・kill_group・exited_without_reaping・wait_without_reaping）は同じにする。この段階では Job Object の中身まで作らず、まず Child::kill・try_wait の仮の実装でもよい（段階 2 で作り直す）
- storage.rs のパーミッション、workspace.rs の dev・ino（canonicalize の比較）、random.rs の OwnedFd、submit.rs の localtime_r
- Cargo.toml：libc を `cfg(unix)` に、windows-sys を `cfg(windows)` に（8 章）
- HOME の読み方、`.exe` の付け方、app_config_dir の場所の確認
- 外部コマンドを作る共通の関数を作り、CREATE_NO_WINDOW を付ける（4.6）
- menu.rs の macOS 向けの項目を分ける

どう確認するか
- `cargo build`・`cargo clippy` がエラー・警告なし。`cargo test --no-run` が通る（Unix 専用のテストは `#[cfg(unix)]`）
- `npm run tauri dev` で窓が出て、フォルダを開き、ファイルの一覧・編集・保存ができる。2 つ目の起動が断られる（単一起動のロック）
- `%APPDATA%\io.github.ishigaki892.acdstudio` に state.json・settings.json ができる
- `npm run tauri build` の release で、操作のたびにコンソールの窓が出ない

### 段階 2：問題の取得とサンプルテスト（AC・WA・RE・TLE・MLE）

何を変えるか
- process.rs の Windows の実装：Job Object（KILL_ON_JOB_CLOSE）、WaitForSingleObject、TerminateJobObject と、終わるまで待つ処理（4.7）、WorkingSetSize で実行中の上限、PeakWorkingSetSize で最大使用メモリ
- RE の表示：NTSTATUS の終了コードの名前
- 問題の取得：curl.exe（問題ページ・問題の一覧）
- C++：g++ の探索、`.exe`、`/dev/null` → `NUL`、logLinks.ts の `\`
- Python：探索（エイリアスを除く）、`-X utf8`（4.4）
- 改行の扱い（4.3 で決めたもの）、Defender の初回の遅れの対策（4.5 で決めたもの）
- 対話テスト（4.9）・ランダムテスト

どう確認するか
- process.rs のテストを、`/bin/sh` の代わりにテスト用の Rust のプログラムで書き直し、Windows で通す（出力の上限、時間の上限で子孫ごと止まる、メモリの上限、両方の出力を詰まらずに読む、run_pair の各テスト）
- Rust・C++・Python のそれぞれで、AC・WA・RE（0 除算・範囲外・panic・アクセス違反）・TLE（無限ループ）・MLE（大きな配列）になる解答を作り、判定と時間・メモリの表示を確かめる
- 止めたあとすぐにビルドし直せる（4.7）。タスクマネージャーで子が残っていない

### 段階 3：ファイル・URL を開く処理、ログイン情報の保存

何を変えるか
- `/usr/bin/open` の 5 か所を、OS ごとの「開く」関数 1 つにまとめる（ShellExecuteW など。9 章で選んだもの）
- Keychain を資格情報マネージャーに（4.1 で選んだ方式）。`Keychain` の型の形（app・named・save・load・exists・delete）はそのまま
- VS Code のスニペットの場所、画面の「Finder」「Keychain」の文字

どう確認するか
- 問題ページ・提出ページ・提出結果のページがブラウザで開く。テンプレートのフォルダがエクスプローラーで開く。settings.json・スニペットが決めたエディタで開く
- 4.1 の手順 A・B・C
- VS Code のスニペットの取り込み

### 段階 4：LSP（補完・診断）

何を変えるか
- file_uri・path_of_uri（`file:///C:/…`、ドライブ名の大文字・小文字と `%3A`）、src/editor/lsp.ts の documentUri
- clangd・basedpyright・rust-analyzer の探索、clangd の `--query-driver` と MinGW
- `\\?\` の正規化（dunce）、sources.rs の読み取り専用で開いてよい場所（HOME・RUSTUP_HOME・CARGO_HOME）

どう確認するか
- 3 言語それぞれで、補完・診断・定義へのジャンプ（標準ライブラリの中へも）・参照の検索・整形・ホバー
- 作業領域の外のファイルの URI が断られる（lsp.rs のテストを Windows のパスで）
- 言語サーバーの再起動・IDE を終えたときに、言語サーバーのプロセスが残らない

### 段階 5：提出

何を変えるか
- session.rs の curl を curl.exe に（`--config -`）
- 提出の時刻の表示（localtime_r の置き換え。段階 1 で済ませていれば確認だけ）
- 提出の前の検査のうち、OS の違いが出るもの（Python の import の検査〔submit.rs:2527 付近〕など）

どう確認するか
- session.rs・submit.rs の単体テスト（手元の http の口に送るもの）を Windows で通す
- 実際に練習用のコンテストの問題へ提出し、判定が提出結果の欄に出るまで追える。Cookie の入れ替えで資格情報が更新される

### 段階 6：デバッグ

何を変えるか
- 6 章で選んだ方式のアダプターの探索と起動、DAP の違いの吸収
- 標準入力の渡し方（入力のファイルがないときは `NUL`）
- debugpy は今のままで動くか確かめる（`-X utf8` を足すかも 4.4 に合わせる）

どう確認するか
- debug.rs の本物のアダプターを使うテスト（debugs_with_real_lldb_dap・debugs_cpp_with_real_lldb_dap にあたるもの）を、選んだアダプターで書き直して通す
- ブレークポイント・ステップ・変数・式の評価・panic と例外と abort で止まる・止めたあとにプロセスが残らない

---

## 8. 新しい依存の候補

| クレート | 理由 | 代替案 | 確認状況 |
| --- | --- | --- | --- |
| windows-sys（`[target.'cfg(windows)'.dependencies]`、0.61 系） | Job Object・WaitForSingleObject・GetProcessMemoryInfo・資格情報マネージャー・ShellExecuteW・CREATE_NO_WINDOW の値。すでに tauri などの間接依存として Cargo.lock に 0.61.2 があるので、新しく取ってくるものは少ない見込み（使う features によっては増える） | windows（同じく Cargo.lock に 0.62.2 がある。より高水準で大きい）、keyring・tauri-plugin-opener などの用途別のクレートを組み合わせる | Cargo.lock にあること：確認済み／ビルド：未確認 |
| dunce | canonicalize の `\\?\` を外す。Cargo.lock に 1.0.5 がある | 自分で `\\?\` を外す小さな関数を書く（UNC の `\\?\UNC\` の扱いに注意） | Cargo.lock にあること：確認済み |
| keyring（選んだ場合だけ） | 資格情報の保存を OS ごとに書かずに済む | windows-sys の CredWriteW など（4.1 の案 A） | 未確認 |
| tauri-plugin-opener（選んだ場合だけ） | URL・ファイル・フォルダを開く処理を OS ごとに書かずに済む。Tauri の公式 | ShellExecuteW（windows-sys）、`explorer.exe` | 未確認 |
| chrono（選んだ場合だけ） | 提出の時刻の表示。Cargo.lock に 0.4.45 がある | JS 側で整形する、windows-sys の時刻の API | Cargo.lock にあること：確認済み |

---

## 9. 利用者が決める項目

1. Job Object への入れ方：(a) CREATE_SUSPENDED で起動し、主スレッドを Toolhelp32 で探して再開する (b) CreateProcessW を自前で呼ぶ（標準の Command を使わない） (c) 起動したあとで入れる（子がすぐに孫を作ると逃す、短い競合が残る）
2. 外部の API の呼び方：windows-sys を直接使うか、keyring・tauri-plugin-opener などの用途別のクレートを足すか（8 章）
3. 資格情報の保存：windows-sys の CredWriteW など（案 A）か keyring（案 B）か。keyring にするなら macOS も置き換えるか
4. 「テキストエディタで開く」（settings.json・スニペット）：.json の関連付け／メモ帳／VS Code があれば VS Code
5. Windows の既定のキー割り当て：⌘ を Ctrl に置き換えるだけか、Windows の習慣（設定は Ctrl+,、終了は Alt+F4 など）に合わせて変えるか。WebView2 のブラウザの既定のキーを切るか
6. 改行：exact の比較で CRLF を LF に直すか、期待出力を保存するときに直すか、`.gitattributes` を足すか（4.3）
7. Python に `-X utf8` を付けるか（4.4）
8. Defender の初回の遅れへの対策（4.5）
9. 手元のビルドでスタックを大きくするか（C++ の `-Wl,--stack`、Rust の `/STACK`。5.1・5.3）
10. 勧める C++ のコンパイラ：既存の MinGW-Builds（win32 スレッド）／WinLibs（POSIX スレッド）／MSYS2。settings.json の cpp.compiler で指定してもらう前提にするか、決まった場所を探すか
11. clangd・basedpyright の場所の指定を settings.json に足すか
12. デバッグの方式（6 章の A〜D）。段階 6 を今回の移植に含めるか
13. check_name で `\` と `:` を断るのを Windows だけにするか、macOS でも断るか（4.10）
14. ログインの窓の incognito が効いていなかったときの対策（4.1）
15. tauri.conf.json の bundle.targets を Windows でどうするか（msi・nsis の両方か片方か）。署名をするか
16. メニューの並び（Windows ではアプリ名のメニューがない）

---

## 10. 未確認・不明のまま残っている項目

未確認（確かめ方は各節に書いた）

- 2.2：cargo build のエラーを直したあとに出る、型検査より後の段階のエラー
- 3 章の代替案のすべて（Windows で動かしていない）
- std の Command が `.exe` のないパスに `.exe` を補うか
- curl.exe が今の引数と `--config -` で同じに動くか
- app_config_dir の実際の場所、WebView2 の利用者データのフォルダの場所
- Windows の rustc・cargo の診断のパスの区切り（`\` になるか）
- menu.rs の macOS 向けの項目を Windows で使ったときの動き
- WebView2 のブラウザの既定のキー（Ctrl+R など）と、keydown の preventDefault の効き方
- 4.1：資格情報マネージャーの値の大きさの上限、incognito の効き方、パスワードの自動保存・自動入力の既定値
- 4.3：CPython・Rust の出力の改行、exact の比較で WA になること、include_str! の testdata のテスト
- 4.4：GUI から起動した子のコードページ、g++ が CP932 で出すもの、`-E` で PYTHONUTF8 が無視されること、PyPy の `-X utf8`
- 4.5：遅れの原因が Defender か、作業領域・大きな exe での遅れ、GetProcessTimes で検査を除けるか
- 4.6：release で窓が出ること、Job Object との組み合わせ
- 4.7：実行中の exe の上書きの失敗、TerminateJobObject が非同期か
- 4.8：引数の引用、.cmd の制約、clangd の `--query-driver` の読み方、`\\?\` を受け付けないツール
- 4.9：BrokenPipe、ハンドルの継承、CRLF の影響
- 4.10：Windows の Path の join の動き（文書の記述どおりか）、check_name の経路で外に出られること、`:`・末尾の `.` と空白、大文字・小文字の比較
- 5 章：WinLibs・MSYS2・CPython・PyPy の入手できる版、ジャッジのパッケージの Windows 用の wheel、MSVC の Rust の主スレッドのスタック、std::thread・std::print のリンクと実行
- 6 章：すべてのデバッグの方式（LLVM の lldb-dap の有無、CodeLLDB のライセンス、gdb の Python と DAP）
- macOS の動きを壊していないこと（macOS の機械で確かめる）

不明（調べても分からなかった）

- 開発者モードの状態（レジストリの値を読めなかった）
- 実際の REVEL_SESSION の長さ
- Windows 11 でフォルダの中の予約名（NUL など）が装置を指すか
- 8.3 の短い名前がこのボリュームで作られるか
- MinGW の ld の既定のスタックの大きさ、ジャッジのスタックの大きさ
- CP932 で表せない文字を含む利用者名・フォルダでの MinGW・Python の動き
- MSYS2 の今の GCC の版
- Tauri からの WebView2 の自動入力の設定の仕方
