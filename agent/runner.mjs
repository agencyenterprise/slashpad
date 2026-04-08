import { query } from "@anthropic-ai/claude-agent-sdk";

function emit(event) {
  process.stdout.write(JSON.stringify(event) + "\n");
}

try {
  const payload = JSON.parse(Buffer.from(process.argv[2], "base64").toString());
  const { prompt, systemPrompt, cwd, apiKey } = payload;

  // If an explicit API key is provided (from Settings), use it as override.
  // Otherwise the Agent SDK uses the existing Claude CLI authentication.
  if (apiKey) {
    process.env.ANTHROPIC_API_KEY = apiKey;
  }

  let emittedText = false;

  for await (const message of query({
    prompt,
    options: {
      cwd: cwd || process.env.HOME,
      systemPrompt: systemPrompt || undefined,
      allowedTools: ["Read", "Write", "Bash", "Glob", "Grep"],
      permissionMode: "bypassPermissions",
      allowDangerouslySkipPermissions: true,
      maxTurns: 10,
    },
  })) {
    if ("result" in message) {
      // Final result — only emit text if we haven't already from assistant messages
      if (message.result && !emittedText) {
        emit({ type: "text_delta", delta: message.result, timestamp: Date.now() });
      }
      emit({ type: "complete", timestamp: Date.now() });
    } else if (message.type === "assistant") {
      for (const block of message.message?.content ?? []) {
        if (block.type === "text" && block.text) {
          emit({ type: "text_delta", delta: block.text, timestamp: Date.now() });
          emittedText = true;
        } else if (block.type === "tool_use") {
          emit({ type: "tool_start", tool: block.name, timestamp: Date.now() });
          emit({ type: "tool_end", tool: block.name, args: block.input, timestamp: Date.now() });
        }
      }
    }
  }
} catch (e) {
  emit({
    type: "error",
    error: e.message || String(e),
    timestamp: Date.now(),
  });
  process.exit(1);
}
