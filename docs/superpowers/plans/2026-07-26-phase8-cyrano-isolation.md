# Phase 8 — cyrano 隔離 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓「cyrano 的讀取範圍被限縮」從三份文件的宣稱變成一處真正執行的機制——設定檔指定唯讀目錄、經 `--add-dir` 交給 CLI、同時移除放行全機器讀取的裸 `Read` 規則,並把 cyrano 接上 Phase 7 既有的寫入 sandbox。

**Architecture:** 設定值的剖析是 `core/` 的純函式(字串進、驗證過的結構出);檔案讀取、路徑解析與 `.claude` 注入面防護放在 `adapters/config_file.rs`(唯一碰 IO 的地方,兩個 composition root 共用同一份,避免安全檢查在兩處漂移);argv 組裝放 `adapters/pty.rs`,以單一函式同時產生初次與重啟 argv,結構上排除「重啟後掉出隔離」的靜默倒退。

**Tech Stack:** Rust 2024 edition · serde_json 1.0.150 · tempfile 3.27.0(dev)· macOS `sandbox-exec` · Claude Code CLI

前置文件:[spec](../specs/2026-07-26-phase8-cyrano-isolation-design.md)、[architecture.md](../../../architecture.md)、[Phase 7 spec](../specs/2026-07-25-phase7-sandbox-exec-taster-design.md)

## Global Constraints

1. **token 絕不落檔**:bot token 只經 `CLACKS_BOT_TOKEN` 環境變數(Keychain 注入)進入 Rust。本次新增的設定檔**只放非機密設定**,`ClacksConfig` 不得出現任何 credential 欄位。
2. **不得存在靜默退回無隔離的路徑**:沿用 Phase 7 Global Constraint。任何錯誤路徑只能退到**更嚴格**的狀態或直接拒絕啟動,不得退到比預期更寬鬆的狀態。
3. **`config_dir` 為 `None` 時不套用 sandbox、不加 `--add-dir`**:這是 `bin/smoke.rs` 的無隔離基準線,是刻意設計。`smoke.rs` 本次不得改動。
4. **平台假設 macOS**:不加 `cfg(target_os)` 判斷、不寫非 macOS fallback。
5. **依賴規則(architecture.md)**:`core/` 不碰 IO、不依賴 tokio/tauri,只有 std + serde;adapter 之間不互相依賴;composition root 是唯一知道具體型別的地方。
6. **設定檔位置為 repo 根**,不得放在 `../clacks-runtime/` 之下——後者在 taster 的 sandbox 寫入白名單內,放那裡等於讓被隔離的一端能改自己的隔離設定。
7. **`sandbox-exec` 執行失敗走既有 `CliError` fail-closed 路徑**,不退回無 sandbox 模式。
8. **`templates/sandbox/clacks.sb` 內容零改動**:讀取本來就被 `(allow default)` 放行,cyrano 與 taster 共用同一份 profile,只有 `-D WORKDIR=` 的值不同。
9. 所有指令從 repo 根執行;Rust 指令一律 `cargo test --manifest-path src-tauri/Cargo.toml ...`(不要 `cd`)。

---

## File Structure

| 檔案 | 動作 | 職責 |
|---|---|---|
| `src-tauri/src/core/config.rs` | 建立 | `ClacksConfig` 型別 + `parse_config()` 純函式。零 IO |
| `src-tauri/src/core/mod.rs` | 修改 | 宣告 `pub mod config;` |
| `src-tauri/src/adapters/config_file.rs` | 建立 | 讀檔、路徑解析、存在性檢查、`.claude` 注入面防護。兩個 composition root 的共用入口 |
| `src-tauri/src/adapters/mod.rs` | 修改 | 宣告 `pub mod config_file;` |
| `src-tauri/src/adapters/pty.rs` | 修改 | 抽出 `base_argv()`;新增 `cyrano_argv()`;`spawn_continue()` 改用它產生兩份 argv |
| `src-tauri/src/gui.rs` | 修改 | 載入設定並傳給 `spawn_continue` |
| `src-tauri/src/bin/pipeline.rs` | 修改 | 同上(dev-only 也不得繞過隔離) |
| `clacks.config.example.json` | 建立 | 版控範本 |
| `.gitignore` | 修改 | 忽略 `/clacks.config.json` |
| `README.md` | 修改 | 新增「Configuration」小節 |
| `templates/cyrano/.claude/settings.json` | 修改 | 移除裸 `"Read"` |
| `templates/cyrano/CLAUDE.md` | 修改 | 能力邊界改為據實描述 |
| `docs/superpowers/specs/2026-07-17-clacks-design.md` | 修改 | 安全模型表 cyrano 列更新 |
| `architecture.md` | 修改 | 目錄樹補上新檔案 |

**不得改動**:`templates/sandbox/clacks.sb`、`src-tauri/src/bin/smoke.rs`、`src-tauri/src/app.rs`、`src-tauri/src/core/session.rs`。

---

## Task 1: `core/config.rs` — 設定剖析純函式

**Files:**
- Create: `src-tauri/src/core/config.rs`
- Modify: `src-tauri/src/core/mod.rs`
- Test: 同檔 `#[cfg(test)] mod tests`(本專案慣例,不另開 tests/ 目錄)

**Interfaces:**
- Consumes: 無(本 plan 第一個任務)
- Produces:
  - `pub struct ClacksConfig { pub cyrano_readable_dir: Option<String> }`(derive `Debug, PartialEq`)
  - `pub struct ConfigError(pub String)`(derive `Debug, PartialEq`)
  - `pub fn parse_config(json: &str) -> Result<ClacksConfig, ConfigError>`

**契約(Task 2 依賴這些語意):**

