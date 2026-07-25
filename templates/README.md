# templates/ — runtime 工作目錄的版控正本

這裡是 CLI 工作目錄(角色 CLAUDE.md、settings、hooks)的 **source of truth**,不是拿來直接跑的。

**live runtime 一律在 repo 目錄樹之外**(`../clacks-runtime/`):CLI 若在 repo 內啟動,祖先 CLAUDE.md 遍歷會把整個專案脈絡(含安全設計)灌進被隔離的 CLI——實證見
[docs/superpowers/notes/2026-07-17-skeleton-findings.md](../docs/superpowers/notes/2026-07-17-skeleton-findings.md)「工作目錄嵌套污染」。

部署(改了範本之後同步;runtime 一律在 repo 目錄樹之外):

```bash
cp -R templates/echo/   ../clacks-runtime/echo/    # Phase 2 smoke
cp -R templates/taster/ ../clacks-runtime/taster/  # Phase 4 消毒者
cp -R templates/cyrano/ ../clacks-runtime/cyrano/  # Phase 4 回應者
```

注意:`../clacks-runtime/*/outbox/`、transcript 等執行期產物只存在 live 側,不回流 repo。

## OS sandbox profile(taster 已接線,cyrano 未接線)

`templates/sandbox/clacks.sb` 是 `sandbox-exec` 用的 macOS sandbox profile 正本(檔案系統隔離為主:唯讀為預設,寫入白名單;網路無法全禁,靠檔案系統層隔離)。**taster 自 Phase 7 起已佈線**——`ClaudePtySession::spawn`(`src-tauri/src/adapters/pty.rs` 的 `taster_argv()`)在 `config_dir` 為 `Some` 時,把原本的裸 `claude` 包成 `sandbox-exec -p <inline profile> -D WORKDIR=… -D CONFIG_DIR=… claude`。**cyrano(`spawn_continue`)刻意仍未套用**——目前只有 Claude Code 權限層(deny 一切工具、僅 allow 白名單目錄的 `Read`)做隔離,OS sandbox 留待未來獨立評估。

⚠️ `templates/sandbox/` 是 `templates/` 底下**唯一不由 `cp -R` 部署**的子目錄:`clacks.sb` 的內容經 `include_str!` 在編譯期內嵌進 Rust 二進位,執行期用 `-p` 傳 inline 字串,不會在執行期解析檔案路徑。改 `clacks.sb` 的內容之後,要讓改動生效必須 `cargo build`(或 `cargo run`/`cargo test`),**不是** `cp -R` 到 `../clacks-runtime/`——那裡本來就沒有這個檔案。

手動套用驗證(路徑需自行代換,`CONFIG_DIR` 對應呼叫端傳入 `spawn()` 的 `config_dir`,即 `../clacks-runtime/cli-config`):

```bash
sandbox-exec -D WORKDIR="$(pwd)/../clacks-runtime/taster" \
  -D CONFIG_DIR="$(pwd)/../clacks-runtime/cli-config" \
  -f templates/sandbox/clacks.sb claude
```

device 白名單(`/dev/null`、`/dev/tty`、`/dev/dtracehelper`)是依 skeleton 教訓的保守補齊,實際所需 device 以真機為準。
