"""Real private native choice/detail lifecycle with synthetic provider jobs."""
import json
import copy
import hashlib
import os
import shutil
import time
import unittest
from pathlib import Path
import execution_process as execution
import input_process as inputs
import bridge_process as bridge
import runtime_process as rt

class RichResults(unittest.TestCase):
    host=execution.Execution.host;command=execution.Execution.command
    file=execution.Execution.file;replace_command=execution.Execution.replace_command
    show=inputs.NativeInput.show;key=inputs.NativeInput.key;idle=inputs.NativeInput.idle
    type=inputs.NativeInput.type
    def provider(self,mode='close',**extra):
        code='''import json,sys,pathlib,time
value=sys.argv[1];root=pathlib.Path(sys.argv[2]);mode=sys.argv[3]
if not value:
    phase=root/'phase';n=int(phase.read_text())+1 if phase.exists() else 1;phase.write_text(str(n))
    print(json.dumps(dict(ok=True,choices=dict(title='Project Koppelen',placeholder='Zoek project…',items=[dict(id='reused',title='Café Project',subtitle='EX / PLAN',value='PRIVATE $(literal) Ω '+str(n)),dict(id='two',title='Another',subtitle='Elsewhere',value='SECOND')]),detail='PRIVATE report Ω\\n'+('Detail line\\n'*100))))
else:
    (root/'selected').write_text(value)
    if mode=='fail':print(json.dumps(dict(ok=False,error='Provider refused',detail='Failure detail')));sys.exit(7)
    if mode=='replace':print(json.dumps(dict(ok=True,choices=dict(title='Updated',placeholder='Find…',items=[dict(id='reused',title='Updated project',subtitle='New',value='REPLACED')]))))
    elif mode=='detail':print(json.dumps(dict(ok=True,detail='Final report')))
    elif mode=='slow':
        (root/'ready').write_text('ready')
        while not (root/'release').exists():time.sleep(.01)
        print(json.dumps(dict(ok=True,detail='Late report')))
    elif mode=='invalid':print('ordinary text')
    else:print(json.dumps(dict(ok=True)))
'''
        command=self.command(code,**extra);command['arguments']=['-c',code,'{input}','ROOT',mode];return command
    def fixture(self,mode='close',width=None,**extra):
        if width is None:host=self.host(native=True)
        else:
            host=bridge.Bridge(start=False);self.addCleanup(host.close);cfg=copy.deepcopy(rt.DEFAULT);host.replace(cfg);host.env['RUYCAST_NATIVE_TEST_WIDTH']=str(width);host.start();host.settled()
        command=self.provider(mode,**extra);command['arguments'][3]=str(host.base)
        self.replace_command(host,command);return host
    def view(self,host,predicate=lambda r:True,timeout=6):
        end=time.monotonic()+timeout
        while time.monotonic()<end:
            host.settled();v=host.probe('read');r=v.get('scriptResult')
            if r is not None and predicate(r):return v
            time.sleep(.02)
        raise AssertionError('Rich state missing: '+repr(v))
    def open(self,host):
        self.show(host);self.key(host,36,'\r');return self.view(host)
    def click(self,host,name):host.probe('result-control',name=name);host.settled()
    def capture(self,host,name):
        v=host.probe('screenshot');self.assertFalse(v['windowShown']);self.assertFalse(v['windowKey'])
        if out:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            p=Path(out).parent/'rich';p.mkdir(exist_ok=True);shutil.copy2(host.profile/'native-probe.png',p/(name+'.png'));(p/(name+'.json')).write_text(json.dumps(v,ensure_ascii=False,indent=2))
        return v
    def test_rejected_query_blocks_old_choice_and_can_be_repaired(self):
        host=self.fixture();self.open(host);host.probe('query',query=' '.join('term'+str(i) for i in range(65)));r=self.view(host,lambda r:not r['query_valid'])['scriptResult'];self.assertIsNone(r['selection'])
        self.key(host,36,'\r');host.settled();self.assertFalse((host.base/'selected').exists());host.probe('query',query='cafe');self.view(host,lambda r:r['query_valid']);self.click(host,'choose');self.idle(host);self.assertTrue((host.base/'selected').exists())
    def test_failure_report_back_retains_ordinary_input_draft(self):
        payload=dict(ok=False,error='Provider failed',detail='Failure report')
        host=self.host(native=True,commands=[self.command('print(%r)'%json.dumps(payload),inputPrompt='Message')]);token=inputs.NativeInput().open(host)['editorToken'];self.type(host,'PRIVATE input');host.probe('save-editor');self.idle(host)
        v=self.view(host,lambda r:r['detail_visible']);self.assertEqual(v['editorToken'],token);self.capture(host,'input-failure-report');self.click(host,'close');host.settled();v=host.probe('read');self.assertEqual(v['editorToken'],token);self.assertIn('PRIVATE input',v['screenText']);self.capture(host,'input-restored');self.assertTrue(host.probe('run-input-read')['focused']);self.assertTrue(host.probe('run-input-read')['canUndo']);self.type(host,' continued');self.key(host,6,'z',1<<20);self.assertEqual(host.probe('run-input-read')['text'],'PRIVATE input')
    def test_native_light_dark_empty_and_long_labels_renders(self):
        for theme in ['light','dark']:
            host=self.fixture(width=600);cfg=json.loads(host.config.read_bytes());cfg['theme']=theme;host.replace(cfg);host.rpc({'op':'reload'});self.open(host);self.capture(host,'choices-'+theme)
            host.probe('query',query='no result');self.view(host,lambda r:not r['rows']);v=self.capture(host,'empty-'+theme);self.assertIn('No results',v['screenText']);host.close()
    def test_retained_old_detail_text_cannot_close_replacement(self):
        host=self.fixture();self.open(host);self.click(host,'details');self.view(host,lambda r:r['detail_visible']);host.probe('retain-result-text');self.click(host,'choices');old=self.view(host)['scriptResult'];self.click(host,'reload');new=self.view(host,lambda r:r['token']!=old['token'])['scriptResult'];self.click(host,'details');self.view(host,lambda r:r['detail_visible']);host.probe('stale-result-text');host.settled();self.assertEqual(self.view(host)['scriptResult']['token'],new['token'])
    def test_close_during_busy_owner_rejects_its_old_published_view(self):
        host=self.fixture('slow');self.open(host);self.click(host,'choose');self.file(host.base/'ready');self.view(host,lambda r:r['busy']);host.probe('schedule-result-close')
        delay=host.rpc(dict(op='native.delay',executionBarrier=True));self.assertTrue(delay['result']['newIntentDuringDelay']);proof=json.loads(self.file(host.profile/'result-close-barrier.json'));self.assertTrue(proof['rejected']);self.assertFalse(proof['windowShown']);self.idle(host);host.settled();self.assertIsNone(host.probe('read')['scriptResult'])
    def test_post_effect_usage_warning_keeps_valid_reloaded_choices(self):
        host=self.fixture();old=self.open(host)['scriptResult'];usage=host.profile/'usage.json';usage.unlink();usage.mkdir();self.click(host,'reload');r=self.view(host,lambda r:r['token']!=old['token'])['scriptResult'];self.assertTrue(r['rows']);self.assertIn('usage could not be saved',r['error']);self.assertFalse(r['busy'])
    def test_config_change_after_admission_keeps_concrete_usage_warning(self):
        host=self.fixture('slow');self.open(host);self.click(host,'choose');self.file(host.base/'ready');self.replace_command(host,self.command('print("changed")'));(host.base/'release').write_text('go');r=self.view(host,lambda r:r['detail_visible'])['scriptResult'];self.assertIn('configuration',r['error']);self.assertEqual(r['detail'],'Late report')

    def test_native_ime_blocks_return_escape_details_and_mouse_until_commit(self):
        host=self.fixture();old=self.open(host)['scriptResult']
        for action in ['return','escape','details','mouse']:
            self.assertTrue(host.probe('rich-query-mark',text='CA')['marked'])
            if action=='return':self.key(host,36,'\r')
            elif action=='escape':self.key(host,53)
            elif action=='details':host.probe('result-control',name='details')
            else:host.probe('click',row=0)
            host.settled();self.assertFalse((host.base/'selected').exists());r=self.view(host)['scriptResult'];self.assertEqual(r['token'],old['token']);self.assertFalse(r['detail_visible'])
            host.probe('rich-query-unmark');host.probe('query',query='')
        host.probe('query',query='CAFE');self.view(host,lambda r:r['query']=='CAFE');host.probe('click',row=0);self.idle(host);self.assertEqual((host.base/'selected').read_text(),'PRIVATE $(literal) Ω 1')
    def native_event_boundary(self,host):
        before=host.probe('rich-query-read')['groupingLevel'];host.probe('rich-query-native-event');end=time.monotonic()+3
        while time.monotonic()<end:
            after=host.probe('rich-query-read')['groupingLevel']
            if after==0:return dict(before=before,after=after)
            time.sleep(.01)
        self.fail('Own AppKit event did not close its automatic Undo group')
    def test_native_query_undo_survives_detail_and_back(self):
        host=self.fixture();self.open(host);host.probe('rich-query-type',text='cafe');self.view(host,lambda r:r['query']=='cafe');first=self.native_event_boundary(host);self.assertTrue(host.probe('rich-query-read')['canUndo']);self.key(host,2,'d',1<<20);details=self.view(host,lambda r:r['detail_visible']);self.key(host,53);self.view(host,lambda r:not r['detail_visible']);self.assertTrue(host.probe('rich-query-read')['canUndo']);host.probe('rich-query-type',text=' PLAN');self.view(host,lambda r:r['query']=='cafe PLAN');second=self.native_event_boundary(host);self.key(host,6,'z',1<<20);self.view(host,lambda r:r['query']=='cafe');self.key(host,6,'z',1<<20);self.view(host,lambda r:r['query']=='');self.key(host,6,'z',(1<<20)|(1<<17));self.view(host,lambda r:r['query']=='cafe');self.assertFalse((host.base/'selected').exists())
        if out:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            folder=Path(out).parent/'rich';folder.mkdir(exist_ok=True);(folder/'query-undo-event-boundaries.json').write_text(json.dumps(dict(first=first,second=second,roleGroupBefore=details['resultPanel']['queryGroupBeforeRole'],roleGroupAfter=details['resultPanel']['queryGroupAfterRole'],undo1='cafe',undo2='',redo='cafe',binarySHA256=hashlib.sha256(bridge.TOOLS.read_bytes()).hexdigest()),indent=2))

    def test_failure_can_add_first_readable_detail_without_replacing_choices(self):
        initial=dict(ok=True,choices=dict(title='Projects',placeholder='Find…',items=[dict(id='one',title='Project',subtitle='EX',value='literal')]))
        code='import sys;print(%r if not sys.argv[1] else %r)'%(json.dumps(initial),json.dumps(dict(ok=False,error='Provider failed',detail='New failure report')))
        command=self.command(code);command['arguments']+=['{input}'];host=self.host(native=True,commands=[command]);old=self.open(host)['scriptResult'];self.assertIsNone(old['detail']);self.assertNotIn('Details ⌘D',host.probe('read')['resultPanel']['buttons']);self.click(host,'choose');r=self.view(host,lambda r:r['detail'] is not None)['scriptResult'];self.assertEqual(r['token'],old['token']);self.assertIn('Details ⌘D',host.probe('read')['resultPanel']['buttons']);self.click(host,'details');self.assertEqual(self.view(host,lambda r:r['detail_visible'])['resultPanel']['detail'],'New failure report')

    def test_retired_query_undo_redo_and_typing_cannot_modify_new_session(self):
        host=self.fixture();old=self.open(host)['scriptResult'];host.probe('rich-query-type',text='cafe');self.view(host,lambda r:r['query']=='cafe');self.assertTrue(host.probe('rich-query-read')['canUndo']);host.probe('retain-result-query');self.click(host,'reload');new=self.view(host,lambda r:r['token']!=old['token'])['scriptResult'];proof=host.probe('stale-result-query');self.assertTrue(proof['cleared'] and proof['detached']);self.assertFalse(proof['editable'] or proof['canUndoAfter']);host.settled();r=self.view(host)['scriptResult'];self.assertEqual(r['token'],new['token']);self.assertEqual(r['query'],'');self.assertFalse((host.base/'selected').exists())

    def test_empty_choices_and_blank_provider_labels_keep_functional_accessibility(self):
        command=self.command('print(\'{"ok":true,"choices":{"title":"","placeholder":"","items":[]}}\')')
        host=self.host(native=True,commands=[command]);v=self.open(host);r=v['scriptResult'];self.assertEqual(r['title'],'Execution Fixture');self.assertIn('Search choices',v['accessibilityText']);self.assertEqual(r['rows'],[]);self.assertIsNone(r['selection']);self.assertIn('No results',v['screenText']);old=r['token'];self.key(host,36,'\r');host.settled();self.assertEqual(self.view(host)['scriptResult']['token'],old);self.click(host,'reload');self.assertNotEqual(self.view(host,lambda r:r['token']!=old)['scriptResult']['token'],old);self.capture(host,'blank-empty-labels')

    def test_query_queued_before_close_cannot_put_an_error_on_restored_search(self):
        host=self.fixture();self.open(host);host.probe('schedule-result-close',queryBeforeClose=True);delay=host.rpc(dict(op='native.delay',executionBarrier=True));self.assertTrue(delay['result']['newIntentDuringDelay']);proof=json.loads(self.file(host.profile/'result-close-barrier.json'));self.assertTrue(proof['queryBeforeClose'] and proof['queryEpochAdvanced']);host.settled();v=host.probe('read');self.assertIsNone(v['scriptResult']);self.assertFalse(any('no longer open' in t for t in v['screenText']));self.assertFalse((host.base/'selected').exists())

    def test_actual_process_failure_without_envelope_keeps_real_error_and_choices(self):
        initial=dict(ok=True,choices=dict(title='Projects',placeholder='Find…',items=[dict(id='one',title='Project',subtitle='EX',value='literal')]))
        code='import sys;print(%r if not sys.argv[1] else "ordinary failure text");raise SystemExit(0 if not sys.argv[1] else 7)'%json.dumps(initial)
        command=self.command(code);command['arguments']+=['{input}'];host=self.host(native=True,commands=[command]);old=self.open(host)['scriptResult'];self.click(host,'choose');v=self.view(host,lambda r:bool(r['error']));self.assertEqual(v['scriptResult']['token'],old['token']);self.assertIn('status 7',v['scriptResult']['error']);self.assertFalse(v['scriptResult']['busy']);self.assertTrue(any(e.get('exitStatus')==7 and not e['success'] for e in host.rpc({'op':'logs'})['result'] if e['phase']=='command.finish'))

    def test_retired_result_actions_are_effectless_after_close_and_replacement(self):
        for replacement in [False,True]:
            with self.subTest(replacement=replacement):
                host=self.fixture();old=self.open(host)['scriptResult']
                if replacement:
                    self.click(host,'reload');current=self.view(host,lambda r:r['token']!=old['token'])['scriptResult']
                else:
                    self.click(host,'close');host.settled();self.assertIsNone(host.probe('read')['scriptResult']);current=None
                epoch=host.rpc({'op':'status'})['result']['nativeUI']['queryEpoch']
                for name in ['details','select','choose','activate','reload','close']:
                    host.probe('result-event',token=old['token'],epoch=epoch,name=name,id='reused');host.settled();v=host.probe('read')
                    self.assertFalse(any('no longer open' in text for text in v['screenText']))
                    if replacement:self.assertEqual(v['scriptResult'],current)
                    else:self.assertIsNone(v['scriptResult'])
                    self.assertFalse((host.base/'selected').exists())

    def test_retired_result_ingress_cannot_suppress_new_initial_job(self):
        host=self.fixture();old=self.open(host)['scriptResult']
        root=str(host.base);code='import pathlib,sys,time,json;p=pathlib.Path(sys.argv[1]);(p/"new-ready").write_text("ready");\nwhile not (p/"new-release").exists():time.sleep(.01)\n(p/"new-effect").write_text("done");print(json.dumps(dict(ok=True,detail="NEW JOB REPORT")))'
        command=self.command(code);command['arguments'].append(root);self.replace_command(host,command)
        self.show(host);self.key(host,36,'\r');self.file(host.base/'new-ready')
        epoch=host.rpc({'op':'status'})['result']['nativeUI']['queryEpoch']
        for name in ['details','close','choose']:
            host.probe('result-event',token=old['token'],epoch=epoch,name=name,id='reused')
        host.probe('result-query-event',token=old['token'],query='retired query')
        self.assertEqual(host.rpc({'op':'status'})['result']['nativeUI']['queryEpoch'],epoch)
        (host.base/'new-release').write_text('go');self.file(host.base/'new-effect');self.idle(host)
        v=self.view(host,lambda r:r['detail']=='NEW JOB REPORT');self.assertTrue(v['scriptResult']['detail_visible']);self.assertEqual(host.rpc({'op':'usage'})['result']['script.fixture'],2)

if __name__=='__main__':unittest.main()
