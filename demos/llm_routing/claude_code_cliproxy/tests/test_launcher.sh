#!/usr/bin/env bash
set -euo pipefail

TEST_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
DEMO_DIR=$(cd "$TEST_DIR/.." && pwd)
LAUNCHER="$DEMO_DIR/claude-plano"
SYSTEM_RUBY=$(command -v ruby || true)
FIXTURE_ROOT=
trap '[[ -z $FIXTURE_ROOT ]] || rm -rf "$FIXTURE_ROOT"' EXIT

fail() {
  printf 'test_launcher: %s\n' "$*" >&2
  exit 1
}

assert_contains() {
  local haystack=$1
  local needle=$2
  local label=$3
  [[ $haystack == *"$needle"* ]] ||
    fail "$label: expected output containing '$needle', got: $haystack"
}

assert_not_contains() {
  local haystack=$1
  local needle=$2
  local label=$3
  [[ $haystack != *"$needle"* ]] ||
    fail "$label: unexpected output containing '$needle'"
}

write_mocks() {
  local bin_dir=$1

  cat >"$bin_dir/claude" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
{
  printf 'ANTHROPIC_BASE_URL=%s\n' "${ANTHROPIC_BASE_URL:-}"
  printf 'ANTHROPIC_AUTH_TOKEN=%s\n' "${ANTHROPIC_AUTH_TOKEN:-}"
  printf 'ANTHROPIC_API_KEY=%s\n' "${ANTHROPIC_API_KEY:-<unset>}"
  printf 'CLIPROXY_LOCAL_API_KEY=%s\n' "${CLIPROXY_LOCAL_API_KEY:-<unset>}"
  printf 'ANTHROPIC_MODEL=%s\n' "${ANTHROPIC_MODEL:-}"
  printf 'ANTHROPIC_DEFAULT_FABLE_MODEL=%s\n' "${ANTHROPIC_DEFAULT_FABLE_MODEL:-}"
  printf 'ANTHROPIC_DEFAULT_OPUS_MODEL=%s\n' "${ANTHROPIC_DEFAULT_OPUS_MODEL:-}"
  printf 'ANTHROPIC_DEFAULT_SONNET_MODEL=%s\n' "${ANTHROPIC_DEFAULT_SONNET_MODEL:-}"
  printf 'ANTHROPIC_DEFAULT_HAIKU_MODEL=%s\n' "${ANTHROPIC_DEFAULT_HAIKU_MODEL:-}"
  printf 'ARGC=%s\n' "$#"
  index=0
  for arg in "$@"; do
    printf 'ARG[%s]=<%s>\n' "$index" "$arg"
    index=$((index + 1))
  done
} >"$MOCK_CALLS/claude.log"
env | LC_ALL=C sort >"$MOCK_CALLS/claude.env"
MOCK

  cat >"$bin_dir/curl" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
[[ ${1:-} == -q ]] || exit 65
[[ -z ${CLIPROXY_LOCAL_API_KEY:-} ]] || exit 66
proxy_bypass=false
previous=
for arg in "$@"; do
  [[ $previous != --noproxy || $arg != '*' ]] || proxy_bypass=true
  previous=$arg
done
[[ $proxy_bypass == true ]] || exit 67
url=${!#}
printf '%s\n' "$*" >>"$MOCK_CALLS/curl.log"
case $url in
  */v1/models)
    IFS= read -r auth_header
    [[ $auth_header == 'Authorization: Bearer '* ]] || exit 68
    if [[ ${MOCK_INVALID_KEY:-0} == 1 ]]; then
      printf 'curl: (22) The requested URL returned error: 401\n' >&2
      exit 22
    fi
    if [[ -n ${MOCK_MODELS_JSON:-} ]]; then
      printf '%s\n' "$MOCK_MODELS_JSON"
    else
      printf '%s\n' '{"data":[{"id":"gpt-5.6-sol"},{"id":"gpt-5.6-terra"},{"id":"gpt-5.6-luna"}]}'
    fi
    ;;
  */healthz)
    printf '%s' "${MOCK_PLANO_STATUS:-200}"
    ;;
  *)
    printf 'unexpected mock curl URL: %s\n' "$url" >&2
    exit 64
    ;;
esac
MOCK

  cat >"$bin_dir/lsof" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
