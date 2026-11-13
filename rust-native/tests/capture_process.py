"""Trusted raw capture metadata across the real core/helper/CLI boundary."""
import json
import subprocess
import unittest
import time
import supervision_process as supervision
import script_result_process as results
import input_process as inputs

def valid_prefix_code():
    native=dict(ok=True,detail='',capture=dict(stdout=dict(truncated=False,eof=True,validUtf8=True)))
    length=65536-len(json.dumps(native,separators=(',',':')).encode())
    return 'import os,json;v=%r;v["detail"]="x"*%d;b=json.dumps(v,separators=(",",":")).encode();assert len(b)==65536;os.write(1,b+b"not JSON")'%(native,length)

def pipe_holder_code(base,held):
    ready=base/('ready-'+held);gate=base/('release-'+held);done=base/('done-'+held)
    child='import os,pathlib,time;pathlib.Path(%r).write_text("ready");p=pathlib.Path(%r);end=time.monotonic()+4\nwhile not p.exists() and time.monotonic()<end:time.sleep(.01)\nos.close(1);os.close(2);pathlib.Path(%r).write_text("closed")'%(str(ready),str(gate),str(done))
    code='import pathlib,subprocess,sys,time;subprocess.Popen([sys.executable,"-c",%r],start_new_session=True,stdout=%s,stderr=%s);p=pathlib.Path(%r);end=time.monotonic()+2\nwhile not p.exists() and time.monotonic()<end:time.sleep(.01)\nassert p.exists();print(\'{"ok":true,"detail":"prefix while pipe is open","capture":{"stdout":{"eof":true}}}\')'%(child,'None' if held in ['both','stdout'] else 'subprocess.DEVNULL','None' if held in ['both','stderr'] else 'subprocess.DEVNULL',str(ready))
    return code,ready,gate,done

def release_holder(ready,gate,done):
    gate.write_text('release')
    if ready.exists():
        end=time.monotonic()+2
        while not done.exists() and time.monotonic()<end:time.sleep(.01)
        assert done.exists(),'Own detached fixture did not self-close'

class CaptureProcesses(unittest.TestCase):
    setUp=supervision.Supervision.setUp;command=supervision.Supervision.command
    probe=supervision.Supervision.probe;fake=supervision.Supervision.fake
    def test_ready_detached_pipe_holders_do_not_create_false_eof(self):
        for held in ['stdout','stderr','both']:
            code,ready,gate,done=pipe_holder_code(self.base,held);self.addCleanup(release_holder,ready,gate,done)
            result=self.probe(self.command(code));self.assertTrue(ready.exists());self.assertFalse(done.exists())
            for stream in ['stdout','stderr']:
                self.assertEqual(result['capture'][stream],dict(truncated=False,validUtf8=True,eof=held not in ['both',stream]))
            self.assertTrue(json.loads(result['response']['result']['stdout'])['capture']['stdout']['eof'])
            release_holder(ready,gate,done)

