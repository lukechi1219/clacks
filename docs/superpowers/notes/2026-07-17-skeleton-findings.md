# Walking Skeleton 整合發現

> 本文件是 Phase 2 port 語意的實證依據。每項發現寫:操作、觀察、對設計的影響。
> 標記說明:✅ 已驗證(附驗證方式)/ ⏳ 待真人實驗

## Stop hook 觸發時機

- ✅(真機實測 2026-07-17,Task 3 Step 5)**`CLAUDE_PROJECT_DIR` 解析為 CLI 的 cwd**(`runtime/echo`),非外層 git root——linchpin 成立。回覆結束後 hook 觸發,產物 `runtime/echo/outbox/1784301128-11516-reply.json`,`text` 與 TUI 回覆全文一致,抽取管道端到端正確。

## 工作目錄嵌套污染(真機實測,重大)

- ❌(真機實測 2026-07-17)`runtime/echo` 嵌在 clacks git repo 內,CLI 啟動時**跳出「是否 import 上層 CLAUDE.md」對話框**;同意後,echo 角色被完全覆蓋——回覆變成 repo 開發助理(知道 branch、未 commit 檔案、taster/cyrano 設計),50k+ tokens 的 repo 脈絡被吸入,`ECHO:` 前綴完全沒出現。
- 兩層含義:
  1. **自動化阻斷**:這個對話框(還有 trust 對話框)是啟動期互動式 UI,自動注入的訊息會打進對話框而不是 prompt——骨架/正式版的 PTY 自動 spawn 都會被擋
  2. **安全模型**:正式版 taster(隔離 LLM)的工作目錄若嵌在專案 repo 內,等於把整個專案(含安全設計文件)餵進被隔離者的 context——**runtime 工作目錄必須與專案 repo 隔離**(移出 repo,或各自成為獨立 git repo),此為 Phase 2+ 的架構修正項
- ❌(真機實測)`git init` 成獨立 repo **無效**:對話框變形為「Allow external CLAUDE.md file imports?」(因上層 CLAUDE.md 的 `@architecture.md` 指向 cwd 外)。選「No, disable external imports」後,**上層 CLAUDE.md 本文仍被載入**——回覆仍知道 Clacks/taster/cyrano。結論:**祖先目錄 CLAUDE.md 遍歷跨 git 邊界,唯一可靠隔離是 runtime 目錄搬出 repo 目錄樹之外**。
- ❌(真機實測)**角色強度不足(污染環境下)**:即使 `runtime/echo/CLAUDE.md` 被載入,CLI 把它當「關於此目錄的文件」描述(「This particular directory is the echo test role…」),而非執行角色——`hello` 沒有得到 `ECHO: hello`。
- ⚠️(真機事故)污染 session(開發助理模式,可及整個 repo)期間,`runtime/echo/.claude/{settings.json,hooks/extract-reply.sh}` 被從工作區刪除(git `D`,已 checkout 恢復)。**具體示範了為什麼隔離 LLM 不能看得到專案 repo**——連測試角色都會動到管線自身的設定檔。
- ✅(真機實測,決定性)**工作目錄搬出 repo 目錄樹(`../clacks-runtime/echo/`,無 .git、無祖先 CLAUDE.md)後全部正常**:無任何 import 對話框(只剩首次 trust)、`hello` → 乾淨的 `ECHO: hello`、hook 觸發、outbox 產物 `{"text":"ECHO: hello"}` 正確。
- **裁決**:(1) runtime 工作目錄一律放 repo 外——`clacks-runtime/` 成為固定架構決策;(2) 乾淨 context 下純 CLAUDE.md 的角色強度**足夠**,先前的角色失效是污染所致;`--append-system-prompt` 仍保留給不可協商安全規則(縱深防禦),但非角色生效的必要條件。

## /clear 行為

- ✅(真機實測 2026-07-18,Task 8)Telegram 發 `/clear` → 注入 CLI 後**被當成真正的 slash 指令執行**(非文字 echo),每次 `/clear` 開一個全新 session 檔(transcript 確認)。**關鍵:`/clear` 不產生 outbox 產物**(Stop hook 不觸發)。
- **骨架實測的完整因果**:骨架回覆 `[skeleton] timeout`——因為它的迴圈是「注入→等 outbox 產物(120s)」,而 `/clear` 不產物,於是傻等到 timeout。這揭示**核心 orchestrator 語意**:注入分兩類——**訊息注入(期待回覆產物)** vs **控制指令注入(`/clear`、`/compact`,不期待產物)**。Phase 2 的 `CliSession` port 必須區分兩者,SessionKeeper 注入 `/clear` 時不可套用「等產物否則 failed」的邏輯。
- **對設計的影響(正面)**:因 `/clear` 不產物,taster 每則消毒後注入 `/clear` 時 Rust 端**不會收到幽靈產物**,outbox 契約單純;代價是 orchestrator 要自己知道「這是控制指令、不等產物」。
- 觀察:`/clear` 執行明顯較久(完整 context 清除),SessionKeeper 注入 `/clear` 後需給足時間再注入下一則。⏳ 延遲量級待 Phase 2 量測。
- 附帶(格式變異實證):某次 echo 回覆為空 `{"text": "ECHO: "}`(模型未帶原文)。實證**角色指示不保證輸出格式**——正式版 taster 的「嚴格 JSON 契約 + Rust schema 驗證」正是為此存在;骨架先掙得證據。

## outbox 檔案事件語義

(FSEvents 對 create/modify 的合併行為、事件到達 vs 內容寫完的時序——Phase 2 port 語意的依據)

