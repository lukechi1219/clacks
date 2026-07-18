use notify::event::ModifyKind;
use notify::{recommended_watcher, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

/// 監看 outbox 目錄,新 .json 檔路徑送進 channel。
/// hook 契約是 rename-into-place(.partial 寫完後 mv 成 .json),
/// 故事件到達即內容完整;macOS FSEvents 對 mv 產生 rename 類事件
/// 而非 Create,兩類都要接。`.partial` 檔靠副檔名過濾排除。
/// 同一產物可能觸發多個事件(FSEvents flag 合併)——重複路徑由
/// CliSession 的 drain-before-inject 語意吸收,此處不去重。
pub fn watch_outbox(
    dir: &Path,
    tx: Sender<PathBuf>,
) -> Result<RecommendedWatcher, notify::Error> {
    std::fs::create_dir_all(dir).map_err(notify::Error::io)?;
    let mut watcher = recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        if let Ok(event) = res {
            let relevant = matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(_))
            );
            if relevant {
                for path in event.paths {
                    if path.extension().is_some_and(|e| e == "json") && path.exists() {
                        let _ = tx.send(path);
                    }
                }
            }
        }
    })?;
    watcher.watch(dir, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn emits_path_when_json_file_created() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _watcher = watch_outbox(dir.path(), tx).unwrap();

        std::thread::sleep(Duration::from_millis(800)); // macOS FSEvents stream 啟動有延遲,太短會 flaky
        let file = dir.path().join("123-reply.json");
        std::fs::write(&file, r#"{"text":"hi"}"#).unwrap();

        let got = rx.recv_timeout(Duration::from_secs(3)).expect("watcher event");
        assert_eq!(got.file_name(), file.file_name());
    }

    // 釘死 rename-into-place 的事件語意(hook 契約,Task 6):
    // 此測試失敗 = FSEvents 事件種類假設錯誤,必須回報實際 EventKind,不得改斷言
    #[test]
    fn emits_path_when_json_renamed_into_place() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _watcher = watch_outbox(dir.path(), tx).unwrap();

        std::thread::sleep(Duration::from_millis(800)); // macOS FSEvents stream 啟動有延遲,太短會 flaky
        let tmp = dir.path().join("123-reply.json.partial");
        std::fs::write(&tmp, r#"{"text":"hi"}"#).unwrap();
        let done = dir.path().join("123-reply.json");
        std::fs::rename(&tmp, &done).unwrap();

        let got = rx.recv_timeout(Duration::from_secs(3)).expect("watcher event");
        assert_eq!(got.file_name(), done.file_name());
    }

    #[test]
    fn ignores_non_json_files() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _watcher = watch_outbox(dir.path(), tx).unwrap();

        std::thread::sleep(Duration::from_millis(800)); // macOS FSEvents stream 啟動有延遲,太短會 flaky
        std::fs::write(dir.path().join("junk.tmp"), "x").unwrap();

        assert!(rx.recv_timeout(Duration::from_secs(1)).is_err());
    }
}
