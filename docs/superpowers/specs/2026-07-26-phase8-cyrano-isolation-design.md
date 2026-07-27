# Phase 8 — cyrano 隔離設計文件

> 日期:2026-07-26
> 前置文件:[architecture.md](../../../architecture.md)、[docs/superpowers/specs/2026-07-17-clacks-design.md](2026-07-17-clacks-design.md)(產品設計與安全模型,§安全模型三層表)、[docs/superpowers/specs/2026-07-25-phase7-sandbox-exec-taster-design.md](2026-07-25-phase7-sandbox-exec-taster-design.md)(taster 的 sandbox 接線,本文件沿用其結構性不變式)
> 狀態:已與使用者討論定案,待寫 implementation plan

## 背景:一個被三份文件宣稱、被零行程式碼執行的限制

cyrano(回應者 CLI)是唯一會讀到不可信訊息**內容**的一端。它的讀取範圍被限縮這件事,目前在三個地方被宣稱:

| 宣稱處 | 內容 |
|---|---|
| `2026-07-17-clacks-design.md` 安全模型表 | 「僅 allow 白名單目錄的 `Read`」 |
| `templates/cyrano/.claude/settings.json` | `"allow": ["Read"]` |
| `templates/cyrano/CLAUDE.md` | 「你**只能讀取**(`Read`),且限定於本工作目錄的白名單」 |

三處互相佐證,但**沒有一處真的執行它**。`settings.json` 的 `"Read"` 是不帶路徑的裸規則——Claude Code 的權限規則裡,裸工具名代表「任何路徑」,所以 cyrano 目前可以讀取整台機器上任何檔案。設計文件那句在 Phase 8 前的文件修正輪已改為據實描述,`CLAUDE.md` 那句尚未改。

Phase 7 只包了 taster 的 OS sandbox,並在其「刻意不做」明列 cyrano 留待獨立評估。本文件即那次評估。

> **打破這種循環的判準**:要驗證一個安全限制是否存在,必須指出**執行它的那一行程式碼或設定鍵**。指不出來,它就不存在——不管有幾份文件說它存在。

## 設計

### 範圍:只做「加白名單」,不做「砍黑名單」

三個候選方案:

- **A(採用)**:透過 `--add-dir` 給 cyrano 一個明確的唯讀專案目錄,同時把 `settings.json` 的裸 `Read` 拿掉,並把 cyrano 接上 Phase 7 既有的寫入 sandbox。
- **B(不做)**:在 OS sandbox 層加 `deny file-read*` + 讀取白名單,做真正的作業系統級讀取封閉。
- **C(不做)**:在 `settings.json` 加一堆針對性 deny 規則(如 `Read(~/.ssh/**)`、`Read(//etc/**)`)。

選 A 的理由:B 的爆炸半徑太大——claude CLI 自身要讀的路徑(二進位、node、憑證、locale、字型…)無法窮舉,`deny file-read*` 幾乎必然踩到啟動失敗,而且失敗模式是難以歸因的執行期錯誤;它值得單獨一輪、配合真機探測來做,不該和本輪混著。C 則是註定漏的做法:黑名單只能擋你想得到的路徑,而「想得到的路徑」永遠不是完整集合。

A 有一個必須誠實記錄的限制:**它不是作業系統層的讀取封閉**。它降低的是「cyrano 被訊息內容誘導去讀敏感檔案」的機率(權限層會要求核准,而 GUI 上人看得到),不是把讀取能力從行程上拿掉。真正的封閉是 B。本輪的價值在於把「三份文件宣稱、零處執行」變成「一處明確執行、文件與之相符」。

### 設定檔機制

唯讀目錄的路徑是部署環境相關的值,不該寫死在程式碼裡。採用類似 `.env` 的機制,但用 JSON:

- **`clacks.config.example.json`**(repo 根,版控):範本,值為 `"../clacks-runtime/cyrano-readable"`。
- **`clacks.config.json`**(repo 根,`.gitignore`):實際生效的檔案,使用者從範本複製。

```json
{
  "cyrano_readable_dir": "../clacks-runtime/cyrano-readable"
}
```

**本檔只放非機密設定**。bot token 仍然只經 `CLACKS_BOT_TOKEN` 環境變數(由 Keychain 取出)進入 Rust,**絕不落檔**——這是原始設計文件的既有規則(`不用 .env(token 絕不落檔)`),不因為本輪引入設定檔而鬆動。實作時必須確認 `ClacksConfig` 結構裡沒有任何 credential 欄位。

檔案位置在 repo 根,不在 `../clacks-runtime/` 之下——後者是 taster 的 sandbox 寫入白名單所涵蓋的目錄樹,設定檔放在那裡等於讓被隔離的一端能改自己的隔離設定。

### 分層落點

