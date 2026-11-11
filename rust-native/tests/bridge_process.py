"""Visible shared Bridge/TOOLS/NativeBridge fixture helpers for remaining Python callers until #363.
The twelve original Bridge methods now run in bridge_process.rs.
"""
import json
import shutil
import unittest
import subprocess
import time
import os
from pathlib import Path
import runtime_process as rt

TOOLS=Path(os.environ.get('RUYCAST_TEST_NATIVE_HOST',rt.ROOT/'target/native-tools/debug/ruycast-native'))
rt.HOST=TOOLS

class Bridge(rt.Host):
    def start(self):
        self.process=subprocess.Popen([str(TOOLS),'--native-hidden'],env=self.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        end=time.monotonic()+8
        while time.monotonic()<end:
            if self.process.poll() is not None:
                out,err=self.process.communicate()
                raise AssertionError(out.decode()+err.decode())
            try:
                if self.rpc({'op':'status'},timeout=.3).get('ok'):return
            except (OSError,ValueError,rt.ClosedResponse):pass
            time.sleep(.02)
        raise AssertionError('Native bridge did not start')
    def settled(self,predicate=lambda value:True):
        return self.wait(lambda s: not s.get('configurationUpdating',False) and s.get('nativeUI',{}).get('revision',0)>0 and s['nativeUI']['revision']==s['nativeUI']['appliedRevision'] and predicate(s))
    def probe(self,action,**fields):
        value=self.rpc(dict(op='native.test',action=action,**fields))
        if not value.get('ok'):raise AssertionError(value)
        return value['result']

class NativeBridge(unittest.TestCase):
    def host(self):
        host=Bridge();self.addCleanup(host.close);host.settled();return host
    def capture(self,host,name):
        value=host.probe('screenshot')
        image=host.profile/'native-probe.png'
        self.assertTrue(image.read_bytes().startswith(b'\x89PNG\r\n\x1a\n'))
        if evidence:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            output=Path(evidence).parent/'bridge';output.mkdir(parents=True,exist_ok=True)
            shutil.copy2(image,output/(name+'.png'))
            (output/(name+'.json')).write_text(json.dumps(value,ensure_ascii=False,indent=2))
        return value
