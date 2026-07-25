#!/usr/bin/env bash
set -euo pipefail

input=$(cat)
transcript=$(printf '%s' "$input" | jq -r '.transcript_path')

outbox="${CLAUDE_PROJECT_DIR:?}/outbox"
mkdir -p "$outbox"

# Stop hook 讀檔競態(findings 2026-07-21):Stop 事件觸發 hook 讀檔的當下,CLI
# 可能還沒把當輪 entry 寫進磁碟上的 transcript 檔。用「候選 entry 的 uuid 是否
# 與上次成功送出的 uuid 相同」偵測是否抓到舊內容;文字為空(全新 session 首次
# 寫入還沒落地)也視為未就緒,一併重試。狀態記在 outbox 目錄下的隱藏檔
# .last-uuid(不可加 .json 副檔名——會被 adapters/outbox.rs 的 watcher 誤判為
# 回覆檔案)。
#
# 校準值(若真機測試發現不足需調大,數字本身是估計值不是精確量測):
RETRY_INTERVAL=0.1
RETRY_MAX_ATTEMPTS=20

last_uuid_file="$outbox/.last-uuid"
last_uuid=""
if [ -f "$last_uuid_file" ]; then
  last_uuid=$(cat "$last_uuid_file")
fi

candidate_uuid=""
candidate_text=""
attempt=0
while [ "$attempt" -lt "$RETRY_MAX_ATTEMPTS" ]; do
  if [ -f "$transcript" ]; then
    # thinking-race 實證(skeleton findings, Task 9):最後一個 assistant entry
    # 可能只有 thinking 區塊(text 尚未 flush)——取「最後一個含 text 區塊的
    # entry」而非單純 last;全都沒有 text 時 uuid/text 皆為空字串。既有 fixture
    # 沒有 uuid 欄位,以 `// ""` 容錯避免 null 造成後續比對出錯。
    candidate=$(jq -rs '
      [.[] | select(.type == "assistant")
           | select(([.message.content[]? | select(.type == "text")] | length) > 0)
           | {uuid: (.uuid // ""), text: ([.message.content[]? | select(.type == "text") | .text] | join("\n"))}]
      | last // {uuid: "", text: ""}
    ' "$transcript" 2>/dev/null || printf '{"uuid":"","text":""}')
  else
    candidate='{"uuid":"","text":""}'
  fi
  candidate_uuid=$(printf '%s' "$candidate" | jq -r '.uuid')
  candidate_text=$(printf '%s' "$candidate" | jq -r '.text')

  if [ -n "$candidate_text" ] && [ "$candidate_uuid" != "$last_uuid" ]; then
    break
  fi
  attempt=$((attempt + 1))
  sleep "$RETRY_INTERVAL"
done

if [ "$attempt" -ge "$RETRY_MAX_ATTEMPTS" ]; then
  echo "[extract-reply] retry budget exhausted (${RETRY_MAX_ATTEMPTS} attempts), emitting possibly-stale entry uuid=$candidate_uuid" >&2
fi

reply="$candidate_text"

# rename-into-place(skeleton findings, final review):.partial 寫完再 mv
# 成 .json(同目錄 rename 原子),watcher 收到事件時內容保證完整;
# .partial 不以 .json 結尾,不會被 watcher 的副檔名過濾撿走
final="$outbox/$(date +%s)-$$-reply.json"
tmp="$final.partial"
printf '{"text": %s}\n' "$(printf '%s' "$reply" | jq -Rs .)" > "$tmp"
mv "$tmp" "$final"

if [ -n "$reply" ]; then
  printf '%s' "$candidate_uuid" > "$last_uuid_file"
fi
