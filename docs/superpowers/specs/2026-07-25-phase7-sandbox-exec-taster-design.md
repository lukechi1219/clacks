# Phase 7 — sandbox-exec 接進 taster 設計文件

> 日期:2026-07-25
> 前置文件:[architecture.md](../../../architecture.md)、[docs/superpowers/specs/2026-07-17-clacks-design.md](2026-07-17-clacks-design.md)(產品設計與安全模型,§安全模型三層表)、[docs/superpowers/notes/2026-07-17-skeleton-findings.md](../notes/2026-07-17-skeleton-findings.md)(§sandbox-exec,Phase 1 骨架真機驗證)
> 狀態:已與使用者討論定案,待寫 implementation plan

## 背景

原始設計文件的安全模型表(`2026-07-17-clacks-design.md:126`)寫明 taster/cyrano 都要有 OS sandbox(`sandbox-exec` profile,檔案系統隔離為主,`deny file-write*` + 白名單)。Phase 1 walking skeleton 已經真機驗證過這個 profile 本身可行(`sandbox-exec -f sandbox.sb` 下 claude 能正常啟動、連得到 API、Stop hook 能寫 outbox),`templates/sandbox/clacks.sb` 是那次驗證後定稿的版本。

但這份 profile **從未接進正式管線**。`src-tauri/src/adapters/pty.rs` 的 `spawn_program`(taster 與 cyrano 共用的底層 spawn 函式)目前直接執行 `["claude", ...]`,完全沒有 `sandbox-exec` 包裝。Phase 2-6 的安全強化(`env_clear` + 白名單環境變數、`CLAUDE_CONFIG_DIR` 隔離、契約驗證)都做了,唯獨 OS 層的檔案系統隔離這塊是空的。

## 設計:只接 taster,套用既有 profile(修正參數對齊現況)

### 範圍

只包 taster(`ClaudePtySession::spawn()`)。cyrano(`spawn_continue()`)這輪不動——cyrano 已有 Claude Code 層級的權限限制(deny 一切工具、僅 allow `Read` 且限定白名單專案目錄),OS sandbox 對 cyrano 屬於錦上添花而非最高風險缺口;taster 是直接碰不可信輸入的那一端,風險最高,優先補這塊。cyrano 是否要接、要用什麼 profile(設計文件原表提到「同左 + 白名單專案目錄唯讀」,細節尚未落地),留給下一輪獨立評估。

### Profile 交付:編譯期內嵌,執行期用 `-p` inline 傳遞

`templates/sandbox/clacks.sb` 保持版控正本,不新增部署步驟。編譯期以 `include_str!` 把內容內嵌進 Rust 常數;執行期用 `sandbox-exec -p <內嵌字串>` 取代 `-f <路徑>`,不需要在真機上解析 profile 檔案的執行期路徑。

這個選擇是直接吸取 Phase 6 的教訓:Phase 6 才剛踩過「`templates/` 改了但 runtime 目錄沒同步」的坑(hook 腳本部署陷阱)。Profile 若維持外部檔案 + 執行期路徑解析,會重新引入同一種「改了原始碼,忘記部署」的風險;`include_str!` 讓 profile 內容跟著二進位一起編譯,沒有執行期路徑可以漏同步,唯一代價是改 profile 內容要重新編譯(可接受,change 頻率低)。

### 參數修正:`HOME_CLAUDE` → `CONFIG_DIR`

現有 profile 的參數註解(`templates/sandbox/clacks.sb:9`)寫 `HOME_CLAUDE = ~/.claude`,這是 Phase 1 骨架階段(2026-07-17)的假設,當時還沒有 `CLAUDE_CONFIG_DIR` 重定向機制。Phase 2/4 之後,`pty.rs::spawn_program` 已經用 `CLAUDE_CONFIG_DIR` 環境變數把設定/OAuth 寫入重定向到 `../clacks-runtime/cli-config`(`config_dir: Option<&Path>` 參數),不再是真實的 `~/.claude`。

若照舊參數接線,taster 的寫入白名單會指向錯誤路徑(真正需要放行寫入的是 `config_dir` 指向的目錄,不是使用者真實家目錄下的 `~/.claude`)。本次修正:
- Profile 內的參數名稱從 `HOME_CLAUDE` 改為 `CONFIG_DIR`。
- 呼叫端(`pty.rs`)傳入的值改為 `config_dir` 參數所指的路徑(而非任何 `$HOME` 相關路徑)。
- Profile 頂部註解同步更新,反映現行 `CLAUDE_CONFIG_DIR` 機制,不再描述已經過時的 `~/.claude` 假設。

`WORKDIR` 參數不變,沿用既有的 `workdir: &Path` 引數(taster 的 runtime 工作目錄)。

### 啟用條件:`config_dir` 為 `Some` 才套用 sandbox 包裝

`spawn_program` 目前有三個呼叫來源:
- `gui.rs`(GUI composition root,production):taster 呼叫 `spawn(&taster_dir, Some(&cli_config), ...)`。
- `bin/pipeline.rs`(headless bin,dev-only,Phase 5 Task 6 裁決):同樣傳 `Some(&cli_config)`。
- `bin/smoke.rs`(Phase 2 診斷 bin):刻意傳 `None`——這個 bin 的目的是驗證「`env_clear` 最小環境下 claude 仍可啟動」這條**不含** `CLAUDE_CONFIG_DIR` 隔離的基準線,是有意的設計,不是遺漏。

因此:sandbox 包裝只在 `config_dir` 為 `Some` 時套用。兩個 production 呼叫點(`gui.rs`、`pipeline.rs`)自然會被包住;`smoke.rs` 因為傳 `None`,自然不受影響、維持現有行為,不需要改動這個檔案。這個規則不需要額外的旗標或設定,是既有參數形狀的自然延伸。

