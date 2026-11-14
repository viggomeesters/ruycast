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
    def test_failure_report_back_retains_ordinary_input_draft(self):
        payload=dict(ok=False,error='Provider failed',detail='Failure report')
        host=self.host(native=True,commands=[self.command('print(%r)'%json.dumps(payload),inputPrompt='Message')]);token=inputs.NativeInput().open(host)['editorToken'];self.type(host,'PRIVATE input');host.probe('save-editor');self.idle(host)
        v=self.view(host,lambda r:r['detail_visible']);self.assertEqual(v['editorToken'],token);self.capture(host,'input-failure-report');self.click(host,'close');host.settled();v=host.probe('read');self.assertEqual(v['editorToken'],token);self.assertIn('PRIVATE input',v['screenText']);self.capture(host,'input-restored');self.assertTrue(host.probe('run-input-read')['focused']);self.assertTrue(host.probe('run-input-read')['canUndo']);self.type(host,' continued');self.key(host,6,'z',1<<20);self.assertEqual(host.probe('run-input-read')['text'],'PRIVATE input')
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

    def test_empty_choices_and_blank_provider_labels_keep_functional_accessibility(self):
        command=self.command('print(\'{"ok":true,"choices":{"title":"","placeholder":"","items":[]}}\')')
        host=self.host(native=True,commands=[command]);v=self.open(host);r=v['scriptResult'];self.assertEqual(r['title'],'Execution Fixture');self.assertIn('Search choices',v['accessibilityText']);self.assertEqual(r['rows'],[]);self.assertIsNone(r['selection']);self.assertIn('No results',v['screenText']);old=r['token'];self.key(host,36,'\r');host.settled();self.assertEqual(self.view(host)['scriptResult']['token'],old);self.click(host,'reload');self.assertNotEqual(self.view(host,lambda r:r['token']!=old)['scriptResult']['token'],old);self.capture(host,'blank-empty-labels')

    def test_actual_process_failure_without_envelope_keeps_real_error_and_choices(self):
        initial=dict(ok=True,choices=dict(title='Projects',placeholder='Find…',items=[dict(id='one',title='Project',subtitle='EX',value='literal')]))
        code='import sys;print(%r if not sys.argv[1] else "ordinary failure text");raise SystemExit(0 if not sys.argv[1] else 7)'%json.dumps(initial)
        command=self.command(code);command['arguments']+=['{input}'];host=self.host(native=True,commands=[command]);old=self.open(host)['scriptResult'];self.click(host,'choose');v=self.view(host,lambda r:bool(r['error']));self.assertEqual(v['scriptResult']['token'],old['token']);self.assertIn('status 7',v['scriptResult']['error']);self.assertFalse(v['scriptResult']['busy']);self.assertTrue(any(e.get('exitStatus')==7 and not e['success'] for e in host.rpc({'op':'logs'})['result'] if e['phase']=='command.finish'))

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
