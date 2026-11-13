"""Actual native host/owner registration bridge, complete owned Carbon SDK only."""
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import shutil
import sys
import tempfile
import time
import unittest
import bridge_process as bridge
import actions_process as actions
import runtime_process as rt

if native_host := os.environ.get('RUYCAST_TEST_APP_HOST'):
    bridge.TOOLS=Path(native_host)
    rt.HOST=bridge.TOOLS

class NativeKeyboard(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        parent=Path(os.environ.get('RUYCAST_NATIVE_TEST_FIXTURE_DIR',rt.ROOT/'target'));parent.mkdir(parents=True,exist_ok=True)
        cls.sdk = tempfile.TemporaryDirectory(prefix='native-keyboard-sdk-',dir=parent)
        cls.shim=Path(cls.sdk.name)/'keyboard.dylib'
        target=Path(os.environ.get('CARGO_TARGET_DIR',rt.ROOT.parent/'.build/full-rust-release/native'))
        artifact=Path(os.environ.get('RUYCAST_TEST_KEYBOARD_HOST_SHIM',target/'release/examples/libkeyboard_host.dylib'))
        if not artifact.is_absolute() or not artifact.is_file():
            raise RuntimeError('Build --features test-tools --example keyboard-host first')
        shutil.copyfile(artifact,cls.shim)
        if out:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            dest=Path(out)/'host-sdks';dest.mkdir(parents=True,exist_ok=True);artifact=dest/(cls.__name__+'.dylib');shutil.copy2(cls.shim,artifact)
            sources={str(rt.ROOT.parent/name):hashlib.sha256((rt.ROOT.parent/name).read_bytes()).hexdigest() for name in ['rust-native/Cargo.toml','rust-native/Cargo.lock','rust/Cargo.toml','rust-toolchain.toml','rust-native/tests/fixtures/keyboard_host_loader.json',*[f'rust-native/tests/fixtures/{name}.rs' for name in ['keyboard_host','caps_sdk','hid_sdk','keyboard_sdk','keyword_core','keyword_notification','keyword_board','keyword_ax','keyword_events','workspace_sdk']]]}
            (dest/(cls.__name__+'.json')).write_text(json.dumps(dict(sources=sources,artifact=str(artifact),sha256=hashlib.sha256(artifact.read_bytes()).hexdigest()),indent=2))
    @classmethod
    def tearDownClass(cls):cls.sdk.cleanup()
    def host(self,commands=None,start=True,optin=True,marker=True,theme=None):
        host=bridge.Bridge(start=False);self.addCleanup(host.close)
        if commands is not None or theme is not None:
            config=copy.deepcopy(rt.DEFAULT);config['commands']+=commands or []
            if theme is not None:config['theme']=theme
            host.replace(config)
        if marker:(host.base/'.ruycast-keyboard-fixture').write_text('own-keyboard-fixture-v1')
        host.env.update(DYLD_INSERT_LIBRARIES=str(self.shim),RUYCAST_KEYWORD_FIXTURE_HOME=str(host.base),RUYCAST_KEYBOARD_FIXTURE_HOME=str(host.base),RUYCAST_WORKSPACE_FIXTURE_HOME=str(host.base),RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD='trace')
        if optin:host.env['RUYCAST_NATIVE_TEST_KEYBOARD']='1'
        if start:
            host.start()
            host.settled(lambda s:s['keyboardGeneration']>0 if optin else not s['nativeKeyboardService'])
        return host
    def emit(self,host,code=49,modifiers=6144,**kwargs):return host.probe('keyboard-press',code=code,modifiers=modifiers,**kwargs)['statuses']
    def trace(self,host):return [json.loads(line) for line in (host.base/'keyboard-trace.jsonl').read_text().splitlines()]
    def wait_file(self,path):
        end=time.monotonic()+4
        while not path.exists():
            self.assertLess(time.monotonic(),end,'Own marker not observed: '+path.name);time.sleep(.002)
    def script(self,id='script.key',key='cmd+a',target='/usr/bin/true',args=None):
        return dict(id=id,title='Own keyboard command',kind='script',target=target,arguments=args or [],aliases=['keyfixture'],hotkey=key,timeout=5)
    # Reuse actual native input helpers, never synthetic OS/global key injection.
    key=actions.ActionsProcesses.key
    view=actions.ActionsProcesses.view