- ✅(程式碼審閱,final review)骨架的 outbox channel 有 **stale 事件誤配 race**:訊息 N 在 120s timeout 後才遲到的 hook 產物會留在 channel,被下一則訊息 N+1 的 `recv_timeout` 立即收走、當成 N+1 的回覆送出。骨架接受此行為;**Phase 2 `CliSession` port 必須定義「產物歸屬」語意**(產物內容需可關聯到觸發它的注入,或注入前 drain channel)。
- ✅(程式碼審閱)hook 以 `>` redirect 寫檔,Create 事件可能早於內容寫完——骨架用 200ms sleep 掩蓋;正式版應改 rename-into-place(temp 檔寫完後 `mv` 進 outbox)。順帶解決 outbox 檔名 `$(date +%s)-$$` 同秒碰撞問題。

## PTY 環境繼承

- ✅(portable-pty 0.9.0 原始碼確認,final review)`CommandBuilder::new` 以 `get_base_env()`(= `std::env::vars_os()`)**繼承整個父環境**給子行程。含義:任何 secret(bot token)只要在父行程環境,預設就會進 CLI。骨架已加 `cmd.env_remove("CLACKS_BOT_TOKEN")`;**Phase 2 `CliSession` port 應把「顯式最小環境、token 排除」定為一級需求 + 測試**,不能靠隱含約定。
- ✅(程式碼審閱)`telegram.rs` 的 `.expect` panic 會以 Debug 格式印出 `reqwest::Error`,其中 URL 含 token(Bot API 的 token 在 path,reqwest 只遮 userinfo 不遮 path)。骨架合法;Phase 2 adapter 必須 `map_err` 去除 URL 再上報。

## E2E 首跑失敗的四個發現(真機實測 2026-07-17)

- ❌→✅ **bracketed paste 的 `\r` 同寫不觸發送出**:「skeleton one」成功注入並顯示在 TUI 輸入框,但緊跟 `ESC[201~` 的 `\r`(同一次 write)沒有被當 Enter,文字停留在輸入框。修正:paste 信封與 `\r` 分兩次寫,中間延遲 150ms(`pty_input::bracketed_paste` 契約改為不含 `\r`)。⏳ 待重測確認。
- ⚠️ **token 洩漏實際發生**:poller 的 `.expect` panic 把含 token 的完整 URL 印上終端(且進了截圖)。token 已透過 BotFather 輪替作廢。骨架已改 `map_err(reqwest::Error::without_url)` 後再 expect;Phase 2 adapter 錯誤處理把「URL/憑證遮蔽」列為硬需求。
- ✅ **瞬時網路錯誤(os 53 ECONNABORTED)直接殺死 poller 執行緒**:骨架無重試(護欄如此),thread panic → `msg_tx` drop → 主迴圈結束、管線靜默死亡。Task 7 review 的預言有了實證;Phase 2 orchestrator 的重試/退避政策(設計文件已列)是必要品,且「poller 死亡」必須是可觀測事件而非靜默。
- ⚠️ **os 53 是本環境的系統性現象,非偶發**:E2E 第二跑再次發生(幾分鐘內兩次),第二次的連鎖效應完整觀測到:poller 死 → channel 關 → 主迴圈結束 → **行程退出把 PTY 帶走 → claude CLI 一併消失**(使用者觀感 =「claude crash」)。護欄豁免:poller 加最小生存迴圈(get_updates 改回 Result,錯誤時固定 3s 重拉、無退避)——沒有它 E2E 根本跑不完;退避/斷路政策仍留給 Phase 2。root cause(VPN/防火牆/代理?)未查,列 Phase 2 環境調查項。
- ✅ **TUI 逃逸序列穿透**:claude TUI 啟用 mouse tracking / focus reporting,骨架原樣轉發到真實終端,滑鼠移動即出現 `^[[<35;82;46M` 亂碼、焦點切換出現 `^[[I`/`^[[O`。對骨架無害;正式版 xterm.js 會正確消化這些序列。附帶:焦點/滑鼠序列會迴流到骨架 stdin(骨架不讀 stdin,無實害)。

- ✅ **E2E 完成線達標(2026-07-18,第四跑)**:連續兩則訊息端到端走通、順序正確——paste 信封 + 延遲 150ms 單獨送 `\r` 的注入修正確認有效;poller 生存迴圈下管線存活。walking skeleton 的四大假設中三項(PTY 注入、Stop hook、隔離工作目錄)已全數實證,剩 sandbox-exec。
- ✅ **OAuth refresh 競爭連鎖失效(強佐證)**:re-login 後骨架的 CLI 仍要求 login;停掉 happy daemon(持舊 token family 的並行實例)→ 再 re-login → 立即啟動骨架 → 成功。機制推定:舊實例用已輪替的 refresh token 刷新觸發 reuse 保護,整個帳號 token family(含新登入)被撤銷。**部署含義**:clacks 兩個常駐 CLI 與同帳號其他 claude 實例共存是環境級風險——需列入部署前提(專用帳號、或確保無其他實例背景刷新)。
- ✅ **CLI 可在任意時點要求 re-login,而骨架無 stdin 橋接 = 死鎖**(E2E 第三跑實測):claude 啟動時跳 re-login 對話框,使用者在骨架終端按 Enter 只進骨架 stdin(無人讀取),TUI 永遠收不到。兩層含義:(1) 疑似成因是**多個並行 claude 實例競爭 OAuth token refresh**(本機同時有 happy daemon、`claude -p` cron 等實例)——正式版兩個常駐 CLI 同樣暴露於此;(2) 設計文件的「每個 pane 附手動 prompt input」從 nice-to-have 升級為**必要品**:trust/import/login 這類啟動期與隨機互動對話框,orchestrator 必須能偵測「CLI 等待互動」狀態並讓人工介入,否則管線死鎖。

## sandbox-exec

