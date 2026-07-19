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

## OS sandbox profile(未佈線,opt-in)

`templates/sandbox/clacks.sb` 是 `sandbox-exec -f` 用的 macOS sandbox profile 正本(檔案系統隔離為主:唯讀為預設,寫入白名單;網路無法全禁,靠檔案系統層隔離)。**目前未佈線進 `ClaudePtySession::spawn`**——經 portable-pty spawn 的包裝屬 Phase 5(隨 GUI 一併做),此階段只是把 skeleton 實證過的正本(含 `/dev/null` 缺口修正)落 `templates/` 供之後接線,也可手動套用驗證:

```bash
sandbox-exec -D WORKDIR="$(pwd)/../clacks-runtime/taster" \
  -D HOME_CLAUDE="$HOME/.claude" \
  -f templates/sandbox/clacks.sb claude
```

device 白名單(`/dev/null`、`/dev/tty`、`/dev/dtracehelper`)是依 skeleton 教訓的保守補齊,實際所需 device 以真機為準。