[[ -z ${CLIPROXY_LOCAL_API_KEY:-} ]] || exit 66
case ${MOCK_LISTENER:-loopback} in
  loopback) printf 'p4242\nn127.0.0.1:8317\n' ;;
  ipv6) printf 'p4242\nn[::1]:8317\n' ;;
  wildcard) printf 'p4242\nn*:8317\n' ;;
  missing) exit 1 ;;
  *) exit 64 ;;
esac
MOCK

  chmod +x "$bin_dir/claude" "$bin_dir/curl" "$bin_dir/lsof"
}

new_fixture() {
  FIXTURE_ROOT=$(mktemp -d)
  mkdir -p "$FIXTURE_ROOT/bin" "$FIXTURE_ROOT/calls" "$FIXTURE_ROOT/home"
  MOCK_CALLS="$FIXTURE_ROOT/calls"
  export MOCK_CALLS
  write_mocks "$FIXTURE_ROOT/bin"

  CLIPROXY_CONFIG="$FIXTURE_ROOT/cliproxy.yaml"
  cat >"$CLIPROXY_CONFIG" <<'YAML'
host: 127.0.0.1
port: 8317
api-keys:
  - config-canary-key
YAML

  export HOME="$FIXTURE_ROOT/home"
  export PATH="$FIXTURE_ROOT/bin:/usr/bin:/bin:/usr/sbin:/sbin"
  export CLIPROXY_CONFIG
  export CLAUDE_BIN="$FIXTURE_ROOT/bin/claude"
  export CURL_BIN="$FIXTURE_ROOT/bin/curl"
  export LSOF_BIN="$FIXTURE_ROOT/bin/lsof"
  export RUBY_BIN="$SYSTEM_RUBY"
  export CLIPROXY_URL=http://127.0.0.1:8317
  export PLANO_URL=http://127.0.0.1:12000
  unset ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN ANTHROPIC_BASE_URL
  unset CLIPROXY_LOCAL_API_KEY MOCK_INVALID_KEY MOCK_MODELS_JSON
  unset MOCK_LISTENER MOCK_PLANO_STATUS
  STDOUT_FILE="$FIXTURE_ROOT/stdout"
  STDERR_FILE="$FIXTURE_ROOT/stderr"
}

cleanup_fixture() {
  rm -rf "$FIXTURE_ROOT"
}

run_launcher() {
  "$LAUNCHER" "$@" >"$STDOUT_FILE" 2>"$STDERR_FILE"
}

run_launcher_with_allexport() {
  bash -a "$LAUNCHER" "$@" >"$STDOUT_FILE" 2>"$STDERR_FILE"
}

expect_failure() {
  local expected=$1
  shift
  if run_launcher "$@"; then
    fail "expected launcher failure containing: $expected"
  fi
  assert_contains "$(<"$STDERR_FILE")" "$expected" "failure message"
  [[ ! -f $MOCK_CALLS/claude.log ]] || fail 'Claude ran after failed preflight'
}

[[ -x $LAUNCHER ]] || fail "missing executable launcher: $LAUNCHER"
[[ -n $SYSTEM_RUBY ]] || fail 'ruby is required to run launcher tests'
if grep -Fq -- '--dangerously-skip-permissions' "$LAUNCHER"; then
  fail 'launcher must not disable Claude Code permission checks'
fi
if grep -Eq 'planoai (up|down)|PLANOAI_BIN|PLANO_STATE|PLANO_LOCK|kill |owner\.pid|attestation' "$LAUNCHER"; then
  fail 'launcher must not supervise services, PIDs, locks, or state'
fi
launcher_lines=$(wc -l <"$LAUNCHER" | tr -d ' ')
((launcher_lines < 140)) || fail "launcher must stay under 140 lines, got $launcher_lines"