- ✅(真機實測 2026-07-18,Task 9)`sandbox-exec -f sandbox.sb`(profile:`allow default` + `deny file-write*` + 白名單 WORKDIR/HOME_CLAUDE/tmp)下 **claude 正常啟動、連得到 Anthropic API、`sandbox probe` 得到乾淨 `ECHO:` 回覆**。
- **核心矛盾裁決**:taster「**完全無網路」不可行**——CLI 本身必須連 API。設計文件安全模型表的「無網路」須改為「**檔案系統隔離為主**(唯讀白名單 + 限制寫入),網路層面只能限制到『允許 Anthropic API』而非全禁」。⚠️ 待改:設計文件安全模型表 taster/cyrano 的「OS sandbox」欄。
- ✅ Stop hook 在 sandbox 下**有觸發並成功寫入 outbox**(檔案存在)——證明 WORKDIR 白名單放行 outbox 寫入,`sandbox.sb` 的檔案寫入策略對管線本身可行。
- ⚠️ **profile 缺口(實測抓到)**:`security-guidance` plugin hook 報 `line 48: /dev/null: Operation not permitted`——`deny file-write*` 擋掉了 `/dev/null`。non-blocking 故 echo 仍成功,但正式 profile 必須白名單 `/dev/null`(及可能的 `/dev/urandom` 等常見 device)。教訓:**profile 太緊會讓 hook 靜默失敗**,需逐一補齊必要 device/path。
- ⚠️ **hook 抽取 race(thinking 區塊,重要)**:本次 outbox 產物為空 `{"text":""}`,但畫面有 `ECHO: sandbox probe`。根因:assistant entry 序列為 `thinking` → `text`,**Stop hook 在最終 `text` 區塊 flush 進 transcript 前就執行**,「最後一個 assistant entry」當下是純 thinking(無 text)→ 抽空。Opus high-effort 思考模式下此 race 更易發生。修正方向(Phase 2 hook 契約):jq 應取「**最後一個含 text 區塊的 assistant entry**」而非單純 last;或 hook 加重試/等 transcript 穩定。這也強化了 taster 必須有 JSON schema 驗證(空回覆要能被判為 failed 而非送出)。

## Telegram 整合對照(nexus receiver)

參照 `../nexus/receiver/telegram/fetch.py`(Pyrogram/MTProto User API,已在生產 cron 運行)與 clacks skeleton(Bot API long-poll)的比對,2026-07-17:

- ✅ **去重模式互相印證**:nexus 用 `.last_message_id` 狀態檔 + `msg.id <= since_id` 早停;skeleton 用 `next_offset(update_id+1)`。同構,但 nexus 把狀態**落地**(cron 重啟不重複收),skeleton 只在記憶體(重啟會重收 backlog)——Phase 2 `MessageStore`(rusqlite update_id 去重)的必要性有了活佐證。
- ✅ **單實例約束**:nexus 用 `flock` 防 cron 重疊。clacks 對應風險更硬:Bot API `getUpdates` 同 token 雙 poller 會 409 Conflict。骨架單行程無虞;Phase 2 需考慮行程重啟/重複啟動的防護。
- ✅ **非文字訊息盲點**:skeleton 只取 `msg.text`,**帶 caption 的照片(text 空、caption 有字)會被靜默丟掉**;nexus 另外處理 media/caption/location。骨架期正確(細線);taster 管線正式設計時必須明訂非文字訊息政策(拒收並告知?只取 caption?)。
- ✅ **測試隔離(已解決)**:nexus 的目標 chat 是 @backlog_general_bot(個人 capture bot,cron 每 5 分鐘收「Luke 與它的對話」進 `raw/` 日記),clacks 不可共用。**已決定:clacks 測試用 @ChatSummary_37927_bot**,token 經 Keychain 注入 `CLACKS_BOT_TOKEN`(見設計文件「Bot token 存放」)。前提:確認沒有其他行程拿同一 token 打 `getUpdates`(否則 409 Conflict)。
- ✅ **webhook 互斥(實測,2026-07-17)**:該 bot 原掛 Pipedream webhook,`getUpdates` 直接 409(「webhook is active」)。已徵得同意刪除;若要還原 ChatSummary 整合:`curl "https://api.telegram.org/bot<token>/setWebhook?url=https://54f734a3070a184fcf573fc53666fed6.m.pipedream.net&allowed_updates=%5B%22message%22,%22edited_message%22%5D"`。教訓:**接手既有 bot 前先 `getWebhookInfo`**——正式版 clacks 啟動時應檢查並提示,而非讓 poller 默默 409。

## Phase 2 smoke(真機實測 2026-07-18)

smoke bin(`src-tauri/src/bin/smoke.rs`,echo 管線全程走 ports/adapters)對照 plan Task 8 checklist:

- ✅ **env_clear 最小環境下 claude 正常開機**:白名單(PATH/HOME/TERM/LANG/LC_ALL/USER/SHELL/TMPDIR)足夠,無 login/trust 要求——「顯式最小環境」的真機驗證通過,token 結構性排除確立。
- ✅ 連續兩則訊息 ECHO 順序正確;`/clear` **立即**收到 `[smoke] control injected`(不再空等 120s timeout)——`inject_control` 不等產物的 port 語意實戰驗證。
- ✅ outbox 只有 `*-reply.json`(5 檔),無 `.partial` 殘留——hook rename-into-place 契約 + watcher 雙接事件端到端成立。
- ✅ **token 遮蔽實戰驗證**:smoke 期間出現瞬時 poll 錯誤,終端印出的錯誤僅 `("error sending request")`,無 URL 無 token;全程輸出檢查無 token。
- ⚠️ **控制指令後立即注入的競態(⏳ 轉 ✅ 實證)**:`/clear` 尚在處理時注入「after clear」,得到 `ECHO: ` + 模型自行發揮(「(管線就緒,等待訊息。)」)——推定 paste 信封在 `/clear` 處理期間被 TUI 丟棄,殘留的 `\r` 送出空 prompt。重送一次即正常。骨架期「`/clear` 需給足時間再注入下一則」的觀察正式實證;**Phase 3 SessionKeeper 必須在控制指令後留緩衝**(固定延遲或狀態偵測;延遲量級未量測,列 Phase 3 量測項)。
- ⚠️ **全域 `~/.claude` 設定滲入隔離 CLI(新發現,重要)**:echo CLI 內觸發了使用者全域 `UserPromptSubmit` hook(`timed out after 10s — output discarded` 上畫面)。成因:HOME 在 env 白名單(CLI 登入/OAuth 必需)→ 全域 settings/hooks/plugins 一併載入。含義:**隔離邊界只隔 workdir 層(CLAUDE.md/settings),不隔 user 層**——正式版 taster/cyrano 部署前提需盤點全域 hooks 對隔離角色的影響(噪音、逾時、行為改變);候選解法:專屬乾淨帳號/HOME、或 CLI 層面停用 user 設定(待查可行性),列 Phase 3+ 設計項。

