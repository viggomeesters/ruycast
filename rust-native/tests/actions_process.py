"""Real native Actions/binding controls against disposable Rust profiles."""
import copy
import fcntl
import json
import time
import unittest
import bridge_process as bridge
from runtime_process import DEFAULT

CMD=1<<20;OPT=1<<19;SHIFT=1<<17;CTRL=1<<18

class ActionsProcesses(unittest.TestCase):
    host=bridge.NativeBridge.host
    capture=bridge.NativeBridge.capture
    # Reuse fixture/capture helpers without inheriting the other suite's tests.
    def view(self,host,predicate=lambda v:True):
        end=time.monotonic()+4
        while time.monotonic()<end:
            value=host.probe('read')
            if predicate(value):return value
            time.sleep(.01)
        raise AssertionError('Native action/editor did not reach expected state: '+json.dumps(value))
    def show(self,host,query='finder'):
        self.assertTrue(host.rpc({'op':'show','query':query})['ok']);host.settled(lambda s:s['nativeUI']['visible']);self.view(host,lambda v:v['input']==query)
    def key(self,host,code,flags=0,chars='',repeated=False):return host.probe('key-event',code=code,flags=flags,chars=chars,repeat=repeated)
    def alias(self,host):self.key(host,43,CMD|OPT,',');return self.view(host,lambda v:v['formVisible'] and 'Alias' in v['screenText'])
    def hotkey(self,host):self.key(host,43,CMD|SHIFT,',');return self.view(host,lambda v:v['formVisible'] and v['recording'])
    def saved(self,host):return self.view(host,lambda v:not v['formVisible'])
    def command(self,host,id='app.finder'):return next(c for c in json.loads(host.config.read_bytes())['commands'] if c['id']==id)
    def action(self,host,name):
        if not host.probe('read')['actionsVisible']:self.key(host,40,CMD,'k')
        view=self.view(host,lambda v:v['actionsVisible']);index=next(i for i,label in enumerate(view['actionLabels']) if label.startswith(name));host.probe('action-button',index=index)
