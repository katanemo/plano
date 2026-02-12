#!/usr/bin/env bash
# Pretty-print Plano MODEL_RESOLUTION lines from docker logs
# - hides Arch-Router
# - prints timestamp
# - colors MODEL_RESOLUTION red
# - colors req_model cyan
# - colors resolved_model magenta
# - removes provider and streaming

docker logs -f plano 2>&1 \
| awk '
/MODEL_RESOLUTION:/ && $0 !~ /Arch-Router/ {
  # extract timestamp between first [ and ]
  ts=""
  if (match($0, /\[[0-9-]+ [0-9:.]+\]/)) {
    ts=substr($0, RSTART+1, RLENGTH-2)
  }

  # split out after MODEL_RESOLUTION:
  n = split($0, parts, /MODEL_RESOLUTION: */)
  line = parts[2]

  # remove provider and streaming fields
  sub(/ *provider='\''[^'\'']+'\''/, "", line)
  sub(/ *streaming=(true|false)/, "", line)

  # highlight fields
  gsub(/req_model='\''[^'\'']+'\''/, "\033[36m&\033[0m", line)
  gsub(/resolved_model='\''[^'\'']+'\''/, "\033[35m&\033[0m", line)

  # print timestamp + MODEL_RESOLUTION
  printf "\033[90m[%s]\033[0m \033[31mMODEL_RESOLUTION\033[0m: %s\n", ts, line
}'
