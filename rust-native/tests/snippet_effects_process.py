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


def command(target='A{cursor}😀end', **extra):
    value=dict(id='snippet.fixture',title='Snippet Fixture',kind='snippet',target=target,aliases=['snipfixture']);value.update(extra);return value


class SnippetEffects(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        run=subprocess.run(['cc','-Wall','-Wextra','-Werror','-fobjc-arc','-fblocks','-dynamiclib',
                            '-framework','AppKit','-framework','ApplicationServices',
                            str(ROOT/'tests/snippet_effects_shim.m'),'-o',str(SHIM)],capture_output=True)
        if run.returncode:raise AssertionError(run.stderr.decode())
        if evidence:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            out=Path(evidence);out.mkdir(parents=True,exist_ok=True)
            sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
            sources=[ROOT/'tests/snippet_effects_shim.m',ROOT/'tests/workspace_shim.m']
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

    def test_real_palette_run_has_complete_copy_hide_activate_paste_and_cursor_sequence(self):
        h=self.host(command(snippetOutput='html',timeout=.1));before=h.config.read_bytes();self.show(h);self.run_native(h)
        self.wait_event(h,'snippet-post',lambda r:r['key']==123 and not r['down']);self.done(h);h.settled()
        self.assertEqual(h.cli('usage')['result']['snippet.fixture'],1)
        self.assertEqual(h.config.read_bytes(),before)
        self.assertEqual(self.writes(h)[0]['text'],'A😀end')
        self.assertIn('<div>A😀end</div>',self.writes(h)[0]['html'])
        posts=self.posts(h);self.assert_keys(posts,[9,9]+[123]*8)
        activate=next(r for r in self.trace(h) if r['phase']=='snippet-activate')
        self.assertGreaterEqual(posts[0]['atNs']-activate['atNs'],149_000_000)
        self.assertGreaterEqual(posts[2]['atNs']-posts[1]['atNs'],79_000_000)
        self.assertFalse(h.probe('read')['windowShown'])
        self.assertFalse(h.probe('read')['formVisible'])
        state=json.loads((h.base/'snippet-status.json').read_text())
        self.assertFalse(state['active']);self.assertFalse(state['timer']);self.assertEqual(state['pending'],0)
        self.assertEqual(state['ownChangeCount'],self.writes(h)[0]['changeCount'])
        notice=self.notice(h,'snippet-done');self.assertTrue(notice['success']);self.assertEqual(notice['text'],'Done');self.assertEqual(notice['height'],65)

    def test_preparation_failures_keep_the_input_draft_and_never_clear(self):
        for fault in ['fail-string','fail-html']:
            h=self.host(command('{message}',snippetOutput='html'),flags=[fault]);self.show(h);self.run_native(h)
            h.settled();self.assertTrue(h.probe('read')['runInputInline'])
            h.probe('run-inline-type',text='Own draft');self.run_native(h)
            self.wait_event(h,'snippet-item-failed');self.done(h);h.settled()
            view=h.probe('read');self.assertTrue(view['runInputInline'])
            self.assertIn('Could not copy text','\n'.join(view['screenText']))
            self.assertEqual(self.writes(h),[]);self.assertEqual(self.posts(h),[])
            self.assertFalse(any(r['phase']=='snippet-clear' for r in self.trace(h)))
            (h.base/fault).unlink();self.run_native(h);self.wait_event(h,'snippet-post');self.done(h)
            self.assertEqual(self.writes(h)[0]['text'],'Own draft')

    def test_postcommit_failures_retire_input_and_use_passive_notice(self):
        for fault,needle in [('fail-write','clipboard may have changed'),('activation-noop','target app did not activate'),('fail-event','input events')]:
            h=self.host(command('{message}'),flags=[fault]);self.show(h);self.run_native(h)
            h.settled();h.probe('run-inline-type',text='Owned text');self.run_native(h)
            self.wait_event(h,'snippet-clear');self.done(h);h.settled()
            notice=self.notice(h,'snippet-'+fault);self.assertFalse(notice['success']);self.assertIn(needle,notice['text'])
            self.assertFalse(h.probe('read')['runInputInline']);self.assertFalse(h.probe('read')['formVisible'])
            self.assertEqual(self.posts(h),[])
            if fault!='fail-write':self.assertFalse(h.rpc(dict(op='status'))['result']['nativeUI']['visible'])
            self.assertNotIn('snippet.fixture',h.cli('usage')['result'])

    def test_external_clipboard_target_relaunch_and_termination_cancel_delayed_paste(self):
        for change in [1,2,3,'terminated']:
            h=self.host(command('Owned'));self.show(h);self.run_native(h);self.wait_event(h,'snippet-activate')
            if change=='terminated':(h.base/'target-terminated').write_text('Own terminated target')
            else:h.probe('snippet-fixture-control',operation=change)
            self.done(h);h.settled();self.assertEqual(self.posts(h),[])
            self.assertFalse(self.notice(h,'snippet-change-'+str(change))['success'])
            self.assertFalse(h.rpc(dict(op='status'))['result']['nativeUI']['visible'])

    def test_new_show_and_configuration_replacement_retire_delayed_effect_without_old_notice(self):
        for change in ['show','replace']:
            h=self.host(command('Owned'));self.show(h);self.run_native(h);self.wait_event(h,'snippet-activate')
            if change=='show':self.show(h,'safari')
            else:
                cfg=json.loads(h.config.read_text());cfg['commands'][-1]['target']='Replacement';h.replace(cfg);h.cli('reload')
            self.done(h);h.settled();time.sleep(.2)
            self.assertEqual(self.posts(h),[])
            if change=='show':
                self.assertTrue(h.rpc(dict(op='status'))['result']['nativeUI']['visible'])
                self.assertEqual(h.probe('read')['input'],'safari')
                self.assertFalse(h.probe('notice-read')['present'])

    def test_cli_uses_current_app_without_show_and_refreshes_after_external_switch(self):
        h=self.host(command('CLI'))
        self.assertEqual(h.cli('run','snipfixture'),dict(ok=True,result=dict(state='pasted')))
        self.assertEqual([r['pid'] for r in self.posts(h)],[4260,4260])
        h.probe('snippet-fixture-control',operation=2)
        self.assertTrue(h.cli('run','snipfixture')['ok'])
        self.assertEqual([r['pid'] for r in self.posts(h)],[4260,4260,4261,4261])

    def test_three_concurrent_cli_requests_keep_payload_until_own_paste_completes(self):
        h=self.host(command('{x}'));results=[]
        threads=[threading.Thread(target=lambda value=value:results.append(h.rpc(dict(op='execute',id='snipfixture',values=dict(x=value))))) for value in ['First','Second','Third']]
        for thread in threads:thread.start()
        for thread in threads:thread.join(5);self.assertFalse(thread.is_alive())
        self.assertEqual(results,[dict(ok=True,result=dict(state='pasted'))]*3)
        events=[r for r in self.trace(h) if r['phase'] in ['snippet-write','snippet-post']]
        self.assertEqual([r['phase'] for r in events],['snippet-write','snippet-post','snippet-post']*3)
        self.assertEqual({r['text'] for r in self.writes(h)},{'First','Second','Third'})
        self.assertEqual(h.rpc(dict(op='status'))['result']['snippetTransactionsPending'],0)

    def test_real_multiple_field_choices_and_marked_text_use_same_snippet_pipeline(self):
        h=self.host(command('{a name="Message"}/{b name="Place" options="One,Two"}'))
        self.show(h);self.run_native(h);h.settled()
        self.assertEqual(h.probe('template-read')['values'],['',''])
        h.probe('template-mark',index=0,text='日本語');h.probe('save-editor');h.settled();self.assertEqual(self.writes(h),[])
        h.probe('template-unmark');h.probe('template-type',index=0,text='日本語');h.probe('template-choose',index=1,selected=1)
        self.capture(h,'snippet-fields');h.probe('save-editor');self.wait_event(h,'snippet-post');self.done(h);h.settled()
        self.assertEqual(self.writes(h)[0]['text'],'日本語/Two');self.assertFalse(h.probe('read')['formVisible'])

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

    def test_disconnect_and_quit_cancel_owned_timers_and_never_post_late(self):
        for action in ['disconnect','quit']:
            h=self.host(command('Owned'));stream=h.connect();stream.sendall(b'{"op":"execute","id":"snipfixture"}\n')
            self.wait_event(h,'snippet-activate')
            if action=='disconnect':
                stream.close();self.done(h);time.sleep(.2);self.assertEqual(self.posts(h),[])
                self.assertEqual(h.rpc(dict(op='status'))['result']['snippetTransactionsPending'],0)
            else:
                self.assertTrue(h.rpc(dict(op='quit'))['ok']);out,err=h.process.communicate(timeout=3)
                stream.close();self.assertEqual(h.process.returncode,0,out.decode()+err.decode());self.assertEqual(self.posts(h),[])

    def test_clipboard_demand_and_validation_error_priority_match_existing_pipeline(self):
        h=self.host(command('{missing} {clipboard}'),headless=True,flags=['deny-ax'],clipboard='Own input')
        result=h.rpc(dict(op='execute',id='snipfixture'))
        self.assertFalse(result['ok']);self.assertIn('missing',result['error'])
        self.assertTrue((h.base/'clipboard-read').exists());self.assertEqual(self.writes(h),[])
        self.assertFalse(any(r['phase']=='snippet-trust' for r in self.trace(h)))

    def test_management_run_preserves_management_and_hides_only_palette_peer(self):
        h=self.host(command('Settings'));self.show(h,'safari');h.cli('show','settings')
        h.wait(lambda s:s['managementUI']['revision']==s['managementUI']['appliedRevision'] and s['managementUI']['visible'])
        def probe(action,**fields):return h.probe(action,surface='settings',**fields)
        probe('query',query='snipfixture');h.wait(lambda s:s['managementUI']['revision']==s['managementUI']['appliedRevision'])
        probe('key-event',code=40,chars='k',flags=1<<20)
        h.wait(lambda s:s['managementUI']['revision']==s['managementUI']['appliedRevision'])
        labels=probe('read')['actionLabels'];probe('action-button',index=labels.index('Insert Snippet'))
        self.wait_event(h,'snippet-post');self.done(h);h.settled()
        status=h.rpc(dict(op='status'))['result'];self.assertTrue(status['managementUI']['visible']);self.assertFalse(status['nativeUI']['visible'])
        self.assertEqual(self.writes(h)[0]['text'],'Settings');self.assertFalse(probe('read')['windowShown'])

    def test_notice_expires_without_focus_and_replacement_keeps_single_timer(self):
        h=self.host(command('Owned'));self.show(h);self.run_native(h);self.wait_event(h,'snippet-post');self.done(h);h.settled()
        first=self.notice(h,'snippet-notice-first')
        self.show(h);self.run_native(h);self.wait_event(h,'snippet-post',lambda r:r['atNs']>self.posts(h)[1]['atNs'])
        self.done(h);h.settled();second=h.probe('notice-read')
        self.assertEqual(second['serial'],first['serial']+1);self.assertTrue(second['timer'])
        end=time.monotonic()+6
        while h.probe('notice-read')['timer']:
            self.assertLess(time.monotonic(),end);time.sleep(.05)
        self.assertFalse(h.probe('read')['windowShown']);self.assertFalse(h.probe('notice-read')['windowShown'])

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

    def test_unchanged_cli_on_gui_host_receives_original_success_shape_after_event_phase(self):
        h=self.host(command('PRIVATE [Guide](https://example.com)',snippetOutput='teams'))
        self.show(h)
        result=h.cli('run','snipfixture')
        self.assertEqual(result,dict(ok=True,result=dict(state='pasted')))
        self.assert_keys(self.posts(h),[9,9])
        self.assertEqual(self.writes(h)[0]['text'],'PRIVATE Guide')
        self.assertIn('<p style="margin:0">PRIVATE <a href=',self.writes(h)[0]['html'])
        raw=(h.profile/'command-log.json').read_text()
        self.assertNotIn('PRIVATE',raw);self.assertNotIn('https://example.com',raw)
        self.assertEqual(h.cli('usage')['result']['snippet.fixture'],1)

    def test_ax_headless_missing_target_and_incomplete_fixture_never_mutate_board(self):
        for headless,flags,complete,error in [(True,(),True,'Choose an app'),
                                              (True,('deny-ax',),True,'Allow Ruycast'),
                                              (False,('missing-target',),True,'Choose an app'),
                                              (False,(),False,'Complete snippet fixture')]:
            h=self.host(command('Plain'),headless=headless,flags=flags,complete=complete)
            if not headless:self.show(h)
            value=h.rpc(dict(op='execute',id='snipfixture'))
            self.assertFalse(value['ok']);self.assertIn(error,value['error'])
            self.assertEqual(self.writes(h),[]);self.assertEqual(self.posts(h),[])
            self.assertFalse(any(r['phase']=='snippet-clear' for r in self.trace(h)))


if __name__=='__main__':unittest.main(verbosity=2)
