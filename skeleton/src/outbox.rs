use notify::{recommended_watcher, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

pub fn watch_outbox(dir: &Path, tx: Sender<PathBuf>) -> RecommendedWatcher {
    std::fs::create_dir_all(dir).expect("create outbox dir");
    let mut watcher = recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        if let Ok(event) = res {
            if matches!(event.kind, EventKind::Create(_)) {
                for path in event.paths {
                    if path.extension().is_some_and(|e| e == "json") {
                        let _ = tx.send(path);
                    }
                }
            }
        }
    })
    .expect("create watcher");
    watcher.watch(dir, RecursiveMode::NonRecursive).expect("watch outbox dir");
    watcher
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
        let _watcher = watch_outbox(dir.path(), tx);

        std::thread::sleep(Duration::from_millis(800)); // macOS FSEvents stream 啟動有延遲,太短會 flaky
        let file = dir.path().join("123-reply.json");
        std::fs::write(&file, r#"{"text":"hi"}"#).unwrap();

        let got = rx.recv_timeout(Duration::from_secs(3)).expect("watcher event");
        assert_eq!(got.file_name(), file.file_name());
    }

    #[test]
    fn ignores_non_json_files() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _watcher = watch_outbox(dir.path(), tx);

        std::thread::sleep(Duration::from_millis(800)); // macOS FSEvents stream 啟動有延遲,太短會 flaky
        std::fs::write(dir.path().join("junk.tmp"), "x").unwrap();

        assert!(rx.recv_timeout(Duration::from_secs(1)).is_err());
    }
}