| 輸入 | 輸出 |
|---|---|
| `{"cyrano_readable_dir": "../x"}` | `Ok(Some("../x"))` |
| `{}` 或缺鍵 | `Ok(None)` |
| 鍵值為 `null` | `Ok(None)` |
| 鍵值為空字串或全空白 | `Err` |
| 鍵值非字串(數字、陣列、物件) | `Err` |
| JSON 語法錯誤 | `Err` |
| 最外層不是物件(如 `[]`、`"x"`) | `Err` |
| 含未知額外欄位 | 忽略,不報錯(留設定檔演進空間) |

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/core/config.rs` 建立檔案,先只寫測試(實作在 Step 3):

```rust
//! clacks.config.json 的剖析與驗證(純函式,零 IO——architecture.md 依賴規則 1)。
//! 讀檔、路徑解析與 .claude 注入面防護在 adapters/config_file.rs。
//!
//! ⚠ 本結構只放非機密設定。bot token 只經 CLACKS_BOT_TOKEN 環境變數進入 Rust,
//! 絕不落檔(設計文件既有規則:不用 .env)——不得在此新增任何 credential 欄位

use serde_json::Value;

#[derive(Debug, PartialEq)]
pub struct ClacksConfig {
    /// cyrano 的唯讀專案目錄(未設 = 無額外讀取範圍)。原始字串,尚未解析成路徑
    pub cyrano_readable_dir: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct ConfigError(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cyrano_readable_dir() {
        assert_eq!(
            parse_config(r#"{"cyrano_readable_dir": "../clacks-runtime/cyrano-readable"}"#),
            Ok(ClacksConfig {
                cyrano_readable_dir: Some("../clacks-runtime/cyrano-readable".to_string())
            })
        );
    }

    #[test]
    fn missing_key_means_no_readable_dir() {
        assert_eq!(parse_config("{}"), Ok(ClacksConfig { cyrano_readable_dir: None }));
    }

    #[test]
    fn null_value_means_no_readable_dir() {
        assert_eq!(
            parse_config(r#"{"cyrano_readable_dir": null}"#),
            Ok(ClacksConfig { cyrano_readable_dir: None })
        );
    }

    // 空字串無從區分「不想設」與「設錯」;不想設就整個移除該鍵(fail-closed)
    #[test]
    fn empty_string_is_rejected() {
        assert!(parse_config(r#"{"cyrano_readable_dir": ""}"#).is_err());
    }

    #[test]
    fn whitespace_only_string_is_rejected() {
        assert!(parse_config(r#"{"cyrano_readable_dir": "   "}"#).is_err());
    }

    #[test]
    fn wrong_type_is_rejected() {
        assert!(parse_config(r#"{"cyrano_readable_dir": 42}"#).is_err());
        assert!(parse_config(r#"{"cyrano_readable_dir": ["a"]}"#).is_err());
        assert!(parse_config(r#"{"cyrano_readable_dir": {"a": 1}}"#).is_err());
    }

    #[test]
    fn malformed_json_is_rejected() {
        assert!(parse_config("{not json").is_err());
        assert!(parse_config("").is_err());
    }

    #[test]
    fn non_object_root_is_rejected() {
        assert!(parse_config("[]").is_err());
        assert!(parse_config(r#""just a string""#).is_err());
    }

    // 額外欄位刻意忽略:未來新增設定項時,舊版二進位不會因為看到新鍵就拒絕啟動
    #[test]
    fn unknown_fields_are_ignored() {
        assert_eq!(
            parse_config(r#"{"cyrano_readable_dir": "../x", "future_option": true}"#),
            Ok(ClacksConfig { cyrano_readable_dir: Some("../x".to_string()) })
        );
    }

    // 錯誤訊息必須指出是哪個鍵出問題,否則使用者得靠猜的
    #[test]
    fn error_message_names_the_key() {
        let ConfigError(message) = parse_config(r#"{"cyrano_readable_dir": 42}"#).unwrap_err();
        assert!(message.contains("cyrano_readable_dir"), "訊息未指出鍵名:{message}");
    }
}
```

在 `src-tauri/src/core/mod.rs` 的模組宣告清單加入一行(維持字母序,放在 `contract` 之前):

```rust
pub mod config;
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cargo test --manifest-path src-tauri/Cargo.toml core::config`
Expected: 編譯失敗,`cannot find function 'parse_config' in this scope`

- [ ] **Step 3: 寫最小實作**

在 `src-tauri/src/core/config.rs` 的 `pub struct ConfigError` 之後、`#[cfg(test)]` 之前插入:

```rust
/// 刻意用 serde_json::Value 逐鍵取值,而非 derive(Deserialize):
/// (1) 要讓未知欄位被忽略而非報錯;(2) 要給出指名道姓的錯誤訊息
/// (serde 的預設訊息對非工程師使用者無用)
pub fn parse_config(json: &str) -> Result<ClacksConfig, ConfigError> {
    let value: Value = serde_json::from_str(json)
        .map_err(|error| ConfigError(format!("JSON 剖析失敗:{error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| ConfigError("設定檔最外層必須是 JSON 物件".to_string()))?;

    let cyrano_readable_dir = match object.get("cyrano_readable_dir") {
        None | Some(Value::Null) => None,
        Some(Value::String(raw)) => {
            if raw.trim().is_empty() {
                return Err(ConfigError(
                    "cyrano_readable_dir 不得為空字串;不需要唯讀目錄請整個移除此鍵".to_string(),
                ));
            }
            Some(raw.clone())
        }
        Some(other) => {
            return Err(ConfigError(format!(
                "cyrano_readable_dir 必須是字串,實際是 {other}"
            )));
        }
    };

    Ok(ClacksConfig { cyrano_readable_dir })
}
```

- [ ] **Step 4: 跑測試確認通過**

Run: `cargo test --manifest-path src-tauri/Cargo.toml core::config`
Expected: PASS,10 個測試全綠

- [ ] **Step 5: 確認沒有 warning**

Run: `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets 2>&1 | tail -20`
Expected: 沒有指向 `core/config.rs` 的 warning

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/core/config.rs src-tauri/src/core/mod.rs
```

```bash
git commit -m "Phase 8 Task 1: 設定剖析純函式 core::config::parse_config"
```

---

## Task 2: `adapters/config_file.rs` — 讀檔、路徑解析、`.claude` 防護

**Files:**
- Create: `src-tauri/src/adapters/config_file.rs`
- Modify: `src-tauri/src/adapters/mod.rs`
- Test: 同檔 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: Task 1 的 `clacks::core::config::{parse_config, ClacksConfig, ConfigError}`
- Produces:
  - `pub const CONFIG_FILE: &str = "clacks.config.json";`
  - `pub fn load_cyrano_readable_dir(config_path: &Path) -> Result<Option<PathBuf>, String>`

**為什麼放 adapters/**:它碰檔案系統,不能放 `core/`(依賴規則 1)。它不實作任何 port,這是刻意的——把它放進 adapters 而不是複製到兩個 composition root,是為了讓 `.claude` 防護這條安全檢查只有一份實作。兩份互相漂移的安全檢查正是本專案反覆踩過的坑。

**行為契約(Task 4 依賴):**

| 情況 | 回傳 |
|---|---|
| 設定檔不存在 | `Ok(None)`——退到更嚴格的狀態(無額外讀取範圍),不是更寬鬆 |
| 讀檔失敗(權限等) | `Err` |
| `parse_config` 回 `Err` | `Err`,訊息含設定檔路徑 |
| 設定檔存在但未設該鍵 | `Ok(None)` |
| 指定目錄不存在或無法解析 | `Err`,訊息含 `mkdir -p` 建議 |
| 指定路徑存在但不是目錄 | `Err` |
| 目錄下有 `.claude` | `Err`,訊息說明注入風險 |
| 一切正常 | `Ok(Some(canonical 絕對路徑))` |

**相對路徑的基準點是設定檔所在目錄**,不是行程 cwd。原因:`sandbox-exec` 與 `--add-dir` 都需要 canonical 絕對路徑(含 symlink 的路徑會比不中而讓存取被拒),而以設定檔為基準比以 cwd 為基準更可預測。

- [ ] **Step 1: 寫失敗的測試**

建立 `src-tauri/src/adapters/config_file.rs`,先只寫檔頭 + 測試:

```rust
//! clacks.config.json 的載入(IO 端;剖析與驗證的純函式在 core::config)。
//!
//! 本檔是兩個 composition root(gui.rs / bin/pipeline.rs)的共用入口。
//! 刻意不把這段複製到兩個 root:.claude 注入面防護是安全檢查,兩份實作必然漂移。
//!
//! 本檔不實作任何 port——它是「碰 IO 的具體實作」,依 architecture.md 的分層
//! 歸屬 adapters,但不佔 ports 預算

use crate::core::config::{parse_config, ConfigError};
use std::path::{Path, PathBuf};

/// 設定檔檔名(repo 根)。刻意不放 ../clacks-runtime/:那裡在 taster 的 sandbox
/// 寫入白名單內,設定檔放那裡等於讓被隔離的一端能改自己的隔離設定
pub const CONFIG_FILE: &str = "clacks.config.json";

#[cfg(test)]
mod tests {
    use super::*;

    /// 測試輔助:在 tempdir 裡寫一份設定檔,回傳 (tempdir, 設定檔路徑)
    fn write_config(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, contents).unwrap();
        (dir, path)
    }

    #[test]
    fn missing_config_file_yields_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE);
        assert_eq!(load_cyrano_readable_dir(&path), Ok(None));
    }

    #[test]
    fn config_without_key_yields_none() {
        let (_dir, path) = write_config("{}");
        assert_eq!(load_cyrano_readable_dir(&path), Ok(None));
    }

    #[test]
    fn malformed_config_is_rejected() {
        let (_dir, path) = write_config("{not json");
        let error = load_cyrano_readable_dir(&path).unwrap_err();
        assert!(error.contains(CONFIG_FILE), "訊息未指出設定檔:{error}");
    }

    #[test]
    fn resolves_relative_path_against_config_file_directory() {
        let dir = tempfile::tempdir().unwrap();
        let readable = dir.path().join("cyrano-readable");
        std::fs::create_dir(&readable).unwrap();
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, r#"{"cyrano_readable_dir": "cyrano-readable"}"#).unwrap();

        let resolved = load_cyrano_readable_dir(&path).unwrap().unwrap();
        assert_eq!(resolved, std::fs::canonicalize(&readable).unwrap());
        assert!(resolved.is_absolute(), "必須是絕對路徑:{}", resolved.display());
    }

    #[test]
    fn missing_readable_dir_is_rejected_with_mkdir_hint() {
        let (_dir, path) = write_config(r#"{"cyrano_readable_dir": "nope"}"#);
        let error = load_cyrano_readable_dir(&path).unwrap_err();
        assert!(error.contains("mkdir -p"), "訊息缺少建立指令:{error}");
    }

    #[test]
    fn file_instead_of_directory_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("not-a-dir"), "x").unwrap();
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, r#"{"cyrano_readable_dir": "not-a-dir"}"#).unwrap();

        let error = load_cyrano_readable_dir(&path).unwrap_err();
        assert!(error.contains("不是目錄"), "訊息未說明原因:{error}");
    }

    // --add-dir 會從加入的目錄載入 .claude/skills/、.claude/agents/ 與
    // settings.json 的 enabledPlugins/extraKnownMarketplaces——實質的指令注入面。
    // 這條檢查把「靜默載入未知指令」變成「大聲的啟動失敗」
    #[test]
    fn readable_dir_containing_dot_claude_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let readable = dir.path().join("cyrano-readable");
        std::fs::create_dir(&readable).unwrap();
        std::fs::create_dir(readable.join(".claude")).unwrap();
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, r#"{"cyrano_readable_dir": "cyrano-readable"}"#).unwrap();

        let error = load_cyrano_readable_dir(&path).unwrap_err();
        assert!(error.contains(".claude"), "訊息未指出 .claude:{error}");
    }

    // .claude 是檔案而非目錄時同樣拒絕:不去猜使用者的意圖
    #[test]
    fn readable_dir_containing_dot_claude_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let readable = dir.path().join("cyrano-readable");
        std::fs::create_dir(&readable).unwrap();
        std::fs::write(readable.join(".claude"), "x").unwrap();
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, r#"{"cyrano_readable_dir": "cyrano-readable"}"#).unwrap();

        assert!(load_cyrano_readable_dir(&path).is_err());
    }

    #[test]
    fn absolute_path_is_accepted_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let readable = dir.path().join("cyrano-readable");
        std::fs::create_dir(&readable).unwrap();
        let (_cfg_dir, path) = write_config(&format!(
            r#"{{"cyrano_readable_dir": "{}"}}"#,
            readable.display()
        ));

        assert_eq!(
            load_cyrano_readable_dir(&path).unwrap().unwrap(),
            std::fs::canonicalize(&readable).unwrap()
        );
    }
}
```

在 `src-tauri/src/adapters/mod.rs` 加入(維持字母序,放在 `outbox` 之前):

```rust
pub mod config_file;
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cargo test --manifest-path src-tauri/Cargo.toml adapters::config_file`
Expected: 編譯失敗,`cannot find function 'load_cyrano_readable_dir' in this scope`

- [ ] **Step 3: 寫最小實作**

在 `config_file.rs` 的 `pub const CONFIG_FILE` 之後、`#[cfg(test)]` 之前插入:

```rust
/// 載入 cyrano 的唯讀目錄設定。
///
/// 錯誤處理的判準不是「有錯就爆」,而是**沒有任何路徑會退到比預期更寬鬆的狀態**
/// (Global Constraint 2):設定檔不存在 → 退到「無額外讀取範圍」(更嚴格);
/// 設定寫了但有問題 → 意圖無從得知,拒絕啟動。
///
/// 回傳的路徑必為 canonical 絕對路徑:sandbox 與 --add-dir 以 real path 比對,
/// 含 symlink 的路徑會比不中而讓存取被拒(Phase 7 taster_argv 的同一個教訓)
pub fn load_cyrano_readable_dir(config_path: &Path) -> Result<Option<PathBuf>, String> {
    if !config_path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(config_path)
        .map_err(|error| format!("讀取 {} 失敗:{error}", config_path.display()))?;

    let config = parse_config(&contents)
        .map_err(|ConfigError(message)| format!("{} 設定錯誤:{message}", config_path.display()))?;

    let Some(raw) = config.cyrano_readable_dir else {
        return Ok(None);
    };

    // 相對路徑以設定檔所在目錄為基準(比行程 cwd 可預測)
    let base = config_path.parent().filter(|p| !p.as_os_str().is_empty());
    let candidate = match base {
        Some(base) => base.join(&raw),
        None => PathBuf::from(&raw),
    };

    let resolved = std::fs::canonicalize(&candidate).map_err(|error| {
        format!(
            "cyrano_readable_dir 指定的目錄無法解析({}):{error};先建立它:mkdir -p {}",
            candidate.display(),
            candidate.display()
        )
    })?;

    if !resolved.is_dir() {
        return Err(format!(
            "cyrano_readable_dir 指定的路徑不是目錄:{}",
            resolved.display()
        ));
    }

    // --add-dir 會從該目錄載入 .claude/skills/、.claude/agents/ 與 settings.json
    // 的 enabledPlugins/extraKnownMarketplaces。把一個真實專案資料夾放進唯讀目錄
    // 是這個功能的預期用法,而真實專案帶 .claude/ 是常態——故結構性拒絕,
    // 不倚賴文件叮嚀。(CLAUDE.md 的載入另需 CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD,
    // 已被 pty.rs 的 ENV_ALLOWLIST 擋在子行程之外)
    let dot_claude = resolved.join(".claude");
    if dot_claude.exists() {
        return Err(format!(
            "cyrano_readable_dir 底下存在 .claude({}):--add-dir 會從中載入 skills/agents/plugin 設定,\
             等同讓唯讀目錄的內容對 cyrano 下指令。請移除或改用不含 .claude 的目錄",
            dot_claude.display()
        ));
    }

    Ok(Some(resolved))
}
```

- [ ] **Step 4: 跑測試確認通過**

Run: `cargo test --manifest-path src-tauri/Cargo.toml adapters::config_file`
Expected: PASS,9 個測試全綠

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/adapters/config_file.rs src-tauri/src/adapters/mod.rs
```

```bash
git commit -m "Phase 8 Task 2: 設定檔載入 + .claude 注入面防護"
```

---

## Task 3: `pty.rs` — `cyrano_argv()` 與 `spawn_continue` 的單一來源 argv

**Files:**
- Modify: `src-tauri/src/adapters/pty.rs`(`taster_argv` 附近 ~57-71 行;`spawn` ~97-107 行;`spawn_continue` ~111-123 行;測試區 ~356 行起)
- Modify: `src-tauri/src/gui.rs:189`(僅補 `None` 引數,讓程式碼編得過)
- Modify: `src-tauri/src/bin/pipeline.rs:97`(同上)

**Interfaces:**
- Consumes: 既有的 `SANDBOX_PROFILE`(`include_str!` 內嵌的 `templates/sandbox/clacks.sb`)
- Produces:
  - `fn base_argv(workdir: &Path, config_dir: Option<&Path>) -> Vec<String>`(private)
  - `fn cyrano_argv(workdir: &Path, config_dir: Option<&Path>, readable_dir: Option<&Path>, resume: bool) -> Vec<String>`(private)
  - `pub fn spawn_continue(workdir: &Path, config_dir: Option<&Path>, readable_dir: Option<&Path>, make_output: Box<dyn FnMut() -> Box<dyn Write + Send> + Send>) -> Result<Self, CliError>`

**本任務刻意讓兩個 composition root 傳 `None`**:先讓新的引數形狀站穩、測試證明 argv 組裝正確,Task 4 才把真實設定接上去。這樣兩個任務都能各自獨立測試與審查。

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/adapters/pty.rs` 的 `mod tests` 內,緊接在既有的 `taster_argv_is_plain_claude_without_config_dir` 測試之後,加入:

```rust
    // cyrano 生產路徑:sandbox 包裝 + --add-dir,重啟版本只多一個 --continue
    #[test]
    fn cyrano_argv_wraps_in_sandbox_with_add_dir() {
        let argv = cyrano_argv(
            Path::new("/tmp/cyrano"),
            Some(Path::new("/tmp/cfg")),
            Some(Path::new("/tmp/readable")),
            false,
        );
        assert_eq!(
            argv,
            vec![
                "sandbox-exec".to_string(),
                "-p".to_string(),
                SANDBOX_PROFILE.to_string(),
                "-D".to_string(),
                "WORKDIR=/tmp/cyrano".to_string(),
                "-D".to_string(),
                "CONFIG_DIR=/tmp/cfg".to_string(),
                "claude".to_string(),
                "--add-dir".to_string(),
                "/tmp/readable".to_string(),
            ]
        );
    }

    // 唯讀目錄未設定時不得出現 --add-dir(空引數會讓 CLI 解析出錯)
    #[test]
    fn cyrano_argv_omits_add_dir_when_readable_dir_absent() {
        let argv = cyrano_argv(Path::new("/tmp/cyrano"), Some(Path::new("/tmp/cfg")), None, false);
        assert!(!argv.iter().any(|a| a == "--add-dir"), "不該有 --add-dir:{argv:?}");
        assert_eq!(argv.last().unwrap(), "claude");
    }

    // smoke.rs 的無隔離基準線(config_dir = None):不包 sandbox、不加 --add-dir
    #[test]
    fn cyrano_argv_is_plain_claude_without_config_dir() {
        assert_eq!(
            cyrano_argv(Path::new("/tmp/cyrano"), None, Some(Path::new("/tmp/readable")), false),
            vec!["claude".to_string()]
        );
        assert_eq!(
            cyrano_argv(Path::new("/tmp/cyrano"), None, None, true),
            vec!["claude".to_string(), "--continue".to_string()]
        );
    }

    // 核心不變式(Phase 7 教訓的延伸):初次與重啟 argv 出自同一函式,
    // 唯一差異是 --continue。差異超過這一項 = 重啟時隔離設定會漂移
    #[test]
    fn cyrano_respawn_argv_differs_only_by_continue_flag() {
        let workdir = Path::new("/tmp/cyrano");
        let cfg = Some(Path::new("/tmp/cfg"));
        let readable = Some(Path::new("/tmp/readable"));

        let initial = cyrano_argv(workdir, cfg, readable, false);
        let respawn = cyrano_argv(workdir, cfg, readable, true);

        let mut expected = initial.clone();
        expected.push("--continue".to_string());
        assert_eq!(respawn, expected);
    }

    // 漂移防護:兩份 argv 都必須帶著 sandbox 與 --add-dir。
    // 未來有人把 spawn_continue 改回寫死字面量時,這條會擋下來
    #[test]
    fn both_cyrano_argvs_carry_sandbox_and_add_dir() {
        let workdir = Path::new("/tmp/cyrano");
        let cfg = Some(Path::new("/tmp/cfg"));
        let readable = Some(Path::new("/tmp/readable"));

        for resume in [false, true] {
            let argv = cyrano_argv(workdir, cfg, readable, resume);
            assert_eq!(argv[0], "sandbox-exec", "resume={resume} 未包 sandbox:{argv:?}");
            assert!(argv.iter().any(|a| a == "--add-dir"), "resume={resume} 缺 --add-dir");
        }
    }

    // cyrano 的權限模板不得放行不帶路徑的 Read——裸工具名在 Claude Code 的規則語意
    // 裡代表「任何路徑」,等同讓 cyrano 讀取整台機器。這是 Phase 8 要修掉的那個缺口,
    // 本測試存在的意義是擋住未來「順手」把它加回去
    #[test]
    fn cyrano_settings_do_not_allow_bare_read() {
        const CYRANO_SETTINGS: &str =
            include_str!("../../../templates/cyrano/.claude/settings.json");
        let parsed: serde_json::Value = serde_json::from_str(CYRANO_SETTINGS).unwrap();
        let allow = &parsed["permissions"]["allow"];
        if let Some(rules) = allow.as_array() {
            assert!(
                !rules.iter().any(|rule| rule == "Read"),
                "cyrano settings.json 的 allow 含裸 Read:{allow}"
            );
        }
    }
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cargo test --manifest-path src-tauri/Cargo.toml adapters::pty::tests::cyrano`
Expected: 編譯失敗,`cannot find function 'cyrano_argv' in this scope`

- [ ] **Step 3: 抽出 `base_argv` 並新增 `cyrano_argv`**

在 `pty.rs` 中,把既有的 `taster_argv`(約 57-71 行)整段替換為以下三個函式(保留原有的 doc comment 內容,只是重新分配到對應函式上):

```rust
/// sandbox 包裝的共用底座。config_dir 為 Some(生產路徑:gui.rs / bin/pipeline.rs)
/// 時包進 sandbox-exec 做檔案系統隔離;None(bin/smoke.rs 的無隔離基準線)時
/// 維持純 claude。
///
/// workdir / config_dir 必須是 canonical 絕對路徑(呼叫端已 canonicalize):
/// sandbox 以 real path 比對 subpath,含 symlink 的路徑會比不中而讓寫入被拒。
/// 本函式刻意不做 canonicalize——它是純函式,不碰 IO
fn base_argv(workdir: &Path, config_dir: Option<&Path>) -> Vec<String> {
    let Some(cfg) = config_dir else {
        return vec!["claude".to_string()];
    };
    vec![
        "sandbox-exec".to_string(),
        "-p".to_string(),
        SANDBOX_PROFILE.to_string(),
        "-D".to_string(),
        format!("WORKDIR={}", workdir.display()),
        "-D".to_string(),
        format!("CONFIG_DIR={}", cfg.display()),
        "claude".to_string(),
    ]
}

/// taster 的 argv。
///
/// spawn() 以本函式的輸出**同時**作為初次 argv 與 respawn_argv——單一來源,
/// 結構上保證重啟後仍在 sandbox 內(重啟掉出 sandbox 是靜默的安全倒退)
fn taster_argv(workdir: &Path, config_dir: Option<&Path>) -> Vec<String> {
    base_argv(workdir, config_dir)
}

/// cyrano 的 argv:sandbox 底座 + 唯讀目錄 + 續談旗標。
///
/// readable_dir 只在 config_dir 為 Some 時附加(smoke 基準線不加任何隔離參數)。
/// resume 是初次(false)與重啟(true)的唯一差異——spawn_continue 以本函式產生
/// 兩份 argv,結構上不可能發生「首次有 sandbox/--add-dir,重啟後沒有」的靜默倒退。
///
/// ⚠ --add-dir 不只授予檔案存取:CLI 會從該目錄載入 .claude/skills/、
/// .claude/agents/ 與 settings.json 的 enabledPlugins/extraKnownMarketplaces。
/// 擋這條注入面的是 adapters::config_file 的 .claude 啟動檢查,不是本函式
fn cyrano_argv(
    workdir: &Path,
    config_dir: Option<&Path>,
    readable_dir: Option<&Path>,
    resume: bool,
) -> Vec<String> {
    let mut argv = base_argv(workdir, config_dir);
    if config_dir.is_some() {
        if let Some(dir) = readable_dir {
            argv.push("--add-dir".to_string());
            argv.push(dir.display().to_string());
        }
    }
    if resume {
        argv.push("--continue".to_string());
    }
    argv
}
```

- [ ] **Step 4: 改寫 `spawn_continue`**

把 `pty.rs` 既有的 `spawn_continue`(約 109-123 行)整段替換為:

```rust
    /// cyrano 專用:首次啟動全新對話,重啟以 `claude --continue` 續談。
    /// readable_dir = 設定檔指定的唯讀專案目錄(None = 無額外讀取範圍)。
    ///
    /// 初次與 respawn 兩份 argv 都出自 cyrano_argv——比照 spawn() 的單一來源
    /// 不變式,重啟不可能掉出 sandbox 或掉失 --add-dir
    pub fn spawn_continue(
        workdir: &Path,
        config_dir: Option<&Path>,
        readable_dir: Option<&Path>,
        make_output: Box<dyn FnMut() -> Box<dyn Write + Send> + Send>,
    ) -> Result<Self, CliError> {
        let argv = cyrano_argv(workdir, config_dir, readable_dir, false);
        let respawn_argv = cyrano_argv(workdir, config_dir, readable_dir, true);
        let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        Self::spawn_program(&argv_refs, respawn_argv, workdir, config_dir, make_output)
    }
```

- [ ] **Step 5: 補上兩個呼叫點的引數(暫傳 `None`)**

`src-tauri/src/gui.rs`(原第 189 行)改為:

```rust
    let mut cyrano = match ClaudePtySession::spawn_continue(&cyrano_dir, Some(&cli_config), None, cyrano_factory) {
```

`src-tauri/src/bin/pipeline.rs`(原第 97 行)改為:

```rust
    let mut cyrano = ClaudePtySession::spawn_continue(&cyrano_dir, Some(&cli_config), None, Box::new(|| Box::new(std::io::stdout())))
```

- [ ] **Step 6: 跑全部測試確認通過**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS。特別確認既有的 `taster_argv_wraps_in_sandbox_when_config_dir_present`、`taster_argv_is_plain_claude_without_config_dir`、`sandbox_denies_write_outside_whitelist` 仍全綠——`base_argv` 抽取是結構性改動,不得改變 taster 的行為

- [ ] **Step 7: 移除裸 `Read` 讓漂移防護測試變綠**

`cyrano_settings_do_not_allow_bare_read` 此時仍是紅的。修改 `templates/cyrano/.claude/settings.json`,把 `permissions` 物件中的整個 `allow` 陣列刪掉(deny 清單維持不變),結果應為:

```json
{
  "permissions": {
    "deny": [
      "Bash",
      "Edit",
      "Write",
      "Glob",
      "Grep",
      "WebFetch",
      "WebSearch",
      "Task",
      "NotebookEdit"
    ]
  },
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"$CLAUDE_PROJECT_DIR/.claude/hooks/extract-reply.sh\""
          }
        ]
      }
    ]
  }
}
```

- [ ] **Step 8: 跑測試確認漂移防護通過**

Run: `cargo test --manifest-path src-tauri/Cargo.toml adapters::pty`
Expected: PASS,含 `cyrano_settings_do_not_allow_bare_read`

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/adapters/pty.rs src-tauri/src/gui.rs src-tauri/src/bin/pipeline.rs templates/cyrano/.claude/settings.json
```

```bash
git commit -m "Phase 8 Task 3: cyrano_argv 單一來源 argv + 移除裸 Read"
```

---

## Task 4: composition root 接線 + 設定檔範本

**Files:**
- Create: `clacks.config.example.json`
- Modify: `.gitignore`
- Modify: `src-tauri/src/gui.rs`(在 `cli_config` 檢查之後、`spawn_continue` 之前)
- Modify: `src-tauri/src/bin/pipeline.rs`(同位置)
- Modify: `README.md`

**Interfaces:**
- Consumes: Task 2 的 `clacks::adapters::config_file::{load_cyrano_readable_dir, CONFIG_FILE}`;Task 3 的 `spawn_continue(workdir, config_dir, readable_dir, make_output)`
- Produces: 無(終端接線)

- [ ] **Step 1: 建立設定檔範本**

建立 `clacks.config.example.json`:

```json
{
  "cyrano_readable_dir": "../clacks-runtime/cyrano-readable"
}
```

- [ ] **Step 2: 把實際設定檔加入 .gitignore**

在 `.gitignore` 末端加入兩行:

```
# 實際生效的設定(從 clacks.config.example.json 複製);只放非機密設定
/clacks.config.json
```

- [ ] **Step 3: 接線 `gui.rs`**

在 `src-tauri/src/gui.rs` 的 CLI config pre-seed 檢查區塊之後(即 `let gateway = TelegramHttp::from_env();` 這行之前)插入:

```rust
    // Phase 8:cyrano 唯讀目錄。設定檔缺席 → None(退到更嚴格的無額外讀取範圍);
    // 設定寫了但有問題 → fail-closed 拒絕啟動(Global Constraints 2)
    let cyrano_readable = match load_cyrano_readable_dir(std::path::Path::new(CONFIG_FILE)) {
        Ok(dir) => dir,
        Err(error) => {
            let _ = app.emit("fatal", error);
            running.store(false, Ordering::SeqCst);
            return;
        }
    };
```

在同函式頂端的 `use` 區塊(即 `use crate::adapters::store::SqliteStore;` 附近)加入:

```rust
    use crate::adapters::config_file::{load_cyrano_readable_dir, CONFIG_FILE};
```

把 Task 3 暫傳 `None` 的那一行改為:

```rust
    let mut cyrano = match ClaudePtySession::spawn_continue(&cyrano_dir, Some(&cli_config), cyrano_readable.as_deref(), cyrano_factory) {
```

- [ ] **Step 4: 接線 `bin/pipeline.rs`**

在 `src-tauri/src/bin/pipeline.rs` 的 `use` 區塊加入:

```rust
use clacks::adapters::config_file::{load_cyrano_readable_dir, CONFIG_FILE};
```

在 `cli_config` 的 pre-seed 檢查之後、spawn 之前插入:

```rust
    // Phase 8:cyrano 唯讀目錄。dev-only bin 同樣不繞過隔離設定
    let cyrano_readable = load_cyrano_readable_dir(std::path::Path::new(CONFIG_FILE))
        .expect("load clacks.config.json");
```

把 Task 3 暫傳 `None` 的那一行改為:

```rust
    let mut cyrano = ClaudePtySession::spawn_continue(&cyrano_dir, Some(&cli_config), cyrano_readable.as_deref(), Box::new(|| Box::new(std::io::stdout())))
```

- [ ] **Step 5: 確認編譯與全部測試通過**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS,無 warning

Run: `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets 2>&1 | tail -20`
Expected: 沒有新增的 warning

- [ ] **Step 6: 手動驗證 fail-closed 行為(不需真的啟動 CLI)**

先建立測試用的唯讀目錄與設定檔:

```bash
mkdir -p ../clacks-runtime/cyrano-readable && cp clacks.config.example.json clacks.config.json
```

寫一個壞掉的設定檔並確認 headless bin 拒絕啟動:

```bash
printf '{"cyrano_readable_dir": 42}' > /tmp/bad-clacks.config.json
```

Run: `cargo run --manifest-path src-tauri/Cargo.toml --bin pipeline 2>&1 | head -5`(先把 `clacks.config.json` 換成上面那份壞的:`cp /tmp/bad-clacks.config.json clacks.config.json`)
Expected: panic 訊息含 `cyrano_readable_dir 必須是字串`,且**沒有**啟動任何 CLI

還原正常設定:

```bash
cp clacks.config.example.json clacks.config.json
```

- [ ] **Step 7: 更新 README**

在 `README.md` 的 `## Status` 小節之後、`## Planned stack` 之前插入:

```markdown
## Configuration

Copy the example config and adjust it:

```bash
cp clacks.config.example.json clacks.config.json
```

- `cyrano_readable_dir` — a directory that cyrano (the responder) may read from, passed to the CLI as `--add-dir`. Relative paths resolve against the config file's directory. Omit the key entirely if cyrano should have no extra read scope.

**This directory must not contain a `.claude` entry.** `--add-dir` loads Skills, Subagents, and plugin settings from the added directory, so a stray `.claude/` there would let its contents issue instructions to cyrano. The app refuses to start if it finds one.

`clacks.config.json` is gitignored and holds **non-secret settings only** — the bot token reaches the backend exclusively through the `CLACKS_BOT_TOKEN` environment variable (injected from the macOS Keychain) and is never written to a file.
```

- [ ] **Step 8: Commit**

```bash
git add clacks.config.example.json .gitignore README.md src-tauri/src/gui.rs src-tauri/src/bin/pipeline.rs
```

```bash
git commit -m "Phase 8 Task 4: composition root 接上唯讀目錄設定"
```

---

## Task 5: 文件與現實對齊

**Files:**
- Modify: `templates/cyrano/CLAUDE.md`
- Modify: `docs/superpowers/specs/2026-07-17-clacks-design.md`(安全模型三層表的 cyrano 列)
- Modify: `architecture.md`(目錄樹)

本任務沒有程式碼異動,但它是本 phase 的核心目的之一:讓「三份文件宣稱、零處執行」的狀態不再以另一種形式復發。

- [ ] **Step 1: 修正 cyrano 的角色指示**

`templates/cyrano/CLAUDE.md` 的「你的能力邊界」第一點,把:

```markdown
- 你**只能讀取**(`Read`),且限定於本工作目錄的白名單。不寫檔、不跑指令、不上網。
```

改為:

```markdown
- 你**只能讀取**(`Read`):本工作目錄,以及後端以 `--add-dir` 指定的唯讀專案目錄。讀取這兩者以外的路徑會觸發人工核准。不寫檔、不跑指令、不上網。
```

- [ ] **Step 2: 同步部署到 runtime**

範本改動不會自動生效(Phase 6 踩過的部署漂移):

```bash
cp templates/cyrano/CLAUDE.md ../clacks-runtime/cyrano/CLAUDE.md
```

```bash
cp templates/cyrano/.claude/settings.json ../clacks-runtime/cyrano/.claude/settings.json
```

- [ ] **Step 3: 更新設計文件的安全模型表**

在 `docs/superpowers/specs/2026-07-17-clacks-design.md` 的安全模型三層表(約 129-132 行)中,把 **cyrano 欄**(每列最後一個 `|` 分隔的儲存格)的內容替換掉。

「**Claude Code 權限**」列(約 131 行)的 cyrano 儲存格,替換為:

```
專屬 settings:deny `Bash`/`Edit`/`Write`/`Glob`/`Grep`/`WebFetch`/`WebSearch`/`Task`/`NotebookEdit`。**已移除原本裸的 `Read` allow 規則**(Phase 8)——裸工具名在 Claude Code 的規則語意裡代表任何路徑,等同放行整台機器。現在 cyrano 的讀取範圍是工作目錄 + 啟動時 `--add-dir` 指定的唯讀專案目錄(路徑由 repo 根的 `clacks.config.json` 提供),其餘路徑會觸發人工核准。見 [phase8 設計](2026-07-26-phase8-cyrano-isolation-design.md)
```

「**OS sandbox**」列(約 132 行)的 cyrano 儲存格,替換為:

```
**已接線**(Phase 8):與 taster 共用同一份 [clacks.sb](../../../templates/sandbox/clacks.sb),`-D WORKDIR=` 指向 cyrano 工作目錄。⚠ 這層是**寫入**隔離——profile 的 `(allow default)` 表示**讀取未在 OS 層封閉**,限縮讀取的是上列的 Claude Code 權限層。真正的 OS 層讀取封閉(`deny file-read*` + 讀取白名單)是 Phase 8 明列的刻意不做,爆炸半徑需獨立一輪配合真機探測,目前仍是 cyrano 隔離的最大剩餘缺口
```

同檔 §對話模式與存取控制 的狀態註記(約 98 行)有一句「而 cyrano 的隔離缺口(見下方安全模型表)尚未補齊——建議先補防線再擴功能」。把它改為:

```
而 cyrano 的讀取範圍已在 Phase 8 收斂(Claude Code 權限層;OS 層讀取封閉仍未做,見下方安全模型表)——動工前請確認擴大攻擊面的代價是否值得
```

⚠ 該檔混用全形括號（）與全形冒號:,做字串比對編輯時必須逐字複製既有內容,不可換成半形。

- [ ] **Step 4: 更新 architecture.md 目錄樹**

在 `architecture.md` 的目錄樹中:

在 `core/` 區塊的 `contract.rs` 之前加入一行:

```
│   │   ├── config.rs         # clacks.config.json 剖析(純函式;讀檔在 adapters)
```

在 `adapters/` 區塊的 `outbox.rs` 之前加入一行:

```
│   │   ├── config_file.rs    # 設定檔載入 + 唯讀目錄 .claude 注入面防護(不實作 port)
```

在 `templates/` 那一行之前加入一行:

```
├── clacks.config.json   # 非機密設定(gitignore;範本為 clacks.config.example.json)
```

- [ ] **Step 5: 確認沒有殘留的不實宣稱**

Run: `rtk proxy rg -n "白名單目錄的 .Read|限定於本工作目錄的白名單" docs/ templates/ architecture.md README.md`
Expected: 無輸出。有輸出代表還有一處文件宣稱了程式碼沒有執行的限制——必須一併修掉

- [ ] **Step 6: Commit**

```bash
git add templates/cyrano/CLAUDE.md docs/superpowers/specs/2026-07-17-clacks-design.md architecture.md
```

```bash
git commit -m "Phase 8 Task 5: 文件與 cyrano 實際隔離範圍對齊"
```

---

## Task 6(human): 真機驗證

**Files:** 無程式碼異動。結果寫入 `docs/superpowers/notes/2026-07-26-phase8-findings.md`

本任務必須由人操作(需要真實 Anthropic 登入、真實 Telegram 訊息),比照 Phase 1/4/5/6/7 的模式。前置:`clacks.config.json` 已從範本複製、`../clacks-runtime/cyrano-readable/` 已建立。

- [ ] **Step 1: 準備唯讀目錄的測試內容**

```bash
mkdir -p ../clacks-runtime/cyrano-readable && printf 'The passphrase is thunderbird-seventeen.\n' > ../clacks-runtime/cyrano-readable/note.txt
```

- [ ] **Step 2: 啟動 GUI 並確認 cyrano 正常開機**

```bash
CLACKS_BOT_TOKEN="$(security find-generic-password -s clacks-bot -w)" npm run tauri dev
```

Expected:cyrano pane 正常顯示 TUI、無 `Operation not permitted` 造成的啟動失敗。

⚠ CLI 自身安裝目錄(`~/.local/share/claude/…`、`~/.local/bin/claude`)或 `$HOME` 中 `CONFIG_DIR` 以外的路徑出現 `Operation not permitted` 是**預期行為**,**不得**加進 sandbox 白名單來「修掉」。

- [ ] **Step 3: 驗證唯讀目錄可讀**

從 Telegram 傳:`note.txt 裡寫了什麼?`
Expected:回覆含 `thunderbird-seventeen`。

- [ ] **Step 4: 驗證範圍外路徑需核准**

從 Telegram 傳:`幫我看看 ~/.ssh/config 的內容`
Expected:cyrano pane 出現讀取核准提示,**不**直接回傳檔案內容。在 pane 拒絕該核准。

- [ ] **Step 5: 驗證 `--continue` 續談仍正常**

連續傳兩則訊息,第二則指涉第一則(例:先傳 `我叫 Luke`,再傳 `我剛剛說我叫什麼?`)。
Expected:第二則回覆記得 `Luke`(cyrano 有記憶,`--continue` 有生效)。

- [ ] **Step 6: 驗證 `.claude` 檢查會擋下啟動**

先停掉 app,然後:

```bash
mkdir -p ../clacks-runtime/cyrano-readable/.claude
```

重新啟動 app。
Expected:GUI 顯示 fatal 錯誤,訊息提到 `.claude` 與 `--add-dir` 的載入風險,**沒有**啟動任何 CLI。

還原:

```bash
rmdir ../clacks-runtime/cyrano-readable/.claude
```

- [ ] **Step 7: 驗證設定檔缺席時仍可運作**

```bash
mv clacks.config.json /tmp/clacks.config.json.bak
```

重新啟動 app,傳一則普通訊息。
Expected:管線正常運作(cyrano 只是沒有額外讀取範圍),**不會**拒絕啟動。

還原:

```bash
mv /tmp/clacks.config.json.bak clacks.config.json
```

- [ ] **Step 8: 寫下發現並 commit**

把每一步的實際結果(含任何與預期不符之處)寫入 `docs/superpowers/notes/2026-07-26-phase8-findings.md`,特別記錄:sandbox 下是否出現非預期的 `Operation not permitted`、核准提示的實際呈現方式、`--add-dir` 目錄在 CLI 內的可見範圍。

```bash
git add docs/superpowers/notes/2026-07-26-phase8-findings.md
```

```bash
git commit -m "Phase 8 Task 6: 真機驗證發現"
```

---

## 刻意不做(沿用 spec)

- 方案 B(OS 層 `deny file-read*` 讀取封閉)——本 phase 完成後仍是 cyrano 隔離的最大剩餘缺口
- 方案 C(針對性 deny 規則)
- 多個唯讀目錄、設定檔熱重載
- 切分 taster/cyrano 的 `CONFIG_DIR`
- `--settings` 或執行期產生 settings.json
- `/compact` 接線(`core/session.rs::should_compact()` 仍零消費端)
