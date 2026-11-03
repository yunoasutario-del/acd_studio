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
| Python（段階 1 まで） | `%LOCALAPPDATA%\Microsoft\WindowsApps` の 0 バイトの実行エイリアス（python.exe・python3.exe）だけ。実体は入っていない | 確認済み |
| CPython（2026-10-31 に入れた） | 3.13.7。`winget install --id Python.Python.3.13 --version 3.13.7 --scope user --exact --custom "PrependPath=0 Include_launcher=0 AssociateFiles=0 Shortcuts=0 Include_test=0"`（python.org の python-3.13.7-amd64.exe。ハッシュは winget が照合）。`%LOCALAPPDATA%\Programs\Python\Python313\python.exe`（exe は python.exe・pythonw.exe だけで、`python3.13.exe` はない）。`--version` は `Python 3.13.7`（MSC v.1944 64 bit）。py ランチャー・関連付け・スタートメニューは入れていない。PEP 514 のキー `HKCU\Software\Python\PythonCore\3.13`（InstallPath の ExecutablePath が上の python.exe）ができた。`%LOCALAPPDATA%\Package Cache` の中身は見ていない | 確認済み（Package Cache は未確認） |
| PyPy（2026-10-31 に入れた） | 3.11 v7.3.20。downloads.python.org の pypy3.11-v7.3.20-win64.zip（32,758,691 バイト）の SHA256 が pypy.org/checksums.html の値（a8d36f6c…0755b）と一致したことを確かめてから、`%LOCALAPPDATA%\Programs\pypy3.11-v7.3.20-win64\` に展開した（展開の前に、展開先がないこと、zip の一番上が 1 つのフォルダだけであること、`..`・`:`・先頭の `\` を含む名前が 0 個であることを確かめた）。exe は pypy.exe・pypy3.exe・pypy3.11.exe・python.exe・python3.exe・python3.11.exe など。`--version` は `Python 3.11.13 … [PyPy 7.3.20 with MSC v.1941 64 bit]` | 確認済み |
| Python の確かめたこと | CPython・PyPy とも、`-X utf8 -c "print('あ')"` が終了コード 0 で、出力は `E3 81 82 0D 0A`（UTF-8・CRLF）。`-E -B -X int_max_str_digits=0 -X utf8` で動き、`sys.flags` で各フラグが効いていること、`str(10**5000)` が通ることを確かめた。`-X utf8` がないと、CPython はパイプへ cp932 で出し（`あ` は `82 A0`）、絵文字で UnicodeEncodeError（終了コード 1）。PyPy は付けなくても utf-8。どれも PowerShell（端末）から起動した子で、GUI から起動した子では未確認 | 確認済み（GUI からは未確認） |
| PATH・実行エイリアス | Python を入れたあとも、利用者・システムの PATH は入れる前と同じ。WindowsApps のエイリアスもそのまま（`where python` はエイリアスだけ）。そのため、段階 1 までの探し方では CPython・PyPy は見つからなかった。S7 で、CPython はレジストリから見つかるようにした（PyPy は PATH にないので、settings.json の python.pypy が要る） | 確認済み |
| ジャッジのパッケージ | numpy などは入れていない | 確認済み |
| ほかのツール | clangd・basedpyright・lldb-dap・perl・sh は PATH にない | 確認済み |
| VS Code | 入っている（code.cmd）。`%APPDATA%\Code\User\snippets` がある | 確認済み |
| Defender | リアルタイム保護は有効（Get-MpComputerStatus）。除外の一覧は管理者でないと読めない | 確認済み |
| Smart App Control | VerifiedAndReputablePolicyState = 0（無効） | 確認済み |
| git | `core.autocrlf=true`（`C:/Program Files/Git/etc/gitconfig`）。作業ツリーのテキストは CRLF（`git ls-files --eol`） | 確認済み |
| 開発者モード | レジストリの値は読めなかった。管理者でない利用者ではシンボリックリンクを作れず（「Administrator privilege required」）、ジャンクションは作れた（2026-10-29。段階 1 C4） | シンボリックリンク・ジャンクション：確認済み／開発者モードの設定：不明 |

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
| process.rs:146–167 | `wait4` の `ru_maxrss`（最大 RSS）。macOS はバイト、Linux はキロバイト（:157–159 の cfg） | 段階 2 S4：直接の子の PeakWorkingSetSize（K32GetProcessMemoryInfo）を、終わったあとに読む。pid で開き直さず、Job に入れるときに作ったプロセスのハンドルの写し（DuplicateHandle）で読む。終わったあとも、Job で止めたあとも読めた。Job の PeakProcessMemoryUsed（コミット量で、触っていないページも入る）は使わない（9.1 の追加 16） | 確認済み（テスト reports_max_memory_of_the_child・stops_a_program_over_the_memory_limit_while_running） |
| process.rs:70–77 | `proc_pid_rusage` の `ri_resident_size`（実行中の RSS。メモリの上限で止める。D-175） | 段階 2 S4：同じハンドルの写しで、WorkingSetSize を同じ間隔（20 ms）で読み、上限を超えたら Job ごと止める。Job の JOB_OBJECT_LIMIT_PROCESS_MEMORY は確保が失敗する形になり、今の「超えたら止める」と挙動が違うので使わない（推測）。WorkingSet は触ったページだけを数える（test_child の `alloc:50` で 3.8 → 53.8 MiB、`alloc:200` で 203.8 MiB。確認済み）。共有の DLL のページも入るので、小さなプログラムでも数 MiB になり、macOS より大きく出る（推測）。メモリが足りなくなると OS が WorkingSet を減らし、少なく出ることがある（推測） | 上限で止まること：確認済み／macOS・ジャッジとの差：推測 |
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
| commands.rs:1514–1519 | `app_config_dir()`（macOS は `~/Library/Application Support/<識別子>`） | Windows は `%APPDATA%\io.github.ishigaki892.acdstudio`（Roaming）。`npm run tauri dev` で起動して、instance.lock・settings.json・state.json ができた（段階 1 C8） | 確認済み |
| commands.rs:1713–1725、snippets.rs:67・100 | VS Code のスニペット `~/Library/Application Support/Code/User/snippets/` | `%APPDATA%\Code\User\snippets\`（この環境にフォルダがあることは確認済み） | 箇所：確認済み、フォルダも確認済み／代替案：未確認 |
| lsp.rs:147–159・265–306 | `file_uri`・`path_of_uri` が `/` で始まる Unix のパスを前提にする | `file:///C:/…` の形にし、ドライブ名の大文字・小文字と `%3A` の揺れを吸収する。src/editor/lsp.ts:301–315 の documentUri も合わせる | 箇所：確認済み／代替案：未確認 |
| workspace.rs:59–71 など canonicalize の各所 | canonicalize は Windows では `\\?\C:\…` を返す（推測） | dunce（Cargo.lock に 1.0.5 がある）で普通の形にしてから、外部ツール・URI・表示に渡す | 箇所：確認済み／代替案：未確認 |
| workspace.rs:448–463 | 大文字・小文字だけの名前の変更で、`MetadataExt::dev`・`ino` で同じものかを確かめる | canonicalize した結果どうしを比べる（新しい依存なし）か、same-file（Cargo.lock にある）。ハードリンクの判定には same-file の方が確か（推測） | 箇所：確認済み／代替案：未確認 |
| workspace.rs:479–486 | リンクは remove_file で消し、ほかは trash でゴミ箱へ | フォルダへのシンボリックリンク・ジャンクションは remove_dir で消す必要がある（推測）。trash は Windows に対応（推測） | 箇所：確認済み／代替案：未確認 |
| storage.rs:13・441・466・469・820 | アプリのフォルダを 0o700、ファイルを 0o600 で作る | Windows では cfg で外す（`%APPDATA%` は利用者だけの ACL。推測）。必要なら ACL を明示する | 箇所：確認済み／代替案：未確認 |
| storage.rs:814–828 | 単一起動のロック（`File::try_lock`） | Windows でも LockFileEx で動く（推測。2 つ起動して確かめる） | 箇所：確認済み／代替案：未確認 |
| storage.rs:448–473 | 一時ファイルに書いて rename で置き換える | Windows の rename は置き換えに対応（MoveFileExW。Rust 1.99.0 の fs::rename は、5 で失敗すると POSIX の決まりの置き換えも試す）。段階 1 C7 で、5・32 のとき 20 ms おきに 100 回試し直す形にした。置き換える先を、削除の共有を許さずに開かれていると、試し直しを使い切って「アクセスが拒否されました (os error 5)」で失敗する（段階 2 S10d のテストで確認）。全部のテストと一緒に同時の保存をくり返すと、50 回中 2 回、同じ形で失敗した（原因は未確認）。S10d で、同じパスへの保存を IDE の中で 1 つずつにし、諦めたときにほかのプログラムの可能性を案内する。試し直しの時間（約 2 秒）を延ばすかは、測った長さを見てから決める | 箇所・試し直し：確認済み／同時の保存の失敗の原因：未確認 |
| src/panels/logLinks.ts:11–13 | `src/main.rs:5:10` の形だけを場所として読む（`\` を含まない） | Windows の rustc の診断のパスは `examples\gen.rs` のように `\` 区切り（random.rs のテストで確認済み。段階 1 C7）→ `\` も区切りとして受け付ける。今のままだと `main.rs:5:10` だけに一致して、別のファイルへ移動しうる。エラー一覧に出す診断の file は、段階 2 S10a で `/` に直した（実行ログの本文は `\` のままなので、logLinks.ts は S10 で直す） | 箇所：確認済み／診断の file：確認済み（S10a）／logLinks.ts：未確認 |
| run.rs:1457–1466、workspace.rs:310–320 | 相対パスを `/` でつなぐ | Windows でも components を `/` でつないでいるので、そのままでよい（推測） | 箇所：確認済み／代替案：未確認 |

### 3.4 画面・メニュー・設定

| ファイル・行 | 内容 | Windows での代替案 | 確認状況 |
| --- | --- | --- | --- |
| src/keybindings.ts:86–117、storage.rs:81–120 | 既定のキー割り当てが meta（⌘）。両方に同じ表がある | Windows では meta を ctrl にした表を既定にする。両方の表を OS ごとに分け、storage.rs:1508 の重なりの検査を両方で動かす | 箇所：確認済み／代替案：未確認 |
| src/keybindings.ts:141–149 | 表示が `⌃⌥⇧⌘` の記号 | Windows は `Ctrl+Shift+R` の形 | 箇所：確認済み／代替案：未確認 |
| src/keybindings.ts:157–170・216–230 | `metaKey` を照合に使う。⌥ で文字が変わる macOS の対策 | Windows キー（metaKey）は OS に取られることが多い（推測）。AltGr（Ctrl＋Alt と同じに見える）の扱い | 箇所：確認済み／代替案：未確認 |
| src/settings/SettingsDialog.tsx:51–85 | 予約のキー（⌘Q・⌘H など）と、文言「macOS やメニューで使うキー」「⌘・⌃・⌥ のどれか」 | Windows の予約（Alt+F4・Ctrl+C/V/X/Z/A、Alt 単独でメニュー）に分ける | 箇所：確認済み／代替案：未確認 |
| 既定の割り当て全体 | Ctrl+R（実行）・Ctrl+W・Ctrl+F・Ctrl+P などが WebView2 のブラウザの既定動作（再読み込みなど）と重なる | WebView2 の AreBrowserAcceleratorKeysEnabled を false にする（9.1 の 5）。wry 0.57.0 に `with_browser_accelerator_keys` があるが、tauri 2.12.4・tauri-utils・tauri-runtime-wry からは設定できない（ソースで確認済み）ので、`with_webview` の `controller()`（webview2-com の型）から設定する。編集のキー（Ctrl+C/V など）が使えるままかは未確認 | 箇所・API：確認済み／動き：未確認 |
| App.tsx・RunButtons.tsx:102・inputMacro.ts・lsp.ts などのコメントと表示 | 「⌘.」「⌘Z」「Finder」「Keychain」の文字（SubmissionPanel.tsx:235、ipc/scaffold.ts:104 など） | 表示する文字は OS ごとに出し分ける。コメントはそのままでよい | 箇所：確認済み／代替案：未確認 |
| menu.rs:20–113 | アプリ名のメニュー（about・services・hide・hide_others）、fullscreen、CmdOrCtrl+Q | muda 0.20.0 の文書では services・hide_others・fullscreen などが「Windows: Unsupported」（ソースで確認済み）。Windows はメニューが窓の中に出るので、設定・終了を「ファイル」に移すなど並べ替える。Ctrl+Q は Windows の習慣ではない（Alt+F4） | 箇所：確認済み／代替案：未確認 |
| main.rs:1–2 | release だけ `windows_subsystem = "windows"` | そのままでよい。dev ではコンソールが出る | 確認済み |
| tauri.conf.json:47–61 | bundle.targets "all"、icon に icon.ico あり、macOS の署名の設定 | Windows では msi（WiX）と nsis が作られる（推測）。macOS の節は無視される（推測）。tauri-utils は `tauri.windows.conf.json` を主の設定にマージする（ソースで確認済み）ので、Windows だけ `targets: ["nsis"]` にする（9.1 の 15） | 箇所・マージ：確認済み／代替案：未確認 |
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
| ログインの窓の incognito | WebView2 で InPrivate になり、閉じたあと Cookie・履歴・キャッシュ・パスワードの自動保存がディスクに残らないか。wry 0.57.0 は incognito を WebView2 の `SetIsInPrivateModeEnabled` に渡す（webview2/mod.rs:414。ソースで確認済み）。InPrivate のときにディスクに何が残るかは未確認。パスワードの自動保存は WebView2 の IsPasswordAutosaveEnabled（既定は無効と推測）、フォームの自動入力は IsGeneralAutofillEnabled（既定は有効と推測）で、Tauri からどう設定されるかは不明 | 未確認（自動入力の設定は不明） | 下の「手順 A」 |
| パスワードが IDE に渡らない | src-tauri/src で password を含むのは security の副コマンド名だけ（session.rs:78・101・121・133。grep で確認済み）。IDE は cookies() から REVEL_SESSION だけを読む（session.rs:523–536） | コード：確認済み／Windows 版：未確認 | 下の「手順 B」 |
| ログアウト・期限切れで消える | logout（session.rs:431–435）で delete。ログインのページへ移されたら delete（session.rs:287–290）。Cookie が入れ替われば保存し直す（session.rs:281–285） | コード：確認済み／Windows 版：未確認 | 下の「手順 C」 |

手順 A（incognito の確かめ方）

1. WebView2 の利用者データのフォルダは `%LOCALAPPDATA%\io.github.ishigaki892.acdstudio\EBWebView`（段階 1 で起動したときにできた。確認済み）
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
| CPython・PyPy の Windows の標準出力 | `print` の `\n` が、パイプでは `\r\n` になった（どちらも `E3 81 82 0D 0A`。2026-10-31） | 確認済み |
| Rust の println! | 変えない（推測） | 未確認 |
| 比較：whitespace・float | compare.rs:150–153 の is_space が `\r` を空白として扱うので、CRLF でも WA にならない | 確認済み（コード） |
| 比較：exact | compare.rs の judge はバイトの完全一致だったので、CRLF の出力は LF の期待出力と一致しなかった（段階 2 S6 の直す前のテストで、`"1 2\n3\n"` と `"1 2\r\n3\r\n"` が Some(false)）。S6 で、Windows だけ、両方の `\r\n` を `\n` に直してから比べるようにした（`\r` だけのものは残す） | 確認済み（単体テスト。C++・Python の解答を exact で動かすのは S7・S8 の画面の確かめ） |
| 期待出力の保存 | 「実際の出力を期待出力として保存」（D-137、workspace.rs の write_expected）と、ランダムテストの失敗の保存（random.rs の save_case の random-N.out）は、出力をそのまま書いていたので、CRLF の `.out` ができた（S6 の直す前のテストで確認）。S6 で、Windows だけ、`\r\n` を `\n` に直してから書くようにした。random-N.in（生成器の出力）はそのまま残す | 確認済み（単体テスト） |
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
| GUI から起動した子 | IDE（dev）から起動した解答が起動した ping.exe は、日本語の部分を CP932 で出した。IDE は UTF-8 として表示するので、実行ログで日本語の部分だけが文字化けし、英数字は正しく出た（段階 2 S3 の画面の確かめ。2026-11-01）。直すかどうかは S7 か S13 で案を出してから決める | 確認済み（ping で。ほかのプログラムは未確認） |
| g++ の診断 | 本文は英語（MinGW-Builds は NLS なし。推測）。日本語を含むパスやソースの文字列は CP932 のバイトで出うる（推測）。process の出力は from_utf8_lossy などで読むので文字化けで済むが、logLinks の場所の照合が外れる | 未確認 |
| Python の `-E` | python.rs:489・504、debug.rs:283 で `-E` を付けている（確認済み）。`-E` は PYTHON で始まる環境変数を無視するので、**PYTHONUTF8・PYTHONIOENCODING を設定しても効かない**（-E の仕様は資料の記述で、未確認） | 付けていること：確認済み／効かないこと：未確認 |
| Python の標準入出力 | CPython はパイプのとき cp932 で出した（`あ` は `82 A0`）。cp932 で表せない文字（絵文字）は UnicodeEncodeError で終了コード 1（RE）。PyPy 3.11 は付けなくても utf-8 だった（2026-10-31。端末から起動した子で） | 確認済み（GUI から起動した子では未確認） |
| 案 | `-X utf8` を付ける（9.1 の 7）。CPython・PyPy とも `-E` と一緒に受け付け、`sys.flags.utf8_mode` が 1 になり、`あ` を UTF-8 で出した（2026-10-31） | 確認済み |
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
| Job に入れて再開するまでの時間（段階 2） | 実行時間は起動の前から数えるので、子を Job に入れて主スレッドを再開するまでの時間も入る。子 1 つで 53〜73 ms、同時に 30 個起動すると最大 1.7〜2.3 秒（段階 2 の表の S13 の行）。IDE は解答を 1 つずつ動かすので、ふつうは前者。言語サーバー・ビルドと重なったときにどれだけ延びるかは未確認 | 子 1 つ・30 個同時：確認済み（2026-11-03）／重なったとき：未確認 |

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
| Job Object との組み合わせ | CREATE_SUSPENDED・CREATE_NO_WINDOW を同時に付けて起動し、Job に入れて再開できた（段階 2 S3 のテスト）。画面の確かめ（2026-11-01）では、dev で解答が起動した ping の窓は出ず、npm と cargo の下で動く IDE から入れた Job が働いた（止める・TLE・子が自然に終わったあと・IDE をタスクの終了で止めたとき、孫まで消えた）。ただし IDE 自身が別の Job に入っていたかは見ていない | 確認済み（dev。IDE が別の Job に入っていたかは未確認） |
| dev での見え方 | dev は IDE にコンソールがあるので、子は同じコンソールを使い、窓は出ない（推測）。そのため dev だけでは気づけない | 未確認 |

### 4.7 実行中の exe の上書き

| 項目 | 内容 | 確認状況 |
| --- | --- | --- |
| 制約 | 動いている exe（孫が使っているものも）には書き込めない。段階 2 S3 の前の仮の実装（直接の子だけを止める）では、止めたあとも孫が使っていて、上書きが os error 32（「別のプロセスが使用中です」）で失敗した | 確認済み（テスト overwrites_the_program_right_after_stopping） |
| 起こること | 前の実行・デバッグ・止めたが終わりきっていない子が残ると、cargo のリンクや g++ の `-o target\main.exe` が失敗する（推測） | 未確認 |
| 止めることとの関係 | 段階 2 S3 で、kill_group は TerminateJobObject のあと、Job の ActiveProcesses（JOBOBJECT_BASIC_ACCOUNTING_INFORMATION）が 0 になるまで待ってから Job を閉じる。測ると、TerminateJobObject のあと 0.0〜0.2 ms で 0 になった | 確認済み（下の測定） |
| 止めたあとの短い間（測定。2026-10-31） | Job が空になり、直接の子の終わりも待ったあとでも、exe を上書きできないことがある。試しのプログラム（scratchpad。コミットしていない。IDE と同じ手順：CREATE_SUSPENDED → Job → 再開 → 孫 → 300 ms 動かす → TerminateJobObject → 空になるまで待つ → 子を待つ → すぐ `fs::write`）で、写してすぐ動かしたとき 40 回中 3 回と 40 回中 5 回、写してから 3 秒待って動かしたとき 30 回中 1 回、os error 32 になった。書けるようになるまで 9.7〜37.4 ms（9 回）。写しただけで動かさないときは 40 回とも書けた。失敗したときに Restart Manager（RmGetList）で調べると、使っているプロセスの一覧は毎回空だった。テスト全体を 20 回くり返したときは、overwrite のテスト（止めたあとすぐ 1 回だけ書く形）が 20 回中 7 回落ちた | 測った値：確認済み／原因（OS が exe の読み込みを少しの間解放しないのか、Restart Manager から見えない Defender などが開いているのか）：未確認 |
| 抜けた子孫 | macOS の「setsid でグループから抜けた子孫」（process.rs:10–13）にあたるものは、Job に JOB_OBJECT_LIMIT_BREAKAWAY_OK を付けない限り作れない（推測）。段階 2 S3 で、子が作った孫が、時間の上限・止める操作・子が自然に終わったあと・Job を閉じたときに止まることを確かめた | 孫が止まること：確認済み／抜けられないこと：推測 |
| Job を閉じたとき | KILL_ON_JOB_CLOSE で止まった子の終了コードは 0 だった（KILLED_EXIT_CODE ではない）。IDE が異常に終わったときにあたる | 確認済み（テスト closing_the_job_stops_the_child_and_descendants） |
| ほかの原因 | rust-analyzer の `cargo check` は別のファイルを作るので、普通は衝突しない（推測） | 未確認 |
| 確かめ方 | テスト overwrites_the_program_right_after_stopping：止めたあと 2 秒のうちに書けることを確かめ、かかった時間を失敗の文に出す（段階 2 S3）。画面では、孫を作って回り続ける解答を止めた直後にコメントを足してビルドし直すのを 5 回、できるだけ素早く行い、5 回ともエラーなくビルドできた（2026-11-01。dev） | テスト・画面：確認済み（画面の 5 回では失敗は起きなかった） |
| ビルドの前に待つ（段階 2 S3b） | Rust・C++・ランダムテストの example の出力の exe があれば、ビルドの前に、書き込みで開けるようになるまで待つ（全部で最大 2 秒。10 ms ごと。開いてすぐ閉じ、作らない・切り詰めない・書かないので、中身と更新日時は変わらない〔テストで確認〕。exe がなければ待たない）。Rust の exe の場所は `cargo metadata` の target_directory から求める。待っても開けない exe があれば、実行ログに「前の実行が終わっていない可能性があります（<名前> を書き換えられません）」を出してビルドは続け、ビルドが失敗したらもう一度出す（9.1 の追加 15）。開いて閉じることで Defender などの検査が起きて書けなくなることを心配し、待ったあと 1 回で書くテストを process::windows_tests ごと 20 回くり返したが、落ちなかった（待った時間は最大 10 ms。2026-11-01） | テスト：確認済み／画面：未確認 |

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
| ハンドルの継承 | 標準ライブラリ（Rust 1.99.0 の library/std/src/sys/process/windows.rs）は、`inherit_handles` の既定が true（193 行）で、CreateProcessW の bInheritHandles にそのまま渡す（329・416–421 行）。受け継ぐハンドルを絞る仕組み（PROC_THREAD_ATTRIBUTE_HANDLE_LIST）は使わない。子の標準入出力のハンドルは、起動の間だけロックして受け継げるようにする（334–345 行）ので、IDE が続けて起動する 2 つの子（対話テストの解答とジャッジ）が互いのパイプを受け継ぐことはない（ソースから。推測）。一方、子（解答）の中では、自分の標準入出力のハンドルが受け継げるままなので、子が Rust の Command で孫を起動すると、孫の標準入出力を空にしても、子の出力のパイプを孫が受け継ぐ。そのため孫が残っていると、子が終わってもパイプが閉じない（段階 2 S3 の前の仮の実装で、子が自然に終わったあと output_left_open が true になった。PowerShell から孫を作る test_child を動かすと、孫が終わるまで返らなかった）。Job で孫を止めればパイプも閉じる（S3 のテスト stops_descendants_left_after_exit で output_left_open が false）。IDE の側で受け継ぎを止める仕組みは足さない | ソースの記述・孫がパイプを持ち続けること・Job で閉じること：確認済み／2 つの子が互いに受け継がないこと：推測 |
| 終わり方 | シグナルがないので ExitKind::Signal（process.rs:393–409）は出ない。止めた側の終了コードは TerminateJobObject に渡した値になる | 推測（未確認） |
| CRLF | C++・Python のジャッジや解答は `\r\n` を出す（C++ は 4.3 で確認済み、Python は未確認）。やり取りの表示に `\r` が混じり、`\r` を考えていないジャッジ（利用者が書いたもの）が読み違えうる | 一部確認済み |
| バッファ | パイプへの出力は完全バッファ（推測）。flush を忘れたときに TLE になるのは macOS と同じ | 未確認 |
| ジャッジに渡す入力のパス | interactive.rs:217–219 で `<ケース>.in` のパスを引数で渡す。`\\?\` の形になるかは 4.8 | 未確認 |
| 確かめ方 | process.rs の run_pair のテストを、`/bin/sh` の代わりにテスト用の子のプログラムで書き直した（段階 2 S2・S3）。keeps_a_long_exchange_in_order（1000 往復）・finds_who_failed_first・stops_the_other_side_after_a_grace_period・stops_descendants_of_both_sides が Windows で通る。interactive.rs:1215 以降の Python のジャッジのテストはまだ | run_pair：確認済み／Python のジャッジ：未確認 |

### 4.10 パスの安全性（作業領域の外を触らないための検証）

**この節の 1 つ目の問題は、Windows 版でほかの機能より先に直す（段階 1）。**

| 項目 | 内容 | 確認状況 |
| --- | --- | --- |
| check_name（workspace.rs:302–308） | `/`・NUL・`.`・`..`・空だけを断る。`\` と `:` は見ていない | 確認済み（コード） |
| Windows の Path の動き | `\` も区切りとして扱う。`join` に接頭辞（`C:`）や根（`\`）のあるパスを渡すと、前の部分を置き換える。さらに `\\?\` の付いたパスに join すると `..` がその場で解決される（`\\?\C:\base\ws` + `..\escape.txt` → `\\?\C:\base\escape.txt`） | 確認済み（小さなプログラムで。段階 1 C5） |
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
| スタックの大きさ | MinGW の g++ で作った exe の既定の予約は 2 MiB（`objdump -p` の SizeOfStackReserve が 0x200000）。`-Wl,--stack,536870912` を付けると 0x20000000 になった。ジャッジのスタックの大きさは不明。深い再帰が手元だけで落ちる（0xC00000FD）おそれ → 手元のビルドで 1 GiB にする（9.1 の 9・追加 6） | 既定値・付け方：確認済み／ジャッジ：不明 |
| `-march=native`・`-fopenmp`・`-pthread`・`-static` | cpp.rs:22 の文のとおり、今も手元では付けていない | 確認済み（コード） |
| AC Library | ヘッダーだけなので動く見込み | 推測（未確認） |

### 5.2 Python

| 候補 | 入手方法 | ジャッジとの一致 | 確認状況 |
| --- | --- | --- | --- |
| CPython 3.13 | winget の `Python.Python.3.13` で `--version 3.13.7` を選べる（3.13.0〜3.13.16 がある。中身は python.org のインストーラー） | 3.13.7 で同じ（1 章）。ジャッジのパッケージ（numpy など、python.rs:294 以降の一覧）は pip で入れる。Windows 用の wheel がないものがあるかは未確認 | 入手と版：確認済み／パッケージ：未確認 |
| PyPy 3.11 v7.3.20 | pypy.org の Windows 64 ビットの zip（pypy3.11-v7.3.20-win64.zip）がある | 版が同じ（1 章） | 確認済み |
| 注意 | WindowsApps の 0 バイトの実行エイリアス（python.exe・python3.exe）は実体がない。探すときに除く。`python3.13.exe` という名前は Windows の公式版には入らない（python.exe・pythonw.exe だけ） | — | 確認済み |

### 5.3 Rust

| 項目 | 内容 | 確認状況 |
| --- | --- | --- |
| ジャッジの版 | `rustup toolchain install 1.89.0`（x86_64-pc-windows-msvc）。MSVC のリンカーは VS 18 BuildTools で足りる見込み | 未確認 |
| 主スレッドのスタック | rustc（x86_64-pc-windows-msvc）で作った exe の既定の予約は 1 MiB（SizeOfStackReserve が 0x100000）。深い再帰が手元だけで落ちうる → 手元のビルドで 1 GiB にする（9.1 の 9・追加 6・追加 7）。`cargo rustc --bin p -- -C link-arg=/STACK:1073741824` と `cargo rustc --example gen -- …` で、どちらも 0x40000000 になった。`cargo rustc` の `--` の後ろの引数は 1 つの対象にしか渡せない（`--bin` と `--example` を一緒に渡すと「extra arguments to `rustc` can only be passed to one target」で失敗） | 既定値・付け方・1 つの対象の制約：確認済み（2026-10-31。依存のないパッケージで） |
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

段階 1 は、次のまとまりごとに 1 つずつコミットする（2026-10-29 に決めた順。各コミットのあとに、番号・入れたファイル・テストの結果を報告してから次へ進む）。
Unix / macOS の経路のコードは、cfg を付ける・場所を移す以外はできるだけ変えない。共通の関数の引数の形は変えず、Windows 側に別の関数を用意する。
Unix 側の文字を変えたときは、そのコミットの報告に明記する。push は、段階 1 の最後に利用者が許可するまでしない。

| 順 | まとまり | 何を変えるか | どう確認するか |
| --- | --- | --- | --- |
| C1 | 文書 | この表と、9 章の「決めたこと」 | — |
| C2 | .gitattributes（9 章 6c） | `* text=auto eol=lf`。`*.bat`・`*.cmd`・`*.ps1` は `eol=crlf`（今は該当するファイルが 0 個。確認済み）。画像は `binary` | `git add --renormalize .` で索引が変わらない。`git checkout-index --all --force` の前に `git status --short` を見て、autogenerated の改行だけの差と .gitattributes 以外に変更があれば止める。書き直したあと `git status` が .gitattributes だけ |
| C3 | `cargo build` を通す（2.2 の 79 件） | libc を `cfg(unix)`、windows-sys 0.61 を `cfg(windows)` の依存に。process.rs は Unix の関数に `#[cfg(unix)]` を付けて中身を変えず、Windows の仮の実装を `src/process_windows.rs` に置く（直接の子だけを止める・終わりは WaitForSingleObject・メモリは測らない。Job Object は段階 2）。lsp.rs・debug.rs は `process::Pid`。storage.rs のパーミッション、workspace.rs の同じものの判定（Windows は GetFileInformationByHandle のボリュームとファイルの番号）、random.rs の OwnedHandle、submit.rs の時刻（SystemTimeToTzSpecificLocalTime） | `cargo build`・`cargo clippy` がエラー・警告なし |
| C4 | `cargo test --no-run` を通す（テストの 33 件） | Unix にしかないものを使うテストに `#[cfg(unix)]`（中身は変えない）。process.rs の仮の実装のテストを `cmd /c` で足す | `cargo test --no-run`・`cargo clippy --all-targets` |
| C4b | CI | `.github/workflows/ci.yml`：macos-latest・windows-latest で `npm ci`・`npm run build`・`cargo build`・`cargo clippy --all-targets`・`cargo test`。windows-port への push と workflow_dispatch で動く | push を許可してもらったあとで結果を見る（それまでは未確認） |
| C5 | 4.10 パスの検証 | Windows のテストを先に書いて失敗を確かめてから、check_name と check_relative の各要素で `\`・`:`・`*?"<>|`・制御文字・末尾の `.` と空白・予約名を断る（Windows だけ。9 章 13）。テストと修正は 1 つのコミットにし、直す前の失敗をメッセージに書く | 直す前に失敗し、直したあとに通る。外のファイル・フォルダができていない |
| C6 | 外部コマンドの作り方・HOME・`.exe` | `process::command` で CREATE_NO_WINDOW を付ける（本番のコードの 26 か所）。`toolchain::home_dir()`（Windows は `std::env::home_dir()`）。cargo・rustc・rust-analyzer の名前に `EXE_SUFFIX` | 該当のテスト。窓が出ないことは release で利用者に見てもらう |
| C7 | `cargo test` を通す | 失敗を分類し、Unix 専用は `#[cfg(unix)]`、後の段階で直すものは `#[cfg_attr(windows, ignore = "…段階 N…")]`、段階 1 の不具合は直す | `cargo test` が Windows で通る |
| C8 | メニュー（9 章 16） | macOS の今のメニューは `#[cfg(target_os = "macos")]` のまま。Windows は「ファイル」（フォルダを開く・タブを閉じる・言語サーバーを再起動・設定・終了）と「編集」だけ | dev でメニューが出て動く |
| C9 | キー割り当て・ブラウザの既定のキー（9 章 5） | macOS 以外は既定の割り当ての ⌘ を Ctrl に（storage.rs と keybindings.ts）。表示は `Ctrl+Shift+R` の形。設定の画面の予約のキー。WebView2 のブラウザの既定のキーは、webview2-com で SetAreBrowserAcceleratorKeysEnabled(false) | `npm run build`。dev で Ctrl+S が保存、Ctrl+R が再読み込みにならない、Ctrl+C/V が使える |
| C10 | バンドル（9 章 15）・まとめ | `src-tauri/tauri.windows.conf.json` で Windows だけ `targets: ["nsis"]`（macOS は "all" のまま）。署名はしない。この文書に段階 1 の結果を書く | `npm run tauri build` で nsis のインストーラーだけができる |

