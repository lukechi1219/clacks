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

check tests/hook/fixture-transcript.jsonl "ECHO: second reply" "basic"
check tests/hook/fixture-thinking-race.jsonl "ECHO: real reply" "thinking-race"

exit "$fail"