# Happy path: key comes from explicit config and all required models are present.
new_fixture
run_launcher --print ready
claude_log=$(<"$MOCK_CALLS/claude.log")
assert_contains "$claude_log" 'ANTHROPIC_BASE_URL=http://127.0.0.1:12000' 'Plano base URL'
assert_contains "$claude_log" 'ANTHROPIC_AUTH_TOKEN=local-plano-proxy' 'local gateway token'
assert_contains "$claude_log" 'ANTHROPIC_MODEL=opus' 'default model alias'
assert_contains "$claude_log" 'ANTHROPIC_DEFAULT_FABLE_MODEL=claude-fable-5' 'Fable alias target'
assert_contains "$claude_log" 'ANTHROPIC_DEFAULT_OPUS_MODEL=claude-opus-4-8' 'Opus alias target'
assert_contains "$claude_log" 'ANTHROPIC_DEFAULT_SONNET_MODEL=claude-sonnet-5' 'Sonnet alias target'
assert_contains "$claude_log" 'ANTHROPIC_DEFAULT_HAIKU_MODEL=claude-haiku-4-5' 'Haiku alias target'
claude_env=$(<"$MOCK_CALLS/claude.env")
assert_not_contains "$claude_env" 'config-canary-key' 'config-file CLIProxy key absent from Claude environment'
cleanup_fixture

# Invalid local key: authenticated model discovery must fail closed.
new_fixture
export CLIPROXY_LOCAL_API_KEY=wrong-key
export MOCK_INVALID_KEY=1
expect_failure 'CLIProxyAPI authentication or model discovery failed' --print blocked
cleanup_fixture

# Missing required model: all three routing tiers must be advertised.
new_fixture
export CLIPROXY_LOCAL_API_KEY=environment-key
export MOCK_MODELS_JSON='{"data":[{"id":"gpt-5.6-sol"},{"id":"gpt-5.6-terra"}]}'
expect_failure 'missing required model: gpt-5.6-luna' --print blocked
cleanup_fixture

# Wildcard listener: a reachable service is not enough; it must be loopback-only.
new_fixture
export CLIPROXY_LOCAL_API_KEY=environment-key
export MOCK_LISTENER=wildcard
expect_failure 'CLIProxyAPI listener must be loopback-only' --print blocked
cleanup_fixture

# Plano unavailable: launcher never starts services and fails before Claude.
new_fixture
export CLIPROXY_LOCAL_API_KEY=environment-key
export MOCK_PLANO_STATUS=503
expect_failure 'Plano health check failed with HTTP 503' --print blocked
cleanup_fixture

# The CLIProxy secret must not survive anywhere in Claude's environment.
new_fixture
export CLIPROXY_LOCAL_API_KEY=secret-canary
export cliproxy_key=secret-canary
run_launcher --print scrubbed
claude_log=$(<"$MOCK_CALLS/claude.log")
claude_env=$(<"$MOCK_CALLS/claude.env")
assert_contains "$claude_log" 'CLIPROXY_LOCAL_API_KEY=<unset>' 'CLIProxy key scrubbed'
assert_not_contains "$claude_env" 'secret-canary' 'pre-exported CLIProxy key absent from Claude environment'
cleanup_fixture

# Allexport must not turn the internal key copy into a Claude environment variable.
new_fixture
export CLIPROXY_LOCAL_API_KEY=allexport-secret-canary
run_launcher_with_allexport --print scrubbed
claude_env=$(<"$MOCK_CALLS/claude.env")
assert_not_contains "$claude_env" 'allexport-secret-canary' 'allexport CLIProxy key absent from Claude environment'
cleanup_fixture

# IPv6 loopback literals are valid for the CLIProxyAPI URL.
new_fixture
export CLIPROXY_LOCAL_API_KEY=environment-key
export CLIPROXY_URL='http://[::1]:8317'
export MOCK_LISTENER=ipv6
run_launcher --print ipv6-cliproxy
cleanup_fixture

# IPv6 loopback literals are valid for the Plano URL.
new_fixture
export CLIPROXY_LOCAL_API_KEY=environment-key
export PLANO_URL='http://[::1]:12000'
run_launcher --print ipv6-plano
cleanup_fixture

# Exact argument forwarding preserves spaces, shell metacharacters, and empties.
new_fixture
export CLIPROXY_LOCAL_API_KEY=environment-key
run_launcher --print 'argument with spaces' 'semi;colon' ''
claude_log=$(<"$MOCK_CALLS/claude.log")
assert_contains "$claude_log" 'ARGC=4' 'argument count'
assert_contains "$claude_log" 'ARG[0]=<--print>' 'first argument'
assert_contains "$claude_log" 'ARG[1]=<argument with spaces>' 'spaced argument'
assert_contains "$claude_log" 'ARG[2]=<semi;colon>' 'metacharacter argument'
assert_contains "$claude_log" 'ARG[3]=<>' 'empty argument'
cleanup_fixture

printf 'test_launcher: PASS\n'
