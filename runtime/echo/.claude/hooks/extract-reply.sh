#!/usr/bin/env bash
set -euo pipefail

input=$(cat)
transcript=$(printf '%s' "$input" | jq -r '.transcript_path')

outbox="${CLAUDE_PROJECT_DIR:?}/outbox"
mkdir -p "$outbox"

reply=$(jq -rs '
  [.[] | select(.type == "assistant")]
  | last
  | [.message.content[]? | select(.type == "text") | .text]
  | join("\n")
' "$transcript")

printf '{"text": %s}\n' "$(printf '%s' "$reply" | jq -Rs .)" \
  > "$outbox/$(date +%s)-$$-reply.json"
