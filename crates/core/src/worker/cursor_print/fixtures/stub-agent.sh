#!/bin/sh
scenario="${1:-${STUB_AGENT_SCENARIO:-success}}"
dir="$(cd "$(dirname "$0")" && pwd)"
file="$dir/$scenario.jsonl"
[ -f "$file" ] || { echo "unknown scenario: $scenario" >&2; exit 2; }
cat >/dev/null
cat "$file"
case "$scenario" in
interrupted)
  trap 'exit 130' INT TERM
  while :; do sleep 1; done
  ;;
crash) exit 1 ;;
esac
