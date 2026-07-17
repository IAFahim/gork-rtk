# RTK + Gork Build

This fork branch combines:

1. **Gork Build** privacy hard-offs (`PRIVACY_BUILD`, no Mixpanel / research upload / vendor auto-update)
2. **RTK-compatible PreToolUse rewrite** (`updatedInput` / Claude `hookSpecificOutput`)

## How rewrite works

1. Model proposes a shell command, e.g. `git status`
2. PreToolUse hook runs `rtk hook claude`
3. RTK returns Claude-shaped JSON:

   ```json
   {
     "hookSpecificOutput": {
       "hookEventName": "PreToolUse",
       "permissionDecisionReason": "RTK auto-rewrite",
       "updatedInput": { "command": "rtk git status" }
     }
   }
   ```

4. Gork shallow-merges `updatedInput` into tool args and re-parses
5. The rewritten command executes; the model sees compact RTK output

Grok-native form also works:

```json
{ "decision": "allow", "updatedInput": { "command": "rtk git status" } }
```

## Setup

```sh
# 1. Install RTK
curl -fsSL https://raw.githubusercontent.com/rtk-ai/rtk/refs/heads/master/install.sh | sh
rtk --version

# 2. Install Gork (this build) on PATH as gork / grok

# 3. Hook (global)
mkdir -p ~/.grok/hooks
# see README "RTK (this branch)" for rtk-rewrite.json

# 4. Restart Gork; /hooks should list rtk-rewrite
```

## Verify

```sh
which gork; gork --version
printf '%s\n' '{"tool_name":"Bash","tool_input":{"command":"git status"}}' | rtk hook claude
# In a Gork session: run `git status` via shell — should execute as `rtk git status`
rtk gain
```

## What this is not

- Does not rewrite built-in tools (`read_file`, `grep`, …) — only shell tool calls
- Does not change model API traffic (prompts still go to Grok for inference)
- Privacy hard-offs remain independent of RTK
