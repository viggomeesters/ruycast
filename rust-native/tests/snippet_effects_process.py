#!/usr/bin/env python3
"""Actual GUI/CLI owner -> worker -> main clipboard/timers -> complete SDK
replacement. All boards/targets/events are owned synthetic fixtures.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import threading
import time
import unittest
import bridge_process as native
import runtime_process as rt

ROOT=rt.ROOT
SHIM=ROOT/'target/snippet-effects-shim.dylib'
if host:=os.environ.get('RUYCAST_TEST_APP_HOST'):
    native.TOOLS=Path(host);rt.HOST=Path(host)
if cli:=os.environ.get('RUYCAST_TEST_CLI'):
    rt.CLI=Path(cli)


def command(target='A{cursor}😀end', **extra):
    value=dict(id='snippet.fixture',title='Snippet Fixture',kind='snippet',target=target,aliases=['snipfixture']);value.update(extra);return value


class SnippetEffects(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        target=Path(os.environ.get('CARGO_TARGET_DIR',ROOT.parent/'.build/full-rust-release/native'))
        artifact=Path(os.environ.get('RUYCAST_TEST_SNIPPET_EFFECTS_SDK',target/'release/examples/libsnippet_effects_fixture.dylib'))
        if not artifact.is_absolute() or not artifact.is_file():
            raise RuntimeError('Build --features test-tools --example snippet-effects-fixture first')
        SHIM.parent.mkdir(parents=True,exist_ok=True)
        shutil.copyfile(artifact,SHIM)
        if evidence:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            out=Path(evidence);out.mkdir(parents=True,exist_ok=True)
            sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
            sources=[ROOT/'tests/fixtures'/name for name in ['workspace_sdk.rs','snippet_effects_sdk.rs','snippet_effects_fixture.rs']]
            sources.extend([ROOT/'Cargo.toml',ROOT/'Cargo.lock',ROOT.parent/'rust/Cargo.toml',ROOT.parent/'rust-toolchain.toml'])
            (out/'snippet-effects-bindings.json').write_text(json.dumps(dict(
                sources={str(p.relative_to(ROOT.parent)):sha(p) for p in sources},
                artifacts={str(SHIM.relative_to(ROOT.parent)):sha(SHIM)}),indent=2))

    def host(self,cmd=None,*,headless=False,flags=(),complete=True,clipboard=None,dispatch=None):
        h=(rt.Host if headless else native.Bridge)(start=False)
        self.addCleanup(h.close)
        cfg=copy.deepcopy(rt.DEFAULT);cfg['commands'].append(cmd or command());h.config.write_text(json.dumps(cfg))
        h.env.update(DYLD_INSERT_LIBRARIES=str(SHIM),RUYCAST_WORKSPACE_FIXTURE_HOME=str(h.base),
                     RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD='1' if headless else 'trace')
        if not headless:h.env['RUYCAST_NATIVE_TEST_REQUIRE_HIDE']='1'
        if complete:h.env['RUYCAST_NATIVE_TEST_SNIPPET']='1'
        if dispatch is not None:h.env['RUYCAST_NATIVE_TEST_DISPATCH_MS']=str(dispatch)
        if clipboard is not None:h.env.update(RUYCAST_NATIVE_TEST_CLIPBOARD='named',RUYCAST_NATIVE_TEST_CLIPBOARD_TEXT=clipboard)
        for flag in flags:(h.base/flag).write_text('own fault')
        self.addCleanup(lambda:(h.base/'snippet-release').write_text('Own release') if h.base.exists() else None)
        h.start()
        h.env.pop('DYLD_INSERT_LIBRARIES',None)
        h.env.pop('RUYCAST_WORKSPACE_FIXTURE_HOME',None)
        if not headless:h.settled()
        return h

    def trace(self,h):
        p=h.base/'workspace-trace.jsonl'
        if not p.exists():return []
        text=p.read_text();lines=text.splitlines()
        if not text.endswith('\n'):lines=lines[:-1]
        return [json.loads(line) for line in lines]

    def wait_event(self,h,phase,predicate=lambda event:True):
        end=time.monotonic()+8
        while time.monotonic()<end:
            events=[r for r in self.trace(h) if r['phase']==phase and predicate(r)]
            if events:return events[-1]
            self.assertIsNone(h.process.poll(),'Own snippet host exited')
            time.sleep(.005)
        self.fail('Own snippet phase absent: '+phase)

    def show(self,h,query='snipfixture'):
        self.assertTrue(h.rpc(dict(op='show',query=query))['ok'])
        h.settled(lambda s:s['nativeUI']['visible'])

    def run_native(self,h):h.probe('key-event',code=36,chars='\r',flags=0)
    def done(self,h):
        return h.wait(lambda s:s['snippetRequestsRunning']==0 and s['openRequestsRunning']==0)
    def posts(self,h):return [r for r in self.trace(h) if r['phase']=='snippet-post']
    def writes(self,h):return [r for r in self.trace(h) if r['phase']=='snippet-write']

    def wait_file(self,h,name):
        end=time.monotonic()+5
        while not (h.base/name).exists():
            self.assertLess(time.monotonic(),end,'Own barrier absent: '+name);time.sleep(.005)

    def capture(self,h,name):
        value=h.probe('screenshot')
        if directory:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            out=Path(directory).parent/'native-snippets';out.mkdir(parents=True,exist_ok=True)
            shutil.copy2(h.profile/'native-probe.png',out/(name+'.png'))
            (out/(name+'.json')).write_text(json.dumps(value,ensure_ascii=False,indent=2))
        return value

    def notice(self,h,name):
        value=h.probe('notice-screenshot')
        if directory:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            out=Path(directory).parent/'native-snippets';out.mkdir(parents=True,exist_ok=True)
            shutil.copy2(h.profile/'native-notice-probe.png',out/(name+'.png'))
            (out/(name+'.json')).write_text(json.dumps(value,ensure_ascii=False,indent=2))
        self.assertTrue(value['present']);self.assertFalse(value['windowShown'])
        self.assertFalse(value['canBecomeKey']);self.assertTrue(value['nonactivating']);self.assertTrue(value['ignoresMouse'])
        return value

    def assert_keys(self,posts,keys):
        self.assertEqual([r['key'] for r in posts],keys)
        self.assertTrue(all(r['mainThread'] and r['pid']==4260 and r['tag']==0x52554859504552 for r in posts))
        self.assertEqual([r['down'] for r in posts],[i%2==0 for i in range(len(posts))])
        self.assertEqual([r['flags'] for r in posts],[1<<20 if key==9 else 0 for key in keys])

    def test_blocked_main_sdk_retains_ticket_capacity_after_worker_expiry(self):
        for phase,committed in [('prepare',False),('write',True)]:
            h=self.host(command('Owned'),flags=['hold-'+phase],dispatch=350);results=[]
            worker=threading.Thread(target=lambda:results.append(h.rpc(dict(op='execute',id='snipfixture'))));worker.start()
            self.wait_file(h,phase+'-entered');self.assertTrue(h.rpc(dict(op='status'))['ok'])
            worker.join(2);self.assertFalse(worker.is_alive());self.assertFalse(results[0]['ok'])
            self.assertEqual(results[0].get('actionCompleted',False),committed)
            self.assertEqual(h.wait(lambda s:s['snippetRequestsRunning']==0)['snippetTransactionsPending'],1)
            (h.base/'snippet-release').write_text('Own release')
            h.wait(lambda s:s['snippetTransactionsPending']==0);self.assertEqual(self.posts(h),[])
            if not committed:self.assertEqual(self.writes(h),[])
            self.assertNotIn('snippet.fixture',h.cli('usage')['result'])

    def test_transport_expiry_preserves_snippet_commit_truth_when_owner_is_busy(self):
        for phase,committed in [('prepare',False),('write',True)]:
            h=self.host(command('Owned'),flags=['hold-'+phase],dispatch=350);results=[];busy=[]
            worker=threading.Thread(target=lambda:results.append(h.rpc(dict(op='execute',id='snipfixture'))));worker.start()
            self.wait_file(h,phase+'-entered')
            owner=threading.Thread(target=lambda:busy.append(h.rpc(dict(op='workspace.fixture.delay'))));owner.start()
            self.wait_file(h,'owner-open-delay-started');worker.join(2);owner.join(2)
            self.assertFalse(worker.is_alive());self.assertFalse(owner.is_alive())
            self.assertTrue(results[0]['transportExpired']);self.assertFalse(results[0]['ok'])
            self.assertEqual(results[0].get('actionCompleted',False),committed)
            if committed:self.assertEqual(results[0]['code'],'snippet-outcome-unknown')
            (h.base/'snippet-release').write_text('Own release');h.wait(lambda s:s['snippetTransactionsPending']==0)
            self.assertEqual(self.posts(h),[])

    def test_target_terminated_or_relaunched_after_preflight_never_clears_clipboard(self):
        for flag in ['target-terminated','target-new-generation']:
            h=self.host(command('Owned'),flags=['hold-prepare']);results=[]
            worker=threading.Thread(target=lambda:results.append(h.rpc(dict(op='execute',id='snipfixture'))));worker.start()
            self.wait_file(h,'prepare-entered');(h.base/flag).write_text('Own changed process')
            (h.base/'snippet-release').write_text('Own release');worker.join(3);self.assertFalse(worker.is_alive())
            self.assertFalse(results[0]['ok']);self.assertNotIn('actionCompleted',results[0]);self.assertIn('Choose an app',results[0]['error'])
            self.assertEqual(self.writes(h),[]);self.assertEqual(self.posts(h),[])
            self.assertFalse(any(r['phase']=='snippet-clear' for r in self.trace(h)))

    def test_cursor_cap_is_exact_and_target_change_after_paste_prevents_cursor_events(self):
        h=self.host(command('{cursor}'+'x'*10001))
        result=h.rpc(dict(op='execute',id='snipfixture'),timeout=15)
        self.assertTrue(result['ok']);posts=self.posts(h)
        self.assertEqual(len(posts),20002);self.assert_keys(posts,[9,9]+[123]*20000)
        self.assertEqual(len(self.writes(h)[0]['text']),10001)
        h=self.host(command('A{cursor}😀end'));self.show(h);self.run_native(h)
        self.wait_event(h,'snippet-post',lambda r:r['key']==9 and not r['down'])
        h.probe('snippet-fixture-control',operation=2);self.done(h);h.settled()
        self.assertEqual([r['key'] for r in self.posts(h)],[9,9]);self.assertFalse(self.notice(h,'snippet-cursor-target-changed')['success'])



if __name__=='__main__':unittest.main(verbosity=2)
