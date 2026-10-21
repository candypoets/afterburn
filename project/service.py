#!/usr/bin/env python3
"""Mindmap's project service.

Turns a Herdr workspace into a map document the client can open, without a join link in the
human's hands. The link exists here, in the record; the app is launched with it and joins over its
control channel.

    python3 project/service.py list
    python3 project/service.py open <workspace_id|label>
    python3 project/service.py serve

Environment:
    MINDMAP_SERVER   public address of the document server (default 100.104.9.15:47910)
    MINDMAP_MAC      ssh host of the Mac client (default macbook)
    MINDMAP_HTTP     bind address for serve (default 0.0.0.0:47820)
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import time
import urllib.parse
from datetime import datetime, timezone

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "design"))

DATA = ROOT / "project" / "data"
DATA.mkdir(parents=True, exist_ok=True)
RECORDS = DATA / "projects.json"
KEY_FILE = ROOT / "server" / "data" / "server.key"
CLI = ROOT / "server" / "figcraft" / "target" / "release" / "figcraft-cli"

SERVER = os.environ.get("MINDMAP_SERVER", "100.104.9.15:47910")
MAC = os.environ.get("MINDMAP_MAC", "macbook")
APP = os.environ.get("MINDMAP_APP", "~/Applications/Mindmap.app")
HTTP = os.environ.get("MINDMAP_HTTP", "0.0.0.0:47820")


def now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


# --- records ------------------------------------------------------------------

def records() -> dict:
    if RECORDS.exists():
        return json.loads(RECORDS.read_text())
    return {}


def save_records(r: dict) -> None:
    RECORDS.write_text(json.dumps(r, indent=1, sort_keys=True))


# --- Herdr --------------------------------------------------------------------

def herdr(*args: str) -> dict:
    env = dict(os.environ, HERDR_ENV="1")
    out = subprocess.run(["herdr", *args], capture_output=True, text=True, env=env, timeout=20)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip() or f"herdr {' '.join(args)} failed")
    return json.loads(out.stdout)["result"]


def workspaces() -> list[dict]:
    """Herdr's workspaces, with their agents and our map state."""
    ws = herdr("workspace", "list")["workspaces"]
    agents = herdr("agent", "list")["agents"]
    by_ws: dict[str, list[dict]] = {}
    for a in agents:
        by_ws.setdefault(a["workspace_id"], []).append(a)
    recs = records()
    out = []
    for w in ws:
        wid = w["workspace_id"]
        tree = w.get("worktree") or {}
        path = tree.get("checkout_path")
        mine = by_ws.get(wid, [])
        state = "map" if recs.get(wid, {}).get("built") else ("empty" if wid in recs else "none")
        out.append({
            "id": wid,
            "label": w["label"],
            "path": path,
            "repo": tree.get("repo_name"),
            "worktree": bool(tree.get("is_linked_worktree")),
            "agents": len(mine),
            "agent_names": sorted({a["agent"] for a in mine}),
            "agent_status": w.get("agent_status", "unknown"),
            "focused": w.get("focused", False),
            "map": state,
            "file": recs.get(wid, {}).get("file"),
            "tabs": w.get("tab_count"),
            "panes": w.get("pane_count"),
        })
    return sorted(out, key=lambda x: x["label"].lower())


# --- publishing ---------------------------------------------------------------

def rewrite_host(link: str, host: str) -> str:
    u = urllib.parse.urlsplit(link)
    q = urllib.parse.parse_qs(u.query)
    q["host"] = [host]
    return urllib.parse.urlunsplit((u.scheme, u.netloc, u.path, urllib.parse.urlencode(q, doseq=True), ""))


def publish_skeleton(ws: dict) -> dict:
    """Publish a one-page document for an empty workspace and store its records."""
    import fc
    import build_map

    label = ws["label"]
    doc = fc.Doc(label)
    page = doc.page("Overview")
    doc.place(page, fc.T(label, 34, font="Charter", style="Regular", color="#1B150D",
                         name="Workspace", w=900), 80, 80)
    doc.place(page, fc.T("No map yet", 20, font="Inter", style="Medium", color="#B5452F",
                         name="State"), 80, 150)
    doc.place(page, fc.T(
        "The project service made this workspace a document. Init will ask an agent to read the "
        "repository and draw the first overview.",
        13, font="Inter", style="Regular", color="#6E6C68", name="Note", w=720, lh=20), 80, 186)
    if ws.get("path"):
        doc.place(page, fc.T(ws["path"], 11, font="JetBrains Mono", style="Regular",
                             color="#6E6C68", name="Path"), 80, 248)
    skeleton = DATA / f"{ws['id']}.figcraft"
    doc.save(str(skeleton))

    key = KEY_FILE.read_text().strip()
    build_map.CLI = str(CLI)
    s = build_map.Session(bridge=False)
    try:
        s.tool("open_file", path=str(skeleton))
        s.cmd("collab.publish", server=f"ws://127.0.0.1:47910/?key={key}",
              name=f"Mindmap · {label}")
        chosen = None
        for _ in range(40):
            time.sleep(0.5)
            peers = s.cmd("collab.peers")
            if peers.get("status") == "live":
                chosen = peers
                break
        if not chosen:
            raise RuntimeError(f"publish did not become live: {peers.get('status')} {peers.get('reason')}")
        pub = chosen.get("published") or {}
        edit = pub.get("edit") or chosen.get("link")
        if not edit:
            raise RuntimeError("publish returned no edit link")
        rec = {
            "id": ws["id"],
            "label": label,
            "path": ws.get("path"),
            "file": pub.get("file"),
            "edit": rewrite_host(edit, SERVER),
            "view": rewrite_host(pub["view"], SERVER) if pub.get("view") else None,
            "name": pub.get("name"),
            "built": False,
            "created": now(),
        }
        r = records()
        r[ws["id"]] = rec
        save_records(r)
        return rec
    finally:
        s.close()