class NativeCapture(unittest.TestCase):
    host=results.ScriptResults.host;command=results.ScriptResults.command;file=results.ScriptResults.file
    show=inputs.NativeInput.show;view=inputs.NativeInput.view;key=inputs.NativeInput.key;type=inputs.NativeInput.type;idle=inputs.NativeInput.idle;capture=inputs.NativeInput.capture
    def open(self,host):return inputs.NativeInput.open(self,host)
    def test_incomplete_or_altered_success_warns_and_does_not_offer_completed_input_again(self):
        cases=['import os;os.write(1,b"a"*65537)','import os;os.write(1,b\'{"ok":true,"detail":"\\xff"}\')']
        cases.append(valid_prefix_code())
        for index,code in enumerate(cases):
            host=self.host(native=True,commands=[self.command(code,inputPrompt='Message')]);self.open(host);self.type(host,'PRIVATE input');host.probe('save-editor');self.idle(host)
            self.view(host,lambda v:not v['formVisible'] and any('incomplete or invalid' in t for t in v['screenText']));self.assertEqual(host.rpc({'op':'usage'})['result'],{});self.assertNotIn('PRIVATE',(host.profile/'command-log.json').read_text());self.capture(host,'capture-success-warning-'+str(index))
    def test_native_requires_stdout_eof_but_not_stderr_eof(self):
        for held in ['stdout','stderr','both']:
            host=self.host(native=True);code,ready,gate,done=pipe_holder_code(host.base,held);self.addCleanup(release_holder,ready,gate,done)
            command=self.command(code,inputPrompt='Message');results.ScriptResults.replace_command(self,host,command);self.open(host);self.type(host,'input');host.probe('save-editor');self.idle(host)
            if held=='stderr':
                view=self.view(host,lambda v:not v['formVisible'] and v.get('scriptResult') is not None);self.assertTrue(view['resultPanel']['detailVisible']);self.assertFalse(view['resultPanel']['editable']);self.assertEqual(view['resultPanel']['detail'],'prefix while pipe is open');self.assertEqual(host.rpc({'op':'usage'})['result']['script.fixture'],1)
            else:
                self.view(host,lambda v:not v['formVisible'] and any('incomplete or invalid' in t for t in v['screenText']));self.assertEqual(host.rpc({'op':'usage'})['result'],{})
            self.assertTrue(ready.exists());self.assertFalse(done.exists());release_holder(ready,gate,done)
    def test_incomplete_process_failure_retains_input_and_actual_process_error(self):
        code='import os;os.write(1,b\'{"ok":true,"detail":"\\xff","actionCompleted":true}\');raise SystemExit(7)'
        host=self.host(native=True,commands=[self.command(code,inputPrompt='Message')]);token=self.open(host)['editorToken'];self.type(host,'PRIVATE keep');host.probe('save-editor');self.idle(host)
        self.view(host,lambda v:v['formVisible'] and any('status 7' in t for t in v['screenText']));self.assertEqual(host.probe('read')['editorToken'],token);self.assertEqual(host.probe('run-input-read')['text'],'PRIVATE keep');self.type(host,' continued');self.assertEqual(host.probe('run-input-read')['text'],'PRIVATE keep continued');self.capture(host,'capture-failure-keeps-input')
    def test_stderr_loss_does_not_reject_otherwise_complete_stdout(self):
        code='import os;os.write(1,b\'{"ok":true,"detail":"valid"}\');os.write(2,bytes([255])*65537)'
        host=self.host(native=True,commands=[self.command(code,inputPrompt='Message')]);self.open(host);self.type(host,'input');host.probe('save-editor');self.idle(host)
        view=self.view(host,lambda v:not v['formVisible'] and v.get('scriptResult') is not None);self.assertTrue(view['resultPanel']['detailVisible']);self.assertFalse(view['resultPanel']['editable']);self.assertEqual(view['resultPanel']['detail'],'valid');self.assertFalse(any('incomplete or invalid' in t for t in view['screenText']));self.assertEqual(host.rpc({'op':'usage'})['result']['script.fixture'],1)
    def test_cli_valid_prefix_and_spoofed_capture_remain_provider_data(self):
        host=self.host(commands=[self.command(valid_prefix_code())]);response=host.rpc(dict(op='execute',id='runfixture'))
        self.assertTrue(response['ok']);self.assertGreater(len(response['detail']),65000);self.assertEqual(response['capture'],dict(stdout=dict(truncated=False,eof=True,validUtf8=True)))
    def test_cli_keeps_provider_capture_fields_and_legacy_lossy_behavior(self):
        for code in ['print(\'{"ok":true,"detail":"valid","capture":{"truncated":true}}\')','import os;os.write(1,b\'{"ok":true,"detail":"\\xff","capture":{"validUtf8":true}}\')']:
            host=self.host(commands=[self.command(code)]);result=host.rpc(dict(op='execute',id='runfixture'));self.assertTrue(result['ok']);self.assertIn('capture',result);self.assertNotIn('stdout',result['capture']);self.assertNotIn('stderr',result['capture'])

if __name__=='__main__':unittest.main()