## Phase 3 規劃承接項(2026-07-18 裁決)

- ✅(final review Important,使用者裁決)**`ClaudePtySession` teardown/kill 列 Phase 3 一級任務**:目前無 Drop/teardown,session 被替換或重啟時 claude child 會孤兒化(portable-pty 的 `Child` 不保證 kill-on-drop)。依規劃守則「約束落到會觸犯它的那個任務」——Phase 3 plan 中凡引入 session 重啟/替換(PtyManager 自動重啟、`claude --continue` 恢復)的任務,必須同時實作顯式 teardown(kill + wait)並附驗證步驟(如:重啟後 `pgrep` 確認無殘留 claude 行程),不得只寫在 Global Constraints。

## Phase 4 規劃承接項(2026-07-18,Phase 3 final review 裁定)

- ⚠️(final review Important,**安全義務**)**SessionLost 恢復設計必須保證 taster 乾淨**:安全路徑動作序列 [InjectCyrano, ClearTaster] 中,若 InjectCyrano 注入失敗,迴圈在執行 ClearTaster 前中斷 → 健康的 taster 帶著剛判定完的訊息 context 停留(消毒者無記憶被打破),且 SessionLost 不記名是哪個 session 死了。Phase 4 凡引入 session 重啟/恢復的任務,必須保證恢復後 taster 是乾淨的(兩個 session 一起重啟、或補 /clear),並沿用 teardown 裁決的驗證要求
- ⚠️(final review Important,**release-gating**)**telegram.rs 無 `.error_for_status()`**:non-2xx 回應以 decode 錯誤浮現、send_reply 對失敗送出**靜默回 Ok**——orchestrator 的 SendFailed 可信度取決於 gateway 如實上報。Phase 4 真實部署前必須補(Phase 2/3 均刻意不動 telegram.rs)
- 觀察(硬化測試候選,順手做):(1) contract 補「合法 JSON 後跟垃圾字尾」回歸測試(現行為 by construction 正確,防未來換 streaming deserializer 時靜默放行);(2) store 失敗時該則訊息 offset 已推進、不重試(與 gateway 錯誤重試不對稱)——fail-closed 哲學一致但未文件化,Phase 4 決定 halt/retry/skip 並補註解
- 觀察:`Clock` port 與 SystemClock/ManualClock 在 Phase 3 無消費端(orchestrator 用注入 sleep + port 內部 recv_timeout)——Phase 4 確認真有需要(timeout 記帳/GUI)再接,否則考慮刪 port

## Phase 4 ~/.claude 隔離調查(2026-07-19,Task 7,真機實測)

承接 findings「Phase 2 smoke」的全域 ~/.claude 滲入問題。以 `../clacks-runtime/echo-probe`(echo 範本,CLAUDE.md 僅三行、無任何 @import)在 `ENV_ALLOWLIST` 最小環境下實測 `CLAUDE_CONFIG_DIR` 能否隔離 user 層。裁決:**部分可隔離**。

### CLAUDE_CONFIG_DIR 有效隔離的(✅ 實證)

對照組(Step 1,現行 spawn 環境,HOME 真、無 CLAUDE_CONFIG_DIR)vs 隔離組(Step 2,加 `CLAUDE_CONFIG_DIR=../clacks-runtime/probe-config`):

- ✅ **MCP servers**:對照組畫面頂端有「4 MCP servers need authentication」banner(user 層 jira/notion/postman/trello 設定滲入);隔離組 banner 消失。
- ✅ **model / effort**:對照組顯示「Sonnet 5 with medium effort」(user 設定);隔離組顯示乾淨預設「Opus 4.8 (1M context)」——user 的 model/effort 設定未帶入。
- ✅ **plugins / settings / sessions / history / OAuth**:全數在 `probe-config/` 重新生成(`.claude.json`、`settings.json`、`plugins/`、`sessions/`、`history.jsonl`)。`probe-config/.claude.json` 的 `oauthAccount=true`——**登入態落在 config dir**,故同一 config dir 的後續啟動不需重登。

### CLAUDE_CONFIG_DIR 未隔離的(❌ 洩漏點,決定性證據)

- ❌ **user 全域 CLAUDE.md(`$HOME/.claude/CLAUDE.md`)仍被載入**:隔離組啟動時仍跳「Allow external CLAUDE.md file imports?」對話框,列出 `/Users/lukechimbp2023/.claude/RTK.md`。因 echo 的 CLAUDE.md 無任何 @import、`probe-config/` 無 CLAUDE.md,此 `@RTK.md`(全域 `~/.claude/CLAUDE.md:190`)只可能來自 user 全域 CLAUDE.md 被載入。**user memory 靠 HOME 定位,與 CLAUDE_CONFIG_DIR 無關**——外部 import(RTK.md)可選「No」擋掉,但父檔 `~/.claude/CLAUDE.md` 本文(190+ 行的 workflow 編排、TDD、subagent 策略…)不可拒、照載。
- 對隔離角色的影響:taster(最小消毒者 + 嚴格 JSON 契約)吃進這些指示 = context 污染,可能誘導契約外輸出(縱深防禦靠 `--append-system-prompt` 強化 + core 嚴格 schema 兜底判 failed)。

