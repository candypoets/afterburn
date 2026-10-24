#!/usr/bin/env python3
"""Install an immutable native node and give one supervisor ownership of it."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time

LABEL = "dev.mindmap.node"
UNIT = "mindmap-node.service"


class InstallError(Exception):
    def __init__(self, message, code=1):
        super().__init__(message)
        self.code = code


def run(args, check=True):
    result = subprocess.run(args, text=True, capture_output=True, timeout=30)
    if check and result.returncode:
        raise InstallError(f"{' '.join(args)}: {result.stderr.strip() or result.stdout.strip()}")
    return result


def rpc(method, absent_ok=False, **parameters):
    try:
        connection = socket.create_connection(("127.0.0.1", 47822), timeout=3)
    except ConnectionRefusedError:
        if absent_ok:
            return None
        raise InstallError("Node disappeared while checking upgrade safety.", 21)
    with connection:
        try:
            connection.sendall((json.dumps({"id": 1, "version": 1, "method": method, **parameters}) + "\n").encode())
            reader = connection.makefile("rb")
            line = reader.readline(16 * 1024 * 1024 + 1)
        except (ConnectionResetError, BrokenPipeError, ConnectionAbortedError, socket.timeout):
            # A node launchd or systemd is stopping or starting drops the connection: no node yet.
            if absent_ok:
                return None
            raise InstallError(f"Cannot inspect node {method}; leaving it untouched.", 21)
        if not line and absent_ok:
            return None
        with reader:
            for attempt in range(100):
                if attempt:
                    line = reader.readline(16 * 1024 * 1024 + 1)
                if not line or len(line) > 16 * 1024 * 1024:
                    break
                reply = json.loads(line)
                if not isinstance(reply, dict):
                    raise InstallError("Malformed node response; leaving node untouched.", 21)
                if reply.get("id") == 1:
                    if reply.get("ok") is not True:
                        break
                    return reply
    raise InstallError(f"Cannot inspect node {method}; leaving it untouched.", 21)


def ensure_idle():
    try:
        reply = rpc("session.list")
        sessions = reply.get("sessions")
        if not isinstance(sessions, list):
            raise ValueError("missing sessions")
        for session in sessions:
            if session.get("syncPrepared") is not False or session.get("mergeReady") is not False:
                raise InstallError("Update pending: finish/cancel prepared Syncs and ready merges first.", 21)
    except InstallError:
        raise
    except (ValueError, OSError, AttributeError) as error:
        raise InstallError(f"Cannot inspect live sessions; leaving node untouched: {error}", 21)


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest() if hasattr(hashlib, "file_digest") else hashlib.sha256(stream.read()).hexdigest()


def atomic_write(path, data, mode=0o644):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".mindmap-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
        os.chmod(temporary, mode)
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def link_cli(path, binary):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".mindmap-node-{os.getpid()}")
    try:
        temporary.symlink_to(binary)
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def unit_quote(value, command=False):
    # systemd specifiers and command-line dollar expansion are separate from quoting.
    value = str(value).replace("\\", "\\\\").replace('"', '\\"').replace("%", "%%")
    return '"' + (value.replace("$", "$$") if command else value) + '"'


class Supervisor:
    def __init__(self, scope, home):
        self.mac = sys.platform == "darwin"
        self.home = home
        self.scope = scope
        if self.mac:
            if scope != "user":
                raise InstallError("macOS supports --scope user (launchd) or manual only.")
            self.domain = f"gui/{os.getuid()}"
            self.job = f"{self.domain}/{LABEL}"
            self.path = home / "Library/LaunchAgents" / f"{LABEL}.plist"
            if run(["launchctl", "print", self.domain], check=False).returncode:
                raise InstallError("No launchd GUI user domain. Log in to this Mac, or use --scope manual.")
        else:
            self.command = ["systemctl"] + (["--user"] if scope == "user" else [])
            self.path = (home / ".config/systemd/user" if scope == "user" else Path("/etc/systemd/system")) / UNIT
            if run(self.command + ["show-environment"], check=False).returncode:
                raise InstallError("systemd is unavailable for this scope. Use a logged-in user manager, system scope as root, or explicit --scope manual.")

    def pid(self):
        if self.mac:
            result = run(["launchctl", "print", self.job], check=False)
            found = re.search(r"^\s*pid = (\d+)\s*$", result.stdout, re.MULTILINE)
            return int(found[1]) if found else 0
        value = run(self.command + ["show", UNIT, "--property=MainPID", "--value"], check=False).stdout.strip()
        return int(value) if value.isdigit() else 0

    def loaded(self):
        if self.mac:
            return run(["launchctl", "print", self.job], check=False).returncode == 0
        return self.pid() != 0

    def config(self, binary):
        home = self.home
        environment = {
            "HOME": str(home), "HERDR_ENV": "1",
            "HERDR_SOCKET_PATH": os.environ.get("HERDR_SOCKET_PATH", str(home / ".config/herdr/herdr.sock")),
            "PATH": f"{home}/.local/bin:{home}/.cargo/bin:{home}/.volta/bin:{home}/.kimi-code/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        }
        for key in ("HERDR_SESSION", "HERDR_BIN_PATH"):
            if key in os.environ:
                environment[key] = os.environ[key]
        # Retain existing Herdr settings when the installer was launched from a sparse GUI env.
        if self.path.exists():
            if self.mac:
                old = plistlib.loads(self.path.read_bytes()).get("EnvironmentVariables", {})
                for key in ("HERDR_SESSION", "HERDR_SOCKET_PATH", "HERDR_BIN_PATH"):
                    if key not in os.environ and key in old:
                        environment[key] = old[key]
            else:
                for line in self.path.read_text().splitlines():
                    match = re.fullmatch(r'Environment="?(HERDR_SESSION|HERDR_SOCKET_PATH|HERDR_BIN_PATH)=(.*?)"?', line)
                    if match and match[1] not in os.environ:
                        environment[match[1]] = match[2].replace("%%", "%").replace("$$", "$").replace('\\"', '"').replace("\\\\", "\\")
        if self.mac:
            logs = home / "Library/Logs/Mindmap"
            logs.mkdir(parents=True, exist_ok=True)
            return plistlib.dumps({
                "Label": LABEL, "ProgramArguments": [str(binary), "serve", "--bind", "127.0.0.1:47822"],
                "RunAtLoad": True, "KeepAlive": True, "ThrottleInterval": 3,
                "WorkingDirectory": str(home), "EnvironmentVariables": environment,
                "StandardOutPath": str(logs / "node.log"), "StandardErrorPath": str(logs / "node.error.log"),
                "AbandonProcessGroup": True,
            })
        lines = ["[Unit]", "Description=Mindmap node (worktree designs and shared Herdr)", "After=network.target", "", "[Service]", "Type=simple",
                 # WorkingDirectory is a single path, not a shell-style word list.
                 f"WorkingDirectory={str(home).replace('%', '%%')}", f"ExecStart={unit_quote(binary, command=True)} serve --bind 127.0.0.1:47822",
                 "Restart=on-failure", "RestartSec=2", "KillMode=process"]
        lines.extend(f"Environment={unit_quote(key + '=' + value)}" for key, value in environment.items())
        lines.extend(["", "[Install]", "WantedBy=" + ("default.target" if self.scope == "user" else "multi-user.target"), ""])
        return "\n".join(lines).encode()

    def stop(self):
        if self.mac:
            run(["launchctl", "bootout", self.job])
        else:
            # Enforce process-only termination even for a preexisting unit.
            mode = run(self.command + ["show", UNIT, "--property=KillMode", "--value"]).stdout.strip()
            if mode != "process":
                raise InstallError("Existing unit must use KillMode=process before an upgrade; shared Herdr must survive.")
            run(self.command + ["stop", UNIT])

    def start(self):
        if self.mac:
            run(["launchctl", "bootstrap", self.domain, str(self.path)])
        else:
            run(self.command + ["daemon-reload"])
            run(self.command + ["enable", "--now", UNIT])

    def reload(self):
        if not self.mac:
            run(self.command + ["daemon-reload"])


def install(args):
    if not re.fullmatch(r"[a-f0-9]{64}", args.build_id):
        raise InstallError("--build-id must be the 64-character canonical source SHA256.")
    binary = Path(args.binary).resolve(strict=True)
    info = json.loads(run([str(binary), "build-info"]).stdout)
    if info.get("build_id") != args.build_id or info.get("protocol") != 1 or not info.get("version"):
        raise InstallError("Native binary build-info does not match the requested build/protocol.")
    if args.scope == "system" and os.geteuid() != 0:
        raise InstallError("--scope system requires running as root; this installer never invokes sudo.")
    if sys.platform not in ("darwin", "linux"):
        raise InstallError("Supported supervisors are macOS launchd and Linux systemd.")
    home = Path.home()
    base = Path("/usr/local/lib/mindmap/node") if args.scope == "system" else home / ".local/lib/mindmap/node"
    cli = Path("/usr/local/bin/mindmap-node") if args.scope == "system" else home / ".local/bin/mindmap-node"
    base.mkdir(parents=True, exist_ok=True)
    lock = base / ".install-lock"
    lease = None
    try:
        lock.mkdir()
    except FileExistsError:
        raise InstallError(f"Another install is running (or left a stale lock): {lock}")
    try:
        supervisor = None if args.scope == "manual" else Supervisor(args.scope, home)
        current = rpc("hello", absent_ok=True)
        if current:
            if current.get("build_id") != args.build_id and not args.upgrade:
                raise InstallError("A different node is running. Approve the update and rerun with --upgrade.", 20)
            if supervisor is None:
                raise InstallError("A node is running; manual installation never replaces or stops it.", 22)
            if not current.get("pid") or current["pid"] != supervisor.pid():
                raise InstallError("Listener is not owned by the selected supervisor (or is a legacy node). Migrate it explicitly; no process was stopped.", 22)
            if current.get("build_id") == args.build_id:
                installed = base / args.build_id / "mindmap-node"
                if not installed.is_file():
                    raise InstallError("Same-build node uses an older install layout; migrate it explicitly.", 22)
                link_cli(cli, installed)
                return {"status": "already-running", "build_id": args.build_id, "binary": str(installed)}
            ensure_idle()
        elif supervisor and supervisor.pid():
            raise InstallError("Managed node is running but not answering; refusing to replace it.", 21)
        destination = base / args.build_id / "mindmap-node"
        if destination.exists():
            if sha256(destination) != sha256(binary):
                raise InstallError("This build ID already has a different binary; immutable installs are never overwritten.")
        else:
            atomic_write(destination, binary.read_bytes(), 0o755)
        if supervisor:
            config = supervisor.config(destination)
            previous = supervisor.path.read_bytes() if supervisor.path.exists() else None
            was_loaded = supervisor.loaded()
            if current:
                ensure_idle()
                latest = rpc("hello")
                if latest.get("pid") != current["pid"] or supervisor.pid() != current["pid"]:
                    raise InstallError("Node ownership changed during upgrade; retry explicitly.", 22)
                # The node rejects new save gates while this short lease is held.
                # session.list alone would leave a race between inspection and stop.
                try:
                    prepared = rpc("upgrade.prepare")
                    lease = prepared.get("lease")
                    if not isinstance(lease, str) or not lease or prepared.get("pid") != current["pid"]:
                        raise InstallError("Upgrade lease/ownership was not confirmed; leaving node untouched.", 21)
                except (OSError, ValueError) as error:
                    raise InstallError(f"Cannot reserve upgrade: {error}", 21)
            if was_loaded:
                supervisor.stop()
            if rpc("hello", absent_ok=True):
                raise InstallError("Another node acquired the listener; refusing to start a competing service.", 22)
            try:
                atomic_write(supervisor.path, config)
                supervisor.start()
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline:
                    ready = rpc("hello", absent_ok=True)
                    if ready:
                        if ready.get("build_id") != args.build_id or ready.get("pid") != supervisor.pid():
                            raise InstallError("Started service does not own the expected node/build.", 22)
                        break
                    time.sleep(0.25)
                else:
                    raise InstallError("Installed supervisor did not start a responding node within 15 seconds.")
            except Exception:
                # Restore the previous service definition; never remove installed versions.
                if supervisor.loaded():
                    supervisor.stop()
                if previous is not None:
                    atomic_write(supervisor.path, previous)
                    supervisor.reload()
                    if was_loaded:
                        supervisor.start()
                else:
                    supervisor.path.unlink(missing_ok=True)
                    supervisor.reload()
                raise
        link_cli(cli, destination)
        return {"status": "installed" if supervisor else "manual-no-supervisor", "build_id": args.build_id, "binary": str(destination)}
    finally:
        if lease:
            # Only the old process knows this lease; a failed stop must release its gate.
            try:
                rpc("upgrade.cancel", lease=lease)
            except (InstallError, OSError, ValueError):
                pass  # The lease also expires automatically after 30 seconds.
        lock.rmdir()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--build-id", required=True)
    parser.add_argument("--scope", choices=("user", "system", "manual"), default="user")
    parser.add_argument("--upgrade", "--replace", action="store_true", help="explicitly approve replacing a supervised, idle node")
    args = parser.parse_args()
    try:
        print(json.dumps(install(args)))
    except (InstallError, OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"mindmap-node install: {error}", file=sys.stderr)
        return getattr(error, "code", 1)
    return 0


if __name__ == "__main__":
    sys.exit(main())
