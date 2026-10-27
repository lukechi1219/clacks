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
    //
    // 用 symlink_metadata 而非 exists():exists() 會跟隨 symlink,懸空 symlink 與
    // 讀取失敗(權限不足)都回 false 而放行——目標日後建立即生效。只有明確的
    // NotFound 才放行,其餘一律拒絕(fail-closed)
    let dot_claude = resolved.join(".claude");
    match std::fs::symlink_metadata(&dot_claude) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "無法確認 cyrano_readable_dir 底下是否存在 .claude({}):{error};無法排除注入面,拒絕啟動",
                dot_claude.display()
            ));
        }
        Ok(_) => {
            return Err(format!(
                "cyrano_readable_dir 底下存在 .claude({}):--add-dir 會從中載入 skills/agents/plugin 設定,\
                 等同讓唯讀目錄的內容對 cyrano 下指令。請移除或改用不含 .claude 的目錄",
                dot_claude.display()
            ));
        }
    }

    Ok(Some(resolved))
}

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

    // 懸空 symlink:exists() 跟隨連結得 false 而放行,目標日後建立即生效
    #[cfg(unix)]
    #[test]
    fn readable_dir_containing_dangling_dot_claude_symlink_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let readable = dir.path().join("cyrano-readable");
        std::fs::create_dir(&readable).unwrap();
        std::os::unix::fs::symlink(dir.path().join("nonexistent"), readable.join(".claude")).unwrap();
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, r#"{"cyrano_readable_dir": "cyrano-readable"}"#).unwrap();

        let error = load_cyrano_readable_dir(&path).unwrap_err();
        assert!(error.contains(".claude"), "訊息未指出 .claude:{error}");
    }

    // 讀不到(權限不足)無法排除注入面,同樣拒絕而非放行
    #[cfg(unix)]
    #[test]
    fn unreadable_readable_dir_is_rejected_fail_closed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let readable = dir.path().join("cyrano-readable");
        std::fs::create_dir(&readable).unwrap();
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, r#"{"cyrano_readable_dir": "cyrano-readable"}"#).unwrap();
        std::fs::set_permissions(&readable, std::fs::Permissions::from_mode(0o000)).unwrap();

        let result = load_cyrano_readable_dir(&path);
        // 先還原權限,tempdir 才清得掉
        std::fs::set_permissions(&readable, std::fs::Permissions::from_mode(0o755)).unwrap();

        let error = result.unwrap_err();
        assert!(error.contains("無法確認"), "訊息未說明無法確認:{error}");
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