| 層 | 職責 |
|---|---|
| `core/config.rs` | `parse_config(&str) -> Result<ClacksConfig, ConfigError>`:純函式,字串進、驗證過的結構出。不碰 IO |
| composition root(`gui.rs`、`bin/pipeline.rs`) | 讀檔、canonicalize、存在性檢查、`.claude/` 檢查,把 `Option<&Path>` 交給 adapter |
| `adapters/pty.rs` | `cyrano_argv()` 組 argv;不做政策判斷 |

這符合 architecture.md 的依賴規則:驗證邏輯是純函式可窮舉測試,檔案系統互動留在唯一知道具體型別的那一層。

### argv 組裝:沿用 Phase 7 的單一來源不變式

新增與 `taster_argv` 對稱的函式:

```rust
fn cyrano_argv(
    workdir: &Path,
    config_dir: Option<&Path>,
    readable_dir: Option<&Path>,
    resume: bool,
) -> Vec<String>
```

`config_dir` 為 `Some` 時包 `sandbox-exec`(同一份 profile,`-D WORKDIR=` 指向 cyrano 的工作目錄);`readable_dir` 為 `Some` 時附加 `--add-dir <path>`;`resume` 為 `true` 時附加 `--continue`。

`spawn_continue()` 以本函式產生**初次 argv(`resume=false`)與 respawn argv(`resume=true`)**,兩者唯一差異就是 `--continue`。這直接沿用 Phase 7 為 taster 建立的不變式:兩份 argv 出自同一個函式,結構上不可能發生「首次有 sandbox / `--add-dir`,重啟後沒有」的靜默安全倒退。目前 `spawn_continue` 的兩份 argv 是各自寫死的字面量,正是這種倒退的溫床。

`config_dir` 為 `None`(`bin/smoke.rs` 的無隔離基準線)時不包 sandbox 也不加 `--add-dir`,與 Phase 7 的規則一致,`smoke.rs` 不需改動。

### sandbox profile 不需要修改

`templates/sandbox/clacks.sb` 是 `(allow default)` + `(deny file-write*)` + 寫入白名單。讀取本來就被 `allow default` 放行,所以 cyrano 用同一份 profile、只是 `-D WORKDIR=` 傳 cyrano 的工作目錄即可。**profile 內容零改動**,連帶對 taster 的行為零風險。

（這也再次說明本輪不是讀取封閉:profile 的 `allow default` 就是讀取全開。真正封閉要等方案 B。）

### `--add-dir` 的設定載入面與其緩解

`--add-dir` 不只是授予檔案存取——Claude Code 會從加入的目錄載入設定:

| 設定型別 | 是否從 `--add-dir` 目錄載入 |
|---|---|
| `.claude/skills/` 的 Skills | 會,且支援 live reload |
| `.claude/agents/` 的 Subagents | 會 |
| `.claude/settings.json` 的 `enabledPlugins`、`extraKnownMarketplaces` | 會(僅此二鍵) |
| `CLAUDE.md`、`.claude/rules/` | 僅在 `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1` 時 |

前三項是實質的指令注入面:唯讀目錄的預期用法就是「把一個專案資料夾放進去讓 cyrano 參考」,而真實專案帶著 `.claude/skills/` 是常態,不是牽強情境。

最後一項**已被既有結構擋住**:`pty.rs` 的 `ENV_ALLOWLIST` 是八項嚴格白名單(`PATH`/`HOME`/`TERM`/`LANG`/`LC_ALL`/`USER`/`SHELL`/`TMPDIR`),`CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD` 繼承不進子行程。Phase 2 為了排除 token 而做的 `env_clear` + 白名單,順帶封了這條路。

**緩解**:所有危險載入都位於 `.claude/` 之下,因此加一條啟動檢查——**唯讀目錄底下存在 `.claude` 時拒絕啟動**,錯誤訊息說明原因。這把「靜默載入未知指令」變成「大聲的啟動失敗」,而且是結構性的,不倚賴文件叮嚀。

這條約束必須落在會觸犯它的那一段程式碼(composition root 的設定載入路徑),不能只寫在本文件或 `templates/README`——這正是專案 CLAUDE.md 規劃守則明列的要求。

### 錯誤處理

| 情況 | 行為 | 理由 |
|---|---|---|
| `clacks.config.json` 不存在 | `readable_dir = None`,cyrano 照常啟動(無額外讀取範圍) | 往安全方向退。新機器尚未設定不該讓整個 app 起不來 |
| JSON 格式錯誤 / 型別不符 | **拒絕啟動**,錯誤訊息含檔案路徑與剖析錯誤 | 使用者明確表達了意圖但打錯字,靜默忽略等於靜默解除隔離。fail-closed |
| `cyrano_readable_dir` 為空字串 | **拒絕啟動** | 同上,無法區分「不想設」與「設錯」;不想設就刪掉整個鍵 |
| 指定目錄不存在 | **拒絕啟動**,訊息含 `mkdir -p` 建議 | 明確指定了一個不存在的目錄 = 設定錯誤 |
| 目錄底下有 `.claude` | **拒絕啟動**,訊息說明注入風險 | 見上節 |
| `config_dir` 為 `None`(smoke 基準線) | 不包 sandbox、不加 `--add-dir` | 沿用 Phase 7 規則 |

