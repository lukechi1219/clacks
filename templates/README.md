# templates/ — runtime 工作目錄的版控正本

這裡是 CLI 工作目錄(角色 CLAUDE.md、settings、hooks)的 **source of truth**,不是拿來直接跑的。

**live runtime 一律在 repo 目錄樹之外**(`../clacks-runtime/`):CLI 若在 repo 內啟動,祖先 CLAUDE.md 遍歷會把整個專案脈絡(含安全設計)灌進被隔離的 CLI——實證見
[docs/superpowers/notes/2026-07-17-skeleton-findings.md](../docs/superpowers/notes/2026-07-17-skeleton-findings.md)「工作目錄嵌套污染」。

部署(改了範本之後同步):

```bash
cp -R templates/echo/ ../clacks-runtime/echo/
```

注意:`../clacks-runtime/*/outbox/`、transcript 等執行期產物只存在 live 側,不回流 repo。
