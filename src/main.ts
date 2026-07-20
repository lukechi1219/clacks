import "@xterm/xterm/css/xterm.css";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";

function mountTerm(id: string): Terminal {
  const term = new Terminal({ convertEol: true, fontSize: 12 });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(document.getElementById(id)!);
  fit.fit();
  return term;
}

const taster = mountTerm("taster-term");
const cyrano = mountTerm("cyrano-term");
taster.write("taster pane 就緒(等待管線啟動)\r\n");
cyrano.write("cyrano pane 就緒\r\n");

// Task 10 接線(本任務僅骨架,尚未實作):
// - 訂閱 PTY 輸出事件("pty://taster" / "pty://cyrano")→ term.write
// - 訂閱管線結果事件("outcome")→ 狀態列
// - start/stop 按鈕 → 呼叫後端指令啟動/停止管線
// - 人工輸入框 → 呼叫後端指令送出輸入
export { taster, cyrano };
