"""Real hidden native script input: ephemeral drafts, live focus and safe execution."""
import copy
import json
import time
import unittest
import execution_process as execution
import bridge_process as bridge

CMD=1<<20

class NativeInput(unittest.TestCase):
    command=execution.Execution.command
    host=execution.Execution.host
    file=execution.Execution.file
    dead=execution.Execution.dead
    replace_command=execution.Execution.replace_command
    capture=bridge.NativeBridge.capture

    def show(self,host,query='runfixture'):
        self.assertTrue(host.rpc(dict(op='show',query=query))['ok'])
        host.settled(lambda s:s['nativeUI']['visible'])
        view=host.probe('read');self.assertFalse(view['windowShown'],json.dumps(view))

    def view(self, host, predicate=lambda v:True):
        end=time.monotonic()+4
        while time.monotonic()<end:
            host.settled()
            value=host.probe('read')
            if predicate(value):return value
            time.sleep(.01)
        self.fail('Native input state did not settle: '+str(value))

    def open(self, host):
        self.show(host)
        self.key(host,36,'\r')
        self.view(host,lambda v:v['runInputInline'])
        self.key(host,36,'\r',CMD)
        return self.view(host,lambda v:v['formVisible'] and 'Run ⌘↵' in v['screenText'])

    def key(self,host,code,chars='',flags=0):
        return host.probe('key-event',code=code,chars=chars,flags=flags)

    def type(self,host,value):host.probe('run-input-type',text=value)

    def idle(self,host):host.wait(lambda s:s['scriptsRunning']==0)

    def waiting_script(self,host):
        ready=host.base/'ready';gate=host.base/'gate';done=host.base/'done'
        code='import pathlib,time;pathlib.Path(%r).write_text("started");p=pathlib.Path(%r)\nwhile not p.exists():time.sleep(.01)\npathlib.Path(%r).write_text("completed")'%(str(ready),str(gate),str(done))
        return self.command(code,inputPrompt='Message'),ready,gate,done

if __name__=='__main__':unittest.main()
