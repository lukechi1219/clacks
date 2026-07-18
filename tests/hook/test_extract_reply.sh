#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."

workdir=$(mktemp -d)
export CLAUDE_PROJECT_DIR="$workdir"

printf '{"transcript_path": "%s"}' "$(pwd)/tests/hook/fixture-transcript.jsonl" \
  | runtime/echo/.claude/hooks/extract-reply.sh

outfile=$(ls "$workdir"/outbox/*-reply.json)
actual=$(jq -r '.text' "$outfile")
expected="ECHO: second reply"

if [ "$actual" = "$expected" ]; then
  echo "PASS"
else
  echo "FAIL: expected '$expected', got '$actual'"
  exit 1
fi
