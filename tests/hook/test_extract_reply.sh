#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."

fail=0

check() {
  local fixture="$1" expected="$2" label="$3"
  local workdir outfile actual
  workdir=$(mktemp -d)

  printf '{"transcript_path": "%s"}' "$(pwd)/$fixture" \
    | CLAUDE_PROJECT_DIR="$workdir" templates/echo/.claude/hooks/extract-reply.sh

  outfile=$(find "$workdir/outbox" -name '*-reply.json' 2>/dev/null | head -1)
  if [ -z "$outfile" ]; then
    echo "FAIL($label): no outbox artifact"
    fail=1
  else
    actual=$(jq -r '.text' "$outfile")
    if [ "$actual" = "$expected" ]; then
      echo "PASS($label)"
    else
      echo "FAIL($label): expected '$expected', got '$actual'"
      fail=1
    fi
  fi

  if find "$workdir/outbox" -name '*.partial' 2>/dev/null | grep -q .; then
    echo "FAIL($label): leftover .partial file"
    fail=1
  fi

  rm -rf "$workdir"
}

# Phase 6 findings(2026-07-21):Stop 事件觸發 hook 讀檔的當下,CLI 可能還沒把
# 當輪 entry 寫進磁碟上的 transcript 檔。以下兩個函式各自重現一種真機觀察到
# 的分岔:(1)續接 session 讀到「上一輪」內容重複誤送(2)全新 session 讀到
# 空字串(taster 逐則 /clear 情境)。兩者都用背景 delayed-write 模擬磁碟落地
# 的時間差。
check_retry_stale_duplicate() {
  local label="stale-duplicate-uuid-guard"
  local workdir transcript outfile actual last_uuid_written bgpid
  workdir=$(mktemp -d)
  mkdir -p "$workdir/outbox"
  transcript="$workdir/transcript.jsonl"

  # 上一輪(A)已經送出過,對應 uuid 預先寫進 .last-uuid,模擬 hook 上次成功
  # 寫出 A、狀態已落地
  printf 'A-UUID\n' > "$workdir/outbox/.last-uuid"
  printf '{"type":"assistant","uuid":"A-UUID","message":{"content":[{"type":"text","text":"reply A"}]}}\n' > "$transcript"

  # 背景延遲 0.3 秒後才把當輪(B)entry 落地——重試預算(下個任務會實作)是
  # 20 次 * 0.1 秒 = 2 秒,0.3 秒落在預算內
  (
    sleep 0.3
    printf '{"type":"assistant","uuid":"B-UUID","message":{"content":[{"type":"text","text":"reply B"}]}}\n' >> "$transcript"
  ) &
  bgpid=$!

  printf '{"transcript_path": "%s"}' "$transcript" \
    | CLAUDE_PROJECT_DIR="$workdir" templates/echo/.claude/hooks/extract-reply.sh

  wait "$bgpid"

  outfile=$(find "$workdir/outbox" -name '*-reply.json' 2>/dev/null | head -1)
  actual=$(jq -r '.text' "$outfile")
  last_uuid_written=$(cat "$workdir/outbox/.last-uuid" 2>/dev/null || echo "")

  if [ "$actual" = "reply B" ] && [ "$last_uuid_written" = "B-UUID" ]; then
    echo "PASS($label)"
  else
    echo "FAIL($label): got text='$actual' last-uuid='$last_uuid_written'"
    fail=1
  fi

  rm -rf "$workdir"
}

check_retry_empty_until_ready() {
  local label="empty-until-first-write-lands"
  local workdir transcript outfile actual bgpid
  workdir=$(mktemp -d)
  mkdir -p "$workdir/outbox"
  transcript="$workdir/transcript.jsonl"

  # 全新 session:transcript 一開始沒有任何 assistant entry(對應 taster 逐則
  # /clear 的情境),背景延遲後才寫入第一筆
  : > "$transcript"
  (
    sleep 0.3
    printf '{"type":"assistant","uuid":"C-UUID","message":{"content":[{"type":"text","text":"reply C"}]}}\n' >> "$transcript"
  ) &
  bgpid=$!

  printf '{"transcript_path": "%s"}' "$transcript" \
    | CLAUDE_PROJECT_DIR="$workdir" templates/echo/.claude/hooks/extract-reply.sh

  wait "$bgpid"

  outfile=$(find "$workdir/outbox" -name '*-reply.json' 2>/dev/null | head -1)
  actual=$(jq -r '.text' "$outfile")

  if [ "$actual" = "reply C" ]; then
    echo "PASS($label)"
  else
    echo "FAIL($label): expected 'reply C', got '$actual'"
    fail=1
  fi

  rm -rf "$workdir"
}

check_role_scripts_identical() {
  local label="role-hooks-byte-identical"
  if diff -q templates/echo/.claude/hooks/extract-reply.sh templates/taster/.claude/hooks/extract-reply.sh >/dev/null \
     && diff -q templates/echo/.claude/hooks/extract-reply.sh templates/cyrano/.claude/hooks/extract-reply.sh >/dev/null; then
    echo "PASS($label)"
  else
    echo "FAIL($label): role hook scripts diverged"
    fail=1
  fi
}

check tests/hook/fixture-transcript.jsonl "ECHO: second reply" "basic"
check tests/hook/fixture-thinking-race.jsonl "ECHO: real reply" "thinking-race"
check_retry_stale_duplicate
check_retry_empty_until_ready
check_role_scripts_identical

exit "$fail"
