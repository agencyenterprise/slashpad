#!/bin/bash
# PostToolUse hook: fast compilation check after Rust file edits
# Fires only for .rs files (filtered by the "if" field in settings.json)

OUTPUT=$(cargo check --manifest-path "$CLAUDE_PROJECT_DIR/src-tauri/Cargo.toml" --message-format=short 2>&1)
EXIT_CODE=$?

if [ $EXIT_CODE -ne 0 ]; then
  TRIMMED=$(echo "$OUTPUT" | head -40)
  jq -n --arg reason "$TRIMMED" '{
    "decision": "block",
    "reason": $reason
  }'
else
  exit 0
fi
