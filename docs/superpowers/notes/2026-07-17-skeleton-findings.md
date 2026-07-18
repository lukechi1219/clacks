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
