#!/usr/bin/env bash
# Stand-in for the real `claude` CLI used by the brightstaff integration test.
# Reads stdin (so it does not exit early when the bridge writes the user
# JSONL turn) and emits a canned `--output-format stream-json` NDJSON
# sequence that mirrors a one-turn "Hello, world!" response.
#
# All CLI flags are accepted and ignored; only the NDJSON output matters for
# the bridge-side translation.
set -euo pipefail

# Drain any stdin the parent writes so it does not see EPIPE.
( cat > /dev/null ) &
DRAIN_PID=$!
trap 'kill ${DRAIN_PID} 2>/dev/null || true' EXIT

cat <<'EOF'
{"type":"system","subtype":"init","session_id":"fake-session","model":"sonnet","cwd":"/tmp","tools":[]}
{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_fake","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-6","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":3,"output_tokens":0}}}}
{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":", world!"}}}
{"type":"stream_event","event":{"type":"content_block_stop","index":0}}
{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"input_tokens":0,"output_tokens":4}}}
{"type":"stream_event","event":{"type":"message_stop"}}
{"type":"result","subtype":"success","is_error":false,"duration_ms":12,"num_turns":1,"result":"Hello, world!","total_cost_usd":0.0001,"usage":{"input_tokens":3,"output_tokens":4},"session_id":"fake-session"}
EOF