段階 1 の全体の確認
- Windows で `cargo build`・`cargo test`・`cargo clippy --all-targets` がエラー・警告なし、`npm run build` が通る
- `npm run tauri dev` で起動し、`%APPDATA%\io.github.ishigaki892.acdstudio` に instance.lock ができる（app_config_dir の場所の確認）
- 画面の操作は利用者が確かめる：フォルダを開く・ファイル一覧・編集・Ctrl+S で保存・メニュー・Ctrl+R で再読み込みしない・Ctrl+C/V・設定の画面のキーの表示・
  2 つ目の起動が断られる・release の exe でコンソールの窓が出ない
- macOS：`cargo check --target aarch64-apple-darwin --all-targets` を試す（依存の C のビルドで失敗すれば未確認）。本当の確認は CI の macos-latest

### 段階 1 の結果（2026-10-30）

コミット（windows-port ブランチ。2026-10-30 に利用者の許可を得て origin/windows-port に push した）

| 順 | コミット | 入れたもの | 確かめたこと |
| --- | --- | --- | --- |
| C1 | 8ebec58 | この文書（9.1 と段階 1 の表） | — |
| C2 | ba5de4d | .gitattributes | 索引は変わらず、作業ツリーの CRLF は 0 個。tauri-build が作るファイルの「改行だけの差」も消えた |
| C3 | 0583e30 | Windows で cargo build が通る（win.rs を新しく作った） | cargo build・cargo clippy が警告なし |
| C4 | 070e78d | Windows で cargo test --no-run が通る | cargo clippy --all-targets が警告なし。Windows の仮の実装のテスト 11 件が通る |
| C4b | 01ec1d2・1f9e681・866d3a4・12ba8a3・a313c53 | .github/workflows/ci.yml | push したあと（2026-10-30）、windows-latest は最初から通った。macos-latest は debug::tests::debugs_with_real_lldb_dap だけが失敗した（289 件が通り、失敗 1）。直したあと、a313c53 の CI（run 41023624709）で macos-latest・windows-latest ともすべての段階が通った（このテストは飛ばした）。下の「CI の macos-latest の失敗」 |
| C5 | 4309b65 | 4.10 のパスの検証（テストと修正） | 直す前に 5 件中 4 件が失敗し、直したあと workspace のテスト 18 件が通る |
| C6 | b77689f | process::command（CREATE_NO_WINDOW）・home_dir・exe_name | 該当のテストが通る |
| C7 | a13907a | Windows で cargo test が通る（同時の保存の再試行、確認の画面のパスの表示を直した） | 226 件が通り、25 件が ignore、失敗 0 |
| C8 | 7aeb134 | Windows のメニュー（「ファイル」「編集」） | dev で起動。「ファイル」「編集」が表示されることは利用者が画面で確認（コミットのメッセージには「未確認」と書いたが、そのあと確認された） |
| C9 | 31b051f | キー割り当てを Ctrl に、WebView2 のブラウザの既定のキーを切る | npm run build が通る。新しい settings.json は meta 0 件・ctrl 11 件（⌘ だった 11 件がすべて Ctrl に。残り 8 件はもともと ⌘ を使わない F キーなど） |
| C10 | f661fe5 | src-tauri/tauri.windows.conf.json、この節 | npm run tauri build（8 分 9 秒）で bundle/nsis/AtCoder Studio_0.1.0_x64-setup.exe（3,877,393 バイト）だけができた。NSIS 3.13 と nsis_tauri_utils v0.5.3 は tauri-cli が GitHub から取ってきた |

