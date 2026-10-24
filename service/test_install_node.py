#!/usr/bin/env python3
"""Packaging lifecycle tests. All supervisor commands and node RPCs are mocked."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(file))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


installer = load('installer', 'install-node.py')
assets = load('assets', 'node-assets.py')
NEW = 'a' * 64
OLD = 'b' * 64
INFO = {'build_id': NEW, 'protocol': 1, 'version': '0.1.0'}


class FakeSupervisor:
    def __init__(self, home, current):
        self.path = home / 'unit'
        self.path.write_bytes(b'old unit')
        self.current = current
        self.actions = []
        self.fail_start = False

    def pid(self):
        return self.current['pid'] if self.current else 0

    def loaded(self):
        return self.current is not None

    def config(self, binary):
        return str(binary).encode()

    def stop(self):
        self.actions.append('stop')
        self.current = None

    def start(self):
        self.actions.append('start')
        if self.fail_start:
            self.fail_start = False
            raise installer.InstallError('mock startup failed')
        self.current = {'pid': 123, 'build_id': OLD if self.path.read_bytes() == b'old unit' else NEW}

    def reload(self):
        pass


class InstallTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.home = Path(self.temporary.name)
        self.binary = self.home / 'candidate'
        self.binary.write_bytes(b'native candidate')
        self.args = argparse.Namespace(binary=str(self.binary), build_id=NEW, scope='user', upgrade=False)
        self.supervisor = FakeSupervisor(self.home, None)
        self.sessions = []
        self.methods = []
        self.lease_failure = False
        for target, replacement in (
            ('Path.home', lambda: self.home),
            ('run', lambda command: subprocess.CompletedProcess(command, 0, json.dumps(INFO), '')),
            ('Supervisor', lambda scope, home: self.supervisor),
            ('rpc', self.rpc),
        ):
            mocker = patch.object(installer.Path, 'home', replacement) if target == 'Path.home' else patch.object(installer, target, replacement)
            mocker.start()
            self.addCleanup(mocker.stop)

    def rpc(self, method, absent_ok=False, **kwargs):
        self.methods.append(method)
        if method == 'hello':
            return self.supervisor.current
        if method == 'session.list':
            return {'sessions': self.sessions}
        if method == 'upgrade.prepare':
            if self.lease_failure:
                raise installer.InstallError('gate changed during upgrade', 21)
            return {'lease': 'reserved', 'pid': self.supervisor.pid(), 'expires_in': 30}
        if method == 'upgrade.cancel':
            return {'ok': True}
        self.fail('unexpected RPC')

    def running(self):
        self.supervisor.current = {'pid': 123, 'build_id': OLD}

    def assert_refused(self, code):
        with self.assertRaises(installer.InstallError) as result:
            installer.install(self.args)
        self.assertEqual(result.exception.code, code)
        self.assertEqual(self.supervisor.actions, [])

    def test_fresh_supervised_install_and_standalone_cli(self):
        result = installer.install(self.args)
        self.assertEqual(result['status'], 'installed')
        self.assertEqual(self.supervisor.actions, ['start'])
        target = self.home / '.local/lib/mindmap/node' / NEW / 'mindmap-node'
        self.assertEqual(target.read_bytes(), b'native candidate')
        self.assertEqual((self.home / '.local/bin/mindmap-node').resolve(), target)
        self.binary.unlink()
        self.assertTrue(target.is_file(), 'bundle removal must not remove installed helper')

    def test_running_different_build_requires_approval(self):
        self.running()
        self.assert_refused(20)
        self.assertNotIn('session.list', self.methods)

    def test_prepared_sync_merge_and_malformed_state_refused(self):
        for state in ({'syncPrepared': True, 'mergeReady': False}, {'syncPrepared': False, 'mergeReady': True}, {}):
            with self.subTest(state=state):
                self.running()
                self.args.upgrade = True
                self.sessions = [state]
                self.assert_refused(21)

    def test_unmanaged_legacy_listener_refused(self):
        self.supervisor.current = {'build_id': OLD}
        self.args.upgrade = True
        self.assert_refused(22)

    def test_upgrade_lease_prevents_inspect_stop_race(self):
        self.running()
        self.args.upgrade = True
        self.lease_failure = True
        self.assert_refused(21)
        self.assertIn('upgrade.prepare', self.methods)

    def test_approved_idle_upgrade_stops_only_selected_supervisor(self):
        self.running()
        self.args.upgrade = True
        self.sessions = [{'syncPrepared': False, 'mergeReady': False}]
        installer.install(self.args)
        self.assertEqual(self.supervisor.actions, ['stop', 'start'])
        self.assertEqual(self.methods.count('session.list'), 2)
        self.assertIn('upgrade.prepare', self.methods)
        self.assertIn('upgrade.cancel', self.methods)

    def test_failed_start_restores_previous_service(self):
        self.running()
        self.args.upgrade = True
        self.supervisor.fail_start = True
        with self.assertRaisesRegex(installer.InstallError, 'mock startup failed'):
            installer.install(self.args)
        self.assertEqual(self.supervisor.path.read_bytes(), b'old unit')
        self.assertEqual(self.supervisor.current['build_id'], OLD)
        self.assertIn('upgrade.cancel', self.methods)

    def test_same_build_reconnect_does_not_restart(self):
        installer.install(self.args)
        self.supervisor.actions.clear()
        self.methods.clear()
        self.assertEqual(installer.install(self.args)['status'], 'already-running')
        self.assertEqual(self.supervisor.actions, [])
        self.assertNotIn('upgrade.prepare', self.methods)

    def test_manual_reports_no_supervisor(self):
        self.args.scope = 'manual'
        result = installer.install(self.args)
        self.assertEqual(result['status'], 'manual-no-supervisor')
        self.assertEqual(self.supervisor.actions, [])

    def test_immutable_build_collision_refused(self):
        path = self.home / '.local/lib/mindmap/node' / NEW / 'mindmap-node'
        path.parent.mkdir(parents=True)
        path.write_bytes(b'another binary')
        self.assert_refused(1)
        self.assertEqual(path.read_bytes(), b'another binary')


class ConfigAndAssetTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which('systemd-analyze'), 'requires systemd unit parser')
    def test_generated_unit_is_accepted_by_real_systemd_parser(self):
        with tempfile.TemporaryDirectory(prefix='mindmap home ') as directory:
            home = Path(directory)
            supervisor = installer.Supervisor.__new__(installer.Supervisor)
            supervisor.home, supervisor.scope, supervisor.mac = home, 'user', False
            supervisor.path = home / 'mindmap-node.service'
            supervisor.path.write_bytes(supervisor.config(Path('/usr/bin/true')))
            result = subprocess.run(['systemd-analyze', 'verify', '--man=no', str(supervisor.path)], text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_supervisors_preserve_shared_herdr(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            # Bypass command discovery; inspect generated service definitions only.
            supervisor = installer.Supervisor.__new__(installer.Supervisor)
            supervisor.home, supervisor.scope = home, 'user'
            supervisor.path = home / 'service'
            with patch.dict(installer.os.environ, {'HERDR_SESSION': 'shared', 'HERDR_SOCKET_PATH': '/tmp/shared.sock'}, clear=True):
                supervisor.mac = True
                config = plistlib.loads(supervisor.config(home / 'node'))
                self.assertTrue(config['AbandonProcessGroup'])
                self.assertEqual(config['EnvironmentVariables']['HERDR_SESSION'], 'shared')
                supervisor.path.write_bytes(plistlib.dumps(config))
            with patch.dict(installer.os.environ, {}, clear=True):
                config = plistlib.loads(supervisor.config(home / 'next-node'))
                self.assertEqual(config['EnvironmentVariables']['HERDR_SOCKET_PATH'], '/tmp/shared.sock')
            supervisor.path.unlink()
            supervisor.mac = False
            text = supervisor.config(home / 'node').decode()
            self.assertIn('KillMode=process', text)
            self.assertIn('WantedBy=default.target', text)

    def test_foreign_companion_provenance_and_tampering(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            platform = root / 'assets/linux-x64'
            platform.mkdir(parents=True)
            binary = platform / 'mindmap-node'
            binary.write_bytes(b'foreign executable must never run')
            info = {**INFO, 'platform': 'linux-x64', 'sha256': assets.digest(binary)}
            sidecar = platform / 'build-info.json'
            sidecar.write_text(json.dumps(info))
            output = root / 'destination'
            with patch.object(assets.subprocess, 'check_output', side_effect=AssertionError('foreign exec')):
                assets.copy_assets(platform.parent, INFO, output, 'darwin-arm64')
                self.assertEqual((output / 'linux-x64/mindmap-node').read_bytes(), binary.read_bytes())
                for key, value in (('build_id', OLD), ('sha256', '0' * 64), ('platform', 'darwin-x64')):
                    with self.subTest(key=key):
                        sidecar.write_text(json.dumps({**info, key: value}))
                        with self.assertRaises(ValueError):
                            assets.copy_assets(platform.parent, INFO, output, 'darwin-arm64')


class BuildScriptTests(unittest.TestCase):
    def test_native_full_and_headless_build_invocations(self):
        root = Path(__file__).resolve().parent.parent
        with tempfile.TemporaryDirectory() as directory:
            temporary = Path(directory)
            commands = temporary / 'commands'
            commands.mkdir()
            log = temporary / 'cargo.jsonl'
            cargo = commands / 'cargo'
            cargo.write_text("#!/usr/bin/env python3\n" + r"""