### 平台:直接假設 macOS

不加 `cfg(target_os = "macos")` 判斷。整個專案目前就是 macOS 專屬開發/部署(Keychain 存 token、`architecture.md`/findings 全部是 macOS 特定路徑與工具鏈),`sandbox-exec` 雖被 Apple 標記 deprecated,但研究確認:
1. Deprecated 的原因是 Apple 想把第三方導向 Xcode App Sandbox entitlement(給 Mac App Store 應用用),profile 語言本身從未正式公開文件給第三方——這是「不維護」訊號,不是「即將移除」訊號,目前在 macOS 15 上仍正常運作,無公告移除時程。
2. Anthropic 自己的 Claude Code 官方文件([Configure the sandboxed Bash tool](https://code.claude.com/docs/en/sandboxing))與開源的 [sandbox-runtime](https://github.com/anthropic-experimental/sandbox-runtime) 專案本身也是用 `sandbox-exec` 做 macOS 上的隔離(雖然那是針對 Bash tool 呼叫的隔離,跟本次「包住整個 claude process」不是同一層,但佐證這個機制目前是 Anthropic 自己也在用的合理選擇)。

若 `sandbox-exec` 執行失敗(理論上罕見,例如未來 macOS 版本真的移除了這個指令),走既有 `CliError` fail-closed 路徑,**不**靜默退回無 sandbox 模式——退回無 sandbox 屬於安全倒退,不能接受用「至少還能跑」換取「隔離悄悄消失」。

### 測試策略

- **argv/profile 參數組裝的純函式測試**:比照 `pty.rs` 既有的 `config_dir_sets_env_for_child`、`no_config_dir_leaves_env_unset` 測試風格——驗證給定 `workdir`/`config_dir` 時,組出的 `sandbox-exec` 呼叫參數(`-p`/`-D WORKDIR=`/`-D CONFIG_DIR=`/`claude`)符合預期;`config_dir` 為 `None` 時驗證完全不套用 sandbox 包裝(argv 退回現行的純 `["claude"]`)。不需要真的呼叫 `sandbox-exec`。
- **真機驗證(human task,比照 Phase 1/4/5/6 模式)**:Phase 1 骨架已驗證過 `sandbox-exec` 本身跟 claude CLI 相容,但那是獨立 probe(`sandbox probe`),不是正式雙 CLI 管線。這次要在真的 GUI 管線裡重新確認:taster 在 sandbox 包裝下仍能開機、連得到 Anthropic API、Stop hook 仍能寫 outbox、乾淨/惡意訊息端到端仍正常;並確認 `deny file-write*` 沒有意外擋掉正常運作需要的寫入路徑(對照 Phase 1 findings 已知的 `/dev/null` 缺口教訓,`clacks.sb` 現有白名單已補過這個,但仍要在正式管線下重新過一次,不能只信 Phase 1 那次的 probe 結果)。

### 文件更新

- `templates/sandbox/clacks.sb`:參數註解更新(`HOME_CLAUDE`→`CONFIG_DIR`,反映現行 `CLAUDE_CONFIG_DIR` 機制,不再描述過時的 `~/.claude` 假設)。
- `docs/superpowers/specs/2026-07-17-clacks-design.md` 的安全模型表(§安全模型三層,約 line 123-127):taster 的 OS sandbox 欄標記為已接線(引用本文件);cyrano 欄改註記為「設計上該有,尚未接線,留待未來獨立評估」,不讓文件暗示已經做了但其實沒有。

## 刻意不做

- **不接 cyrano**:本次只補 taster(直接碰不可信輸入,風險最高的一端)。cyrano 的 OS sandbox 要用什麼 profile(設計文件原表「白名單專案目錄唯讀」細節未落地)留給下一輪獨立評估,不在本次範圍內混著做。
- **不加跨平台判斷**:直接假設 macOS,不寫 `cfg(target_os)` 或非 macOS fallback。專案目前完全是 macOS 專屬,YAGNI——不為不存在的需求寫防禦性程式碼。
- **不做動態 profile 組合**(例如依 `config_dir` 是否為 `Some` 動態拼接 profile 字串的不同段落):改用「`config_dir` 為 `None` 時整個跳過 sandbox 包裝」這個更簡單的規則達到同樣效果,避免維護兩份 profile 片段拼接邏輯的複雜度。
- **不改 `smoke.rs`**:它傳 `None` 是刻意設計(驗證不含 `CLAUDE_CONFIG_DIR` 隔離的基準線),本次的「`config_dir` 為 `Some` 才套用 sandbox」規則自然讓它維持現狀,不需要改動這個檔案。
- **不切分 taster/cyrano 的 `CONFIG_DIR`**:`gui.rs` 把同一個 `cli_config` 同時傳給 taster 與 cyrano(單一登入,避免 OAuth token-family 因雙帳號而 rotate),這是更早階段的既有決策,本次不變動。後果是本次的 OS sandbox 白名單把 taster 的寫入權放到這個雙方共用的目錄——`settings.json`、`plugins/`、`.claude.json` 對 taster 而言在檔案系統層是可寫的,OS 層因此**不隔離 taster 與 cyrano**。目前擋下「taster 寫壞 cyrano 讀的設定來下毒」這條路徑的,是 Claude Code 權限層(taster 零工具,沒有任何管道能主動寫檔案),不是本次新增的 OS sandbox。這是未來評估 cyrano 是否也要接 OS sandbox 時的已知輸入之一。
