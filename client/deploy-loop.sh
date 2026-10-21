#!/usr/bin/env bash
# Retry building the Mac client and refreshing Mindmap.app until the host answers.
set -u
for i in $(seq 1 40); do
  if timeout 200 ssh -o BatchMode=yes -o ConnectTimeout=15 macbook \
      'cd ~/code/figcraft && set -o pipefail && cargo build -p figcraft 2>&1 | tail -2 && cp target/debug/figcraft ~/Applications/Mindmap.app/Contents/MacOS/Mindmap && codesign --force --sign - ~/Applications/Mindmap.app'; then
    timeout 60 ssh -o BatchMode=yes -o ConnectTimeout=15 macbook \
      'pkill -f "Mindmap.app/Contents/MacOS/Mindmap" 2>/dev/null; sleep 1; open -a ~/Applications/Mindmap.app --args --map-mode --control 49160' || true
    echo "deployed on attempt $i"
    exit 0
  fi
  echo "attempt $i failed"
  sleep 30
done
echo "gave up"
exit 1