import json, os, pathlib, sys
args = sys.argv[1:]
with open(os.environ['TEST_BUILD_LOG'], 'a') as stream:
    stream.write(json.dumps(args) + '\n')
output = pathlib.Path(args[args.index('--target-dir') + 1]) / 'debug'
output.mkdir(parents=True, exist_ok=True)
info = {'build_id': 'a' * 64, 'version': '0.1.0', 'protocol': 1}
node = output / 'mindmap-node'
node.write_text('#!/usr/bin/env python3\nprint(' + repr(json.dumps(info)) + ')\n')
node.chmod(0o755)
app = output / 'figcraft'
app.write_text('same invocation desktop')
app.chmod(0o755)
""")
            cargo.chmod(0o755)
            for command in ('sips', 'iconutil', 'codesign'):
                path = commands / command
                path.write_text('#!/bin/sh\nexit 0\n')
                path.chmod(0o755)
            uname = commands / 'uname'
            uname.write_text('#!/bin/sh\ncase "$1" in -s) echo Darwin ;; -m) echo arm64 ;; esac\n')
            uname.chmod(0o755)
            environment = {**os.environ, 'PATH': str(commands) + os.pathsep + os.environ['PATH'],
                           'APP': str(temporary / 'Mindmap.app'), 'CARGO_TARGET_DIR': str(temporary / 'target'),
                           'PROFILE': 'debug', 'INSTALL_NODE': '0', 'TEST_BUILD_LOG': str(log)}
            for key in ('CARGO_BUILD_TARGET', 'NODE_ASSETS', 'UPGRADE_NODE'):
                environment.pop(key, None)
            result = subprocess.run(['bash', str(root / 'client/build-mac.sh')], env=environment, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            # The build id's crate is cleaned first, so its hash is always recomputed.
            self.assertEqual(len(calls), 2)
            self.assertIn('clean', calls[0])
            self.assertIn('mindmap-build', calls[0])
            self.assertIn('figcraft', calls[1])
            self.assertIn('mindmap-node', calls[1])
            resources = temporary / 'Mindmap.app/Contents/Resources'
            self.assertTrue((resources / 'install-node.sh').is_file())
            self.assertTrue((resources / 'install-node.py').is_file())
            sidecar = json.loads((resources / 'node/darwin-arm64/build-info.json').read_text())
            self.assertEqual(sidecar['build_id'], NEW)
            self.assertEqual(sidecar['sha256'], assets.digest(resources / 'node/darwin-arm64/mindmap-node'))
            result = subprocess.run(['bash', str(root / 'service/build-node.sh'), str(temporary / 'headless')], env=environment, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            self.assertEqual(len(calls), 3)
            self.assertNotIn('figcraft', calls[2])
            self.assertIn('mindmap-node', calls[2])
            self.assertTrue((temporary / 'headless/darwin-arm64/build-info.json').is_file())
            # A package's companion root includes installers, which must be ignored as assets.
            assets.copy_assets(temporary / 'headless', INFO, temporary / 'copied', 'linux-x64')
            self.assertTrue((temporary / 'copied/darwin-arm64/mindmap-node').is_file())


if __name__ == '__main__':
    unittest.main()
