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
| problem.rs:156–191 | `/usr/bin/curl --proto =https --proto-redir =https --location …` で問題ページを取る | `%SystemRoot%\System32\curl.exe`（8.21.0 がある）。S11 で、Windows ではこれを使うようにした（同じ引数で動き、`--proto =https` も効き、404 では同じ「returned error: 404」を出すことをテストで確認） | 箇所：確認済み、curl.exe があることも確認済み／S11 で対応 |
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
| cpp.rs:28–31・150–154・215–218 | 出力 `target/main`・`target/main-debug`・`target/examples/<名前>` | `.exe` を付ける。MinGW の g++ は `-o main` で `main.exe` を作る（確認済み）ので、IDE が `main` のまま起動しようとすると見つからない（std の Command が `.exe` を補うかは未確認）。S8 で、Windows だけ output_path・example_output_path が `.exe` を付けるようにした | 箇所：確認済み、`.exe` が付くことも確認済み／S8 で対応（単体テスト） |
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
| src/panels/logLinks.ts:11–13 | 実行ログの標準エラー・ビルドの出力の中の `src/main.rs:5:10` の形だけを場所として読む（`\` を含まない） | Windows では、Rust のプログラムが実行中に出す場所（panic の `src\main.rs:4:21`、dbg! の `[src\main.rs:3:5]`。file!() が `\` 区切り）が `\` 区切りになる → `\` も区切りとして受け付ける。S10 の前は `main.rs:4:21` だけに一致して、別のファイルへ移動しうるところだった。Rust のコンパイルエラーの場所（`--> src\main.rs:…`）は、`--message-format=json` で受け取ってエラー一覧だけに出すので、実行ログには出ない（はじめは実行ログにも出ると書いていたが、誤り。S10 の画面の確かめで分かった）。エラー一覧に出す診断の file は、段階 2 S10a で `/` に直した。C++ の g++ の出力（実行ログにも出る）は、もともと `/` か区切りなし。S10 で、Windows だけ `\` も区切りとして読み、開くファイル名を `/` にした | 箇所：確認済み／診断の file：確認済み（S10a）／logLinks.ts：S10 で対応（tests/logLinks.test.ts。画面で panic・dbg! を確認済み） |
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
| 値の大きさ | CredentialBlob の上限は CRED_MAX_CREDENTIAL_BLOB_SIZE（windows-sys 0.61.2 の定数で 2560 バイト）。valid_cookie_value（session.rs:38–44）は 4096 バイトまで許す | 定数：確認済み（2026-11-06）。実際の動き：確認済み（2026-11-07。テストで 2560 バイトは書いて読めた。確かめを外して 2561 バイトを渡すと、CredWriteW はエラー 1783 で断った）。実際の REVEL_SESSION の長さ：不明 | ログインして値の長さを記録する（値そのものは記録しない）。上限を超える値は保存の前に断る（9.1 の追加 23） |
| 他のプログラムからの読み出し | 同じ利用者の権限で動くプログラムは確認なしに読める（macOS の限界〔session.rs:9〕と同じ） | 推測（未確認） | 別のプロセス（PowerShell から CredRead を呼ぶ小さなスクリプト）で、確認なしに読めるかを試す |
| ログインの窓の incognito | WebView2 で InPrivate になり、閉じたあと Cookie・履歴・キャッシュ・パスワードの自動保存がディスクに残らないか。wry 0.57.0 は incognito を WebView2 の `SetIsInPrivateModeEnabled` に渡す（webview2/mod.rs:414。ソースで確認済み）。InPrivate のときにディスクに何が残るかは未確認。パスワードの自動保存は WebView2 の IsPasswordAutosaveEnabled（既定は無効と推測）、フォームの自動入力は IsGeneralAutofillEnabled（既定は有効と推測）で、Tauri からどう設定されるかは不明 | 未確認（自動入力の設定は不明） | 下の「手順 A」 |
| パスワードが IDE に渡らない | src-tauri/src で password を含むのは security の副コマンド名だけ（session.rs:78・101・121・133。grep で確認済み）。IDE は cookies() から REVEL_SESSION だけを読む（session.rs:523–536） | コード：確認済み／Windows 版：未確認 | 下の「手順 B」 |
| ログアウト・期限切れで消える | logout（session.rs:431–435）で delete。ログインのページへ移されたら delete（session.rs:287–290）。Cookie が入れ替われば保存し直す（session.rs:281–285） | コード：確認済み／Windows 版：未確認 | 下の「手順 C」 |

手順 A（incognito の確かめ方）

1. WebView2 の利用者データのフォルダは `%LOCALAPPDATA%\io.github.ishigaki892.acdstudio\EBWebView`（段階 1 で起動したときにできた。確認済み）
2. ログインの前・ログイン中・ログインの窓を閉じたあと・アプリを終えたあとに、フォルダの中のファイル一覧と更新日時を記録する
3. `Default\Network\Cookies`・`Default\Login Data`・`Default\Web Data`・`Default\History` などに、`atcoder.jp`・`REVEL_SESSION`・利用者名・パスワードの文字列が残っていないかを検索する。一致した行は出さず、ファイル名と件数だけを出す。利用者名・パスワードの検索は利用者の窓で行う
4. InPrivate の一時フォルダがどこに作られ、閉じたときに消えるかを見る
5. もう一度ログインの窓を開き、ログイン済みになっていないことを確かめる
6. 効いていなかったときの選択肢（利用者が決める）：ログインの窓だけ別の data_directory（一時フォルダ）にして閉じたあと消す／自動入力を切る方法を探す

手順 B（パスワードが保存されないことの確かめ方）

1. Windows 版でも「cookies() から REVEL_SESSION だけを読む」形を保ち、src-tauri/src を `password` で検索し直す
2. ログインのあと `cmdkey /list | findstr /i "acdstudio atcoder"`（そのまま出さない。9.1 の追加 29）で、項目が `io.github.ishigaki892.acdstudio…` の 1 つだけであることを見る。全体の一覧は利用者の窓で見て、AtCoder に関わる項目がほかにないことを確かめる。一覧は貼らない
3. CredReadW で読んだ中身が REVEL_SESSION の値だけで、入力したパスワードを含まないことを、利用者の窓で確かめる（出すのは一致したかどうかだけ。9.1 の追加 25）
4. 手順 A の検索でもパスワードを探す（利用者の窓で）。ログアウトや値の書き換えの前に、本物の値がリポジトリの変更・実行ログ・history.json・state.json・settings.json に含まれないことを調べる。出すのはファイル名と件数だけ

手順 C（ログアウト・期限切れの確かめ方）

1. 「ログアウト」を押したあと、`cmdkey /list | findstr /i "acdstudio atcoder"` から項目が消え、画面がログインしていない表示になる
2. 資格情報マネージャーの値を壊れた値に書き換えて（テスト用の手順）提出結果を読み込み、ログインのページへ移されて項目が消える
3. 単体テストの serve_once（session.rs:702 以降。302 で /login へ）を Windows でも動かす
4. Cookie の入れ替えで値が更新されることを、CredReadW の LastWritten で確かめる（cmdkey では見えない）
5. ログアウトのあと、次の 3 回、`cmdkey /list | findstr /i "acdstudio atcoder"` で項目がないことを見る。(1) ログアウトの直後、(2) 時間をおいてから（1 分後）、(3) アプリを起動し直したあと。(3) では、画面がログインしていない表示であることも見る。項目があれば、消す前に名前を報告して止まる。別のプログラムの資格情報の書き込みと重なると、消したはずの項目が戻ることが、T5 の切り分けで再現している（テスト用の名前で確かめた。本物のログアウトでは未確認）

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
| 計測の起点 | 段階 1 までは、process.rs の `Instant::now()` が spawn の前だったので、起動にかかる時間（初回の検査を含む）が実行時間に入っていた。段階 2 S5d から、Windows では子を Job に入れて主スレッドを再開し終わった時刻（win::start_in_job が返す時刻）から数えるので、CreateProcess・Job に入れる・再開するまでの時間は実行時間に入らない（Unix は今までどおり spawn の前から） | 確認済み（コード。S5d） |
| 1 回目と 2 回目（S5d のあとの数え方。S12 で測定） | 2026-11-04、AC 電源。%TEMP% の下で、`1` を出すだけの C++（g++ -O2）と Rust（rustc -O）の exe を、それぞれ末尾に違うバイトを足して中身の違う 10 個に写し、IDE と同じ道（process::run。Job に入れ、再開したあとから数える）で続けて 2 回ずつ動かした（コミットしない一時的なテスト。測ったあとで消した）。中央値：C++ は実行時間 1 回目 20.7 ms・2 回目 18.3 ms、全体（process::run を呼ぶ前から返るまで）1 回目 166.3 ms・2 回目 88.4 ms。Rust は実行時間 1 回目 21.2 ms・2 回目 19.1 ms、全体 1 回目 197.5 ms・2 回目 89.1 ms。実行時間の差は約 2 ms（1 回目のうち 1 つずつ 42.1 ms・31.4 ms があった）、全体の差は約 80〜110 ms。新しい exe の 1 回目に遅れる分は、ほぼ CreateProcess から再開までの間（実行時間の外）にある | 確認済み（作業領域の場所・大きな exe では測っていない。写した exe なので、ビルドで書いた直後の exe と同じかは推測） |
| 初回の遅れ（測った値） | 54 KB の C++ の exe を 5 つ作り、それぞれ 3 回ずつ PowerShell から起動して測った：初回 33・41・46・69・33 ms、2 回目以降 5〜9 ms | 確認済み（scratchpad の %TEMP% の下。作業領域の場所・大きな exe・Rust の exe では未確認） |
| 遅れの原因 | Defender の検査と思われるが、ほかのキャッシュの影響と切り分けていない | 推測（未確認） |
| TLE への影響 | 既定の制限は 2500 ms（compare.rs:24）。段階 1 までの数え方なら、制限すれすれの解答のときだけ判定が変わりえた。S5d のあとは、1 回目の遅れがほぼ実行時間の外に出たので（上の行。差は約 2 ms）、判定はほとんど変わらない見込み | 段階 1 まで：推測／S5d のあと：測定で確認（写した exe で） |
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
| 起こること | コンソールを持たない GUI のプロセスから、cargo・g++・curl・clangd・rust-analyzer・解答などのコンソールのプログラムを起動すると、そのたびに窓が出る（推測） | 未確認（対策を入れたあとなので、対策がないときに窓が出るかは確かめていない） |
| 対策 | `std::os::windows::process::CommandExt::creation_flags(CREATE_NO_WINDOW)` を、共通の関数（例：toolchain.rs の cargo_command の隣に置く「外部コマンドを作る関数」）で必ず付ける。36 か所を 1 つの関数に寄せる。→ 段階 1 C6 で process::command にした。段階 2 で起動するようになった子（cargo・rustc・g++・python・curl.exe・解答・ジャッジ・生成器・愚直解）も、どれもこの関数を通る（2026-11-05 にコードで確認）。release の exe をエクスプローラーから起動し、実行・各テスト・止める・言語サーバーの再起動・問題の取得・C++ の abort で、黒い窓もエラー報告のダイアログも出なかった（段階 2 S13 の A6。2026-11-05） | 確認済み（target\release の exe で。インストーラーで入れた exe は未確認） |
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
| `\\?\` の付いたパス | canonicalize の結果（`\\?\C:\…`）が、対話テストのジャッジの引数（interactive.rs:217–219）、g++、lldb（debug.rs:372–376 の lldb_quote）、debugpy に渡る。`\\?\` を受け付けないツールがある。g++（MinGW-Builds 15.2.0 の ld）は、すでにある出力ファイルを `\\?\` の形の `-o` では開き直せず「cannot open output file \\main.exe: No such file or directory」で落ちた（1 回目は通り、2 回目に落ちる。`.exe` の有無によらない。普通の形なら通る。2026-11-03 に確認）。S8 で、g++ の `-o` をパッケージのフォルダ（current_dir）からの相対パスにした。current_dir と、作った exe を動かすときの場所は `\\?\` の形のままで動いた。AC Library の `-I`（S10 で調べた）：アプリのフォルダは Tauri の app_config_dir（dirs::config_dir〔SHGetKnownFolderPath〕に識別子を足したもの。tauri 2.12.4・dirs-sys 0.5.0 のソース）で、canonicalize しないので `\\?\` は付かない。cpp::ac_library_dir も join と is_dir だけ。対話テストのジャッジの引数（入力のファイルの場所）は `\\?\` の形で渡るが、Rust（args）・C++（MinGW の ifstream）・Python（open）のジャッジのテストは、`\\?\` の形の作業領域で通っている | g++ の `-o`：確認済み・S8 で対応。AC Library の `-I`：`\\?\` は付かない（ソースで確認。AC Library を置いた画面の確かめはしていない）。ジャッジの引数：Rust・C++・Python のジャッジが読めることをテストで確認。lldb・debugpy：未確認（段階 6） |
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
| スタックの大きさ | MinGW の g++ で作った exe の既定の予約は 2 MiB（`objdump -p` の SizeOfStackReserve が 0x200000）。`-Wl,--stack,536870912` を付けると 0x20000000 になった。ジャッジのスタックの大きさは不明。深い再帰が手元だけで落ちる（0xC00000FD）おそれ → 手元のビルドで 1 GiB にする（9.1 の 9・追加 6）。S9 で、提出用・デバッグ用・example のビルドに `-Wl,--stack,1073741824` を付けた（0x40000000 になることをテストで確認） | 既定値・付け方：確認済み／ジャッジ：不明 |
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
| 主スレッドのスタック | rustc（x86_64-pc-windows-msvc）で作った exe の既定の予約は 1 MiB（SizeOfStackReserve が 0x100000）。深い再帰が手元だけで落ちうる → 手元のビルドで 1 GiB にする（9.1 の 9・追加 6・追加 7）。`cargo rustc --bin p -- -C link-arg=/STACK:1073741824` と `cargo rustc --example gen -- …` で、どちらも 0x40000000 になった。`cargo rustc` の `--` の後ろの引数は 1 つの対象にしか渡せない（`--bin` と `--example` を一緒に渡すと「extra arguments to `rustc` can only be passed to one target」で失敗）。S9 で、bin と example を 1 つずつ `cargo rustc` でビルドするようにした。依存は作り直さない（パスで指定したライブラリで確かめた。2026-11-03） | 既定値・付け方・1 つの対象の制約：確認済み（2026-10-31。依存のないパッケージで）／依存を作り直さないこと：確認済み（2026-11-03） |
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
| 2 | 3 | interactive::tests_interactively_with_real_programs、random::finds_differences_with_real_programs、scaffold::cargo_reads_created_packages_under_a_parent_workspace | 使用メモリを測らない（段階 2 S4 で測るようにしたあとも、rustc の診断のパスで落ちるので、S10a に回した）／rustc の診断のパスが `\` 区切り（S10a）／cargo の出力のパスと canonicalize の `\\?\` の違い（S10）。random は段階 2 S10a で ignore を外した。interactive は、S10a で診断のパスは直ったが、先に失敗した側の取り違えが見つかったので、S10b に回し、S10b で ignore を外した。scaffold は段階 2 S10 で ignore を外した（テストの比べ方だけを直した） |
| 3・5 | 13 | session の 4 件、problem::reads_with_the_session_only_after_a_404_when_logged_in、submit の 8 件 | /usr/bin/security（Keychain）と /usr/bin/curl |
| 4 | 4 | sources の 3 件、lsp::overrides_initialize_and_answers_configuration | file: の URI・`\\?\`・パスの区切り |
| 6 | 5 | debug の 5 件 | lldb-dap（デバッグは今回の移植に含めない） |

（段階 1 の時点の表。段階 2 のあとは 23 件で、「段階 2 の結果」の表にある）

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
不具合を直すときは、先にテストを書いて失敗を確かめ、テストと修正を 1 つのコミットにして、直す前の失敗をメッセージに書く。push と PR は利用者が許可してから。
全部の cargo test（時間を確かめるテストを含む）は、ノート PC を電源（AC）につないで動かす。電池で動かすと、一斉の起動が遅れて
時間を確かめるテストが落ちることがある（S13 の行。2026-11-04）。テストの時間の余裕は広げない（S13 で利用者が決めた）

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
| S8 | C++（9.1 の 10・追加 12） | Windows だけ。`g++.exe` を settings → PATH（相対パスのフォルダは見ない）→ 決まった場所（`%ProgramFiles%\mingw64\bin` → `%ProgramFiles(x86)%\mingw64\bin` → `C:\mingw64\bin` → `C:\msys64\ucrt64\bin`）の順に探す。版は確かめない（15.2.0 でなければ実行ログで知らせる。今までの macOS と同じ。利用者の選択）。出力の名前に `.exe`（cpp::output_path・example_output_path。run.rs の cpp_outputs で足していた処理は外した）。ヘッダーの場所の読み取り（header_dirs）は `/dev/null` の代わりに `-` を読ませ、標準入力を必ず空にする（利用者の選択。`/dev/null` のままでも、g++ は終了コード 1 で終わる前に一覧を出していたので、偶然動いていた）。作った exe（main と、生成器・愚直解・ジャッジの example）を動かすときだけ、子の PATH の先頭にコンパイラの bin を足す（run::Program の dll_dir。Python・Rust・test_child は足さない）。g++ の `-o` は、パッケージのフォルダからの相対パスにする（run::gcc_output。`\\?\` の形の `-o` だと 2 回目のビルドが落ちるため。4.x の「`\\?\` の付いたパス」の行）。g++ で exe を作るのは run::prepare_cpp の build_cpp だけで（実行・サンプルテスト・デバッグ用のビルド・ランダムテストの生成器と愚直解・対話テストのジャッジ）、全部これを通る。GCC_HINT を Windows 向けに（MinGW-Builds の GCC 15.2.0、settings.json の g++.exe のパス）。sdk_path は Windows では xcrun を起動しない。debugs_cpp_with_real_lldb_dap は、ほかのデバッグのテストと同じく Windows では ignore（段階 6） | 先にテストを書いて失敗を確かめる：この機械の g++.exe が見つかる／出力の名前が `.exe` で終わる／Windows の案内に brew が入らない／header_dirs が止まらずに終わり、終了コード 0 で一覧が取れる／`\\?\` の形のフォルダで同じパッケージを 2 回ビルドできる（main と example、提出用とデバッグ用。current_dir と exe の場所は `\\?\` のまま）。直す前は、それぞれ `Err("g++-15 が見つかりません…")`・`target\main`・`brew install gcc`・終了コード 1（`/dev/null: No such file or directory`）・2 回目のビルドが「ビルドに失敗しました」（ld の cannot open output file）で失敗した。前提の確かめ：PATH を System32 だけにすると 0xC0000135、bin を先頭に足すと動く。探す順と dll_dir のテストは新しい関数なので、直したあとに、順を逆にする・PATH を設定しないと落ちることを確かめてから戻した。header_dirs の標準入力を空にすることは、この環境ではテストの標準入力がもともと空（NUL）なので、外しても落ちないことを確かめられない（`Command::output` も標準入力を受け継がない）。S8 のあと、この機械で動き始めたテスト（g++ が見つかるようになったため）：builds_and_runs_with_local_gcc（Windows では、見つかった g++ が 15 でなければ飛ばす）・scaffold の C++ のテスト・connects_cpp_programs・finds_differences_with_cpp_programs。出力の `\r\n` と `.exe` は、テストの比べ方だけを Windows 向けにした（本番は S6 のとおり）。GitHub の windows-latest に g++ があるか、版がいくつかは未確認 |
| S9 | スタック（9.1 の 9・追加 6・追加 7） | Windows だけ。主スレッドのスタックの予約を 1 GiB（toolchain::STACK_SIZE）にする。C++ は `-Wl,--stack,1073741824` を、ビルドの命令（cpp::build_args_with。提出用・デバッグ用・example）だけに付ける（flags_with は clangd の解析の命令と共通なので入れない。compile_flags.txt にも付けない）。Rust は run.rs の `cargo build --bin … --example … --message-format=json` を、`cargo rustc [--release] --bin <名前> --message-format=json -- -C link-arg=<スタック>` と、example ごとの `cargo rustc --example <名前> …` に分けて順に動かす（`--` の後ろの引数は 1 つの対象にしか渡せないため。5.3。利用者の選択：RUSTFLAGS は依存を全部作り直し、利用者の RUSTFLAGS ともぶつかる。ビルドのあとで PE ヘッダーを書き換える案は、ビルドの出力を IDE が書き換えることになる）。リンクの引数は `rustc -vV` のホストで決める：msvc は `/STACK:1073741824`、gnu は `-Wl,--stack,1073741824`（gnu は未確認。この機械にない）、ほかは付けずに実行ログで知らせる（利用者の選択）。実行ログに「手元のビルドは、主スレッドのスタックを 1 GiB にしています（Windows だけ。ジャッジとは違う場合があります）」の 1 行を出す（Rust・C++）。標準出力（JSON）は命令ごとにつなぎ、bin と example が同じライブラリを使うときに cargo が命令ごとに出し直す同じ警告は、1 回だけにする（run::append_new_messages。外すと同じ警告が 2 回出ることを確かめた）。診断の知らせは、今までどおり 1 回だけ送る（全部の命令が通ったとき、または途中で失敗したとき。命令ごとに送ると、1 回目が bin が通った時点の空の一覧になり、finds_differences_with_real_programs・tests_interactively_with_real_programs が落ちた）。途中で失敗したら、あとの対象はビルドしない。Unix も、ビルドの命令（今までの `cargo build` 1 つ）を for で順に動かす形にした（動きは同じ）。時間（2026-11-03、release、example 2 つ〔生成器・愚直解〕、5 回・3 回の中央値）：何も変えずにもう一度ビルドすると、`cargo build` 1 回の 33 ms が `cargo rustc` 3 回で 115 ms（約 +80 ms）、main.rs を変えてビルドすると 237 ms が 298 ms（約 +60 ms）。このほかにビルドの前の `rustc -vV` が中央値 46 ms（36〜69 ms）。ランダムテストでは 1 回のビルドあたり約 +0.1 秒、example のない実行・サンプルテストでは約 +50 ms の見込み（推測。分けては測っていない） | 先にテストを書いて失敗を確かめる：C++ のビルドの引数に `-Wl,--stack` が入り、解析の命令と compile_flags.txt には入らない／約 100 MB の深い再帰（1 KiB ずつ 10 万段）の C++・Rust の main と example が、提出用とデバッグ用の両方で正常に終わり、PE ヘッダーの SizeOfStackReserve が 0x40000000（テストの中で exe を読む）。テストでは exe を process::run（Job と DIE、WerFault の見張り）で動かす（はじめは Job に入れずに動かしていたので、直す前の失敗のときに WER が deep.exe・main.exe のダンプを CrashDumps に残した。process::run にしたあと、スタックの指定を一時的に外して落とすと、0xC00000FD ですぐ終わり、ダンプは残らなかった）。直す前は、それぞれ `-Wl,--stack` なし・C++ も Rust も 0xC00000FD（予約は C++ が 2 MiB、Rust が 1 MiB）で失敗した。ホストからリンクの引数を決める処理の単体テスト。依存（パスで指定したライブラリ）は、IDE のビルド → `cargo build` → IDE のビルドの順に動かしても作り直さない（`"fresh": true`、.rlib の更新日時が変わらない。確認済み）。診断の知らせは、通ったときも、example が失敗したときも 1 回だけ |
| S10 | パスの表記 | rustc の診断の file は S10a で済ませた。logLinks.ts（実行ログの「ファイル名:行:列」のリンク）は、Windows だけ `\` も区切りとして読み、開くファイル名を `/` 区切りにする（`--> src\main.rs:3:5`・panic の `src\main.rs:5:10` が、`main.rs` でなく `src/main.rs` を開く）。直前が `\` の場所もパスの途中とみなし、`C:\…` の絶対パスと `..` を含むものはリンクにしない。Windows かどうかは navigator.userAgent で決める（keybindings.ts の IS_MAC と同じ読み方）。テストのために splitLocations(text, windows) と引数で渡せるようにした（Windows 以外は今までどおり `/` だけ）。画面のコードのテストの仕組みはなかったので、依存を足さずに、Node（.nvmrc の 24.22.0）の組み込みのテスト（node --test）で TypeScript をそのまま動かす tests/logLinks.test.ts を足し、package.json に `npm test`、CI（両方の OS）に npm test の段階を足した（利用者の選択。tests/ は src の外で、tsc の対象にしない）。scaffold::cargo_reads_created_packages_under_a_parent_workspace は、cargo metadata の場所（普通の形）と canonicalize した `\\?\` の形を比べていたので落ちていた。本番のコードは両方を canonicalize して比べている（toolchain.rs）ので、テストの比べ方だけを直し（`\\?\` を外して比べる）、ignore を外した。AC Library の `-I` には `\\?\` が付かないことをソースで確かめた（4.x の「`\\?\` の付いたパス」の行。AC Library を置いた画面の確かめは、リポジトリの外を変えるのでしない。利用者の選択） | 先にテストを書いて失敗を確かめる：Windows で `src\main.rs:3:5`・`src\main.rs:5:10`・`examples\gen.rs:1:5` が `/` 区切りの場所のリンクになる／`C:\x\main.rs:3:1`・`..\a.rs:1:1`・rustc が出した `/rustc/48a229ceaefd4985c50990b14116b6d856af0985/library\std\src\io\mod.rs:978:7` はどの部分もリンクにならない／`/` 区切りと C++ の `main.cpp:4:5:` は今までどおりリンクになる／Windows 以外は今までどおり。直す前は、Windows の 4 つ（`main.rs` だけがリンクになる、`main.rs:3:1`・`mod.rs:978:7` がリンクになる）で失敗した。直したあとに、直前が `\` ならパスの途中とみなす所を一時的に外すと、`C:\x\main.rs:3:1` の `x\main.rs` がリンクになって落ちることを確かめてから戻した（標準ライブラリの場所は、一番左の候補の直前が `/` なので、外しても落ちない）。scaffold のテストは、ignore を外して `\\?\` の違いで落ちることを確かめてから直した。CI の npm test が通るかは、push していないので未確認。画面の確かめ：t1 でコンパイルエラーと panic を起こし、実行ログの `src\main.rs:…` を押すと src/main.rs の正しい行が開く |
| S10c | 同時の保存の失敗（S10b のすぐあと。S5 の前） | Windows で全部のテストと一緒に動かしたとき 6 回中 1 回、storage::tests::concurrent_writes_do_not_mix（8 つのスレッドが同じファイルに write_atomic を 20 回ずつ）が「設定を保存できません」で落ちた（S10b の確かめの途中。段階 1 C7 の win::rename_replacing は、5・32 のとき 20 ms おきに 100 回〔約 2 秒〕試し直す）。まず write_atomic の失敗の文に、段階（一時ファイルを作る・書く・確定する・名前を変えて置き換える）と OS のエラーを入れ、rename_replacing は諦めたときに試し直した回数とかかった時間を入れる（Unix の失敗の文も変わる）。全部のテストと一緒に何回かくり返して、どのエラーで、何回目で諦めたかを確かめてから、直し方を決める。原因が分かるまで、待ちの時間を延ばすだけの直しはしない。**S10c は調べるための変更だけでコミットし、直すのは S10d。** 調べた結果（2026-11-01）：全部のテストを 29 回くり返して 1 回再現（S10b のときと合わせて 35 回中 2 回）。3 つのスレッドが同じ時刻に「名前を変えて置き換える：アクセスが拒否されました (os error 5)。100 回試し直した（2095〜2099 ms）」で諦めた。Rust 1.99.0 の fs::rename は MoveFileExW が 5 で失敗すると POSIX の決まりの置き換え（FileRenameInfoEx）も試すので、その両方が約 2 秒ずっと失敗したことになる。原因（ほかのもの〔Defender など〕が削除の共有を許さずに開いていたのか、自分のスレッドどうしの置き換えが重なったのか）は未確認 | 失敗の文から原因を確かめる。直したあと、全部のテストと一緒に何回もくり返して落ちない |
| S10d | 同時の保存の失敗への手当て（S10c のあと。9.1 の追加 19） | 原因は決めきれなかった（全部のテストを合わせて 50 回くり返して 2 回、試しのプログラム〔同じ形の同時の保存を、全部のテストと同時に 561 秒、約 54 万回の置き換え〕では 0 回）。(2) Windows だけ、write_atomic で同じパスへの保存を IDE の中で 1 つずつにする（storage::save_lock。鍵は渡されたパスの形のままで、同じファイルを別の書き方で渡すと別のロックになる）。(3) win::rename_replacing が 5・32 の試し直しを使い切って諦めたときは、「ほかのプログラム（ウイルス対策など）が使っている可能性があります」を足す。試し直しの回数・時間は変えない（測った長さを見てから相談する） | 先にテストを書いて失敗を確かめる：置き換える先を共有を許さずに開いておくと、案内の入った文で失敗する（直す前は、同じ「アクセスが拒否されました (os error 5)。100 回試し直した」で、案内がなかった）／同じパスには同じロック。直したあと、全部のテストを 30 回くり返す（2026-11-01：30 回とも通った。直す前は 50 回中 2 回落ちていたので、30 回で落ちなくても直ったとは言い切れない） |
| S11 | 問題の取得 | Windows だけ。problem.rs の fetch_public（問題ページ・問題の一覧をログインなしで取る）の `/usr/bin/curl` を、`%SystemRoot%\System32\curl.exe`（problem::windows_curl。PATH は探さない。SystemRoot が分からない・ファイルがないときは「curl.exe が見つかりません（…）」）にし、process::command で起動する（CREATE_NO_WINDOW を付ける）。引数は変えない。ログインなしで 404 のとき（開催中のコンテスト）は、Keychain（/usr/bin/security）を見に行かず、「…AtCoder にログインしていないと読めません。Windows 版では、まだログインできません」を返す（with_session_fallback の Windows だけの分かれ道。段階 3 でログインを入れるまで。利用者の選択）。macOS の文と動きは変えない。証明書の失効の確かめ（Schannel）の引数は足さない（利用者の選択。既知の問題に書いた）。ログインして読む部分（session.rs の curl と Keychain）は段階 3・5 の範囲で、reads_with_the_session_only_after_a_404_when_logged_in は ignore のまま | 先にテストを書いて失敗を確かめる：fetch_public が https://127.0.0.1:9/（つながらない場所）で、curl の「つながりません」（終了コード 7）の文になり、http://127.0.0.1:9/ は http を使わずに失敗する／Windows で 404 のとき、Keychain を見ずに「Windows 版では、まだログインできません」を返す。直す前は、それぞれ「curl を起動できません：指定されたパスが見つかりません。 (os error 3)」・「Keychain を使えません：指定されたパスが見つかりません。 (os error 3)」で失敗した。前提の確かめ：curl.exe も、--fail で 404 のとき「returned error: 404」を出す（127.0.0.1 の小さなサーバーで。is_not_found で見分けられる）。127.0.0.1 のつながらない場所で、curl.exe が失敗するまで 2057〜2076 ms（AC 電源。Windows は、つながらないと分かるまでに約 2 秒かかる）。テストは AC 電源で動かした。画面の確かめ：問題の選択欄で終わったコンテストの一覧と問題ページを取る、「コンテストを作る」、ないコンテストの 404 の文 |
| S12 | Defender の注意（9.1 の 8・追加 13） | はじめの案は、Windows だけ、ビルドの直後の 1 ケース目が TLE のとき、実行ログに 1 行の注意を出すことだった。先に測ると（4.5 の「1 回目と 2 回目」の行）、S5d から実行時間を再開したあとから数えるので、新しい exe の 1 回目の遅れ（全体で約 80〜110 ms）は、ほぼ実行時間の外に出ていて、実行時間の差は約 2 ms だった。1 回目だけ TLE になる場面はほとんどないので、注意は出さずに S12 を閉じた（利用者の選択。コードは変えていない）。押してから結果が出るまでの全体の時間は、1 回目だけ約 0.1 秒延びるが、ビルドの時間に比べて小さいので気にしない（利用者の判断）。測ったのは写した exe だが、ビルドで書いた直後の exe でも、画面の確かめで 1 回目だけ遅くなる様子はなかった（S8 の画面の確かめで、ビルドの直後のサンプルテストが 19 ms で AC、S9 の画面の確かめでも 77〜109 ms で AC。測っていないことの補い） | 測定（コミットしない一時的なテスト。測ったあとで消し、目印の件数が 0 で、git status に出ないことを確かめた）。AC 電源 |
| S13 | まとめ | この文書に段階 2 の結果。後片付け：judges_cases_with_a_real_program の一時フォルダ、%TEMP% に前から残っている atcoder-studio-* のフォルダ（2026-11-03 の時点で 14 個。どれも 2026-10-29 の日付。どのテストが残したかを調べてから扱う）。Job に入れるのが遅いこと（S5d の行）を速くする工夫をするかの案。電池で動いているときの時間のテスト：2026-11-04、ノート PC が電池（放電中・残り 45%・電源プラン「バランス」）のときに全部の cargo test を動かすと、全体が 33.69 秒・29.60 秒かかり（AC のときは約 15〜19 秒）、時間を確かめるテスト（run::tests::stops_a_case_at_the_time_limit〔1500 ms 未満〕、process::windows_tests::stops_both_on_request〔3 秒未満〕、process::windows_tests::stops_a_program_over_the_memory_limit_while_running〔2 秒未満〕）が 1〜3 つ落ちた。単独では 0.36 秒で通り、AC につないで全部を動かすと 297 通過・0 失敗（18.97 秒）。電池のときに CPU が遅くなり、一斉の起動が遅れるためと考えている（推測）。時間の確かめの余裕を見直すか、電源につないで動かすことを決まりにするかを S13 で決める。release でのコンソールの窓の確かめ：段階 2 で起動するようになったもの（Job に入れて動かす解答・ジャッジ、g++・Python、S11 の curl.exe など）を、release でビルドした IDE から動かして、コンソールの窓が出ないことを、まとめて確かめる（段階 1 の確かめの 6 と同じやり方。S11 で決めた）。→ 2026-11-05 に確かめた（A6。exe はエクスプローラーから起動。どの操作でも窓は出なかった。「S13 の release の確かめ」の節）。LSP・デバッグの子の Job の記録（S13 で足した。2026-11-04）：win.rs の JOBS は、run・run_pair では reap（release_job）で消すが、言語サーバーを止めるとき（lsp.rs の stop。kill_group のあと child.wait）と、デバッグのアダプターを止めるとき（debug.rs の shutdown・debugpy につなげなかったとき）は release_job を呼ばないので、その pid の記録（job: None と、起動したときのプロセスのハンドルの写し）が、IDE が終わるか同じ pid が使い回されるまで残る（コードで確認。win.rs のコメントには書いてあったが、この文書には書いていなかった）。言語サーバーを起動し直すたびに、記録とハンドルが 1 つずつ増える。S13 で、言語サーバーは直した（Server::stop で、回収のあとに Windows だけ release_job を呼ぶ。先に書いたテスト forgets_the_job_record_after_stopping_on_windows〔test_child を Server::spawn で起動して Server::stop で止める〕が「止めたあとも JOBS に pid 41248 の記録が残っている」で落ちることを確かめてから直した）。デバッグ（debug.rs の 2 か所。965 行あたりの debugpy につなげなかったときと、1225 行あたりの shutdown）にも同じ記録が残るが、Windows ではまだ動かせずテストで確かめられないので、段階 6 で、テストと一緒に直す（利用者の選択。段階 6 の行に書いた）。設定の画面の Python の欄の説明（S13 の B1・B2 の手順を書くときに見つけた。2026-11-05）：処理系のパスを書いていないときの説明は、Windows でも macOS の探し方（「/opt/homebrew/bin・PATH・/usr/local/bin から探します」）を出していた。S13 で、文を src/settings/pythonNote.ts の pythonSearchNote に移し、Windows だけ S7 の探し方（CPython はレジストリ〔PEP 514〕の 3.13 → PATH の python.exe、Microsoft Store の実行エイリアスは使わない。PyPy は PATH の pypy3.11.exe・pypy3.exe）の文にした。macOS の文は変えていない。先に書いたテスト tests/pythonNote.test.ts の Windows の確かめが「「レジストリ（PEP 514）」がない：処理系は /opt/homebrew/bin・…」で落ちることを確かめてから直した。画面の確かめ（2026-11-05。利用者が dev で。settings.json の python.cpython・python.pypy はどちらも空）：「設定…」の Python の欄の説明に Windows の文がそのまま出て、/opt/homebrew は出なかった。「処理系」を「PyPy 3.11」に切り替えても同じ文のまま。キャンセルで閉じた（settings.json は変えていない）。手順の書き間違い：欄の一覧を「左の」と書いたが、設定の画面の上に横一列（キー・色・表示・問題文・提出・Python・スニペット・テンプレート）に並んでいる | 全体の cargo test・cargo clippy --all-targets・npm run build |

段階 2 で利用者が手で確かめること（各コミットのあとに手順を書いて頼む）
- Rust・C++・CPython・PyPy のそれぞれで、AC・WA・RE（0 除算・範囲外・panic・アクセス違反）・TLE（無限ループ）・MLE（大きな配列）になる解答の判定と、時間・メモリ・RE の名前の表示
- 孫を作る解答を止めたあと、タスクマネージャーに子が残っていない
- 止めたあとすぐにビルドし直せる
- 問題の取得（問題ページ・問題の一覧）
- 深い再帰の解答が手元で落ちない（S9）：C++ と Rust の約 100 MB の深い再帰（1 KiB ずつ 10 万段）が「サンプルテスト」で通ること。
  実行ログに「手元のビルドは、主スレッドのスタックを 1 GiB にしています（Windows だけ。ジャッジとは違う場合があります）」の
  1 行が出ること（Rust・C++）。Rust のランダムテストで、生成器にコンパイルエラーを入れたとき、CE になり、エラー一覧に生成器の
  場所が正しく出ること（S9 でビルドを対象ごとに分けたため）

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

S7 の画面の確かめ（2026-11-03。利用者が Windows 11 で確かめた。dev、Python のパッケージ p1、S7 のコミットのあと。CPython だけ。
PyPy は settings.json の python.pypy の書き換えが要るので、あとで改めて決める）

| 番号 | 確かめたこと | 結果 |
| --- | --- | --- |
| 1 | 設定なしで CPython が見つかるか（`print(1)` のサンプルテスト） | OK。AC・40 ms。実行ログに「ローカル：Python 3.13.7（C:\Users\yunoa\AppData\Local\Programs\Python\Python313\python.exe）」と出た（PEP 514 のレジストリから見つかった） |
| 2 | 日本語と絵文字の出力（`print("あいう")`・`print("😀")` の実行） | OK。どちらもそのまま出て、正常に終了・41 ms（`-X utf8`。付けないと絵文字は UnicodeEncodeError になる。4.4） |
| 3 | 構文の誤りのメッセージの文字（`x = 1 ＋ 2`、全角の＋） | OK。CE。エラー一覧の文は「main.py:1:7: SyntaxError: invalid character '＋' (U+FF0B)」で、化けていなかった |
| 4 | 実行時の誤り（`print(1 // 0)` のサンプルテスト） | OK。RE・終了コード 1・75 ms |

S8 の画面の確かめ（2026-11-03。利用者が Windows 11 で確かめた。dev、S8 のコミット a3da2a4。C++ のパッケージ c1・c2・c3 を
作業領域 C:\Users\yunoa\Documents の直下に IDE で作った。c1 は test/sample-1.in〔`3` と `1 2 3`〕・sample-1.out〔`6`〕、
c2 は c1 と同じものに test/compare.json〔`{"mode": "exact"}`〕、c3 は sample-1.out が `こんにちは` の exact。
サンプルテストの制限は既定の 2500 ms・1024 MiB。g++ は C:\Program Files (x86)\mingw64\bin の MinGW-Builds 15.2.0）

手順と違えたこと：
- 番号 12 の「実行」は test/input.txt を標準入力に使うので、c1/test/input.txt（sample-1.in と同じ中身）を確かめの前に足した
- 番号 4・7 は、手順のコードの誤りで、IDE の不具合ではない。どちらも GCC の -O2 の最適化で、意図した動きが消えていた
  （IDE と同じ引数の `g++ -O2 -S` で確かめた）。直したコードでやり直した
  - 番号 4：手順の `volatile int z = 0; cout << 1 / z;` は WA（12 ms・出力 0）。int の `1 / z` は z が 1・−1 のとき以外は 0 に
    なるので、GCC は割り算（idiv）を使わず、`z + 1` を符号なしで 2 と比べて 0 か z を選ぶ式（cmova）に書き換えていた。z = 0 では
    割り算が起きない。直したコード `volatile int a = 1, z = 0; cout << a / z << endl;` では idivl があり、0xC0000094 になった
  - 番号 7：手順の `vector<char> v(1500000000, 1); cout << (int)v.back();` は WA（15 ms・4.6 MiB）。値が分かるので、GCC は確保
    （operator new・memset）ごと消し、定数 1 を出すだけにしていた。直したコード（入力の n で大きさを決めるので消せない）：
    `int n; cin >> n; size_t m = (size_t)n * 500000000; vector<char> v(m); for (size_t i = 0; i < m; i += 4096) v[i] = (char)(i + n);
    long long s = 0; for (size_t i = 0; i < m; i += 4096) s += v[i]; cout << s << endl;`（n = 3 で約 1.5 GB。operator new と memset が残る）

| 番号 | 確かめたこと | 結果 |
| --- | --- | --- |
| 1 | AC（入力の和） | OK。AC・19 ms・5.1 MiB |
| 2 | WA（和 + 1） | OK。WA・19 ms・4.7 MiB |
| 3 | RE（範囲外の書き込み。`v[100000000000]`） | OK。RE・0xC0000005（アクセス違反：範囲外・null の読み書きなど）・20 ms・4.1 MiB |
| 4 | RE（0 での割り算。直したコード） | OK。RE・0xC0000094（整数の 0 除算）・14 ms・4.0 MiB（手順のコードでは WA。上のとおり） |
| 5 | RE（深い再帰。1 KiB ずつ 10^7 段） | OK。RE・0xC00000FD（スタックあふれ：深い再帰など）・58 ms・6.0 MiB（今は既定のスタック 2 MiB。S9 のあとも、この深さなら落ちる見込み） |
| 6 | TLE（volatile の数を増やし続ける） | OK。TLE「時間の制限を超えたので止めました」・2515 ms・3.7 MiB。タスクマネージャーで、動いている間は main.exe があり、TLE のあと消えた |
| 7 | MLE（約 1.5 GB を確保して書く。直したコード） | OK。MLE「メモリの制限を超えました」・512 ms・1092.2 MiB（手順のコードでは WA。上のとおり） |
| 8 | abort | OK。RE・0xC0000409（異常終了：C++ の abort・assert・捕まえない例外、Rust の abort など）・228 ms・3.7 MiB |
| 9 | assert（`assert(n == 2)`、n = 1） | OK。RE・0xC0000409・117 ms・3.8 MiB |
| 10 | 捕まえない例外（`throw runtime_error`） | OK。RE・0xC0000409・120 ms・4.3 MiB |
| 11 | vector の .at の範囲外 | OK。RE・0xC0000409・119 ms・4.2 MiB |
| 12 | 同じパッケージを続けて「実行」・「サンプルテスト」（`\\?\` の件） | OK。それぞれ 3 回続けた。毎回ビルドが通り、実行は 6・正常に終了（21 ms）、サンプルテストは AC。「ビルドに失敗しました」や cannot open output file は出なかった |
| 13 | exact の比較（c2。MinGW の exe は `6\r\n` を出す） | OK。AC・13 ms・5.1 MiB（S6） |
| 14 | 日本語と絵文字の出力（c3） | OK。実行：「こんにちは」「絵文字😀」が化けずに出た・正常に終了（15 ms）。サンプルテスト：WA（期待出力が 1 行だけなので正しい）・14 ms・4.6 MiB。実際の出力の 2 行とも化けていなかった |
| 15 | 実行ログの「ローカル」と「AC Library」 | OK。「ローカル：g++.exe (x86_64-win32-seh-rev0, Built by MinGW-Builds project) 15.2.0（C:\Program Files (x86)\mingw64\bin\g++.exe）」。版が違うという注意は出なかった。「AC Library：なし（使うときは、アプリのフォルダの libraries/ac-library/atcoder/ に置いてください）」 |
| 16 | 残っているもの（タスクマネージャーの「詳細」） | OK。main.exe・g++.exe・cc1plus.exe・ld.exe・collect2.exe・WerFault.exe は、すべて残っていなかった |

S9 の画面の確かめ（2026-11-04。利用者が Windows 11 で確かめた。dev、S9 のコミット 02b4945 のあと。作業領域 C:\Users\yunoa\Documents
の t1〔Rust〕と c1〔C++〕を使った。どちらも、入力 n〔100000〕から読んだ深さだけ、1 段ごとに 1 KiB の配列〔C++ は volatile、Rust は
std::hint::black_box〕を置いて再帰し、各段の n の偶奇を足して出す〔50000〕。test/sample-1.in・test/input.txt は `100000`、
sample-1.out は `50000`。サンプルテストの制限は既定の 2500 ms・1024 MiB）

手順のコードは、IDE と同じ引数（g++ -O2、cargo の release）のアセンブリで、再帰の呼び出しとスタックの確保が残っていることを
確かめてから渡した。Rust の f は 1 段 1064 バイト（10 万段で約 101.5 MiB。既定の 1 MiB では足りない）。C++ の f は、g++ の -O2 が
f を自分の中に 2 段埋め込み、1 回の呼び出しで n・n−1・n−2 の 3 段（buf を 48・1072・2096(%rsp) に置く）を処理して n−3 から再帰
していた。`subq $3120, %rsp` は 3 段分の確保で、10 万段は約 3 万 3 千回の呼び出し、合わせて約 104 MB（既定の 2 MiB では足りない）。
手順を書いたときは、3120 バイトを 1 段分と読み違えて「約 300 MB」の見込みを書いたが、測った 104.8 MiB と合うのは 3 段分の読み方。
-ftrivial-auto-var-init=zero で各段の buf は memset で 0 にされるので、確保したスタックは全部書かれ、使用メモリに入る。
1 GiB を付けて作った exe にファイルで 100000 を渡すと、どちらも 50000 で終了コード 0（Job の外では落とさず、ダンプを残さない
ように確かめた）。利用者が t1/src/main.rs を貼るときに 1 行目の `use std::io::Read;` が抜けて CE になったので、足してから確かめた
（手順の誤りではない）

| 番号 | 確かめたこと | 結果 |
| --- | --- | --- |
| 1-1 | Rust の深い再帰（t1 のサンプルテスト） | OK。AC・109 ms・108.6 MiB（S9 の前なら 0xC00000FD） |
| 1-2 | Rust の深い再帰（t1 の実行。test/input.txt） | OK。50000、正常に終了・90 ms |
| 2-1 | C++ の深い再帰（c1 のサンプルテスト） | OK。AC・77 ms・104.8 MiB（S9 の前なら 0xC00000FD。使用メモリは上の 3 段分の読み方と合う） |
| 2-2 | C++ の深い再帰（c1 の実行。test/input.txt） | OK。50000、正常に終了・100 ms |
| 3 | 実行ログのスタックの 1 行（Rust・C++） | OK。どちらも、書いた場所（Rust は「ジャッジ：rustc …」のあと、C++ は「手元のビルドは、ジャッジと違い…」のあと、「AC Library：なし…」の前）に「手元のビルドは、主スレッドのスタックを 1 GiB にしています（Windows だけ。ジャッジとは違う場合があります）」が出た |
| 4-1 | Rust のランダムテスト（t1 に examples/gen.rs〔seed % 1000 + 1〕と examples/naive.rs〔(n + 1) / 2〕を足した） | OK。「生成器 examples/gen.rs、愚直解 examples/naive.rs、最大 100 回」、100 回すべて一致。ビルドは bin（0.09s）・gen（0.40s）・naive（0.32s）の 3 回に分かれていた（全体の時間は測っていない） |
| 4-2 | 生成器にコンパイルエラー（`fn main() { let x: i32 = "a"; }`） | OK。CE。エラー一覧に examples/gen.rs:1 の error[E0308]: mismatched types。クリックすると gen.rs の 1 行目が開いた。実行ログでは、bin のビルドが通ったあと、gen で止まった |
| 4-3 | 生成器を直したあとのランダムテスト | OK。エラー一覧から消え、4-1 と同じく 100 回すべて一致 |

S10 の画面の確かめ（2026-11-04。利用者が Windows 11 で確かめた。dev を起動し直してから、S10 のコミット 6981744 のあと。
作業領域 C:\Users\yunoa\Documents の t1〔Rust〕と c1〔C++〕に、わざとコンパイルエラー・panic を起こすコードを貼った。
出る文は、同じコードを scratchpad でビルド・実行して確かめてから手順に書いた）

手順の前提の誤り：手順では「S10 の前は、実行ログの `--> src\main.rs:5:22` の `main.rs:5:22` の部分だけがリンクだった」と
書いたが、誤り。Rust のビルドは `--message-format=json` で受け取るので、rustc の診断の本文（`--> …` を含む）は標準出力の JSON
として出て、エラー一覧（rendered。リンクなし。BottomPanel.tsx の diagnostic-rendered）だけに出る。実行ログに送るのは cargo の
標準エラー（`Compiling …`・`error: could not compile …` などの進み具合）だけなので、Rust のコンパイルエラー・警告の場所は、
S10 の前も後も、macOS でも Windows でも、実行ログには出ない。そのため 1-a・3 は、確かめる場面がない項目だった。
エラー一覧の見出しのリンクは列を渡さない（BottomPanel.tsx の column: null。そもそもエラー一覧の Diagnostic に列の欄がない）ので、
行の一番左が開く。これも macOS と同じ作り（下の「移植の範囲の外の覚え書き」）。C++ の g++ の標準エラーは、実行ログ（build の
種類）とエラー一覧の両方に出る。ビルドが CE で終わると画面が自動でエラー一覧に切り替わる（App.tsx の setPanelTab("errors")）
ので、はじめの 4 ではエラー一覧を見ていた。実行ログのタブに切り替えて見直すと、期待どおりだった。
S10 が効くのは、Rust のプログラムが実行中に標準エラーへ出す場所（panic・dbg! など。file!() が `\` 区切りのため）

| 番号 | 確かめたこと | 結果 |
| --- | --- | --- |
| 1-a | Rust のコンパイルエラー（`std::io::stdin().read_to_string();`）の `--> src\main.rs:5:22` | 確かめる場面がなかった（上の前提の誤り）。文はエラー一覧の本文に出て、下線はなかった。エラー一覧の見出しの場所を押すと、src/main.rs の 5 行目の一番左が開いた |
| 1-b | 同じエラーの `note: method defined here` の `--> /rustc/48a229ceaefd4985c50990b14116b6d856af0985/library\std\src\io\mod.rs:978:7` | OK（どこにも下線がない）。ただし、エラー一覧の本文はもともとリンクを付けないので、S10 の直しの確かめにはなっていない（S10 の直しは tests/logLinks.test.ts で確かめた） |
| 2 | Rust の panic（範囲外の添字）の `src\main.rs:4:21` | OK。実行ログに「thread 'main' (44980) panicked at src\main.rs:4:21:」「index out of bounds: the len is 3 but the index is 10」、終了コード 101・30 ms。下線を押すと、src/main.rs の 4 行目の v と [i] の間（21 列目）が開いた |
| 2b | Rust の dbg!（`fn main() { let x = 1; dbg!(x); }`）の `[src\main.rs:3:5]` | OK。実行ログの `src\main.rs:3:5` に下線があり、押すと src/main.rs の 3 行目の dbg! の直前（5 列目）が開いた |
| 3 | ランダムテストの生成器のコンパイルエラーの `--> examples\gen.rs:1:26` | 確かめる場面がなかった（1-a と同じ理由）。エラー一覧の見出しの場所を押すと、gen.rs の 1 行目の一番左が開いた |
| 4 | C++ のコンパイルエラー（`cout << m`）の `main.cpp:6:13: error: 'm' was not declared in this scope` | OK。実行ログのタブで、灰色の g++ の出力の中の main.cpp:6:13 に下線があり、押すと main.cpp の 6 行目の m の直前（13 列目）が開いた（はじめは、CE で自動で切り替わったエラー一覧を見ていたので、下線がないと見えた） |

S11 の画面の確かめ（2026-11-04。利用者が Windows 11 で確かめた。dev を起動し直してから、S11 のコミット 47fcbe2 のあと。
AtCoder の公開ページにつないだ。この機械の Windows 版では、それまで一度も取得できていなかった）

手順の書き間違い：2 の知らせを「赤い知らせ」と書いたが、誤り。サイドバーの下の知らせ（App.css の .sidebar-error）とエディタの
上の知らせ（.editor-error）は、どちらも color: var(--fg-secondary)（#9d9d9d の灰色。src/styles/tokens.css）で、OS ごとの分かれ道は
ない。最初のコミット（309895d）からの書き方で、移植では App.css・tokens.css を変えていないので、macOS でも同じ灰色。
右の列（問題文の欄）は、はじめ隠れていた（前に細くしすぎていたため。利用者が右端の境目を左へドラッグして出した）

| 番号 | 確かめたこと | 結果 |
| --- | --- | --- |
| 1-2 | `abc400` の問題の一覧（t1/src/main.rs を開いて、ツールバーの左端の欄に入れて「一覧」） | OK。「問題を選ぶ」に A〜G が並んだ。1 秒未満 |
| 1-3 | abc400_a の問題ページ（「問題を選ぶ」から A） | OK。右の列の問題文の欄に、問題名・制限・問題文・サンプルが出た。1 秒未満（t1 に abc400_a が結び付けられて保存された） |
| 1-4 | 問題文の欄の「再取得」 | OK。もう一度取って、同じ問題が出た |
| 2 | ないコンテスト `abc999` の 404（「一覧」） | OK。サイドバーの下に「ページが見つかりません（404）。開催中のコンテストの問題は、AtCoder にログインしていないと読めません。Windows 版では、まだログインできません」がそのまま出た（文字は灰色。上の書き間違い） |
| 3-2 | 「コンテストを作る」（ファイル一覧の見出しの ⤓ → `abc400` →「作る」） | OK。「abc400：7 問のうち 7 問を作りました」（時間は測っていない） |
| 3-3 | できたフォルダ | OK。作業領域の直下の C:\Users\yunoa\Documents\abc400\ の中に abc400_a 〜 abc400_g |
| 3-4 | 作ったパッケージを開いたときの問題文 | OK。abc400_b の src\main.rs（Rust なので main は src の中）を開くと、右の列にその問題の問題文が出た |
| 4 | 開催中のコンテスト（AHC074）の 404 | OK。2 と同じ文が出た |

S13 の画面の確かめ（B1：PyPy、B2：残りの言語とテストの種類。2026-11-05。利用者が Windows 11 で確かめた。dev、AC 電源、
S13 のコミット c531943 のあと。Rust のパッケージ t1、Python の p1、C++ の c1。手順のコードは、Rust の 7 つと Python の 4 つを、
先に scratchpad で動かして確かめた。時間とメモリは書き留めていない）

手順の誤り：
- 4-0・4-7 で、settings.json の `"pypy"` を外で書き換えてから、設定の画面で処理系を選んで保存するように書いたが、IDE を開いたまま
  settings.json を書き換えると、設定の画面の保存は断られる（commands.rs の save_settings が、最後に読んだあとに外で変わっていれば
  書かずに ChangedOutside を返す。コードで確認）。利用者は、IDE を閉じてから `"interpreter"` と `"pypy"` の 2 行を直接書き換え、
  設定の画面は見るだけで保存しなかった。確かめのあと、2 行とも元に戻した
- 5-1 の examples/gen.cpp を `int main(int argc,char**argv)` と書いたので、argc を使っておらず、`-Wextra` の警告
  `unused parameter 'argc' [-Wunused-parameter]` が出た（判定には関係しない）。`int main(int, char**argv)` と書けば出ない

| 番号 | 確かめたこと | 結果 |
| --- | --- | --- |
| 1-1 | Rust のサンプルテストの AC（入力 3、期待出力 6） | OK。AC |
| 1-2 | Rust の WA | OK。WA |
| 1-3 | Rust の範囲外の添字（panic） | OK。RE・終了コード 101（panic） |
| 1-4 | Rust の 0 での割り算（panic） | OK。RE・終了コード 101（panic） |
| 1-5 | Rust の MLE（入力で大きさを決める 1.5 GB の確保） | OK。MLE・メモリの制限を超えました |
| 2 | Rust の対話テスト（examples/judge.rs、test/interactive-1.in が 7、interactive-2.in が 42） | OK。interactive-1 が AC、interactive-2 が WA。interactive-1 を開くと、やり取りが時系列で並び、`! 7` で終わっていた |
| 3-1 | CPython のサンプルテストの AC | OK。AC |
| 3-2 | CPython の WA | OK。WA |
| 3-3 | CPython の範囲外（IndexError） | OK。RE・終了コード 1（1 回目は、利用者が 3 行目を貼り漏れて、何も出さずに終わったので WA だった。貼り直して RE。手順の誤りではない） |
| 3-4 | CPython の TLE | OK。約 2.5 秒で TLE |
| 3-5 | CPython の MLE | OK。MLE |
| 3-6 | CPython のランダムテスト（生成器 gen.py・愚直解 naive.py） | OK。実行ログは期待どおり。7 回目で WA になり、test に random-1.in・random-1.out ができた |
| 3-7 | CPython の対話テスト（ジャッジは Python） | OK。interactive-1 が AC、interactive-2 が WA。やり取りの記録に `\r` などの記号は見えなかった |
| 4-0 | PyPy を処理系にする（上の手順の誤りのとおり、IDE を閉じて settings.json を書き換えた） | OK。設定の画面の「処理系」が PyPy 3.11 で、説明にパスが出た |
| 4-1 | PyPy の AC と「ローカル：」の行 | OK。AC。「ローカル：Python 3.11.13 (413c9b7f57f5, Jul 03 2025, 18:04:37) [PyPy 7.3.20 with MSC v.1941 64 bit (AMD64)]（C:\Users\yunoa\AppData\Local\Programs\pypy3.11-v7.3.20-win64\pypy3.exe）」。版が違うという注意は出なかった |
| 4-2 | PyPy の WA | OK。WA |
| 4-3 | PyPy の 0 での割り算（ZeroDivisionError） | OK。RE・終了コード 1 |
| 4-4 | PyPy の範囲外（IndexError） | OK。RE・終了コード 1 |
| 4-5 | PyPy の TLE | OK。TLE |
| 4-6 | PyPy の MLE | OK。MLE |
| 4-7 | 元に戻したあと | OK。実行ログの「ローカル：」が Python 3.13.7 に戻った |
| 5-1 | C++ のランダムテスト（生成器 gen.cpp・愚直解 naive.cpp） | OK。1 回目は、エラー一覧に「examples/gen.cpp:2:14: warning: unused parameter 'argc' [-Wunused-parameter]」が出た（上の手順の誤り）が、ランダムテストは動いた。2 回目は警告が出なかった（下の注）。3 回目のケースで WA になり、test に random-1.in・random-1.out ができた |
| 5-2 | C++ の対話テスト（examples/judge.cpp） | OK。interactive-1 が AC、interactive-2 が WA |

5-1 の 2 回目に警告が出なかったわけは未確認。コードでは、C++ は「ランダムテスト」を押すたびに、main.cpp と examples の
gen.cpp・naive.cpp をビルドし直し（run.rs の prepare_cpp。前の出力を使い回す仕組みはない）、それまでのビルドの診断を合わせて
エラー一覧に送る（build_cpp の RunEvent::Diagnostics。画面は App.tsx で、実行を始めるときに消し、届くたびに置き換える）。
同じ gen.cpp を、この機械の g++（C:\Program Files (x86)\mingw64\bin）に同じ警告のオプションで 2 回続けて渡すと、2 回とも同じ警告が
出た（scratchpad で確かめた）。そのため、gen.cpp を作り直さなかったからではない。2 回目を押す前に gen.cpp が変わっていたか、
エラー一覧を見た時点が違ったかは分からない

S13 の release の確かめ（A6：コンソールの窓。2026-11-05。利用者が Windows 11 で確かめた。92da88f を `npm run tauri build -- --no-bundle`
〔AC 電源、5 分 11 秒〕でビルドした src-tauri\target\release\atcoder-studio.exe〔10,094,592 バイト〕を、dev を閉じてから、エクスプローラーで
ダブルクリックして起動した。ターミナルから起動すると、子がそのコンソールを受け継いで窓が見えなくなるおそれがあるため。段階 1 の
確かめ〔画面の操作の確認の 6〕でどう起動したかは未確認。インストーラーは作っていない）

見たもの：操作のたびに、黒い窓（コマンドプロンプト・Windows ターミナル）、タスクバーに一瞬増えるアイコン、5 では Windows のエラー報告の
ダイアログ（「動作を停止しました」など）と「Microsoft Visual C++ Runtime Library」のダイアログ。パッケージは B1・B2 と同じ t1・c1・p1

| 番号 | 操作 | 動く子 | 結果 |
| --- | --- | --- | --- |
| 0-3 | ダブルクリックで起動 | — | OK。IDE の窓だけが出た |
| 1-1 | Rust の対話テスト | cargo・rustc -vV・解答・ジャッジ（Job） | OK。窓は出なかった |
| 1-2 | Rust のサンプルテスト（AC） | cargo・解答 | OK。窓は出なかった |
| 1-3 | Rust の TLE（`fn main() { loop {} }`） | 解答 | OK。窓は出なかった |
| 1-4 | 「実行」を「停止」で止める | 解答 | OK。窓は出なかった |
| 1-5 | 言語サーバーの再起動 | rust-analyzer | OK。窓は出なかった |
| 2-1 | C++ の対話テスト | g++（版・ビルド）・解答・ジャッジ | OK。窓は出なかった |
| 2-2 | C++ のランダムテスト | g++・解答・生成器・愚直解 | OK。窓は出なかった |
| 2-3 | C++ の実行 | g++・解答 | OK。窓は出なかった |
| 3-1 | CPython の対話テスト | python（版の確かめ・-X utf8 での実行） | OK。窓は出なかった（利用者が途中で main.py の解答を消したので、B2 の 3-7 と同じコードを貼り直してから確かめた） |
| 3-2 | CPython のランダムテスト | python | OK。窓は出なかった |
| 3-3 | CPython のサンプルテスト | python | OK。窓は出なかった |
| 4-1 | abc401 の一覧 | curl.exe | OK。窓は出なかった（abc400 は保存済みで、一覧・問題ページは保存したものを使い curl.exe を動かさない〔problem.rs の load_problem・load_task_list〕ので、まだ取ったことのない abc401 にした） |
| 4-2 | abc401_a の問題ページ | curl.exe | OK。窓は出なかった |
| 4-3 | 問題文の欄の「再取得」 | curl.exe | OK。窓は出なかった |
| 5 | C++ の abort（`int main(){std::abort();}` を「実行」） | g++・解答 | OK。実行ログの最後に「終了コード 0xC0000409（異常終了：C++ の abort・assert・捕まえない例外、Rust の abort など）で終了しました（実行時間 114 ms（ローカルの計測））」。黒い窓も、エラー報告のダイアログも、Microsoft Visual C++ Runtime Library のダイアログも出なかった（ReportArchive の記録が 1 つ増えることは、利用者が了承した。数えてはいない） |

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
  （画面の表示）にも `\r` が残る（同じテストで、1 行目が `"1 100\r\n"` だった。S13 の画面の確かめの 3-7〔CPython の解答と
  ジャッジ〕では、画面のやり取りの記録に `\r` などの記号は見えなかった。画面が `\r` を取り除いたのか、見えない文字として出したのかは未確認）
- 証明書の失効の確かめ（S11 で扱わないと決めた）。Windows の curl.exe（Schannel）は、https の証明書が失効していないかを
  ネットワークで確かめる。会社などのネットワークで、その確かめの先（CRL・OCSP）につながらないと、問題の取得が失敗することが
  ある（推測。この機械では起きていない）。そのときは、session::curl_failure の終了コード 35 の文（「安全な接続（TLS）を
  始められませんでした…」）か、curl の文がそのまま出る。`--ssl-revoke-best-effort`（確かめの先につながらないときだけ、失効の
  確かめを飛ばす）は、安全さを下げるので付けない

移植の範囲の外の覚え書き（今後の案。Windows 特有ではないので、移植では扱わない。2026-11-04 に利用者が決めた）

- エラー一覧の見出しの場所のリンクは、列を渡さない（BottomPanel.tsx の DiagnosticsView の column: null）。そもそも、エラー一覧に
  送る診断（run.rs の Diagnostic）に列の欄がない（level・message・rendered・file・line だけ）。そのため、押すと行の一番左が開く。
  rustc の JSON（spans の column_start）と g++ の `ファイル:行:列:` には列があるので、Diagnostic に列を足して渡せば、列まで開ける
- エラー一覧の本文（rendered）には、リンクを付けていない（BottomPanel.tsx の diagnostic-rendered は文字のまま）。本文の中の
  `--> src/main.rs:5:22` や、note の場所（同じパッケージの別の行など）も、実行ログと同じく splitLocations で押せる場所にできる。
  付けるなら、標準ライブラリなどパッケージの外の場所はリンクにしない今の決め（logLinks.ts）をそのまま使う

段階 2 のあとの案（段階 2 では直さない。S13 で利用者が決めた。2026-11-04）

- 子を Job に入れて再開するまでの時間（S5d の行）。今は、CREATE_SUSPENDED で起動し、Job に入れ、Toolhelp32 で機械全体の
  スレッドの一覧を取って主スレッドを探し（約 47 ms）、ResumeThread する。標準ライブラリは主スレッドのハンドルを閉じてしまうので、
  一覧から探している。S12 の測定でも、押してから結果が出るまでの全体は、実行時間より約 70 ms 長かった。案（どれも未確認）：
  (a) 自分で CreateProcessW を呼んで主スレッドのハンドルを受け取り、一覧を取らずに再開する（標準ライブラリの Command の代わりに
  なるので、標準入出力のパイプ・環境・引数の引用を自分で扱うことになる）、(b) PROC_THREAD_ATTRIBUTE_JOB_LIST（Windows 10 以降）で、
  起動と同時に Job に入れる（止めて起動して再開する手順そのものが要らなくなる。これも CreateProcessW を自分で呼ぶ必要がある）、
  (c) ntdll の NtResumeProcess で、一覧を取らずにプロセスのスレッドを再開する（公開されていない関数なので、勧めにくい）
- GUI から起動した子のコードページ（S3 の画面の確かめの「ほか」の行・4.4）。解答が起動した ping.exe は、日本語の部分を CP932 で
  出し、IDE は UTF-8 として読むので、実行ログで日本語の部分だけが文字化けした。解答そのもの（Rust・C++〔MinGW〕・Python の
  -X utf8）は UTF-8 で出すので、起きるのは、解答がさらに Windows のコマンドを起動したときなど。案：(a) 今のまま（AtCoder の
  ジャッジは UTF-8 なので、解答の出力は UTF-8 を前提にする）、(b) 出力が UTF-8 として読めないときだけ CP932 として読み直す
  （推し量りが外れると、別の化け方になる）。(a) を勧める（未確認）

### 段階 2 の結果（2026-11-05）

ブランチ windows-port-stage2。main からのコミットは、e800d5d までで 33 個（9545504〜）。origin/windows-port-stage2 には、
2026-11-06 に e800d5d まで push した（S7 の 1df2e10 のあとの 16 個〔a3da2a4〜e800d5d〕。--force なし）。
最後の確かめ（Windows、AC 電源、62daae6）：cargo fmt --check 差なし、cargo clippy --all-targets 警告 0、cargo test 301 通過・0 失敗・
23 無視、npm test 9 通過・0 失敗、npm run build 通過。一つひとつの直し方・直す前の失敗・測った値は、上の段階 2 の表の各行と
コミットのメッセージにある

コミット（段階ごと。並びはコミットの順）

| 段階 | コミット | 入れたもの |
| --- | --- | --- |
| S1 | 9545504 | この文書（段階 2 の計画・決めたこと） |
| S2 | 8271399・eeef85e | テスト用の子のプログラム（examples/test_child.rs。テストの中で rustc で作る）と、process.rs の Windows のテスト |
| S3 | 83f40ac | 子を Job Object に入れ、止めるときは子孫ごと止める |
| S3b | d4c1abf | 止めた直後のビルドの前に、出力の exe が書けるまで待つ |
| S4 | 66d2d4f | 使用メモリを WorkingSet で測る（MLE） |
| S10a | bc85242 | rustc の診断の場所の区切りを / にする |
| S10b | 5b782ca・6c58873 | 対話テストの、先に失敗した側の決め方（**Unix の動きも変わる**） |
| S10c・S10d | ede085a・25370f1 | 同時の保存の失敗の文に段階と OS のエラーを入れ、Windows では同じパスへの保存を 1 つずつにする |
| S5〜S5d | 78d1796・1e4964c・07939e6・dbd91c8 | 例外の終了コードの名前、扱われない例外と __fastfail で落ちた解答がすぐ終わる、実行時間を再開のあとから数える |
| S6 | db1c4e9 | exact の比較と期待出力の保存の前に \r\n を \n に直す |
| S7 | 1df2e10 | CPython をレジストリから見つけ、Python に -X utf8 を付ける |
| S8 | a3da2a4・1a93f87 | MinGW の g++ を見つけてビルドし、作った exe を動かす。画面の確かめ |
| S9 | 02b4945・1b0bb59 | 手元のビルドの主スレッドのスタックを 1 GiB にする。画面の確かめ |
| S10 | 6981744・9417665 | 実行ログの `\` 区切りの場所をリンクにする、npm test（node --test）を足す。画面の確かめ |
| S11 | 47fcbe2・b50904c | curl.exe で問題ページと問題の一覧を取る。画面の確かめ |
| S12 | 6ec7297 | Defender の注意を、測った結果で取りやめる（コードは変えていない） |
| S13 | 9cbe099・c531943・c2f837f・92da88f・62daae6・このまとめ | 決めたこと、言語サーバーの Job の記録、B1・B2 の画面の確かめ、設定の画面の Python の説明、release の確かめ（A6）、このまとめ |

push と CI（段階 2 の間に、利用者の許可を得て push したもの）

- CI の結果を見たもの（どれも macos-latest・windows-latest とも成功。そのときの報告から）：run 41308130068（d4c1abf）、
  run 41312029609（bc85242）、run 41313266501（5b782ca。macOS の cargo test の段階も成功）、run 41446504987（6c58873）
- run 全体の結果だけを見たもの（どれも completed・success。2026-11-06 に公開の API で読んだ。OS ごと・段階ごと・テストごとの結果は
  見ていない）：run 41583129511（#20。dbd91c8、S5d）、run 41585328789（#21。db1c4e9、S6）、run 41590809129（#22。1df2e10、S7）
- d4c1abf より前の push と、78d1796〜07939e6（S5〜S5c）を上げた push の CI の結果は、まとめの中では確かめていない。push のあとの CI で
  まとめて確かめる
- npm test の段階（S10 で ci.yml に足した）は、run #23 で初めて CI で動き、両方の OS で通った
- 段階 2 の最後の CI：run 42210961555（#23。e800d5d）。macos-latest・windows-latest とも、全部の段階が success（公開の API で確認）。
  macos-latest のログの中身は、表のあとの「run #23 の macos-latest のログ」にある。windows-latest のログの中身は見ていない

macOS / Unix の経路を変えた所（cfg・属性を付ける以外の変更。コミットのメッセージと `git diff main` で確かめた。手元の Mac では
何も確かめていない。CI の macos-latest で確かめた結果は、表のあとの「run #23 の macos-latest のログ」にある）

| コミット | 変えた所 | Unix の動き | push のあとの CI（macos-latest）で見ること |
| --- | --- | --- | --- |
| **5b782ca（S10b）** | process.rs の run_pair・spawn_relay：中継のスレッドが EOF を読んだら、相手の標準入力を閉じる前に PairEvent::OutputClosed を送る。両方がこちらが止める前に異常に終わったときは、出力が先に閉じた側を先に失敗した側とする | **変わる**（対話テストで両方が異常に終わったときの、先に失敗した側） | **いちばん大事**。cargo test の段階で、process::tests::blames_the_side_whose_output_closed_first（/bin/sh で順番を作る）と interactive の対話テストが通ること。このコミットの CI（run 41313266501）では通ったが、そのあと process.rs の run・run_pair を S5c・S5d で変えた（Windows の cfg の中だけ）ので、もう一度見る |
| **6c58873（S10b の直し）** | 同じ run_pair：決め直すのは、先に閉じた側の閉じた知らせが、相手の終わりの知らせより先に届いていたときだけにする | **変わる**（出力の閉じた知らせが遅れたとき、終わった順のまま） | **いちばん大事**。process::tests::keeps_the_exit_order_when_output_closes_late と、書き直した blames_the_side_whose_output_closed_first が通ること（run 41446504987 では通った） |
| d4c1abf（S3b） | run.rs：cargo metadata の結果を変数に取っておく形、C++ の build_cpp(...)? を let built = …; built?; に分けた | 同じ | cargo build・clippy・test が通ること（run 41308130068 では通った） |
| ede085a（S10c） | storage.rs の write_atomic：失敗の文に段階と OS のエラーを入れた（「設定を保存できません（名前を変えて置き換える：…）」） | 文だけ変わる（動きは同じ） | storage のテストが通ること |
| 78d1796（S5） | run.rs の exit_code_text を共通にし、RunOutcome::Exited に label（画面に出す文字）を足した。history.rs、interactive.rs・random.rs の文、画面（ipc/run.ts の exitLabel・runLog.ts・TestResults.tsx） | 同じ（Unix の終了コード 0〜255 は今までどおり「終了コード N」。前の版の実行履歴は label がなくても今までの形） | run・interactive・random のテストと npm run build が通ること |
| 1e4964c（S5b） | runLog.ts：label が「）」で終わるときは、「で終了しました」の前の空白を入れない | 同じ（Unix の label は「）」で終わらない） | npm run build が通ること |
| dbd91c8（S5d） | process.rs の run・run_pair：実行時間の数え始めと、ジャッジを起動できなかったときの止め方に cfg(not(windows)) を付けた | 同じ（cfg だけ） | process のテストが通ること |
| db1c4e9（S6） | compare.rs：exact_compares_bytes の「CRLF は一致しない」の行に cfg(not(windows))。Mode::Exact の腕は cfg だけ | 同じ | compare のテストが通ること |
| 1df2e10（S7） | python.rs：names・hint・find_in・program の引数・find_interpreter の最後の形・library_dirs_command を cfg で分ける・場所を移す | 同じ | python・random・interactive の Python のテスト（runner に python3.13 がなければ自分で飛ぶ） |
| a3da2a4（S8） | cpp.rs・run.rs：GCC_HINT・sdk_path・find_compiler の最後の形・find_compiler_in・executable・header_dirs_command・Program::built_with・gcc_output（Unix ではそのまま返す関数を通す） | 同じ | cpp・run のテスト（runner に g++-15 がなければ自分で飛ぶ） |
| 02b4945（S9） | run.rs：Rust のビルドの命令の組み立てを cargo_builds に移し、命令（今までの cargo build 1 つ）を for で順に動かす。診断を送る所を、失敗したときとループのあとに分けた | 同じ（命令が 1 つなので） | run・random・interactive の、本物の cargo を使うテストが通ること |
| 6981744（S10） | logLinks.ts の splitLocations に引数 windows を足した（Windows 以外は今までと同じ正規表現と判定）。scaffold.rs はテストの比べ方だけ。package.json に npm test、ci.yml に npm test の段階（両方の OS） | 同じ | **npm test の段階が macOS で初めて動く**。tests/logLinks.test.ts（7 件）と tests/pythonNote.test.ts（2 件）が通ること（.nvmrc の Node 24.22.0） |
| 47fcbe2（S11） | problem.rs：fetch_public の Command::new("/usr/bin/curl") を let に分けた。with_session_fallback の 404 の腕に cfg(not(windows))。import を 2 行に分けた | 同じ（起動するものと引数、文は同じ） | problem のテストが通ること |
| 92da88f（S13） | 設定の画面の Python の説明の文を SettingsDialog.tsx から src/settings/pythonNote.ts に移した（文字は同じ） | 同じ | npm test の pythonNote の「macOS：今までの文のまま」が通ること |

ほか（Unix のコードは変えていない）：examples/test_child.rs（S2〜。std だけ。Unix の部分は cfg(unix)）は、macOS でも
cargo clippy --all-targets・cargo test がビルドする（run 41308130068 からの CI で通っている）。S3・S4・S5c・S13（c531943）の lsp.rs・debug.rs・
process.rs・win.rs の変更は cfg(windows) の中と Windows のテストだけ。Cargo.toml は windows-sys（Windows だけの依存）の feature を
足しただけ。push のあとの CI では、まずすべての段階（fmt・build・clippy・test・npm test）が通ることを見て、そのあと上の太字の
3 つ（S10b・その直し・npm test）を確かめる。clippy は `-D warnings` を付けていないので、警告の有無はログを開かないと分からない

run #23 の macos-latest のログ（2026-11-06。利用者がログインしたブラウザで見た）

| 探したもの | 結果 |
| --- | --- |
| S10b のテスト | `test process::tests::blames_the_side_whose_output_closed_first ... ok` |
| S10b の直しのテスト | `test process::tests::keeps_the_exit_order_when_output_closes_late ... ok` |
| 対話テスト（本物のプログラム） | `test interactive::tests::tests_interactively_with_real_programs ... ok` |
| cargo test の最初の test result | `test result: ok. 296 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 18.65s` |
| cargo clippy の warning | 0 件 |
| npm test | tests 9・pass 9・fail 0。「macOS：今までの文のまま」に ✔ |

「1 filtered out」は debug::tests::debugs_with_real_lldb_dap（ci.yml の `--skip`）。ci.yml の `--skip` はこの 1 つだけで、この名前のテストは
OS で分けていない。Windows で `cargo test --lib -- --list` を `--skip` のありなしで比べると、消えるのはこの 1 件だけだった（324 件 → 323 件。
名前の似た debugs_cpp_with_real_lldb_dap は残る）。macOS で一覧を比べてはいない。

ログで分からないこと：g++-15・python3.13 などがないと、テストは「飛ばします」と出して ok で終わり、cargo test はその文を出さない。runner に
何が入っているかは見ていないので、S7（Python）・S8（C++）の macOS の経路が CI で本当に動いたかは未確認

Windows で飛ばしているテスト（2026-11-05 の時点で 23 件。段階 1 の表〔25 件〕のうち、段階 2 の 3 件は ignore を外した。
debug の 1 件は S8 で足した）

| 直す段階 | 件数 | テスト | 理由 |
| --- | --- | --- | --- |
| 3・5 | 13 | session の 4 件、problem::reads_with_the_session_only_after_a_404_when_logged_in、submit の 8 件 | /usr/bin/security（Keychain）と /usr/bin/curl（ログインの経路） |
| 4 | 4 | sources の 3 件、lsp::overrides_initialize_and_answers_configuration | file: の URI・`\\?\`・パスの区切り |
| 6 | 6 | debug の builds_launch_configuration・reports_missing_adapter・debugs_with_real_lldb_dap・debugs_cpp_with_real_lldb_dap（S8 で足した）・stops_pauses_and_catches_panics・ignores_lldbinit_and_does_not_wait_for_input | デバッグは段階 6 |

段階 2 で確認済みになったこと（主なもの。詳しくは各行と画面の確かめの表）

- Job Object で子孫まで止まる・使用メモリを測れる・MLE になる（S3・S4）。止めたあと 10〜40 ms ほど exe に書けないことがある（原因は未確認）
- 例外の終了コード（0xC0000005・0xC0000094・0xC00000FD・0xC0000409）と、C++ の abort・assert・捕まえない例外が 0xC0000409 に
  なること（S5・S5b）。WER のせいで落ちた解答が TLE になっていたことと、その直し（S5b・S5c）
- Job に入れて再開するまでの時間（子 1 つで 53〜73 ms、30 個同時で最大 1.7〜2.3 秒）と、それを実行時間から外したこと（S5d）。
  新しい exe の 1 回目の遅れは実行時間の外に出ていること（S12）
- CPython・MinGW の出力の CRLF と、exact の比較（S6）。CPython の PEP 514 のレジストリと -X utf8（S7）
- MinGW の exe が PATH にコンパイラの bin がないと 0xC0000135 で落ちること、ld が `\\?\` の -o で上書きできないこと（S8）
- 既定のスタック（Rust の MSVC 1 MiB・MinGW 2 MiB）と、1 GiB にしたあと深い再帰が通ること。cargo rustc の `--` の後ろの引数で
  依存を作り直さないこと（S9）
- curl.exe（Schannel）で、ログインなしの問題ページと一覧が取れること（S11）
- release の exe で、段階 2 の操作のたびにコンソールの窓もエラー報告のダイアログも出ないこと（A6）
- 画面の確かめ：Rust・C++・CPython・PyPy のサンプルテスト（AC・WA・RE・TLE・MLE）、ランダムテスト、対話テスト（S5b〜S11・B1・B2）

段階 2 のあとも未確認のまま残っていること

- **macOS**：
  - S7（Python）・S8（C++）の macOS の経路が、CI で本当に動いたか（run #23 は通ったが、g++-15・python3.13 がなければテストは自分で飛ぶ。runner に何が入っているかは見ていない）
  - d4c1abf より前と S5〜S5c の push の CI の結果は、まとめの中では確かめていない。S5d〜S7 の run #20〜22 は、run 全体の結果（成功）だけを見た
  - macOS の画面：利用者の手元に Mac がないので、macOS の画面は確かめていない（段階 1 から一度もしていない）。S10b とその直しが変えた対話テストの動きも、CI のテストで確かめただけ。この未確認は PR に書いて merge する（2026-11-06 に利用者が決めた）
  - debugs_with_real_lldb_dap（CI では飛ばしている。段階 1 から）
- **配り方**：インストーラー（nsis）で入れた exe での動き。段階 1 の release の確かめで exe をどう起動したか
- **原因が分からないまま手当てしたもの**：
  - 止めたあと exe に 10〜40 ms 書けないこと（S3b で待つ）
  - 同時の保存がアクセス拒否で失敗すること（S10d で 1 つずつにし、案内を出す）
- **Windows で動かしていない・決めていないもの**：
  - Rust の gnu ターゲットのスタックのリンクの引数（`-Wl,--stack`。S9）
  - AC Library を置いたときの画面の確かめ（`-I` に `\\?\` が付かないことはソースで確かめただけ。S10）
  - IDE が別の Job に入っているときの Job Object の組み合わせ（4.6）
  - 言語サーバー・ビルドと重なったときに、Job に入れて再開する時間がどれだけ延びるか（4.5）
- **画面で見たが、仕組みを確かめていないもの**：
  - 対話テストのやり取りの記録の `\r` が、画面で取り除かれたのか、見えない文字として出たのか（B2 の 3-7）
  - C++ のランダムテストの 2 回目に、gen.cpp の警告が出なかったわけ（B2 の 5-1）
- **段階 2 では直さないと決めたもの**：
  - 子を Job に入れて再開するまでを速くする案（A4）
  - GUI から起動した子の CP932 の文字化けの案（A7）

  どちらも「段階 2 のあとの案」の節にある
- **あとの段階で直すもの**：
  - デバッグのアダプターを止めたあとの Job の記録（debug.rs の 2 か所。段階 6）
  - VS Code のスニペットの場所と `/usr/bin/open`（段階 3）
  - 言語サーバー・デバッグの見つからないときの案内文（段階 4・6）
- 段階 1 から残っているもの：Ctrl+S そのものの保存（自動保存と区別していない）

既知の問題（まだ直さない）は「段階 2 の既知の問題」の節にある：WER の ReportArchive の記録、入力の CRLF、対話テストの `\r`、
証明書の失効の確かめ（Schannel）

段階 2 の merge（2026-11-06）

PR #4 を main に merge した（3a32b0e。2026-11-06 21:12 JST）。CI の結果は次のとおり（公開の API で読んだ。ログの中身は見ていない）。

| run | id | きっかけ | コミット | 結果 |
| --- | --- | --- | --- | --- |
| #24 | 42213488199 | push（windows-port-stage2） | 7e56c86 | run 全体が success（OS ごとの結果は見ていない） |
| #25 | 42215021243 | pull_request（windows-port-stage2） | 7e56c86 | macos-latest・windows-latest とも success。どちらも全 16 段階が success |
| #26 | 42215919158 | push（main） | 3a32b0e | macos-latest・windows-latest とも success。どちらも全 16 段階が success |

### 段階 3：ファイル・URL を開く処理、ログイン情報の保存

何を変えるか
- `/usr/bin/open` の 5 か所（commands.rs 3、problem.rs 1、submit.rs 1）を、OS ごとの「開く」関数にまとめる（新しい open.rs。Windows は ShellExecuteW と、テキストファイルは VS Code・メモ帳）
- Keychain を資格情報マネージャーに（4.1 で選んだ方式）。`Keychain` の型の形（app・named・save・load・exists・delete）はそのまま
- session.rs の curl を curl.exe に（段階 5 から前倒しした。9.1 の追加 22）
- VS Code のスニペットの場所、画面の「Keychain」の文字（「Finder」は画面に出ない。コメントだけ。grep で確認済み）

どう確認するか
- 問題ページ・提出ページ・提出結果のページがブラウザで開く。テンプレートのフォルダがエクスプローラーで開く。settings.json・スニペットが決めたエディタで開く
- 4.1 の手順 A・B・C
- VS Code のスニペットの取り込み

段階 3 は、次のまとまりごとに 1 つずつコミットする（2026-11-06 に決めた順。ブランチは最新の main〔3a32b0e〕から作った windows-port-stage3）。
進め方は段階 2 と同じ：macOS / Unix の経路は cfg を付ける・場所を移す以外は変えず、変えたときはコミットの報告とメッセージに明記する。手元に Mac がないので、確かめは CI で行う。
各コミットの前に報告して返事を待つ。コミットの前に cargo fmt・cargo clippy --all-targets（警告なし）・cargo test・npm test・npm run build を AC 電源で通す。テストの時間の余裕は広げない。
不具合を直すときは、先にテストを書いて失敗を確かめ、直す前の失敗をメッセージに書く。新しい処理は、一時的に外してテストが落ちることも確かめ、戻したことを見せる。

| 順 | まとまり | 何を変えるか | どう確認するか |
| --- | --- | --- | --- |
| T1 | 文書 | PR #4 の merge と run #24〜#26 の結果、この表、9.1 の追加 22〜31、4.1 の手順の直し、段階 5 との分担 | — |
| T2 | URL・フォルダを開く | 新しい open.rs に、問題ページ（problem.rs）・提出のページ（submit.rs）・テンプレートのフォルダ（commands.rs）の 3 か所を移す。**Unix は場所を移すだけで、文は同じ。** Windows は ShellExecuteW。URL は `https://` だけ、フォルダは is_dir のときだけ開く（ShellExecuteW はプログラムも起動できるため） | 先にテストで失敗を確かめる：`https` 以外を断る、ないフォルダを断る、`\\?\` を外す。Unix は open に渡す引数のテストを足し、CI で見る |
| T3 | テキストファイルを開く | settings.json とスニペットの 2 か所を移す。Windows は 9.1 の追加 28 の順で探し、`\\?\` を外して起動する。標準入出力は null にし、終わるのを待たない（spawn） | 先にテストで失敗を確かめる：一時フォルダで探す順、起動できないときのメモ帳、`\\?\` を外す。起動そのものは T8 で見る |
| T4 | VS Code のスニペットの場所 | commands.rs:1723-1725（実際に読む場所）と、snippets.rs の見つからないときの文（実際の場所にする）。場所を組み立てる部分はテストできる関数に分ける。Windows は `%APPDATA%\Code\User\snippets`。**Unix は今までと同じ場所を返す形に移すだけ** | 先にテストで失敗を確かめる：場所の組み立てと文。snippets.rs:725 の確かめは OS で分ける |
| T5 | 資格情報マネージャー | win.rs に CredWriteW・CredReadW・CredDeleteW を足す。session.rs は Unix の `impl Keychain` に cfg(not(windows)) を付け、Windows の `impl` を足す。型の形は変えない。9.1 の追加 23・24 を入れる。problem.rs:718 のテストをテスト用の名前に変える。saves_loads_and_deletes_in_keychain の ignore を外す。テストの順番の仕組み（9.1 の追加 32）を入れる | 先にテストで失敗を確かめる：書く・読む・消す、2560／2561 バイト、エラーの文に値が出ない。追加 24 の仕組みは外すと落ちることも確かめ、戻したことを見せる。そのあと、絞った一覧（9.1 の追加 29）でテストの項目が残っていないことを見せる |
| push | T5 までを push（9.1 の追加 27） | 利用者の許可を得てから。push の前に、秘密の値の調べ、送り先、--force なし、送るコミットの数を見せる | CI の Windows で資格情報を書けるか（未確認）。macOS が壊れていないか |
| T6 | session.rs を curl.exe に（9.1 の追加 22） | windows_curl を session.rs に移し、process::command で起動する。Unix は cfg を付けるだけ。session の残り 3 件と submit の 8 件の ignore を外す。落ちるものは 9.1 の追加 26 のとおりに扱う。T6 のあとに、テストの順番の実際の待ち時間を測って報告する（9.1 の追加 32） | 先に ignore を外して失敗を確かめる。日本語と CRLF を含む `--config -` の送信は posts_with_session_and_rotates_cookie で見る |
| T7 | 問題の取得のログイン | problem.rs の「まだログインできません」の分岐・定数・テストを消す。**Unix は cfg を外すだけで、文字は同じ。** reads_with_the_session_only_after_a_404_when_logged_in の ignore を外す | 先に ignore を外して失敗を確かめる |
| T5b | 資格情報の重なりの判断（9.1 の追加 33） | save が前の値に戻るかの切り分け（テスト用の名前だけ）と、ログアウトのあとに少し待って読み直し、あればもう一度消す形を足すかの判断。コードはまだ書かない | 切り分けの結果を報告して、利用者が決める |
| T9 | 4.1 の手順 A・B・C（T8 より先。9.1 の追加 30） | 結果を文書に書く：REVEL_SESSION の長さ（値は書かない）、InPrivate で何が残るか、他のプログラムからの読み出し（同じ利用者の別のプロセスが確認なしに読めるか）、9.1 の 14。効いていなければ、直すコミットを別に足す。画面に「Keychain」の文字が残っていても先に進む | 本物のログイン（利用者が操作）。ログアウトや値の書き換えの前に、下の「T9 で守ること」を行う |
| T8 | 画面の文字と開く処理の確かめ | SubmissionPanel.tsx:235 を OS で分ける（pythonNote.ts と同じ形で npm test を足す）。画面の確かめの手順はコードと照らしてから出す。提出のページは、提出の処理を始めずに開ける経路があるかをコードで確かめる（未確認）。なければ相談する | ブラウザの 3 種類のページ、エクスプローラー、VS Code とメモ帳、スニペットの取り込み。メモ帳は環境変数を変えて起動して確かめる案（効くかは未確認。先に試す） |
| T10 | まとめ | 段階 3 の結果、Unix を変えた所の表、残る未確認を書く | push の前に、秘密の値の調べ、送り先、--force なし、送るコミットの数を見せる。パスワードとの照合は利用者の窓で行う |

T5 までの push の CI（run 42378818906、866848a、2026-11-07）
- macos-latest・windows-latest とも success。どちらも全部の段階が success（公開の API で確認）
- 次は、利用者がログインしたブラウザで見た（ログインなしの API ではログを取れず 403 だった）
  - macOS の cargo clippy：warning の文字は見えなかった（目視）
  - macOS の cargo test：304 通過、0 失敗、0 無視、1 除外
  - Windows の cargo test：345 通過、0 失敗、21 無視、1 除外（CI の --skip で除外する 1 件は、手元では無視に数える。手元の 22 無視と同じ）
  - saves_loads_and_deletes_in_keychain と stops_a_case_at_the_time_limit：macOS と Windows の両方で ok。CI の Windows でも資格情報を書けた

T6 の結果（2026-11-08）
- 追加 26 の扱いになる失敗はなかった。session の 3 件と submit の 8 件の ignore をすべて外し、ignore を残したものはない。実際の提出と提出の前の検査は段階 5 のまま
- 直す前に一度止まった：curl を起動できないと、keeps_the_saved_session_until_the_window_is_logged_in の is_err() が通り、誰も接続しない server.join() で待ち続けた。順番を持ったままなので、ほかの 10 件は順番の待ちで FAILED になった。テストのプログラムを止めたので、テスト用の項目が 1 つ残り、利用者の許可を得て消した。テストの 2 か所を、期待する文と比べる形に直した（テストのコードだけ。macOS でも動く）
- テストの順番の待ち（lib テストを同時の形で 5 回。AC 電源。計測のコードは一時的に入れて外した）：1 回の最長の待ちは 4944〜5351 ms、1 回の実行で持っていた時間の合計は 5264〜5494 ms。一番長く持つのは curl_failures_do_not_show_the_cookie_value（4172〜4380 ms）。120 秒の上限は変えない。curl の --max-time は 1 回 30 秒なので、通信が止まれば持つ時間は延びる
- reports_login_and_failures（TestKeychain を 2 つ作る）は 5 回とも止まらずに通った（待ち 1184〜3902 ms、持っていた時間 54〜80 ms）
- `--nocapture` の出力に、テストの偽の Cookie の値と `REVEL_SESSION=` は 5 回とも 0 件だった
- 未確認：コンソールの窓が出ないこと（CREATE_NO_WINDOW はテストでは守られていない）。release の画面で確かめる（T8）
- 止まるおそれ（推測。動かしていない）：problem.rs の windows_curl_reports_404_in_the_same_words の join は、curl.exe が接続できないと待ち続ける
- 未確認（推測）：curl は既定で .curlrc を読むので、そこに書いた設定（プロキシや通信の記録など）が Cookie の入った要求にも効くかもしれない。今このマシンにはない。Unix の引数も同じ関数なので T6 では変えていない。--disable を付けるかどうかは T10 で利用者が決める
- 日本語と CRLF を含む --config - の送信は、posts_with_session_and_rotates_cookie で確かめた（Windows で通過）

T7 の結果（2026-11-08）
- Windows でも、問題の取得が 404 のときに with_session_fallback がログインの有無を見て、ログインしていればセッションを付けて読み直す。本物のログインでの動きは未確認（T9 で確かめる）
- 読み直しの request は、本物の項目を読むだけでなく、サーバーが Cookie を入れ替えれば書き換え、ログインのページへ移されれば消す（Unix と同じ処理）
- 「Windows 版では、まだログインできません」の文はなくなり、Unix と同じ NOT_FOUND_LOGGED_OUT の文になる
- テストが守らないこと：取り直しの前の wait_for_interval（1 秒あける）は、外してもテストは落ちなかった。テストは足さない（利用者の判断。cfg を外しただけで処理を変えておらず、時間を測るテストは不安定になりやすいため。コードを読んで確かめた）

秘密の値について守ること
- AtCoder の Cookie・パスワードなどの秘密の値を、ログ・テストの出力・計画書・コミットに出さない
- テストは本物の資格情報に触れない名前で行う。資格情報マネージャーに書くテストは、終わったら自分で消す
- `cmdkey /list` はそのまま出さない（ほかのサービスの項目名と利用者名が出るため）。`cmdkey /list | findstr /i "acdstudio atcoder"` のように絞る
- 値を調べるスクリプトは scratchpad に置き、リポジトリに入れない。.ps1 に日本語を書かない（9.1 の追加 25）

T9 で守ること
- ログアウトや値の書き換えをする前に、本物の値が、リポジトリの変更・実行ログ・history.json・state.json・settings.json に含まれないことを調べる。出すのはファイル名と件数だけで、値は出さない。実行ログがファイルに残るかは、コードで確かめてから調べる（未確認）
- EBWebView の中を探すときは、一致した行を出さない。ファイル名と件数だけを出す
- パスワードの照合は利用者の窓で行う。出すのは一致したかどうかだけ
- 値の書き換えの前に、利用者に聞く（書き換えたあとは、ログインし直しになる）

段階 3 の既知の問題（資格情報マネージャー。T5 の切り分け。2026-11-07）

確認したこと（テスト用の名前と偽の値だけで確かめた）
- 別のプロセスの書き込みと重なると、delete が true を返したあとも項目が戻ることがある。別々の名前を書いて消す 2 つのプロセスを 9〜75 ms 重ねた 50 回のうち 11 回で、スレッド・プロセスが終わった直後も 1 秒後も項目があった。11 回とも先に終わったプロセスの項目だった
- 値が最後に書いた値でないことがあった（別の切り分けで、4 つの値を順に書くテストの項目に、2 番目・3 番目の値が残っていた）
- 同じプロセスの 2 つのスレッドでも、別々の名前で 50 回中 10 回起きた。スレッド 1 つ・名前 1 つでは 50 回起きなかった。`session::` を 1 つずつ動かすと 20 回残らなかった
- 同じ名前を 2 つのスレッドで書いて消す形では、50 回再現しなかった。「起きない」ではない
- 重なりが終わったあとの delete では消えた（テストの Drop のあと、絞った一覧は 0 件だった）
- 順番の仕組みを入れる前は、`session::` を同時に動かすと 7 回中 2 回項目が残った。入れたあとは 20 回残らなかった（回数が少ないので、効果の確定ではない）

推測・未確認
- 仕組みは未確認（動き続けていた側の書き込みが、先に消した分を元に戻したように見える：推測）
- 本物のプログラム（Git など）との重なりは試していない
- ログアウトと save に影響するかもしれない（推測。ログアウトしたのに項目が戻る、Cookie を入れ替えた save が前の値に戻る）。T5b で扱う

テストが守る範囲
- 順番の仕組み（9.1 の追加 32）が守るのは、同じテストのプログラムの中だけ。別のプログラムの書き込みが重なると、テストの項目が残ることはまだある

時間のテスト
- run::tests::stops_a_case_at_the_time_limit（1500 ms 未満）が AC 電源で 1 回落ちた（2026-11-07）。そのあと全部の cargo test を 5 回動かして、5 回とも通った。電池のときに落ちる記録（S13）はあるが、AC で落ちたのは今回が初めての記録。原因は未確認で、T5 の変更との関係も分からない。時間の余裕は広げていない

提出についての注意（段階 3 の版）
- T6 のあとは、Windows でも提出の処理が動くようになるかもしれない。実際の提出と提出の前の検査は段階 5 まで確かめないので、段階 3 の版では提出しない。ボタンは消さない
- 提出の処理は、「提出」のボタンのほか、キーの割り当てからも始まる（App.tsx:1231-1235。初期の割り当てはなく、settings.json で割り当てたとき）。確認のダイアログが出たら、利用者は「提出する」を押さない
- 確認済み：コードは「提出する」「キャンセル」のボタンを指定している（submit.rs:1675-1683）。confirmDialog が true なら省かない（submit.rs:1609-1615）
- 未確認（段階 5 で確かめる）：Windows でのボタンの文字、「キャンセル」や窓を閉じたときに送られないこと、Enter を押したときに選ばれるボタン
- ダイアログが出たときに Enter を押すと『提出する』が選ばれるかもしれない（推測。rfd 0.16.0 の nDefaultButton: 0 と、ボタンの並びから。動かしていない）。段階 3 の版では、ダイアログが出ても Enter を押さない

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
- session.rs の curl を curl.exe に（`--config -`）：ログインにも要るので、段階 3 の T6 に前倒しした（9.1 の追加 22）。段階 5 では、提出の経路で確かめる
- 提出の時刻の表示（localtime_r の置き換え。段階 1 で済ませていれば確認だけ）
- 提出の前の検査のうち、OS の違いが出るもの（Python の import の検査〔submit.rs:2527 付近〕など）

どう確認するか
- session.rs・submit.rs の単体テスト（手元の http の口に送るもの）を Windows で通す（段階 3 の T6 で通した）
- 実際に練習用のコンテストの問題へ提出し、判定が提出結果の欄に出るまで追える。Cookie の入れ替えで資格情報が更新される
- 確認のダイアログ：Windows でのボタンの文字、「キャンセル」や窓を閉じたときに送られないこと、Enter を押したときに選ばれるボタン（『提出する』が選ばれるかもしれない。推測）。どれも段階 3 では未確認のまま（「提出についての注意」の節）

### 段階 6：デバッグ

**今回の移植には含めない（9 章 12。2026-10-29 に決めた）。** 段階 1 では、デバッグのコードは Windows でコンパイルが通るようにするだけで、動かさない。以下は後で行うときのための記録。

何を変えるか
- 6 章で選んだ方式のアダプターの探索と起動、DAP の違いの吸収
- 標準入力の渡し方（入力のファイルがないときは `NUL`）
- debugpy は今のままで動くか確かめる（`-X utf8` を足すかも 4.4 に合わせる）
- アダプターを止めたあとの Job の記録（段階 2 S13 で見つけた）：debug.rs の 2 か所（965 行あたりの、debugpy につなげなかった
  とき、1225 行あたりの shutdown）は、kill_group のあと child.wait で回収するだけで win::release_job を呼ばないので、win.rs の
  JOBS に、その pid の記録（プロセスのハンドルの写し）が、IDE が終わるか同じ pid が使い回されるまで残る。言語サーバー（lsp.rs の
  Server::stop）は S13 で直した。デバッグは Windows ではまだ動かせず、テストで確かめられないので、段階 6 で、アダプターを
  Windows で動かすときに、先にテストを書いて記録が残ることを確かめてから、同じ形（回収のあとに Windows だけ release_job）で直す

どう確認するか
- debug.rs の本物のアダプターを使うテスト（debugs_with_real_lldb_dap・debugs_cpp_with_real_lldb_dap にあたるもの）を、選んだアダプターで書き直して通す
- ブレークポイント・ステップ・変数・式の評価・panic と例外と abort で止まる・止めたあとにプロセスが残らない
- アダプターを止めたあと（shutdown と、つなげなかったとき）に、JOBS に記録が残らない（lsp.rs の forgets_the_job_record_after_stopping_on_windows と同じ形のテスト）

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
| 8 | Defender の初回の遅れ：(d) 注意を出すだけ。優先度は低い。段階 2 S12 で測ると、S5d のあとは 1 回目の遅れがほぼ実行時間の外に出ていた（差は約 2 ms）ので、注意は出さないことにした（利用者の選択。2026-11-04） | 段階 2（S12）で、出さないと決めた |
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
| 追加 7（2026-10-31） | Rust のスタックは `cargo rustc … -- -C link-arg=/STACK:N` で付ける（run.rs の cargo build を置き換える。Windows だけ）。依存を作り直さず（S9 で確かめた）、RUSTFLAGS とも両立するため。GNU のツールチェーンでは `-Wl,--stack,N`（未確認） | 段階 2（S9）で対応 |
| 追加 8（2026-10-31） | テスト用の子のプログラムは `src-tauri/examples/test_child.rs` | 段階 2（S2） |
| 追加 9（2026-10-31） | Job に入れる処理は、`process_group(0)` の代わりの仕組みで CREATE_SUSPENDED を付け、spawn の直後に `#[cfg(windows)]` の 1 行で行う（Unix 側の文字は変えない） | 段階 2（S3） |
| 追加 10（2026-10-31） | IDE が止めたとき（TerminateJobObject）の終了コードは専用の値 0xE0AC0001（i32 では -525598719）にして、「IDE が止めました」と表示する。上位の 4 ビットの E は「エラー」と「利用者が定める値」（ビット 29）の印で、Windows の NTSTATUS とは重ならない | 段階 2（S3・S5） |
| 追加 11（2026-10-31） | Windows での CPython の探し方は、settings → PEP 514 のレジストリ（HKCU → HKLM）→ PATH の python.exe（WindowsApps のエイリアスを除き、版を確かめる）。py ランチャーは使わない | 段階 2（S7） |
| 追加 12（2026-10-31） | MinGW の DLL：作った exe を動かすときだけ、子の PATH の先頭にコンパイラの bin を足す（`-static` などのビルドの引数は変えない）。S8 で入れた（run::Program の dll_dir。PATH を System32 だけにすると 0xC0000135 で落ち、bin を先頭に足すと動くことを確かめた） | 段階 2（S8）で対応 |
| 追加 13（2026-10-31） | Defender の注意は、ビルドの直後の 1 ケース目が TLE のときだけ実行ログに 1 行出す。S12 で、出さないことにした（9.1 の 8） | 段階 2（S12）で取りやめ |
| 追加 14（2026-10-31） | 子のプログラムを使う新しいテストは Windows だけで動かす。Unix の sh のテストはそのまま | 段階 2（S2〜S4） |
| 追加 15（2026-11-01） | 止めた直後のビルドの前に、出力の exe が書けるようになるまで最大 2 秒待つ。待っても書けなければ、案内を出してビルドは続ける（ビルドせずに失敗にはしない）。S3 には入れず、すぐあとの S3b にする | 段階 2（S3b） |
| 追加 16（2026-11-01） | 使用メモリは WorkingSet（触ったページ）で測り、直接の子だけを数える（Unix と同じ）。最大は終わったあとの PeakWorkingSetSize を、起動したときのハンドルの写しで読む（pid で開き直さない）。読めなければ実行中に見た最大で代わりにする案だったが、読めたので使わない | 段階 2（S4） |
| 追加 17（2026-11-01） | rustc の診断のパスの `\` を `/` に直す部分だけを、S4 のすぐあと・S5 の前の S10a にする。直すのは Windows だけ。`\\?\` の付いたパスは、外してから直す。random のテストの ignore を S10a で外す。logLinks.ts と scaffold のテストは S10 に残す | 段階 2（S10a） |
| 追加 18（2026-11-01） | 対話テストの、先に失敗した側の取り違えは、原因と結果の順（出力が先に閉じた側）で決める形に直す。両方の OS にある判定の不具合なので、OS で決め方を分けない（Unix の動きも変わる）。S10a とは別の S10b にし、interactive のテストの ignore は S10b で外す | 段階 2（S10b） |
| 追加 19（2026-11-01） | 同時の保存の失敗は、原因を決めきれないので、(2) 同じパスへの保存を IDE の中で 1 つずつにする（Windows だけ）と、(3) 諦めたときに、ほかのプログラム（ウイルス対策など）が使っている可能性を案内する、の両方を S10d で入れる。試し直しの時間を延ばすのは、測った長さを見てから相談する | 段階 2（S10d） |
| 追加 20（2026-11-02） | RE の表示：i32 で負の終了コード（Windows の例外のコード）だけを 16 進にし、名前の表にあれば名前を添える（表にない値は 16 進だけ）。負でない値は 256 以上も 10 進のまま。名前は Rust で作って画面に送る（画面の側に単体テストの仕組みがないため）。終了コード 3 への C++ の abort・assert の添え書きは、測ると 0xC0000409 だったので S5b でやめた | 段階 2（S5・S5b） |
| 追加 21（2026-11-02） | 扱われない例外で落ちた解答が約 4 秒かかり、サンプルテストで TLE と判定される件は、Job に JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION を足して直す（S5b）。0xC0000409（__fastfail）には効かないので、別に手当てする（案を相談中）。レジストリや WER の設定は変えない | 段階 2（S5b） |
| 追加 22（2026-11-06） | session.rs の curl.exe への切り替えを、段階 5 から段階 3 に前倒しする。ログインの流れ（take_session → confirm_session → Client::send）が curl を使い、これがないと 4.1 の手順 A・B・C を確かめられないため。実際の提出と提出の前の検査は段階 5 に残す | 段階 3（T6） |
| 追加 23（2026-11-06） | 2560 バイトを超える値は、Windows だけ保存の前に断り、分かる文を出す。実際の REVEL_SESSION の長さを測ってから先を考える（複数の項目に分けるのは、要ると分かってから） | 段階 3（T5・T9） |
| 追加 24（2026-11-06。T5 で書き直した。2026-11-07） | Windows のテストのビルドでは、本物の名前（SERVICE）に対する save・load・exists・delete を、API を呼ぶ前に panic にする（Keychain::named(SERVICE) でも止まる）。Keychain::app() を作ること自体は止めない（submit.rs の never_reads_the_status_faster_than_once_a_second が Remote::app() を呼ぶため。利用者が決めた）。もう 1 段の守りとして、win.rs の層でも、テストのビルドでは名前に `.test-` のないものには API を呼ばない。**決まり：この仕組み（refuse_the_real_name）を外す確かめは、API をつなぐ前の段階でだけ行う。CredWriteW などをつないだあとは、外してテストを動かさない（本物の名前に偽の値を書いたり、本物の項目を消したりするおそれがあるため）。つないだあとに確かめ直す必要が出たら、動かす前に利用者に聞く。** Unix は変えない | 段階 3（T5） |
| 追加 25（2026-11-06） | 本物の値を確かめるスクリプトは scratchpad に置き、リポジトリに入れない。大きさと LastWritten だけを出すものは Claude が動かす。パスワードの照合は利用者の PowerShell の窓で行い（Read-Host -AsSecureString）、出すのは一致したかどうかだけ | 段階 3（T9） |
| 追加 26（2026-11-06） | curl.exe に切り替えたあとも落ちるテスト：原因が session.rs か資格情報の処理なら段階 3 で別のコミットにして直す。提出の処理が原因なら、理由を段階 5 に書き換えて ignore を残す | 段階 3（T6） |
| 追加 27（2026-11-06） | T5 のあとに 1 回 push をお願いする（CI の Windows で資格情報を書けるかと、macOS が壊れていないかを早めに確かめる）。その push の前にも、秘密の値の調べ、送り先、--force なし、送るコミットの数を見せる | 段階 3（T5 のあと） |
| 追加 28（2026-11-06） | VS Code の探し方：`%LOCALAPPDATA%\Programs\Microsoft VS Code\Code.exe` → `%ProgramFiles%\Microsoft VS Code\Code.exe` → PATH の code.cmd の 2 つ上のフォルダの Code.exe。code.cmd は cmd.exe を通して引数の引用が難しくなるので、起動には使わない。起動できなければメモ帳にする。settings.json での指定は足さない（9.1 の 4 の詳しい形） | 段階 3（T3） |
| 追加 29（2026-11-06） | `cmdkey /list` はそのまま出さず、`cmdkey /list \| findstr /i "acdstudio atcoder"` のように絞る | 段階 3 |
| 追加 30（2026-11-06） | 順番は T7 → T9（本物のログイン）→ T8。ログインができるか、値の長さがどれくらいかを早めに確かめるため。T9 で画面に「Keychain」の文字が残っていても先に進む | 段階 3 |
| 追加 31（2026-11-06） | T4 の対象に commands.rs:1723-1725（実際に読む場所）を入れ、場所を組み立てる部分をテストできる形に分ける。Windows では %APPDATA% を使う。T3 でも `\\?\` を外し、起動するときの標準入出力を null にする | 段階 3（T3・T4） |
| 追加 32（2026-11-07） | Windows のテストでは、資格情報を使うテストを同じプログラムの中で 1 つずつ動かす（TestKeychain が順番を持つ。Drop で項目を消したあとに返す）。同じスレッドは何回でも取れる（reports_login_and_failures は TestKeychain を 2 つ作るため）。全部を返したら空く。別のスレッドで返すと、順番を返してから分かる文で panic する。ただし panic の最中（std::thread::panicking()）は panic せず、順番だけを返す。待ちの上限は 120 秒で始め、T6 のあとに実際の待ち時間を測って報告する。Unix のテストの動きは変えない（TestKeychain::new のコードの形は変わった） | 段階 3（T5） |
| 追加 33（2026-11-07） | 本番のコードは T5 では変えない。T7 と T9 の間に T5b を置き、save が前の値に戻るかの切り分けと、ログアウトのあとに読み直してもう一度消す形を足すかの判断をする | 段階 3（T5b） |

---

## 10. 未確認・不明のまま残っている項目

未確認（確かめ方は各節に書いた）

- 2.2：cargo build のエラーを直したあとに出る、型検査より後の段階のエラー
- 3 章の代替案のすべて（Windows で動かしていない）
- std の Command が `.exe` のないパスに `.exe` を補うか
- curl.exe が今の引数と `--config -` で同じに動くか（→ ログインなしの取得の引数〔--fail・--proto =https など〕は段階 2 S11 で確認済み。
  `--config -` を使うログインの経路は段階 3）
- menu.rs の macOS 向けの項目を Windows で使ったときの動き
- WebView2 のブラウザの既定のキー（Ctrl+R など）と、keydown の preventDefault の効き方
- 4.1：資格情報マネージャーの値の大きさの上限、incognito の効き方、パスワードの自動保存・自動入力の既定値
- 4.3：Rust の出力の改行、include_str! の testdata のテスト（CPython・PyPy の出力が CRLF になることは 2026-10-31 に確認済み。
  exact の比較で WA になることは段階 2 S6 で確かめ、Windows だけ \r\n を \n に直してから比べるようにした）
- 4.4：GUI から起動した子のコードページ（ping が CP932 で出すことは段階 2 S3 の画面の確かめで確認済み。ほかのプログラムは未確認）、
  g++ が CP932 で出すもの、`-E` で PYTHONUTF8 が無視されること（PyPy が `-X utf8` を受け付けることは 2026-10-31 に、CPython が `-X utf8` で
  パイプへ UTF-8 で出すことは段階 2 S7 のテストで確認済み）
- 4.5：遅れの原因が Defender か、作業領域・大きな exe での遅れ、GetProcessTimes で検査を除けるか（新しい exe の 1 回目の遅れが
  実行時間の外〔起動から再開まで〕に出ることは、段階 2 S12 で測って確認済み）
- 4.6：対策（CREATE_NO_WINDOW）がないときに窓が出ること（推測のまま）、IDE が別の Job に入っているときの Job Object との組み合わせ
  （対策を入れたあとの release の exe で、操作のたびに窓が出ないことは、段階 2 S13 の A6〔2026-11-05〕で確認済み）
- インストーラー（nsis）で入れた exe での動き（段階 1・段階 2 とも、画面の確認は target\release の exe で行った）。段階 1 の release の
  確かめで、exe をどう起動したか（エクスプローラーか、ターミナルか）
- 4.7：TerminateJobObject が非同期か（止めたあと 10〜40 ms ほど exe に書けないことがあるのは段階 2 S3 で確認済み。原因は未確認で、
  S3b でビルドの前に待つようにした）
- 4.8：引数の引用、.cmd の制約、clangd の `--query-driver` の読み方、`\\?\` を受け付けないツール
- 4.9：BrokenPipe、ハンドルの継承、CRLF の影響（ジャッジが Python の input() なら \r\n の影響がないことと、やり取りの記録に \r が
  残ることは段階 2 S7 のテストで確認済み。C++ の getline で読むジャッジは未確認）
- 4.10：`:`・末尾の `.` と空白・予約名を Windows が実際にどう扱うか（段階 1 では名前で断ったので、断る前の動きは確かめていない）、大文字・小文字の比較
- 5 章：WinLibs・MSYS2 の入手できる版、ジャッジのパッケージの Windows 用の wheel、std::thread・std::print のリンクと実行
  （CPython 3.13.7・PyPy v7.3.20 の入手、MinGW と MSVC の Rust の既定のスタックは 2026-10-31 に確認済み）
- 段階 2 の案で前提にしていたこと（→ 段階 2 ですべて確認済み。この項目は記録として残す）：PeakWorkingSetSize を終わったプロセスのハンドルで読めること（S4）、
  各 NTSTATUS の終了コード（0xC0000005・0xC0000094・0xC00000FD・0xC0000409。S5・S5b）、PATH に g++ がないと MinGW の exe が
  0xC0000135 で落ちること（S8）、`cargo rustc` の `--` の後ろの引数で依存を作り直さないこと（S9）
- macOS の S7（Python）・S8（C++）の経路が CI で本当に動いたか（g++-15・python3.13 がなければテストは自分で飛ぶ。runner に何が入っているかは未確認）。
  段階 2 の全体は run #23（e800d5d）で通り、S10b とその直しのテスト、対話テスト、npm test は macos-latest のログで確認済み（「段階 2 の結果」）
- 6 章：すべてのデバッグの方式（LLVM の lldb-dap の有無、CodeLLDB のライセンス、gdb の Python と DAP）
- 段階 3 で確かめること：ShellExecuteW を COM の初期化なしで呼んで動くか、Turnstile が WebView2 の InPrivate と on_navigation の制限のもとで通るか、cookies() が WebView2 で HttpOnly の Cookie を返すか、CI の Windows で資格情報を書けるか、実行ログがファイルに残るか、提出の処理を始めずに提出のページを開ける経路があるか
- テストのあとに消したはずの項目が残ったことがある（原因は未確認。段階 3 T5。T9 の手順 C〔ログアウトで消える〕で、この点も確かめる。分かったことは「段階 3 の既知の問題」の節。T5b で扱う）
- 段階 5 で確かめること：確認のダイアログの Windows でのボタンの文字、「キャンセル」や窓を閉じたときに送られないこと、Enter を押したときに選ばれるボタン（『提出する』が選ばれるかもしれない。推測。rfd 0.16.0 の nDefaultButton: 0 と、ボタンの並びから。動かしていない）
- macOS の動きを壊していないこと（CI の macos-latest でビルド・clippy・テストが通ったことは確認済み〔run 41023624709。段階 2 は run 42210961555〕。clippy の警告は run 42210961555 のログで 0 件。debugs_with_real_lldb_dap と画面の操作は macOS の機械で確かめる。利用者の手元に Mac がないので、macOS の画面は確かめていない）

不明（調べても分からなかった）

- 開発者モードの設定（レジストリの値を読めなかった。シンボリックリンクを作れないことは確認済み）
- 実際の REVEL_SESSION の長さ
- Windows 11 でフォルダの中の予約名（NUL など）が装置を指すか
- 8.3 の短い名前がこのボリュームで作られるか
- ジャッジのスタックの大きさ（MinGW の ld の既定は 2 MiB と 2026-10-31 に確認済み）
- CP932 で表せない文字を含む利用者名・フォルダでの MinGW・Python の動き
- MSYS2 の今の GCC の版
- Tauri からの WebView2 の自動入力の設定の仕方