### 操作面發現(headless pipeline 的硬前提)

- ⚠️ **全新 CLAUDE_CONFIG_DIR 會連跳互動式對話框**:theme picker → trust folder → external-imports 決策 →(Step 1 另見)login。**pipeline 無 stdin 橋接,這些對話框會直接死鎖**(同骨架的 re-login 死鎖 finding)。故每個 config dir 必須**先以互動方式一次性 pre-seed**(onboarding + login + trust + imports 決策)後,headless pipeline 才能非互動啟動。

### 裁決

1. **採用 CLAUDE_CONFIG_DIR 為 Phase 4 隔離機制(部分隔離)**:消掉 MCP banner、plugin hooks、user model/effort、settings 滲入——對 taster/cyrano 是實質淨化。落點:未來 `adapters::pty` 的 spawn 需允許 per-role config dir(taster/cyrano 各一;`ENV_ALLOWLIST` 加 `CLAUDE_CONFIG_DIR` 或 spawn 顯式帶入)——**此為程式碼變更,不在 Task 7 範圍**(Task 7 只定案機制)。
2. **user 全域 CLAUDE.md 洩漏 = 殘留缺口**:CLAUDE_CONFIG_DIR 不隔 user memory。緩解靠 taster `--append-system-prompt` + core 契約驗證兜底。
3. **完全隔離(HOME 重定位)列 Phase 5 真機驗證項**:把 HOME 指向專屬乾淨家目錄(如 `../clacks-runtime/taster-home`)可令 `$HOME/.claude/CLAUDE.md` 解析到受控空檔——但 HOME 重定位與 OAuth 的交互**未驗證**(骨架曾認為 HOME 為 OAuth 必需;然本次實證 OAuth 已落 CLAUDE_CONFIG_DIR/.claude.json,HOME 或可改)。可行則達完全隔離,不得事前斷言,Phase 5 證實。
4. **部署前提(Task 9 與正式版)**:taster/cyrano 各自的 config dir 必須預先 pre-seed(互動完成 onboarding + login + trust + imports),否則 headless spawn 死鎖。

## Phase 4 Task 9 真機 smoke(2026-07-19,進行中)

真雙 CLI 管線(`bin/pipeline.rs`)真機端到端。記已確認發現;#3/#5/#6 待以正確方法補做。

### 已通過
- ✅ 路徑修正後雙 CLI 乾淨啟動(無 onboarding/login/trust)——見下「真機 bug #1」。
- ✅ **#1 乾淨訊息端到端**:taster safe → cyrano 生成回覆 → Telegram 送達(cyrano TUI + `訊息結果:Replied` 實證)。
- ✅ **#2 惡意訊息**:`RejectedByTaster`,不進 cyrano、不回覆。

### 真機 bug #1(已修 commit):CLAUDE_CONFIG_DIR 相對路徑歧義
- `pipeline.rs` 以相對 `../clacks-runtime/cli-config` 當 CLAUDE_CONFIG_DIR 傳給 CLI 子行程。pipeline 自己的 pre-seed 檢查相對 repo root → 通過;但子行程 cwd=workdir(≠pipeline cwd),CLI 對**自己的 cwd** 解析相對路徑 → `clacks-runtime/clacks-runtime/cli-config`(垃圾巢狀目錄,已實證生成)→ 誤判全新 config → 跳 onboarding/login(headless 死鎖)。
- 修:`canonicalize("../clacks-runtime")` 後全部 `join` 絕對路徑。
- **暴露的縫**:pipeline bin 無整合測試;相對/絕對路徑歧義單元測試抓不到(tempdir 皆絕對)。Phase 5 若加 bin smoke 測試可補。