Windows で飛ばしているテスト（`#[cfg_attr(windows, ignore = "…")]`。理由に直す段階を書いた。25 件）

| 段階 | 件数 | テスト | 理由 |
| --- | --- | --- | --- |
| 2 | 3 | interactive::tests_interactively_with_real_programs、random::finds_differences_with_real_programs、scaffold::cargo_reads_created_packages_under_a_parent_workspace | 使用メモリを測らない（段階 2 S4 で測るようにしたあとも、rustc の診断のパスで落ちるので、S10a に回した）／rustc の診断のパスが `\` 区切り（S10a）／cargo の出力のパスと canonicalize の `\\?\` の違い（S10）。random は段階 2 S10a で ignore を外した。interactive は、S10a で診断のパスは直ったが、先に失敗した側の取り違えが見つかったので、S10b に回し、S10b で ignore を外した |
| 3・5 | 13 | session の 4 件、problem::reads_with_the_session_only_after_a_404_when_logged_in、submit の 8 件 | /usr/bin/security（Keychain）と /usr/bin/curl |
| 4 | 4 | sources の 3 件、lsp::overrides_initialize_and_answers_configuration | file: の URI・`\\?\`・パスの区切り |
| 6 | 5 | debug の 5 件 | lldb-dap（デバッグは今回の移植に含めない） |

Unix 専用にしたテスト（`#[cfg(unix)]`）：シンボリックリンク・Unix のパーミッション・/bin/sh・perl・kill・/bin/cat を使うもの（関数ごとに 34 件と、process.rs の sh のテストの mod 全体）。
workspace.rs の 4 件と storage.rs の 3 件は、リンク・パーミッションの文だけを Unix に限り、ほかは Windows でも動かしている。Windows で同じことを確かめるテストは、後の段階で足す。