def ensure_project(ws: dict) -> dict:
    rec = records().get(ws["id"])
    if rec and rec.get("edit"):
        return rec
    return publish_skeleton(ws)


# --- Mac ----------------------------------------------------------------------

def control(port: int, method: str, params: dict) -> dict:
    py = (
        "import socket,json,sys\n"
        f"s=socket.create_connection(('127.0.0.1',{port}),timeout=20)\n"
        "s.sendall((json.dumps({'id':1,'method':%r,'params':json.loads(%r)})+'\\n').encode())\n"
        "b=b''\n"
        "while b'\\n' not in b:\n"
        "    c=s.recv(65536)\n"
        "    if not c: break\n"
        "    b+=c\n"
        "print(b.decode(errors='replace'))\n"
    ) % (method, json.dumps(params))
    enc = base64.b64encode(py.encode()).decode()
    out = subprocess.run(["ssh", "-o", "BatchMode=yes", MAC,
                          f"echo {enc} | base64 -d | python3 -"],
                         capture_output=True, text=True, timeout=40)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip() or "control failed")
    return json.loads(out.stdout.strip().splitlines()[-1])


def open_on_mac(rec: dict, port: int) -> None:
    link = rec["edit"]
    subprocess.run(["ssh", "-o", "BatchMode=yes", MAC,
                    f"pkill -f 'Mindmap.app/Contents/MacOS/Mindmap' 2>/dev/null; sleep 1; "
                    f"open -a {APP} --args --map-mode --control {port} '{link}'"],
                   check=False, capture_output=True, text=True, timeout=40)
    time.sleep(4)
    control(port, "engine.execute", {"command": "collab.join", "params": {"link": link}})
    time.sleep(2)
    control(port, "ui.input", {"events": [{"key": "Escape"}]})


def open_workspace(ws_id: str) -> dict:
    ws = next((w for w in workspaces() if w["id"] == ws_id or w["label"] == ws_id), None)
    if not ws:
        raise SystemExit(f"no workspace {ws_id!r}")
    rec = ensure_project(ws)
    port = 49310 + int(hashlib.sha1(ws["id"].encode()).hexdigest()[:4], 16) % 300
    open_on_mac(rec, port)
    return {"workspace": ws, "project": rec, "control": port}


# --- HTTP ---------------------------------------------------------------------

def serve() -> None:
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

    class H(BaseHTTPRequestHandler):
        def _send(self, code: int, body) -> None:
            data = json.dumps(body, indent=1).encode()
            self.send_response(code)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self) -> None:  # noqa: N802
            path = urllib.parse.urlsplit(self.path).path.strip("/")
            try:
                if path in ("", "health"):
                    self._send(200, {"ok": True, "workspaces": len(workspaces())})
                elif path == "workspaces":
                    self._send(200, {"workspaces": workspaces()})
                elif path.startswith("open/"):
                    self._send(200, open_workspace(path.split("/", 1)[1]))
                elif path.startswith("init/"):
                    self._send(200, open_workspace(path.split("/", 1)[1]))
                else:
                    self._send(404, {"error": "not found"})
            except Exception as e:  # noqa: BLE001
                self._send(500, {"error": str(e)})

        def log_message(self, *a) -> None:  # quiet
            pass

    host, _, port = HTTP.rpartition(":")
    srv = ThreadingHTTPServer((host or "0.0.0.0", int(port)), H)
    print(f"project service on http://{HTTP} (server {SERVER}, Mac {MAC})")
    srv.serve_forever()


# --- cli ----------------------------------------------------------------------

def main() -> None:
    if len(sys.argv) < 2 or sys.argv[1] in ("-h", "--help"):
        print(__doc__)
        return
    cmd = sys.argv[1]
    if cmd == "list":
        for w in workspaces():
            path = w["path"] or "—"
            print(f"{w['id']:4}  {w['label']:<18}  {w['map']:<5}  "
                  f"{w['agents']} agent{'s' if w['agents'] != 1 else ''} · {w['agent_status']:<8}  {path}")
    elif cmd == "open":
        if len(sys.argv) < 3:
            raise SystemExit("open needs a workspace id or label")
        r = open_workspace(sys.argv[2])
        print(json.dumps({"workspace": r["workspace"]["label"], "file": r["project"]["file"],
                          "control": r["control"]}, indent=1))
    elif cmd == "serve":
        serve()
    else:
        raise SystemExit(f"unknown command {cmd}")


if __name__ == "__main__":
    main()