### Phase 5 設計輸入 A:注入前需 idle 偵測(「產物」≠「可輸入」)
- **現象(#7,連發 ~1s 兩則短訊)**:第二則帶**空內容**進到 cyrano(信封在、sanitized_text 掉了),cyrano 回「內容好像沒帶到」。
- **定位**:排除 taster 判定出錯——safe=true + 空 sanitized_text 會被 `parse_verdict` 判 `ContractViolation`、根本到不了 cyrano;既然到了,代表注入 cyrano 的 **bracketed paste 被截斷**。
- **機制**:Rust 等的是 Stop hook 產物(=生成結束),但「生成結束」≠「TUI 回到可接受 bracketed paste」。`CONTROL_BUFFER`(2s)只擋在 taster `/clear` 之後;cyrano「剛回覆完 → 下一則注入」之間**無任何緩衝**。訊息短 → 整條循環快 → cyrano 的下一則注入砸在它上一則回覆的**收尾空窗** → 掉字。
- **非 queue/重疊問題(已排除)**:orchestrator 循序阻塞已確認——`poll_once` 逐則、`process_update` 阻塞等產物(taster 再 cyrano)才換下一則。msg2 不會在 msg1 回覆送出前開始。所以這是**注入時序穩健性**,不是缺 queue。
- **Phase 5 正解**:注入前做 **idle 偵測**(確認 CLI 真的回到可輸入狀態),而非再加死 sleep;與 GUI 的 PTY 生命週期一起做。
- **taster 也中,單發也會(#3 追查實證,transcript 定位)**:#3 破格失敗(taster 無視弱破格行、始終吐合法契約 JSON,故無 NotJson);但過程中出現 `{"text":""}` → `ContractViolation(EmptyReply)`。查 taster transcript:產生空產物的 session **注入訊息為 None**(信封整段沒進 session)、assistant **無任何產出**——即 **bracketed paste 被完全吞掉,只剩延遲送出的 `\r` 送了空 prompt**,taster 對空 prompt 產不出東西 → Stop hook 寫空 → core 判 EmptyReply。對照成功 session 皆有完整注入訊息 + 合法 JSON。故機制是**注入整段吞掉(非 thinking-race)**,且**單發訊息也會發生**(不限 #7 連發),taster/cyrano 皆受影響。
- **安全不變量全程守住**:每個空/掉的 taster 輸出 core 都判 EmptyReply 擋下、未送 cyrano;每個合法的才放行。**這是可靠性問題(訊息靜默未回、需重送),非安全漏洞**——嚴格 schema 正是為此存在(#3 的目的由 EmptyReply 路徑實證,非 NotJson)。

### Phase 5 設計輸入 B:headless 無法 Ctrl-C 停止 + 信號無 teardown
- **現象(#8)**:Ctrl-C 停不掉 pipeline。
- **成因**:cyrano TUI 經 PTY 把 **kitty keyboard protocol** 跳脫序列(`^[[…u`)打到外層終端 → Ctrl-C 不再產生 SIGINT,變成序列送進沒人讀的 pipeline stdin。加上 pipeline **無信號處理器**,被 SIGTERM/`pkill` 殺掉時 `Drop` teardown 不跑 → 孤兒 claude 子行程。
- **正確停法**:另一終端 `pkill -f 'target/debug/pipeline'` + 清孤兒。**辨識子行程的可靠法**(實測修正):claude 子行程命令列只有 `claude`/`claude --continue`,`pgrep -f 'clacks-runtime/…'` 抓不到;改用 `pgrep -P <pipeline_pid>` 抓直接子行程。**`lsof -p <pid> -d cwd` 在本機 macOS 對非自身行程會回報失真的 `/`(權限限制,非真實 cwd)**——改用 `lsof -p <pid> | grep -Eo '[^ ]*taster[^ ]*|[^ ]*cyrano[^ ]*'`(掃打開檔案路徑,如 transcript/outbox,不受 cwd 描述符權限影響)可靠辨識角色。
- **Phase 5 正解**:GUI 版由 Rust 管 PTY 生命週期、外層終端不被 CLI 序列汙染;若保留 headless bin,需加 SIGINT/SIGTERM handler 跑 teardown。

### Phase 5 設計輸入 C:respawn 後同樣需要 settle(非僅開機初次需要)
- **現象(#5/#6 真機實測)**:手動 kill cyrano 後,下則訊息觸發 `SessionLost`→`recover()`(taster+cyrano 一起重啟,pid 皆變:97675→13781、97676→13782,**#6 通過**)。但緊接著處理下一則時又立即撞到 `CLI 注入失敗:Input/output error (os error 5)`(EIO,PTY 對面行程已消失的典型訊號),再觸發一次 SessionLost→recover 才穩定;第二輪之後 pipeline 恢復正常、無再出錯,手機最終收到正確回覆。
- **定位**:`ClaudePtySession::spawn`(pipeline 首次開機)後有顯式 15 秒等待(`[clacks] 等待雙 CLI 開機 15s`),但 `respawn()` **沒有等價的 settle 等待**——重啟後立刻可能被拿去注入下一則,新行程可能還沒完全就緒(或前一個行程 teardown 尚未收乾淨、PTY fd 交接有短暫空窗),導致一次性注入失敗。
- **與設計輸入 A 同類**:都是「注入時機 vs CLI 真實可用狀態」的落差,只是空窗發生的位置不同(A = 連續訊息之間、C = respawn 之後)。
- **非阻斷性**:系統靠既有的重試機制(cyrano 無 dirty 閘,靠下次 Lost 自動重試;taster 有 dirty 閘)自行收斂,最終狀態正確、無需人工介入,**符合設計預期的自癒行為**,只是多繞一圈。
- **Phase 5 正解**:與設計輸入 A 同一機制解決——idle 偵測應同樣覆蓋在 `respawn()` 之後,而非只在原始 `spawn()` 開機時有。

### 待補(以正確方法)
- **#3 畸形 JSON**:taster 對弱破格指令無視(全程輸出合法契約 JSON,無 NotJson);過程改追出 `EmptyReply` 路徑(見上,#3 目的已由此路徑實證)。若要親見 NotJson,需換更強破法(如誘導多一個欄位觸發 `SchemaMismatch`),非必要。
- **#5/#6:已完成,見上「Phase 5 設計輸入 C」——PASS**(recover 兩 session 一起重啟確認、無 security 問題、僅有 settle 時機縫)。
- **#7 量測**:單發乾淨性數據點仍待量(區分「收尾空窗」與「bracketed paste 本身不可靠」)。

## Phase 5 設計輸入 B 裁決(2026-07-20)

headless pipeline 有兩個缺口:(1) cyrano TUI 經 PTY 把 kitty keyboard protocol 序列(`^[[…u`)打到外層終端 → Ctrl-C 不再產生 SIGINT;(2) pipeline 無信號處理器,被 SIGTERM/`pkill` 殺時 `Drop` teardown 不跑 → 孤兒 claude 子行程。

**採「GUI 接管 PTY 生命週期使 B 對受支援路徑失效,headless bin 降級 dev-only,且不對 headless 加信號處理器」。**理由三點:

1. **正面消除而非補丁**:GUI 版 PTY bytes 只進 xterm.js pane、不進控制終端(Global Constraints 2),kitty 序列污染外層終端的根因不復存在,Ctrl-C 語意在 GUI 宿主完全正常;視窗關閉 → GUI stop 路徑主動 drop session → `Drop`→teardown 跑(Global Constraints 6),孤兒問題消除。故 B 對受支援部署路徑(GUI)結構性失效。

2. **headless 加信號處理器與「不重寫 orchestrator」衝突**:`run_forever` 是 `-> !`(永不返回),信號處理器在獨立 thread 無法 drop `main` 堆疊上持有的 session,只能 `process::exit`(不跑 Drop)或自行 SIGKILL 子行程——要跑 teardown 就得改 `run_forever` 讓它以旗標跳出,即重寫 orchestrator 迴圈(Global Constraints 8 禁止)。GUI 版之所以能乾淨關閉,正是因為它用自己的 poll 迴圈(Task 9)而非 `run_forever`。

3. **dev-only 的孤兒清理已有既定程序**:findings「設計輸入 B」已記錄可靠辨識/清理法(`pgrep -P <pid>` 抓直接子行程、`lsof | grep taster/cyrano` 辨角色、`pkill -f 'target/debug/pipeline'`)。dev 場景可接受手動清理,不值得為此違反 Global Constraints 8。

**約束落點:** 此裁決即 Global Constraints 2 與 6 的依據;其「一個任務具體執法」落在 **Task 9**(GUI 的 output 只進 emitter、stop 主動 drop session)與 **Task 12**(真機 pgrep 驗無孤兒)。本任務只定案 + 標註,不編碼。

**headless `bin/pipeline.rs` 自 Phase 5 起為 dev-only,不加信號處理器;受支援部署路徑為 GUI。**

## Phase 5 final review 承接項(2026-07-20,opus 全分支審查)

- ⚠️(Important,**Task 12 真機必查**)**人工介入通道與 30s long-poll 共用同一 thread,worst-case 延遲達 ~30s**:`gui.rs` 的 pipeline thread 在每輪迴圈開頭排空 `input_rx`(轉呼叫 `write_raw_to`),隨後才呼叫 `orchestrator.poll_once`——後者內部 `getUpdates` 是 30s long-poll(`telegram.rs` timeout=30)。若使用者在 `poll_once` 正在等待中途才按 pane 的人工輸入送出(如 trust/login 對話框介入),要等當輪 long-poll 返回才會被轉發,worst-case ~30s。這直接牴觸本 phase 設計動機之一(pane + 輸入框消除 headless 的 pre-seed 死鎖需求)——若對話框本身在 30s 內逾時,人工回應可能來不及送達。**Task 12 真機 smoke 必須刻意驗證**:對一個存活中的 trust/login 對話框在 long-poll 進行中打字,量測實際延遲是否可接受;若不可接受,候選修法為縮短 `getUpdates` timeout 或把人工輸入排空移到獨立更短週期的通道(需再設計,不在本 phase 範圍)。
- 觀察(非 blocking):`write_raw_to`(人工輸入)不經 `wait_idle` 就緒偵測(僅 orchestrator 自己的訊息注入路徑會 settle)——裁定為合理設計(人正盯著 pane 手動打字,不需要 idle 閘),但值得補一行程式碼註解讓不對稱性顯式化。
- 觀察(非 blocking,已知,Task 9 ledger 已記):開機期(15s sleep)按 stop 會阻塞至 sleep 結束;`fatal` 自我退出後前端 start 按鈕未重新啟用(cosmetic)。
- 流程觀察(非 blocking):本 phase 累積 4 個「brief 自身文字與其驗證指令自相矛盾」的 plan-clarity signal(Task 1/4/8 各一 + Task 2 的高並行計時假設)。final reviewer 獨立判斷:分布在「驗證指令精確度」而非設計/安全面,不構成系統性 plan 品質問題;建議未來 plan 若驗證指令本身是中間態(如 Task 1 的 RED 態),應在 plan 內明講而非讓執行者自行發現矛盾。

## Phase 5 HOME 重定位完全隔離驗證(2026-07-21,Task 11,真機實測)

以 `HOME=../clacks-runtime/taster-home`(空目錄)+ 既有 `CLAUDE_CONFIG_DIR=../clacks-runtime/cli-config` 啟動 probe workdir 的 claude,對照 findings「Phase 4 ~/.claude 隔離調查」未驗證的 HOME × OAuth 交互假設。

### 實測結果

- ✅ **OAuth 不受影響**:登入態不需重新來過,如預期落在 `CLAUDE_CONFIG_DIR/.claude.json`(Phase 4 已證實),與 HOME 無關。
- ❌ **user 全域 CLAUDE.md 洩漏未消除**:仍跳出「Allow external CLAUDE.md file imports?」對話框——HOME 重定位到空目錄**沒有**擋掉這個對話框的觸發。
- ❌ **決定性反證**:CLI 自動載入了指向**真實使用者家目錄**的 skill 設定(如 `Path: /Users/lukechimbp2023/.claude/skills/yellow-duck-n-review`),而非 `HOME` 重定位後的空目錄。證明 CLI 對「使用者家目錄」的解析**至少部分不經過 `$HOME` 環境變數**(可能透過 OS API 如 `getpwuid` 直接查真實帳號家目錄,或有獨立於 `$HOME` 的設定路徑),`HOME=` override 對這條路徑無效。

### 裁決:**不可行(fallback)**

HOME 重定位**無法**達成完全隔離——user 全域設定(至少含 skills)有一條不經 `$HOME` 環境變數的解析路徑,直接查真實帳號家目錄,`HOME` override 只能影響部分行為(不影響 OAuth,但也不擋 CLAUDE.md 洩漏與 skills 載入)。維持 Phase 4 的部分隔離方案為終局:

1. **`CLAUDE_CONFIG_DIR`**(隔 MCP/model/plugins/settings/OAuth)+ taster 的 `--append-system-prompt`(縱深防禦,強化角色指示抗污染)+ core 嚴格契約驗證(`parse_verdict` 的 `deny_unknown_fields`——契約外/被污染誘導的輸出兜底判 failed)三層合計是本專案可達到的隔離上限。
2. **不再規劃 HOME 重定位相關的程式碼佈線**——`adapters::pty` 不需為此新增邏輯,此調查項到此收尾。
3. 殘留風險(user 全域 CLAUDE.md/skills 滲入 taster context)維持已知、已記錄、靠縱深防禦緩解的狀態,不視為 blocking(taster 是消毒者非決策者,契約驗證是最終防線)。

## Phase 5 Task 12 真機 smoke(2026-07-21,進行中)

真 GUI(`bin/clacks-gui`)真機端到端。記已確認發現;checklist 逐項勾稽中。

### 真機 bug #2(已修 commit b1d6de9):tauri.conf.json frontendDist 指向未建置原始碼

- 現象:`cargo run --bin clacks-gui` 開出空白視窗,無任何按鈕。
- 根因:Task 7 建 `tauri.conf.json` 時 Task 8 的前端與 `vite.config.ts` 尚未存在,`frontendDist` 沿用初版猜測值 `"../src"`(原始碼目錄,`index.html` 的 `<script type="module" src="./main.ts">` 指向未編譯的 TypeScript)。編譯期無法抓到(`cargo build` 只檢查 Rust 側,不驗證 frontendDist 內容可執行)——只有真的開視窗才會暴露。
- 修:`frontendDist` 改指向 `npm run build` 的實際輸出目錄 `../dist`。
- **暴露的縫**:debug build(`cargo run`,非 `cargo tauri dev`)下 Tauri 會優先用 `devUrl`(`http://localhost:5173`)而非 `frontendDist`——這是為了讓 dev 期熱重載生效的預期行為,但意味著**真機跑 GUI 必須額外開一個終端機跑 `npm run dev`**,否則連不上 devUrl(`Failed to load resource: Could not connect to the server`)。已記入操作前提。

### 真機 bug #3(已修 commit 1a7191e):缺 capabilities 目錄,啟動管線按鈕無反應

- 現象:畫面正常顯示後,按「啟動管線」完全無反應——無狀態變化、無錯誤對話框。
- 根因(讀 `tauri-utils` 原始碼 `acl::get_capabilities` 確認):Task 7 建 Tauri scaffolding 時從未建立 `src-tauri/capabilities/` 目錄。Tauri v2 的 ACL 系統下,主視窗若無任何 capability 授權,IPC bridge 對所有 `invoke()` 一律**靜默**拒絕(不拋可見錯誤、不進 devtools console 明顯訊息)——前端 `start_pipeline` 按鈕的 `await invoke(...)` 因此卡住,後續的 UI 狀態切換都不會執行。此問題 Task 9/10 的 per-task review 都沒抓到,因為兩者都只驗證「編譯成功」與「靜態接線存在」,沒有真的觸發一次 runtime IPC 呼叫。
- 修:加 `src-tauri/capabilities/default.json`,main 視窗授予 `core:default`(啟用 IPC bridge)。本專案自訂 command 非 plugin 命名空間,`core:default` 已足夠,不需逐一列權限。
- **暴露的縫**:per-task review 的「編譯 + 靜態檢查」層級**無法**發現 Tauri v2 的 ACL/capabilities 缺漏——這類問題只有真的觸發一次 command 呼叫(真機互動,或未來若補 GUI 整合測試)才驗得出來。

### 已通過
- ✅ **開窗**:視窗正確顯示(1200×800、標題 Clacks、按鈕/狀態列/兩 pane/人工輸入框皆正確渲染)。
- ✅ **啟動管線**:按下後狀態列變化(booting/running)、兩個 pane 出現 CLI 開機輸出——emitter → xterm.js 端到端確認通過。

### 待補
- 人工介入通道、乾淨訊息端到端、惡意訊息、設計輸入 A(連發/單發)、設計輸入 C(respawn 後 settle)、設計輸入 B(乾淨 teardown)、token 不進 webview 真機面——待續。

### 真機確認(2026-07-21):人工輸入通道 30s long-poll 延遲——final review 預測命中

- final review(2026-07-20)當時預測:人工輸入通道與 Telegram 30s long-poll 共用同一 pipeline thread,worst-case 延遲達 ~30s。
- **真機實測驗證**:cyrano CLI 顯示閒置(prompt 空著,無處理中指示),使用者在 pane 人工輸入框打字送出後,**約 28 秒**才在 pane 看到反應。與 `getUpdates` 的 `timeout=30` 長輪詢完全吻合——確認人工輸入被卡在 `input_rx` 佇列裡,直到當輪 `poll_once` 的 long-poll 返回、迴圈跳回最上面才被排空、真正寫進 PTY。
- **意義**:trust/login 對話框若自身在 30 秒內逾時(常見情況),使用者透過 pane 的介入可能來不及送達——直接牴觸本 phase 設計動機之一(pane+輸入框取代 headless 的 pre-seed 死鎖)。
- **裁決(待使用者決定是否本次一併修復)**:候選修法(1)縮短 `getUpdates` timeout(`telegram.rs:81` 的 `"timeout"` query 參數,目前 30);(2)人工輸入排空移到獨立、更短週期的通道(需要把 `taster`/`cyrano` session 改成跨執行緒共享,牽動目前「orchestrator 獨佔 &mut session 整個生命週期以保 taster_dirty」的設計,屬較大改動,不建議在 Task 12 收尾階段做)。
- **裁決(2026-07-21,使用者確認)**:本次不修,列為已知可用性限制,續走 Task 12 其餘清單項目;是否縮短 timeout 留待之後決定。