「檔案不存在 → 退回無額外讀取範圍」與「格式錯誤 → 拒絕啟動」看似不一致,但兩者都是 fail-closed:前者退到**更嚴格**的狀態(讀取範圍更小),後者的意圖無從得知故拒絕。沒有任何一條路徑會退到比預期**更寬鬆**的狀態——這是判斷標準,不是「有錯就爆」。

### 權限設定收斂

`templates/cyrano/.claude/settings.json` 的 `"allow": ["Read"]` 拿掉。移除後,cyrano 讀取工作目錄外的檔案會走 Claude Code 的預設核准流程(GUI pane 上人看得見),而 `--add-dir` 加入的目錄本來就在可存取範圍內,不需要 allow 規則背書。

deny 清單(`Bash`、`Edit`、`Write`、`Glob`、`Grep`、`WebFetch`、`WebSearch`、`Task`、`NotebookEdit`)維持不變。

### 測試策略

**core 單元測試(`parse_config` 窮舉)**:合法輸入、缺少 `cyrano_readable_dir` 鍵(→ `None`)、JSON 語法錯誤、值型別不符(數字/陣列)、空字串、含未知額外欄位(應忽略而非報錯,留設定檔演進空間)。

**adapter 單元測試(`cyrano_argv`)**:`config_dir` × `readable_dir` 的四種組合,乘上 `resume` 兩態;特別驗證 `resume=true` 的輸出除了多一個 `--continue` 之外與 `resume=false` 完全相同(不變式的直接斷言)。

**漂移防護測試**(比照 Phase 7 的 `profile_params_match_argv_params`):讀取 `templates/cyrano/.claude/settings.json`,斷言 `allow` 陣列不含裸 `"Read"`。這條測試存在的意義是:未來有人「順手」把 allow 加回去時,測試會擋下來,而不是等到下一次文件稽核才發現。

**真機驗證(human task,比照 Phase 1/4/5/6/7 模式)**:
1. 放一份檔案進 `cyrano-readable/`,從 Telegram 提問 → cyrano 應能引用其內容作答。
2. 提問一份範圍外的檔案 → GUI pane 應出現核准提示,而非直接讀出內容。
3. cyrano 在 sandbox 包裝下仍能開機、連得到 API、Stop hook 仍能寫 outbox、`--continue` 仍能續談。
4. 在 `cyrano-readable/` 下建一個空的 `.claude/` → 應拒絕啟動並顯示原因;移除後恢復正常。

### 文件更新

- `templates/cyrano/CLAUDE.md`:「限定於本工作目錄的白名單」改為據實描述(工作目錄 + 設定檔指定的唯讀目錄;其餘路徑需人工核准)。
- `2026-07-17-clacks-design.md` 安全模型表 cyrano 列:Claude Code 權限層與 OS sandbox 兩欄更新為本輪的實況,並保留「非 OS 層讀取封閉」這個已知限制的註記。
- `architecture.md`:目錄樹加入 `core/config.rs` 與 repo 根的兩個設定檔。
- `README.md`:新增「設定檔」小節,說明從 `clacks.config.example.json` 複製、`cyrano_readable_dir` 的用途,以及該目錄不得含 `.claude/`(JSON 不支援註解,故說明放 README 而非範本檔內)。

## 刻意不做

- **方案 B(OS 層讀取封閉)**:`deny file-read*` + 讀取白名單的爆炸半徑需要獨立一輪配合真機探測,不與本輪混做。本輪完成後它仍是 cyrano 隔離的最大剩餘缺口,應記錄為下一輪候選。
- **方案 C(針對性 deny 規則)**:黑名單註定漏,不引入會給人虛假安全感的半套機制。
- **多個唯讀目錄**:單一目錄夠用。需要多個時再改 schema(額外欄位被忽略的設計讓這件事不會破壞相容性)。
- **設定檔熱重載**:改設定重啟即可。
- **切分 taster/cyrano 的 `CONFIG_DIR`**:Phase 7 已記錄的既有決策(單一登入避免 OAuth token-family rotate),本輪不動。
- **`--settings` 或執行期產生 settings.json**:`--add-dir` 加上 `.claude/` 檢查已達成目的,不引入產生設定檔的機制(Phase 7 已因雙重註冊 hook 的問題排除 `--settings`)。
- **`/compact` 接線**:`core/session.rs` 的 `should_compact()` 仍零消費端,與本輪主題無關,不順手夾帶。
