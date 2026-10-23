#!/usr/bin/env bash
# Run on the Mac after bundling the native node with Mindmap.app.
set -euo pipefail
APP="${1:-$HOME/Applications/Mindmap.app}"
NODE="$APP/Contents/Resources/node/darwin-arm64/mindmap-node"
if [[ "$(uname -m)" == x86_64 ]]; then NODE="$APP/Contents/Resources/node/darwin-x64/mindmap-node"; fi
[[ -x "$NODE" ]] || { echo 'Bundle the native Mindmap node first.' >&2; exit 1; }
mkdir -p "$HOME/Library/LaunchAgents" "$HOME/Library/Logs/Mindmap" "$HOME/.local/bin"
ln -sfn "$NODE" "$HOME/.local/bin/mindmap-node"
python3 - "$NODE" <<'PY'
import pathlib, plistlib, sys
home=pathlib.Path.home()
data={
    'Label':'dev.mindmap.node',
    'ProgramArguments':[sys.argv[1],'serve','--bind','127.0.0.1:47822'],
    'RunAtLoad':True,'KeepAlive':True,
    'WorkingDirectory':str(home),
    'EnvironmentVariables':{'HOME':str(home),'HERDR_ENV':'1','HERDR_SOCKET_PATH':str(home/'.config/herdr/herdr.sock'),
        'PATH':f'{home}/.local/bin:{home}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin'},
    'StandardOutPath':str(home/'Library/Logs/Mindmap/node.log'),
    'StandardErrorPath':str(home/'Library/Logs/Mindmap/node.error.log'),
}
with (home/'Library/LaunchAgents/dev.mindmap.node.plist').open('wb') as f: plistlib.dump(data,f)
PY
DOMAIN="gui/$(id -u)"
launchctl bootout "$DOMAIN/dev.mindmap.node" >/dev/null 2>&1 || true
launchctl bootstrap "$DOMAIN" "$HOME/Library/LaunchAgents/dev.mindmap.node.plist"
launchctl kickstart "$DOMAIN/dev.mindmap.node"
