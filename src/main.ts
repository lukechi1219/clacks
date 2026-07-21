import "@xterm/xterm/css/xterm.css";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

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

listen<string>("pty://taster", (e) => taster.write(e.payload));
listen<string>("pty://cyrano", (e) => cyrano.write(e.payload));

const outcomes = document.getElementById("outcomes")!;
listen<string>("outcome", (e) => {
  const li = document.createElement("li");
  li.textContent = `${new Date().toLocaleTimeString()}  ${e.payload}`; // 經過時間屬呈現層（非 Clock port）
  outcomes.prepend(li);
});
const stateEl = document.getElementById("pipeline-state")!;
listen<string>("state", (e) => { stateEl.textContent = e.payload; });
listen<string>("fatal", (e) => { stateEl.textContent = `fatal: ${e.payload}`; });
listen<string>("poll-error", (e) => { stateEl.textContent = `poll-error: ${e.payload}`; });

const startBtn = document.getElementById("start") as HTMLButtonElement;
const stopBtn = document.getElementById("stop") as HTMLButtonElement;
startBtn.onclick = async () => {
  await invoke("start_pipeline");
  startBtn.disabled = true; stopBtn.disabled = false;
};
stopBtn.onclick = async () => {
  await invoke("stop_pipeline");
  startBtn.disabled = false; stopBtn.disabled = true;
};

// 人工介入(trust/login 對話框):Enter 送該 pane,附 \r 觸發
document.querySelectorAll<HTMLInputElement>("input.manual").forEach((box) => {
  box.addEventListener("keydown", async (ev) => {
    if (ev.key === "Enter") {
      await invoke("send_input", { role: box.dataset.role, data: box.value + "\r" });
      box.value = "";
    }
  });
});

export { taster, cyrano };