Unix 側を変えた箇所（cfg・属性を付ける以外の文字の変更。どれも Unix では同じ値・同じ動きのはず。CI の macos-latest で cargo build・cargo clippy --all-targets・cargo test が通った〔run 41023624709。debugs_with_real_lldb_dap は飛ばした〕。手元の Mac での画面の操作は未確認）

- `child.id() as libc::pid_t` → `as Pid`（10 か所）と、lsp.rs・debug.rs の `use` に Pid
- 外部コマンドの 11 か所の `Command::new` → `crate::process::command`
- HOME を読む 4 か所 → `toolchain::home_dir()`、cargo・rustc・rust-analyzer の名前 → `exe_name(...)`（find_cargo_in のテストも）
- storage.rs の write_atomic の最後を `#[cfg(unix)] return fs::rename(&tmp, path);` に
- trust.rs の作業領域の中の表示を `display_inside`（Unix は同じ `to_string_lossy`）に
- keybindings.ts の既定の表を `forPlatform([...])` で包んだ。SettingsDialog の文言 2 つ、RunButtons のヒント 2 つを OS ごとに（macOS では同じ文字）
- lsp.rs の `#[cfg(test)]` 3 か所と process.rs の 1 か所を `#[cfg(all(test, unix))]` に

段階 1 で新しく確認済みになったこと

