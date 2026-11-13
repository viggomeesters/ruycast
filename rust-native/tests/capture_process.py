"""Trusted raw capture metadata across the real core/helper/CLI boundary."""
import json
import subprocess
import unittest
import time
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

class NativeCapture(unittest.TestCase):
    host=results.ScriptResults.host;command=results.ScriptResults.command;file=results.ScriptResults.file
    show=inputs.NativeInput.show;view=inputs.NativeInput.view;key=inputs.NativeInput.key;type=inputs.NativeInput.type;idle=inputs.NativeInput.idle;capture=inputs.NativeInput.capture
    def open(self,host):return inputs.NativeInput.open(self,host)
    def test_native_requires_stdout_eof_but_not_stderr_eof(self):
        for held in ['stdout','stderr','both']:
            host=self.host(native=True);code,ready,gate,done=pipe_holder_code(host.base,held);self.addCleanup(release_holder,ready,gate,done)
            command=self.command(code,inputPrompt='Message');results.ScriptResults.replace_command(self,host,command);self.open(host);self.type(host,'input');host.probe('save-editor');self.idle(host)
            if held=='stderr':
                view=self.view(host,lambda v:not v['formVisible'] and v.get('scriptResult') is not None);self.assertTrue(view['resultPanel']['detailVisible']);self.assertFalse(view['resultPanel']['editable']);self.assertEqual(view['resultPanel']['detail'],'prefix while pipe is open');self.assertEqual(host.rpc({'op':'usage'})['result']['script.fixture'],1)
            else:
                self.view(host,lambda v:not v['formVisible'] and any('incomplete or invalid' in t for t in v['screenText']));self.assertEqual(host.rpc({'op':'usage'})['result'],{})
            self.assertTrue(ready.exists());self.assertFalse(done.exists());release_holder(ready,gate,done)

if __name__=='__main__':unittest.main()
