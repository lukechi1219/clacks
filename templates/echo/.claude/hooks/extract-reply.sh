#!/usr/bin/env bash
set -euo pipefail

input=$(cat)
transcript=$(printf '%s' "$input" | jq -r '.transcript_path')

outbox="${CLAUDE_PROJECT_DIR:?}/outbox"
mkdir -p "$outbox"

# thinking-race 實證(skeleton findings, Task 9):最後一個 assistant entry
# 可能只有 thinking 區塊(text 尚未 flush)——取「最後一個含 text 區塊的
# entry」而非單純 last;全都沒有 text 時輸出空字串,由 Rust 端 schema 驗證判 failed
reply=$(jq -rs '
  [.[] | select(.type == "assistant")
       | [.message.content[]? | select(.type == "text") | .text]
       | select(length > 0)]
  | last // []
  | join("\n")
' "$transcript")

# rename-into-place(skeleton findings, final review):.partial 寫完再 mv
# 成 .json(同目錄 rename 原子),watcher 收到事件時內容保證完整;
# .partial 不以 .json 結尾,不會被 watcher 的副檔名過濾撿走
final="$outbox/$(date +%s)-$$-reply.json"
tmp="$final.partial"
printf '{"text": %s}\n' "$(printf '%s' "$reply" | jq -Rs .)" > "$tmp"
mv "$tmp" "$final"