- Rust の `Path::join` は、`\\?\` の付いたパスで `..` をその場で解決する（4.10。小さなプログラムで確認）。直す前は、新しいファイル・フォルダ・名前の変更・削除の対象で作業領域の外に出られた
- アプリのフォルダは `%APPDATA%\io.github.ishigaki892.acdstudio`、WebView2 の利用者データは `%LOCALAPPDATA%\io.github.ishigaki892.acdstudio\EBWebView`
- 起動したときに作られる settings.json には、キー割り当ての全体が書かれる
- rustc の診断のパスは `\` 区切り
- Windows の名前の変更は、同じファイルを同時に置き換えていると失敗する（アクセス拒否・共有違反）。少し待って試し直せば通る
- この機械では、管理者でないとシンボリックリンクを作れない（ジャンクションは作れる）
- macOS 向けの `cargo check --target aarch64-apple-darwin` は、この機械では依存の objc2-exception-helper の C のビルド（MinGW の cc が使われる）で失敗するので、Unix の経路のコンパイルはここでは確かめられない

CI の macos-latest の失敗（2026-10-30。どれも debugs_with_real_lldb_dap。ログは利用者が画面から写した）

| run | 失敗した場所 | 原因と対処 |
| --- | --- | --- |
| 40971221506（f661fe5） | debug.rs:2188。標準ライブラリのフレームのパスが `/rustc/58cf8f9…` のまま | rust-src がない。debug.rs:394–403 は rust-src があるときだけ target.source-map を設定する → 1f9e681 で CI だけ rust-src を入れた |
| 41020045065（1f9e681） | 同じ debug.rs:2188。パスは `/rustc/b940084…` | テストのパッケージは一時フォルダ（debug.rs:1698–1706）にあって rust-toolchain.toml の外にあり、cargo_command は RUSTUP_TOOLCHAIN を外す（D-104）ので、rustup の既定でビルドされる。b940084 は 1.99.0 のコミット（手元で確認済み）なので版は同じで、既定は別の名前（stable と推測）のフォルダ。rust-src は 1.99.0 のフォルダにだけ入っていた → 866d3a4 で rust-toolchain.toml の版を既定にした |
| 41021999618（866d3a4） | debug.rs:2221。ステップアウトのあとも一番上のフレームが `<alloc::vec::Vec<i64>>::as_slice` で、`solution::main` に戻らない（2188 は通った） | 原因は分かっていない（下の未確認のこと）→ 12ba8a3 で、CI では `--skip debug::tests::debugs_with_real_lldb_dap` で飛ばす |
| 41023624709（a313c53） | なし。macos-latest・windows-latest ともすべての段階が成功（GitHub の API で確認） | — |

画面の操作の確認（2026-10-30。利用者が Windows 11 で確かめた。1〜5 は dev〔npm run tauri dev〕、6・7 は release〔src-tauri\target\release\atcoder-studio.exe、f661fe5 でビルド。そのあとのコミットは ci.yml と文書だけ〕。
試しのフォルダは `cargo new` の Rust のパッケージに `fn main() {}` の build.rs を足したもの。空の build.rs は main がなくビルドが失敗する）

| 番号 | 確かめたこと | 結果 |
| --- | --- | --- |
| 1 | メニューの各項目（フォルダを開く・タブを閉じる・言語サーバーを再起動・設定・終了、編集の各項目） | OK。言語サーバーの再起動は「Unknown binary 'rust-analyzer.exe' in official toolchain 'stable-x86_64-pc-windows-msvc'」の知らせ（rust-analyzer の component が入っていないため。探し方と案内は段階 4） |
| 2 | Ctrl+S・Ctrl+R（再読み込みしない）・Ctrl+Shift+R・Ctrl+C/V/X/Z/A・Ctrl+B・Ctrl+W | OK。保存の印は Ctrl+S の前に自動保存で消えたので、Ctrl+S そのものの保存は自動保存と区別できていない |
| 3 | ブラウザの既定のキー（F5・Ctrl+F・Ctrl+P・Ctrl+Shift+I・F12） | F5・Ctrl+F・Ctrl+P は OK。release では Ctrl+Shift+I・F12 とも DevTools は開かない。dev では Ctrl+Shift+I で DevTools が開いた（下の注） |
| 4 | 設定の画面のキーの表示が `Ctrl+…` の形 | OK |
| 5 | フォルダを開いたときの確認の画面のパスの表示 | OK（`build.rs` とだけ表示。`\\?\` はなし） |
| 6 | release で、フォルダを開く・信頼・実行・サンプルテスト・言語サーバーの再起動のときにコンソールの窓が出ない | OK |
| 7 | 2 つ目の起動が断られる | OK |

注（3 の dev の Ctrl+Shift+I）：Tauri は debug のビルド（`debug_assertions`、または tauri の `devtools` feature。このプロジェクトは付けていない）のときだけ、
Ctrl+Shift+I の keydown で DevTools を開く JS を入れる（tauri-2.12.4 の src/webview/plugin.rs:204–222、scripts/toggle-devtools.js。ソースで確認済み）。
WebView2 のブラウザの既定のキーとは別の仕組みなので、SetAreBrowserAcceleratorKeysEnabled(false) では止まらない。release ではこの JS が入らず、
wry が SetAreDevToolsEnabled に devtools の設定（release では false）を渡す（wry-0.57.0 の webview2/mod.rs:640）ので開かない。不具合ではなく、直さない

段階 1 のあとも未確認のこと

- Ctrl+S で保存されること（自動保存と区別して確かめる。自動保存を切るか、保存の前に書き換えてすぐ Ctrl+S を押す）
- macOS の clippy の警告の有無：ci.yml の `cargo clippy --all-targets` は `-D warnings` を付けていないので、警告があっても段階は成功になる。ログは認証しないと読めず、見ていない
- macOS のテストの件数（通った件数・飛ばした件数）：run 41023624709 のログを見ていない
- **debug::tests::debugs_with_real_lldb_dap（CI では飛ばしている）**：macos-latest で、ステップイン（debug.rs:2215）のあとのステップアウト（:2219）で main に戻らず、
  一番上のフレームが `<alloc::vec::Vec<i64>>::as_slice` のまま（:2221 の `assert_eq!` が失敗）。ステップインでどの関数に入ったかは、テストが `assert_ne!` しか見ないのでログにない。
  考えられる原因（どれも推測）：1.99.0 の標準ライブラリや runner の Xcode の lldb-dap で、ステップイン・アウトの止まる場所が違う／
  wait_stopped がステップインの 2 つ目の stopped を拾い、ステップアウトが終わる前にスタックを読んだ。段階 1 の debug.rs の変更は型の名前・cfg・Windows の関数の追加だけで、Unix の動きは変えていない（git diff main で確認）。
  段階 6 か、Mac の手元（rustup の既定を 1.99.0 にして）で `cargo test debugs_with_real_lldb_dap` を動かして確かめる。確かめたら ci.yml の `--skip` を外す
- インストーラー（nsis）で入れた exe での動き（画面の確認は target\release の exe で行った）

既知の問題（まだ直さない。9.1 の追加の判断）

- 前の版が書いた settings.json に ⌘（meta）の割り当てが残っていると、Windows ではそのキーを押せない。Windows 版はまだ配っていないので、今は対象の利用者がいない
- 仮の実装の限界：止めるのは直接の子だけで子孫は残る、使用メモリを測らない（MLE にならない）。段階 2 で Job Object にする
- **ランダムテストの「止める」の競合（段階 1 より前からある。random.rs と process.rs の Unix の経路は段階 1 で変えていない〔git diff main で確認〕）**：
  CI の macos-latest で random::tests::finds_differences_with_real_programs が、ときどき random.rs:997 で
  `Failed { message: "生成器が失敗しました（seed 107）：出力の全体を得られませんでした" }` になって失敗する（run 41060782033、4cdcf06。
  文書だけのコミットで、同じコードの a313c53 の CI は通っている）。流れ（コードで確認。どちらが先に起きるかは runner の混み具合による）：
  1. テストの 7（:982–1001）は、3 回目（seed 107）の RandomProgress の知らせで止める印を立てるので、生成器を起動する前に印が立っている
  2. process::run（process.rs:376–410）は、子が動いている間だけ止める印を Stopped にする（:394）。生成器はすぐ終わるので、終わりを先に知ると Exited(0)
  3. 残りの出力を読むループ（:422）は、止める印があればすぐやめる。標準出力・標準エラーが閉じきっていなければ output_left_open が true（:451）
  4. run_helper（random.rs:134–135）は Exited(0) で output_left_open のとき「出力の全体を得られませんでした」の失敗にする
  
  利用者への影響（推測）：生成器・愚直解が終わるのと同時に止めると「止めました」ではなく「生成器（愚直解）が失敗しました」と出る。
  解答も同じで、run.rs:1239 で OutputIncomplete になり、ランダムテストの失敗として random-N に保存されうる。
  直し方（2026-10-30 に決めた）：段階 1 の PR とは分け、merge のあと段階 2 の前に、random.rs だけで直す（run_helper と解答の判定で、
  止める印が立っていて output_left_open なら止めたことにする）。process.rs の Unix の経路は変えない。CI ではこのテストを飛ばさず、
  たまたま落ちたら GitHub の「Re-run failed jobs」でやり直す
  
  → 直した（2026-10-30、ブランチ fix-random-stop-race）。random.rs に stopped_while_reading を足し、生成器・愚直解の結果の分け方を
  helper_of に分けて、そこと解答の判定（run_random）で使う。テスト treats_output_cut_by_stop_as_stopped は、結果を作って確かめる
  （実際のプログラムでは起きるかどうかが混み具合による）。直す前は、このテストの最初の確認（Stopped になる）が失敗した。
  同じブランチで、ci.yml をどのブランチの push でも、PR でも動くようにした（今までは windows-port への push だけ）

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

段階 2 は、次のまとまりごとに 1 つずつコミットする（2026-10-31 に決めた順。ブランチは最新の main から作った windows-port-stage2）。
進め方は段階 1 と同じ：Unix / macOS の経路のコードは cfg を付ける・場所を移す以外は変えず、変えたときはそのコミットの報告に書く。
各コミットの前に cargo fmt・cargo clippy --all-targets（警告なし）・cargo test、画面のコードを変えたときは npm run build を通す。
不具合を直すときは、先にテストを書いて失敗を確かめ、テストと修正を 1 つのコミットにして、直す前の失敗をメッセージに書く。push と PR は利用者が許可してから

| 順 | まとまり | 何を変えるか | どう確認するか |
| --- | --- | --- | --- |
| S1 | 文書 | 1 章の Python の行、4.3・4.4・5 章の確かめたこと、この表、9.1 の追加 6〜14 | — |
| S2 | テスト用の子のプログラム（9.1 の追加 8・14） | `src-tauri/examples/test_child.rs`：引数で動きを選ぶ（出力・終了コード・標準入力をそのまま返す・待つ・回り続ける・孫を作る・メモリを使う・落ちる〔アクセス違反・0 除算・スタックあふれ・abort・panic〕・対話の相手）。process.rs の Windows のテスト（段階 1 の `cmd /c` のもの）をこれで書き直し、run.rs の `/bin/cat` のテスト（judges_cases_with_a_real_program）の Windows 版を足す。**S2 の直し（eeef85e）**：はじめは `target/debug/examples/` の exe を使ったが、絞り込みを付けた cargo test が作り直さず古いものが使われたので、テストの中で rustc で直接コンパイルし、ソースのハッシュの名前で `target/debug/test-child/` に置く形にした | 今の仮の実装で通るものだけを入れる。`cargo test` |
| S3 | Job Object（9.1 の 1・追加 4・追加 9・追加 10） | win.rs の中身を差し替える。`process_group(0)` の代わりの仕組みで CREATE_SUSPENDED（と CREATE_NO_WINDOW）を付け、spawn の直後に `#[cfg(windows)]` の 1 行で Job に入れる（KILL_ON_JOB_CLOSE）。主スレッドは Toolhelp32 で探して再開する。spawn の直後の 1 行を足すのは process.rs の 3 か所・lsp.rs・debug.rs。kill_group は TerminateJobObject（終了コードは追加 10 の値）のあと、Job の動いているプロセスが 0 になるまで待つ（4.7）。Job に入れていない pid を kill_group・reap に渡したら debug_assert で知らせる。process.rs の関数の形は変えない。test_child の孫は `grandchild:印のファイル` で、印を排他で開いたまま最大 30 秒生きる（テストは pid ではなく印で、孫が生きている・終わったを見る） | 先にテストを書いて失敗を確かめる：時間の上限・止める操作で孫も止まる／子が自然に終わったあとで孫が残らない／Job を閉じると子が消える／止めたあと 2 秒のうちに exe を上書きできる（1 回目で書けるとは限らない。4.7）／Job に入れられなかったら子を止めてエラーにする |
| S3b | 止めたあとのビルドの前に待つ（4.7・9.1 の追加 15） | win::wait_until_writable：書き込みで開けるようになるまで待つ（開いてすぐ閉じる。中身を変えない。exe がなければ待たない）。run.rs の Rust・C++ のビルドの前に、Windows だけで、Rust・C++・example の出力の exe を全部で最大 2 秒待つ。開けなければ実行ログに「前の実行が終わっていない可能性があります」を出してビルドは続け、失敗したらもう一度出す。Rust の exe の場所は toolchain::built_executables（`cargo metadata` の target_directory）、C++ は cpp::output_path に `.exe` を足す。Windows だけで使う関数には `#[cfg(windows)]` を付ける。process.rs の上書きのテストは、待ってから 1 回で書く形にする | 先にテストを書いて失敗を確かめる：開いている・動いている exe で待って false、閉じる・止めると true、中身と更新日時が変わらない、場所の求め方、案内の文。上書きのテストを process::windows_tests ごと 20 回。画面で、止める→すぐビルドと、exe を開いたままビルドして案内が出ること |
| S4 | メモリ（9.1 の追加 16） | current_rss は WorkingSetSize、最大使用メモリは終わったあとの PeakWorkingSetSize（直接の子だけ。Unix と同じ）。どちらも、Job に入れるときに作ったプロセスのハンドルの写しで読む（pid で開き直さない）。test_child の `alloc:` が全部のページに書くようにする（最後のバイトにも書く）。interactive::tests_interactively_with_real_programs は ignore を外して動かし、1〜9 の確かめ（6 の MLE・OLE を含む）は通ったが、最後の 10 で rustc の診断のパスの `\` で落ちたので、ignore の理由を S10a に書き換えて残す | 先にテスト（実行中に上限で止まる・最大使用メモリが出る・対話の解答の最大使用メモリ・run.rs の memory_kib）を書いて失敗を確かめる。process::windows_tests を 20 回くり返す |
| S10a | rustc の診断のパスの区切り（S4 のすぐあと。S5 の前） | Windows だけ：run.rs の parse_build_output で、rustc の診断の file の `\` を `/` に直す（Unix では `\` もファイル名に使えるので直さない）。`\\?\` の付いたパスは、そのまま直すと `//?/C:/…` になるので、`\\?\` を外してから直す（`\\?\UNC\server\share\…` は `\\server\share\…` にしてから `//server/share/…`）。実行ログの診断の本文（rendered）は `\` のまま（S10 の logLinks.ts で扱う）。random::finds_differences_with_real_programs の ignore を外す。interactive::tests_interactively_with_real_programs は、診断のパスは直り 4 件だけでは通るが、全部のテストと一緒のとき 6 回中 1 回、解答の panic が WA になったので、ignore の理由を S10b に書き換えて残す | 先にテストを書いて失敗を確かめる（Windows：`\`・絶対パス・`\\?\`・`\\?\UNC\` の例。Unix：`\` がそのまま残る）。外したテストが通る |
| S10b | 対話テストの、先に失敗した側の取り違え（S10a のすぐあと） | run_pair は、先に失敗した側を、終わりの知らせが IDE に届いた順で決めていた。両方が異常に終わったとき、2 つの終わりが数ミリ秒の間に重なると、原因（解答）と結果（ジャッジ）を取り違えうる（Windows で全部のテストと一緒に動かしたときに起きた。Unix にも同じ作りがある）。中継のスレッドが EOF を読んだら、相手の標準入力を閉じる前に「この側の出力が閉じた」と知らせ（PairEvent::OutputClosed）、両方がこちらが止める前に異常に終わったときは、出力が先に閉じた側を先に失敗した側とする（止めたあとの終わりは数えない）。OS で分けない（**Unix の動きも変わる**）。test_child に `close-stdout` を足す。interactive::tests_interactively_with_real_programs の ignore を外す。**S10b の直し（2026-11-02）**：出力が閉じた知らせが、プロセスの終わりより遅れる場合（孫が標準出力を持ち続けるなど）に、終わった順と逆の側を先としていたので、決め直すのは、先に閉じた側の閉じた知らせが、相手の終わりの知らせより先に届いていたときだけにした（**Unix の動きも変わる**）。S10b のテストは、時間（200 ms）ではなく印のファイルで順番を作る形に書き直した（test_child に `touch:`・`wait-file:` を足した）。新しいテスト keeps_the_exit_order_when_output_closes_late（Windows と Unix）は、直す前に 5 回とも Some(Judge) で失敗した | test_child で取り違えを毎回起こす：解答は標準出力を閉じてから 200 ms 待って 101 で終わり、ジャッジは EOF を読んだらすぐ異常に終わる（終わった順ではジャッジが先、原因の順では解答が先）。今の実装で毎回失敗することを確かめてから直す |
| S5 | RE の表示（追加 10・追加 20） | run.rs の exit_code_text：i32 で負の値（Windows の例外のコード）は 16 進と名前（0xC0000005 アクセス違反・0xC0000094 整数の 0 除算・0xC00000FD スタックあふれ・0xC0000409 異常終了・0xC0000135 DLL が見つからない・0xC000001D 不正な命令・0xC0000374 ヒープの破損・0xE0AC0001 IDE が止めました）。表にない負の値は 16 進だけ。負でない値は 256 以上も 10 進のまま。S5 では終了コード 3 に「C++ の abort・assert でも 3 になります」を添えたが、測ると C++ の abort・assert・捕まえない例外は 0xC0000409 だったので、S5b でやめた。名前は Rust で作り、RunOutcome::Exited の label で画面に送る（runLog.ts・TestResults.tsx は label を出す。前の版の実行履歴は label がないので今までの形）。interactive.rs（ジャッジの終わり方の補足）と random.rs（生成器・愚直解の失敗）も同じ関数を使う。Unix の表示は変わらない（0〜255 は今と同じ文字） | 先にテストを書いて失敗を確かめる（名前・16 進／256 以上は 10 進／3 の添え書きは Windows だけ）。test_child で本当に落として終了コードを確かめる：アクセス違反 0xC0000005・0 除算 0xC0000094・スタックあふれ 0xC00000FD・Rust の abort 0xC0000409・panic 101（2026-11-02 に確認） |
| S5b | 落ちた解答が終わるまでの時間（9.1 の追加 21） | 画面の確かめ（2026-11-02）で、アクセス違反（3993 ms）・スタックあふれ（3787 ms）の解答が、サンプルテストで RE ではなく TLE（約 2.5 秒）になった。測ると（scratchpad の試しのプログラム。test_child と、この機械の g++ 15.2.0〔MinGW-Builds・UCRT〕で -O2 にした C++）、扱われない例外（0xC0000005・0xC0000094・0xC00000FD）と __fastfail（0xC0000409）は、Job があってもなくても 3.2〜4.8 秒かかり、落ちるたびに WerFault.exe が 2 つ起動した（1 つは落ちた解答が親、1 つは WerSvc が親）。C++ の abort・assert・捕まえない例外（vector の .at() の範囲外）は 3 ではなく 0xC0000409 だった。Job に JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION を足すと、扱われない例外は 69〜114 ms で終わり、WerFault.exe は起動せず、終了コードは変わらなかった。0xC0000409 は DIE を足しても 3.3〜4.3 秒のまま。落ちた解答が親の WerFault.exe を止めると、0xC0000409 も 131〜160 ms で終わり、終了コードは 0xC0000409 のまま（その WerFault.exe は Job の外にあった）。S5b では DIE を足し、0xC0000409 の名前を「異常終了：C++ の abort・assert・捕まえない例外、Rust の abort など」にし、終了コード 3 の添え書きをやめ、実行ログで名前のあとの空白をなくす | 先にテストを書いて失敗を確かめる：サンプルテストの既定の上限（2500 ms）で、アクセス違反・0 除算・スタックあふれが TLE ではなく例外のコードで終わる（process.rs）／既定の制限のサンプルテストで、アクセス違反が RE になる（run.rs）／0xC0000409 の名前と 3 の扱い。はじめは上限 1 秒で書いたが、全部のテストと一緒だと DIE ありでも落ちた。調べると（2026-11-03）、WER は起動しておらず、全部のテストが一斉に子を起動する混み合いで、子を Job に入れて再開するまでに約 2 秒かかっていた（下の S13 の行）ので、既定の 2500 ms にした（WER の遅れの最小 3212 ms より短い）。DIE を一時的に外すと、2500 ms でも 2 件とも TimedOut で落ちることを確かめた |
| S5c | 0xC0000409 の WER の待ち（S5b のすぐあと。9.1 の追加 21） | __fastfail（0xC0000409。C++ の abort・assert・捕まえない例外、Rust の abort）は DIE では速くならず、約 3.3〜4.3 秒かかって TLE になる。落ちた解答（直接の子）が親の WerFault.exe を止めると、131〜160 ms で終わり、終了コードは 0xC0000409 のままだった（試しのプログラムで確認。その WerFault.exe は Job の外にあった）。IDE が、実行中に、親が直接の子の WerFault.exe を探して止める（win::CrashReporterWatch）。止めるのは、名前が WerFault.exe で、親の pid が直接の子で、子より後に作られたものだけ。WerSvc（サービス）が起動した WerFault.exe は親が違うので止めない。探すのは時間の上限がある実行（解答・ジャッジ・生成器・愚直解）だけ。見張りは IDE 全体で 1 つのスレッドで、子は始まると登録し、直接の子が終わると解除する。見張りのスレッドは、最初の登録で作られ、登録がある間だけ 100 ms ごとに、登録から 100 ms たった子について、プロセスの一覧を 1 回だけ取って探す（一覧を取るのに中央値 8.3 ms。スレッドの一覧は 46.8 ms。2026-11-03 に測定。WerFault.exe は早くても子が始まってから約 120 ms で起動する）。登録がなくなったら、次の登録まで眠る（スレッドは終わらせない）。はじめは子ごとに、待つ間 100 ms ごとに一覧を取っていたが、作り直しありで cargo test 全体を 5 回ずつ動かすと、探すと 1 回（stops_both_on_request）、探さないと 0 回落ちたので、1 つのスレッドにまとめた（子が同時に多いとき、一覧を取る処理が重なって混み合うと推測）。ビルド・構文の確かめ・言語サーバーは対象にしない。C++ の _set_abort_behavior は使わない。WER の後始末が、同時に動くほかの落ち方を遅らせることはなかった（0xC0000409 のテストを含めて 3 件だけ同時に動かすと通り、除いた全体で落ちていた。原因は一斉の起動の混み合いだった）。止めたあとも、0xC0000409 で落ちるたびに ReportArchive に記録が 1 つ残る（下の「段階 2 の既知の問題」）。C++ の画面の確かめは S8 のあと | 先にテストを書いて失敗を確かめる：止める WerFault.exe の選び方（単体テスト）／既定の上限で、crash:abort が TLE ではなく 0xC0000409 で終わる。直す前はそれぞれ空の一覧・TimedOut で失敗した |
| S5d | 実行時間を数え始める時（S5c のすぐあと。S6 の前） | Job に入れると起動が遅くなる件（S13 の行から移した）：Job なしと比べて、panic で終わる子の時間が約 30 ms 延びた（S5b の測定：Job なし 23〜27 ms、Job あり 53〜66 ms）。IDE と同じやり方（CREATE_SUSPENDED → Job → Toolhelp32 で主スレッドを探して ResumeThread）で測ると、子 1 つで、起動の前から再開し終わるまで 53〜73 ms（起動そのものは 1〜3 ms）。30 個を同時にすると、再開し終わるまでの最大が 1720〜2302 ms（8 回）に延びた。再開から終わりまでは 14〜25 ms、再開したスレッドが 0 個の子はなく、ResumeThread が返したそれまでの止める数はすべて 1 だった（2026-11-03。主スレッドの取りこぼしはない）。重いのは Toolhelp32 で機械全体のスレッドの一覧を取る所と思われる（推測。Job を作る所と分けては測っていない）。run・run_pair の実行時間は起動の前から数えていたので、この時間も入っていた（run_pair では解答とジャッジの 2 回分）。直し（Windows だけ。Unix は今までどおり spawn の前から数える）：win::start_in_job が再開し終わった時刻を返し、run はそこから数える。run_pair は、win::start_all_in_jobs で解答とジャッジを両方 Job に入れて（どちらも止まったまま）から、スレッドの一覧を 1 回だけ取って続けて再開し、再開し終わった時刻から数える。どちらかで Job を作る・入れる・再開するのに失敗したら、2 つとも止めてエラーにする（再開したあとの側も止める）。Job のハンドルは、子を止めたあとで閉じる（先に閉じると KILL_ON_JOB_CLOSE で止まり、終了コードが 0 になる）。ジャッジを起動できなかったときは、解答はまだ Job に入っていないので、Child のハンドルで止める。起動そのものの遅さ（Toolhelp32）は直さない。速くするかは S13 で案を出す | 先にテストを書いて失敗を確かめる：テストのときだけ（#[cfg(test)]）、Job に入れてから再開するまでをスレッドごとに遅らせる win::RESUME_DELAY を使い、1500 ms 遅らせても、run の `exit:0` が上限 1000 ms で Exited(0) になる／run_pair の 1 往復が上限 1000 ms で TLE にならない。直す前はそれぞれ TimedOut・timed_out（elapsed 3.12 秒）で失敗した。2 つの子のどちらかの、Job を作る・入れる・再開するで、わざと失敗させると、2 つとも KILLED_EXIT_CODE で止まり、Job の記録が残らない（6 通り）。作り直しありで cargo test 全体を 10 回 |
| S6 | 改行（9.1 の 6 の a・b） | Windows だけ。直すのは `\r` のすぐあとに `\n` が来る組だけで、`\r` だけのものは残す（compare::crlf_to_lf）。(a) exact の比較で、期待出力と実際の出力の両方を直してから比べる（Windows のエディタで作った CRLF の期待出力と、Rust の解答の LF の出力も一致する。利用者の選択）。(b) 実際の出力を期待出力として保存するとき（workspace::write_expected）に直す。「すでに同じ内容」の判断も直したあとで行う。ランダムテストの失敗の保存（random::save_case）では、random-N.out（愚直解の出力）だけを直し、random-N.in（生成器の出力）は解答に渡したそのままを残す（利用者の選択）。入力の CRLF（Rust の read_line が `\r` を残す件）と、対話テストのやり取りの表示の `\r` は扱わない | 先にテストを書いて失敗を確かめる：exact で CRLF と LF が一致する（両向き）、`\r` だけのものは一致しない／保存した sample-N.out が LF になり、同じ内容の保存は Unchanged／random-N.out が LF になり、random-N.in は CRLF のまま。直す前は、それぞれ Some(false)・`2\r\n3\r\n` のまま・`1\r2\r\n10\r\n` のままで失敗した。既存の exact_compares_bytes の「CRLF は一致しない」の行は、Windows では成り立たないので `#[cfg(not(windows))]` を付けた |
| S7 | Python（9.1 の 7・追加 11） | Windows だけ。CPython の探し方：settings → PEP 514 のレジストリ（`Software\Python\PythonCore\3.13\InstallPath` の ExecutablePath、なければ既定の値の python.exe。3.13 のキーだけで、3.13-32・3.13t は見ない。HKCU → HKLM、64 ビットの見え方。読むだけ。win::registry_string）→ PATH の python.exe。PATH の候補からは、WindowsApps の実行エイリアス（`%LOCALAPPDATA%\Microsoft\WindowsApps` の中のもの〔大文字・小文字と末尾の `\` を問わない〕と、大きさ 0 のもの）と相対パスのフォルダを、版を確かめる前に除く（動かすと Microsoft Store が開くことがあるため）。残りを順に `--version` で確かめ、最初の CPython 3.13 を使う（PyPy のフォルダにも python.exe があるため。利用者の選択）。PyPy は settings → PATH（pypy3.11.exe・pypy3.exe）。py ランチャーは使わない。`-X utf8` を、実行（program）・構文の検査（check_command）・import の読み取り（imports）・ライブラリの場所の読み取り（library_dirs。引数を確かめるため library_dirs_command に分けた）に付ける。`--version` には付けない。案内文を Windows 向けに（python.org・winget、pypy.org、settings.json の python.exe・pypy3.exe のパス）。windows-sys に Win32_System_Registry を足した。PyPy は、テストでは扱わず画面で確かめる（利用者の選択） | 先にテストを書いて失敗を確かめる：レジストリの CPython が見つかる（答えはテストの中で reg.exe で読む。キーがなければ飛ばす）／4 か所の引数に `-X utf8` が入る／`print('あ')` が UTF-8 で出る／構文の誤りの `'＋' (U+FF0B)` が読める／`import データ` が読める／Windows の案内に brew が入らない。直す前は、それぞれ `Err("python3.13 が見つかりません…")`・`-X utf8` なし・`82 A0 0D 0A`（cp932）・`'�{'`・`�f�[�^`・`brew install python@3.13` で失敗した。エイリアスを除くテスト（エイリアスのフォルダ・大きさ 0・相対パスを除き、版を確かめるのは残りだけで、PyPy の python.exe を飛ばして CPython 3.13 を選ぶ）は新しい関数なので、直したあとに、フォルダで除く・大きさ 0 で除く・版を確かめるのそれぞれを一時的に外すと落ちることを確かめてから戻した（library_dirs_command の `-X utf8` も同じ）。S7 のあと、この機械で動き始めたテスト（CPython が見つかるようになったため）：checks_syntax_with_local_python・reads_imports_with_local_python・connects_python_programs・input_gen の 2 つ。input_gen の 2 つ（出力の `\r\n`）と connects_python_programs（やり取りの記録の `"1 100\r\n"`）は、テストの比べ方だけを Windows 向けにした（本番は変えない。既知の問題）。random の finds_differences_with_python_programs の Windows 版を足した（設定なしでレジストリから見つける。ジャッジと違う版を断ることは、ほかの版がないので確かめない）。GitHub の windows-latest にレジストリの CPython 3.13 があるかは未確認（なければ、これらのテストは CI では飛ばされる）。PyPy を画面で試すには settings.json の python.pypy の指定が要る（リポジトリの外なので、変える前に聞く） |
| S8 | C++（9.1 の 10・追加 12） | `g++.exe` を settings → PATH → 決まった場所（`%ProgramFiles%\mingw64\bin` → `%ProgramFiles(x86)%\mingw64\bin` → `C:\mingw64\bin` → `C:\msys64\ucrt64\bin`。9.1 の 10 の MinGW-Builds を先にする）の順に探す。出力の名前に `.exe`。`/dev/null` → `NUL`。作った exe を動かすときだけ、子の PATH の先頭にコンパイラの bin を足す。GCC_HINT を Windows 向けに | find_compiler_in のテストの Windows 版。builds_and_runs_with_local_gcc を Windows でも動かす。PATH に g++ がない状態で 0xC0000135 にならない |
| S9 | スタック（9.1 の 9・追加 6・追加 7） | Windows だけ。C++ は `-Wl,--stack,1073741824`。Rust は run.rs の `cargo build --bin … --example … --message-format=json` を、`cargo rustc --bin <名前> --message-format=json -- -C link-arg=/STACK:1073741824` と、example ごとの `cargo rustc --example <名前> …` に分けて続けて動かす（`--` の後ろの引数は 1 つの対象にしか渡せないため。5.3）。ビルドの出力・診断・止める操作は今と同じに扱う | 深い再帰の子が、付けると通り、付けないと 0xC00000FD で落ちる。`objdump -p` で 0x40000000。依存のあるパッケージで、`cargo build` と IDE のビルドを交互にしても依存を作り直さない（未確認。ここで確かめる） |
| S10 | パスの表記 | rustc の診断の file は S10a で済ませる。logLinks.ts は Windows だけ `\` も区切りとして読む。scaffold のテストは canonicalize したもの同士で比べる（本番のコードは両側を canonicalize して比べているので、テストだけの問題）。scaffold::cargo_reads_created_packages_under_a_parent_workspace の ignore を外す | 外したテストが通る。logLinks のテスト |
| S10c | 同時の保存の失敗（S10b のすぐあと。S5 の前） | Windows で全部のテストと一緒に動かしたとき 6 回中 1 回、storage::tests::concurrent_writes_do_not_mix（8 つのスレッドが同じファイルに write_atomic を 20 回ずつ）が「設定を保存できません」で落ちた（S10b の確かめの途中。段階 1 C7 の win::rename_replacing は、5・32 のとき 20 ms おきに 100 回〔約 2 秒〕試し直す）。まず write_atomic の失敗の文に、段階（一時ファイルを作る・書く・確定する・名前を変えて置き換える）と OS のエラーを入れ、rename_replacing は諦めたときに試し直した回数とかかった時間を入れる（Unix の失敗の文も変わる）。全部のテストと一緒に何回かくり返して、どのエラーで、何回目で諦めたかを確かめてから、直し方を決める。原因が分かるまで、待ちの時間を延ばすだけの直しはしない。**S10c は調べるための変更だけでコミットし、直すのは S10d。** 調べた結果（2026-11-01）：全部のテストを 29 回くり返して 1 回再現（S10b のときと合わせて 35 回中 2 回）。3 つのスレッドが同じ時刻に「名前を変えて置き換える：アクセスが拒否されました (os error 5)。100 回試し直した（2095〜2099 ms）」で諦めた。Rust 1.99.0 の fs::rename は MoveFileExW が 5 で失敗すると POSIX の決まりの置き換え（FileRenameInfoEx）も試すので、その両方が約 2 秒ずっと失敗したことになる。原因（ほかのもの〔Defender など〕が削除の共有を許さずに開いていたのか、自分のスレッドどうしの置き換えが重なったのか）は未確認 | 失敗の文から原因を確かめる。直したあと、全部のテストと一緒に何回もくり返して落ちない |
| S10d | 同時の保存の失敗への手当て（S10c のあと。9.1 の追加 19） | 原因は決めきれなかった（全部のテストを合わせて 50 回くり返して 2 回、試しのプログラム〔同じ形の同時の保存を、全部のテストと同時に 561 秒、約 54 万回の置き換え〕では 0 回）。(2) Windows だけ、write_atomic で同じパスへの保存を IDE の中で 1 つずつにする（storage::save_lock。鍵は渡されたパスの形のままで、同じファイルを別の書き方で渡すと別のロックになる）。(3) win::rename_replacing が 5・32 の試し直しを使い切って諦めたときは、「ほかのプログラム（ウイルス対策など）が使っている可能性があります」を足す。試し直しの回数・時間は変えない（測った長さを見てから相談する） | 先にテストを書いて失敗を確かめる：置き換える先を共有を許さずに開いておくと、案内の入った文で失敗する（直す前は、同じ「アクセスが拒否されました (os error 5)。100 回試し直した」で、案内がなかった）／同じパスには同じロック。直したあと、全部のテストを 30 回くり返す（2026-11-01：30 回とも通った。直す前は 50 回中 2 回落ちていたので、30 回で落ちなくても直ったとは言い切れない） |
| S11 | 問題の取得 | problem.rs の `/usr/bin/curl` を、Windows では `%SystemRoot%\System32\curl.exe` にする（PATH は探さない） | problem.rs のテスト。画面で問題ページ・問題の一覧を取る |
| S12 | Defender の注意（9.1 の 8・追加 13） | Windows だけ：ビルドの直後の 1 ケース目が TLE のとき、実行ログに 1 行の注意を出す | 単体テスト |
| S13 | まとめ | この文書に段階 2 の結果。後片付け：judges_cases_with_a_real_program の一時フォルダ、%TEMP% に前から残っている atcoder-studio-* のフォルダ（2026-11-03 の時点で 14 個。どれも 2026-10-29 の日付。どのテストが残したかを調べてから扱う）。Job に入れるのが遅いこと（S5d の行）を速くする工夫をするかの案 | 全体の cargo test・cargo clippy --all-targets・npm run build |

段階 2 で利用者が手で確かめること（各コミットのあとに手順を書いて頼む）
- Rust・C++・CPython・PyPy のそれぞれで、AC・WA・RE（0 除算・範囲外・panic・アクセス違反）・TLE（無限ループ）・MLE（大きな配列）になる解答の判定と、時間・メモリ・RE の名前の表示
- 孫を作る解答を止めたあと、タスクマネージャーに子が残っていない
- 止めたあとすぐにビルドし直せる
- 問題の取得（問題ページ・問題の一覧）
- 深い再帰の解答が手元で落ちない（S9）

S3 の画面の確かめ（2026-11-01。利用者が Windows 11 で確かめた。dev〔npm run tauri dev〕、S3 のコミットの前の作業ツリー。
試しのフォルダは Rust のパッケージ t1 で、main.rs は `ping -n 300 127.0.0.1` を孫として起動してから回り続ける。見たのはタスクマネージャーの「詳細」）

| 番号 | 確かめたこと | 結果 |
| --- | --- | --- |
| 1 | 実行して、時間の上限（10 秒）で止まったとき・「停止」を押したとき | OK。どちらも t1.exe と PING.EXE が消えた。ping の窓は出なかった |
| 2 | サンプルテストで TLE になったとき | OK。約 2.5 秒で TLE になり、t1.exe と PING.EXE が消えた |
| 3 | 子が自然に終わったあとの孫 | OK。回り続けない版では、どちらもタスクマネージャーに出る前に消えた。ping を起動して 3 秒眠って終わる版で、3 秒後に t1.exe と PING.EXE が消えた |
| 4 | 止めた直後にコメントを足してビルドし直す（5 回、できるだけ素早く） | OK。5 回ともエラーなくビルドできた（4.7） |
| 5 | ping の出力が 2〜3 行出たところで、atcoder-studio.exe をタスクの終了で止める（IDE が異常に終わったときにあたる） | OK。15 秒後に見て、t1.exe も PING.EXE も残っていなかった。dev で npm と cargo の下で動いていても Job が働いた（4.6） |
| 6 | 終わったあとに残っているもの | OK。PING.EXE・t1.exe・cargo.exe・rustc.exe は残っていなかった |
| ほか | 実行ログの ping の出力 | 日本語の部分だけが文字化けした（英数字は正しい）。ping が CP932 で出し、IDE が UTF-8 として表示するため（4.4）。直すかどうかは S7 か S13 で案を出してから決める |

S5b・S5c の画面の確かめ（2026-11-03。利用者が Windows 11 で確かめた。dev〔npm run tauri dev〕、S5c のコミットのあと。
試しのフォルダは Rust のパッケージ t1。C++ の abort・assert・捕まえない例外の画面の確かめは S8 のあとにする）

| 番号 | 確かめたこと | 結果 |
| --- | --- | --- |
| A | Rust の abort（0xC0000409。S5c） | OK。実行ログは「終了コード 0xC0000409（異常終了：C++ の abort・assert・捕まえない例外、Rust の abort など）で終了しました（実行時間 187 ms）」、サンプルテストは RE・167 ms |
| B | アクセス違反（0xC0000005。S5b） | OK。実行ログは「終了コード 0xC0000005（アクセス違反：…）で終了しました（実行時間 69 ms）」、サンプルテストは RE・55 ms |
| C | スタックあふれ（0xC00000FD。S5b） | OK。実行ログは「終了コード 0xC00000FD（スタックあふれ：…）で終了しました（実行時間 81 ms）」、サンプルテストは RE・57 ms |
| ほか | 表示の形とエラーの窓 | OK。どれも「…など）で終了しました」と、空白なしで出た（S5b の runLog の直し）。エラーの窓は出なかった |

S5d の画面の確かめ（2026-11-03。利用者が Windows 11 で確かめた。dev、t1、S5d のコミットのあと。実行時間を、子を Job に入れて
再開したあとから数えるようになったかを見た。判定はどれも前と同じ）

| 番号 | 確かめたこと | 結果 |
| --- | --- | --- |
| A | Rust の abort（0xC0000409） | OK。実行 114 ms、サンプルテスト RE・114 ms（S5c のあとは 187 ms・167 ms）。約 110 ms 残るのは、落ちたあと WerFault.exe が起動し、IDE がそれを見つけて止めるまでの時間（S5c。WerFault.exe は子が始まってから早くても約 120 ms で起動し、見張りは 100 ms ごと） |
| B | アクセス違反（0xC0000005） | OK。実行 11 ms、サンプルテスト RE・13 ms（S5c のあとは 69 ms・55 ms） |
| C | スタックあふれ（0xC00000FD） | OK。実行 11 ms、サンプルテスト RE・12 ms（S5c のあとは 81 ms・57 ms） |
| D | 普通の解答（`println!("1")`） | OK。実行 13 ms、サンプルテスト AC・10 ms（S4 のときは 100 ms） |

段階 2 の既知の問題（まだ直さない）

- 0xC0000409（__fastfail。C++ の abort・assert・捕まえない例外、Rust の abort）で落ちるたびに、Windows のエラー報告の記録が
  `C:\ProgramData\Microsoft\Windows\WER\ReportArchive` に 1 つ残る。S5c で、落ちた子が親の WerFault.exe を止めても残る
  （WER のサービスが起動した WerFault.exe は止めないため。2026-11-03 の確かめで、S5c のテストのあとも記録が増えていた。推測を含む）。
  ダンプ（`%LOCALAPPDATA%\CrashDumps`）は、S5c で止めたあとは増えなかった。扱われない例外（アクセス違反など）は、S5b の DIE で WER が
  動かないので、どちらも残らない見込み。レジストリや WER の設定は変えない（9.1 の追加 21）。テストと試しで作った CrashDumps の
  test_child のダンプ 9 個は、2026-11-03 に利用者の許可を得て消した。ReportArchive は消していない
- 入力の CRLF は直さない（S6 で扱わないと決めた。S7 で書き足した）。`\r\n` の入力ができるのは次のとき：
  - Windows のメモ帳などで書いた sample-N.in・input.txt
  - ランダムテストの生成器の出力。Python（CPython・PyPy）と C++（MinGW）の生成器は、Windows では改行を `\r\n` で出す
    （4.3・4.4 で確認済み）。この出力は、ランダムテストの間はそのまま解答の標準入力に渡り、失敗したケースの random-N.in にも
    そのまま保存される（S6 の決め。同じ失敗を起こし直せるように。S7 の finds_differences_with_python_programs_on_windows で、
    CPython の生成器の出力が random-1.in に `101\r\n` で保存されることを確かめた）

  IDE は入力をそのまま解答の標準入力に渡すので、Rust の解答が `read_line` で読むと、行の最後に `\r` が残ることがある
  （`read_line` は改行を含めて返すので、`trim` せずに使うと `\r` が残る。`BufRead::lines`・`str::lines` は `\r\n` も取り除き、
  proconio のように空白で区切って読むものは影響を受けない。Rust の標準ライブラリの説明からの推測で、動かしては確かめていない）。
  AtCoder から取ったサンプルは IDE が書くので LF のまま（4.3。推測）
- 対話テストのやり取りの `\r` も直さない（S6 で扱わないと決めた）。Windows では、解答（C++・Python）が `\r\n` を出すと、
  それがそのままジャッジの標準入力に渡る。C++ の `getline` など、行の最後の `\r` を取り除かない読み方をするジャッジ（利用者が
  書いたもの）では、行の最後に `\r` が残りうる（推測。4.9）。ジャッジが Python の `input()` なら `\r\n` を取り除くので影響は
  ない（S7 の connects_python_programs で、CPython の解答とジャッジの判定が正しく出ることを確かめた）。やり取りの記録
  （画面の表示）にも `\r` が残る（同じテストで、1 行目が `"1 100\r\n"` だった。画面で `\r` が見えるかは未確認）

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

**今回の移植には含めない（9 章 12。2026-10-29 に決めた）。** 段階 1 では、デバッグのコードは Windows でコンパイルが通るようにするだけで、動かさない。以下は後で行うときのための記録。

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
| webview2-com 0.39・windows-core 0.62（Windows だけ。**使うと決めた**） | WebView2 のブラウザの既定のキーを切る（SetAreBrowserAcceleratorKeysEnabled）。Tauri 2.12.4 には設定がなく、`with_webview` の `controller()` が webview2-com の型を返す（ソースで確認済み）。どちらも Tauri の間接依存として同じ版が Cargo.lock にある | JS の preventDefault だけで防ぐ（効くキーは未確認） | Cargo.lock にあること・Tauri の API：確認済み／動き：未確認 |

決めたこと（2026-10-29）：Windows の API は windows-sys を直接使い、keyring・tauri-plugin-opener・chrono などの用途別のクレートは足さない。
例外として、WebView2 の設定のために webview2-com と windows-core を足す。dunce を使うかは段階 4 で決める（その時点でも windows-sys の方針に合わせて自前の関数にする案がある）。

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

### 9.1 決めたこと（2026-10-29）

| 番号 | 決めたこと | どの段階で |
| --- | --- | --- |
| 1 | Job Object には (a) CREATE_SUSPENDED で起動し、主スレッドを探して再開する方法で入れる | 段階 2（段階 1 は仮の実装） |
| 2 | Windows の API は windows-sys を直接使う。用途別のクレートは足さない（例外は 8 章の webview2-com・windows-core） | 全体 |
| 3 | 資格情報は案 A（windows-sys の CredWriteW など）。macOS は今のまま Keychain | 段階 3 |
| 4 | テキストエディタで開く：VS Code があれば VS Code、なければメモ帳 | 段階 3 |
| 5 | キー割り当ては ⌘ を Ctrl に置き換える。WebView2 のブラウザの既定のキー（Ctrl+R の再読み込みなど）は切る | 段階 1（C9） |
| 6 | 改行：(a) exact の比較で CRLF を LF に直す（Windows だけ）、(b) 期待出力を保存するときに LF に直す、(c) .gitattributes を足す、のすべて | (c) は段階 1（C2）、(a)(b) は段階 2 |
| 7 | Python に `-X utf8` を付ける。PyPy で使えるかは段階 2 で確かめる | 段階 2 |
| 8 | Defender の初回の遅れ：(d) 注意を出すだけ。優先度は低い | 段階 2 以降 |
| 9 | 手元のビルドでスタックを大きくする。値は段階 2 で決める | 段階 2 |
| 10 | C++ のコンパイラは既存の MinGW-Builds を勧める。settings.json の cpp.compiler の指定と、決まった場所の探索の両方に対応する | 段階 2 |
| 11 | clangd・basedpyright の場所の指定を settings.json に足す | 段階 4 |
| 12 | デバッグは今回の移植に含めない（段階 6 は後回し） | — |
| 13 | check_name で `\` と `:` を断るのは Windows だけ | 段階 1（C5） |
| 14 | incognito が効かなかったときの対策は、段階 3 で確かめてから決める | 段階 3 |
| 15 | bundle.targets は nsis だけ（Windows）。署名はしない | 段階 1（C10） |
| 16 | メニューは「ファイル」に設定・終了をまとめる最小限の変更 | 段階 1（C8） |
| 追加 1 | 段階 1 の順：コンパイルを通す変更を先にし、そのあと 4.10 の Windows のテストを書いて失敗を確かめてから直す | 段階 1 |
| 追加 2 | 4.10 はテストと修正を 1 つのコミットにまとめ、直す前の失敗をメッセージに書く | 段階 1（C5） |
| 追加 3 | WebView2 の既定のキーを切るために、webview2-com・windows-core を Windows だけの依存として足す（2 の例外） | 段階 1（C9） |
| 追加 4 | process.rs は段階 1 では仮の Windows 実装（直接の子だけを止める・メモリは測らない）。関数の形は変えず、段階 2 で中身を Job Object に差し替える | 段階 1（C3）・段階 2 |
| 追加 5 | macOS を壊さないため、CI（GitHub Actions。macos-latest・windows-latest）を足す。push は利用者が許可してから | 段階 1（C4b） |
| 追加 6（2026-10-31） | スタックの大きさは 1 GiB（1073741824 バイト）。既定のメモリの上限（1024 MiB）と同じ。予約するだけなので、使った分だけが使用メモリに入る（推測） | 段階 2（S9） |
| 追加 7（2026-10-31） | Rust のスタックは `cargo rustc … -- -C link-arg=/STACK:N` で付ける（run.rs の cargo build を置き換える。Windows だけ）。依存を作り直さず（未確認。S9 で確かめる）、RUSTFLAGS とも両立するため | 段階 2（S9） |
| 追加 8（2026-10-31） | テスト用の子のプログラムは `src-tauri/examples/test_child.rs` | 段階 2（S2） |
| 追加 9（2026-10-31） | Job に入れる処理は、`process_group(0)` の代わりの仕組みで CREATE_SUSPENDED を付け、spawn の直後に `#[cfg(windows)]` の 1 行で行う（Unix 側の文字は変えない） | 段階 2（S3） |
| 追加 10（2026-10-31） | IDE が止めたとき（TerminateJobObject）の終了コードは専用の値 0xE0AC0001（i32 では -525598719）にして、「IDE が止めました」と表示する。上位の 4 ビットの E は「エラー」と「利用者が定める値」（ビット 29）の印で、Windows の NTSTATUS とは重ならない | 段階 2（S3・S5） |
| 追加 11（2026-10-31） | Windows での CPython の探し方は、settings → PEP 514 のレジストリ（HKCU → HKLM）→ PATH の python.exe（WindowsApps のエイリアスを除き、版を確かめる）。py ランチャーは使わない | 段階 2（S7） |
| 追加 12（2026-10-31） | MinGW の DLL：作った exe を動かすときだけ、子の PATH の先頭にコンパイラの bin を足す（`-static` などのビルドの引数は変えない） | 段階 2（S8） |
| 追加 13（2026-10-31） | Defender の注意は、ビルドの直後の 1 ケース目が TLE のときだけ実行ログに 1 行出す | 段階 2（S12） |
| 追加 14（2026-10-31） | 子のプログラムを使う新しいテストは Windows だけで動かす。Unix の sh のテストはそのまま | 段階 2（S2〜S4） |
| 追加 15（2026-11-01） | 止めた直後のビルドの前に、出力の exe が書けるようになるまで最大 2 秒待つ。待っても書けなければ、案内を出してビルドは続ける（ビルドせずに失敗にはしない）。S3 には入れず、すぐあとの S3b にする | 段階 2（S3b） |
| 追加 16（2026-11-01） | 使用メモリは WorkingSet（触ったページ）で測り、直接の子だけを数える（Unix と同じ）。最大は終わったあとの PeakWorkingSetSize を、起動したときのハンドルの写しで読む（pid で開き直さない）。読めなければ実行中に見た最大で代わりにする案だったが、読めたので使わない | 段階 2（S4） |
| 追加 17（2026-11-01） | rustc の診断のパスの `\` を `/` に直す部分だけを、S4 のすぐあと・S5 の前の S10a にする。直すのは Windows だけ。`\\?\` の付いたパスは、外してから直す。random のテストの ignore を S10a で外す。logLinks.ts と scaffold のテストは S10 に残す | 段階 2（S10a） |
| 追加 18（2026-11-01） | 対話テストの、先に失敗した側の取り違えは、原因と結果の順（出力が先に閉じた側）で決める形に直す。両方の OS にある判定の不具合なので、OS で決め方を分けない（Unix の動きも変わる）。S10a とは別の S10b にし、interactive のテストの ignore は S10b で外す | 段階 2（S10b） |
| 追加 19（2026-11-01） | 同時の保存の失敗は、原因を決めきれないので、(2) 同じパスへの保存を IDE の中で 1 つずつにする（Windows だけ）と、(3) 諦めたときに、ほかのプログラム（ウイルス対策など）が使っている可能性を案内する、の両方を S10d で入れる。試し直しの時間を延ばすのは、測った長さを見てから相談する | 段階 2（S10d） |
| 追加 20（2026-11-02） | RE の表示：i32 で負の終了コード（Windows の例外のコード）だけを 16 進にし、名前の表にあれば名前を添える（表にない値は 16 進だけ）。負でない値は 256 以上も 10 進のまま。名前は Rust で作って画面に送る（画面の側に単体テストの仕組みがないため）。終了コード 3 への C++ の abort・assert の添え書きは、測ると 0xC0000409 だったので S5b でやめた | 段階 2（S5・S5b） |
| 追加 21（2026-11-02） | 扱われない例外で落ちた解答が約 4 秒かかり、サンプルテストで TLE と判定される件は、Job に JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION を足して直す（S5b）。0xC0000409（__fastfail）には効かないので、別に手当てする（案を相談中）。レジストリや WER の設定は変えない | 段階 2（S5b） |

---

## 10. 未確認・不明のまま残っている項目

未確認（確かめ方は各節に書いた）

- 2.2：cargo build のエラーを直したあとに出る、型検査より後の段階のエラー
- 3 章の代替案のすべて（Windows で動かしていない）
- std の Command が `.exe` のないパスに `.exe` を補うか
- curl.exe が今の引数と `--config -` で同じに動くか
- menu.rs の macOS 向けの項目を Windows で使ったときの動き
- WebView2 のブラウザの既定のキー（Ctrl+R など）と、keydown の preventDefault の効き方
- 4.1：資格情報マネージャーの値の大きさの上限、incognito の効き方、パスワードの自動保存・自動入力の既定値
- 4.3：Rust の出力の改行、exact の比較で WA になること、include_str! の testdata のテスト（CPython・PyPy の出力が CRLF になることは 2026-10-31 に確認済み）
- 4.4：GUI から起動した子のコードページ、g++ が CP932 で出すもの、`-E` で PYTHONUTF8 が無視されること（PyPy が `-X utf8` を受け付けることは 2026-10-31 に確認済み）
- 4.5：遅れの原因が Defender か、作業領域・大きな exe での遅れ、GetProcessTimes で検査を除けるか
- 4.6：release で窓が出ること、Job Object との組み合わせ
- 4.7：実行中の exe の上書きの失敗、TerminateJobObject が非同期か
- 4.8：引数の引用、.cmd の制約、clangd の `--query-driver` の読み方、`\\?\` を受け付けないツール
- 4.9：BrokenPipe、ハンドルの継承、CRLF の影響
- 4.10：`:`・末尾の `.` と空白・予約名を Windows が実際にどう扱うか（段階 1 では名前で断ったので、断る前の動きは確かめていない）、大文字・小文字の比較
- 5 章：WinLibs・MSYS2 の入手できる版、ジャッジのパッケージの Windows 用の wheel、std::thread・std::print のリンクと実行
  （CPython 3.13.7・PyPy v7.3.20 の入手、MinGW と MSVC の Rust の既定のスタックは 2026-10-31 に確認済み）
- 段階 2 の案で前提にしていること：`cargo rustc` の `--` の後ろの引数で依存を作り直さないこと、PATH に g++ がないと MinGW の exe が 0xC0000135 で落ちること、
  各 NTSTATUS（スタックあふれ・abort・0 除算など）が実際にどの終了コードになるか、PeakWorkingSetSize を終わったプロセスのハンドルで読めるか
- 6 章：すべてのデバッグの方式（LLVM の lldb-dap の有無、CodeLLDB のライセンス、gdb の Python と DAP）
- macOS の動きを壊していないこと（CI の macos-latest でビルド・clippy・テストが通ったことは確認済み〔run 41023624709〕。clippy の警告の有無、debugs_with_real_lldb_dap、画面の操作は macOS の機械で確かめる）

不明（調べても分からなかった）

- 開発者モードの設定（レジストリの値を読めなかった。シンボリックリンクを作れないことは確認済み）
- 実際の REVEL_SESSION の長さ
- Windows 11 でフォルダの中の予約名（NUL など）が装置を指すか
- 8.3 の短い名前がこのボリュームで作られるか
- ジャッジのスタックの大きさ（MinGW の ld の既定は 2 MiB と 2026-10-31 に確認済み）
- CP932 で表せない文字を含む利用者名・フォルダでの MinGW・Python の動き
- MSYS2 の今の GCC の版
- Tauri からの WebView2 の自動入力の設定の仕方
