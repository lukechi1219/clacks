//! GUI composition root(architecture.md 依賴規則 4:唯一知道具體型別 + tauri 的地方)。
//! 本檔是 Phase 5 新增的組裝點,類比 bin/pipeline.rs 但由 Tauri 事件迴圈驅動。
//! Task 7:最小開窗;Task 9:接 command + 管線 thread + emitter。

pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("啟動 Tauri 應用失敗");
}
