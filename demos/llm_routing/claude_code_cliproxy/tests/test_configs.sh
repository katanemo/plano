#!/usr/bin/env bash
set -euo pipefail

TEST_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
DEMO_DIR=$(cd "$TEST_DIR/.." && pwd)
VALIDATOR="$DEMO_DIR/validate_configs.rb"
PLANO_CONFIG="$DEMO_DIR/config.yaml"
CLIPROXY_CONFIG="$DEMO_DIR/cliproxyapi.conf.example"
README="$DEMO_DIR/README.md"

fail() {
  printf 'test_configs: %s\n' "$*" >&2
  exit 1
}

expect_failure() {
  local expected=$1
  shift
  local output

  if output=$("$@" 2>&1); then
    fail "expected command to fail: $*"
  fi
  [[ $output == *"$expected"* ]] ||
    fail "expected failure containing '$expected', got: $output"
}

for path in "$VALIDATOR" "$PLANO_CONFIG" "$CLIPROXY_CONFIG" "$README"; do
  [[ -f $path ]] || fail "missing required file: $path"
done

ruby "$VALIDATOR" "$PLANO_CONFIG" "$CLIPROXY_CONFIG"

tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT

ruby -ryaml -e '
  config = Psych.safe_load_file(ARGV.fetch(0), aliases: true)
  config.fetch("model_providers").first["model"] += "[1m]"
  File.write(ARGV.fetch(1), Psych.dump(config))
' "$PLANO_CONFIG" "$tmp_dir/plano-suffixed.yaml"
expect_failure 'must not contain [1m]' \
  ruby "$VALIDATOR" "$tmp_dir/plano-suffixed.yaml" "$CLIPROXY_CONFIG"

ruby -ryaml -e '
  config = Psych.safe_load_file(ARGV.fetch(0), aliases: true)
  config["listeners"].first["address"] = "0.0.0.0"
  File.write(ARGV.fetch(1), Psych.dump(config))
' "$PLANO_CONFIG" "$tmp_dir/plano-public.yaml"
expect_failure 'listener address must be 127.0.0.1' \
  ruby "$VALIDATOR" "$tmp_dir/plano-public.yaml" "$CLIPROXY_CONFIG"

ruby -ryaml -e '
  config = Psych.safe_load_file(ARGV.fetch(0), aliases: true)
  config["remote-management"]["secret-key"] = "enabled-by-test"
  File.write(ARGV.fetch(1), Psych.dump(config))
' "$CLIPROXY_CONFIG" "$tmp_dir/cliproxy-management.yaml"
expect_failure 'management API must be disabled' \
  ruby "$VALIDATOR" "$PLANO_CONFIG" "$tmp_dir/cliproxy-management.yaml"

ruby -ryaml -e '
  config = Psych.safe_load_file(ARGV.fetch(0), aliases: true)
  config["host"] = ""
  File.write(ARGV.fetch(1), Psych.dump(config))
' "$CLIPROXY_CONFIG" "$tmp_dir/cliproxy-public.yaml"
expect_failure 'CLIProxyAPI host must be 127.0.0.1' \
  ruby "$VALIDATOR" "$PLANO_CONFIG" "$tmp_dir/cliproxy-public.yaml"

required_readme_text=(
  'Plano 0.4.27'
  'CLIProxyAPI 7.2.70'
  '7.2.75'
  'https://github.com/router-for-me/CLIProxyAPI'
  'https://help.router-for.me/'
  'gpt-5.6-sol'
  'gpt-5.6-terra'
  'gpt-5.6-luna'
  '[1m]'
  'Docker'
  'rollback'
)
for text in "${required_readme_text[@]}"; do
  grep -Fiq "$text" "$README" || fail "README missing required text: $text"
done

in_code_block=false
while IFS= read -r line; do
  if [[ $line == \`\`\`* ]]; then
    if [[ $in_code_block == true ]]; then
      in_code_block=false
    else
      in_code_block=true
    fi
    continue
  fi
  if [[ $in_code_block == true && $line == *"curl "* ]]; then
    curl_args=${line#*curl }
    [[ $curl_args == "-q --noproxy '*'"* ]] ||
      fail "README curl must start with -q --noproxy '*': $line"
  fi
done <"$README"

health_write_out_count=$(grep -Fc -- "--write-out '%{http_code}'" "$README" || true)
((health_write_out_count == 2)) ||
  fail "README must contain two exact-status Plano health examples, found $health_write_out_count"
health_200_count=$(grep -Fc "[[ \$plano_status == 200 ]]" "$README" || true)
((health_200_count == 2)) ||
  fail "README must require HTTP 200 in both Plano health examples, found $health_200_count"

mac_user_path='/''Users/'
linux_user_path='/''home/'
internal_alias='c''cx'
internal_name='Pre''stance'
prohibited_pattern="(${mac_user_path}|${linux_user_path}[^<[:space:]]+|${internal_alias}|${internal_name}|sk-[A-Za-z0-9_-]{12,}|ghp_[A-Za-z0-9]{12,})"
scan_paths=(
  "$README"
  "$PLANO_CONFIG"
  "$CLIPROXY_CONFIG"
  "$DEMO_DIR/claude-plano"
  "$VALIDATOR"
  "$DEMO_DIR/test.sh"
  "$DEMO_DIR/tests/test_launcher.sh"
  "$DEMO_DIR/tests/test_configs.sh"
)
if grep -Eiq "$prohibited_pattern" "${scan_paths[@]}"; then
  fail 'demo contains a prohibited local path, internal reference, or credential-like value'
fi

printf 'test_configs: PASS\n'
