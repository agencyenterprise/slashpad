#!/bin/bash
# Stop hook: run clippy before Claude finishes its turn
# If clippy finds warnings/errors, block the stop so Claude fixes them

INPUT=$(cat)
STOP_HOOK_ACTIVE=$(echo "$INPUT" | jq -r '.stop_hook_active')

# Prevent infinite loops — if we already forced a continue, let it stop
if [ "$STOP_HOOK_ACTIVE" = "true" ]; then
  exit 0
fi

OUTPUT=$(cargo clippy --manifest-path "$CLAUDE_PROJECT_DIR/src-tauri/Cargo.toml" --all-targets --all-features -- -D warnings 2>&1)
EXIT_CODE=$?

if [ $EXIT_CODE -ne 0 ]; then
  TRIMMED=$(echo "$OUTPUT" | head -50)
  jq -n --arg reason "Clippy found issues. Please fix them before finishing:
$TRIMMED" '{
    "decision": "block",
    "reason": $reason
  }'
else
  exit 0
fi
