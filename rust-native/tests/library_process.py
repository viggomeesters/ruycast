"""Separate native Settings and palette, one private worker-owned configuration."""
import copy
import json
import os
from pathlib import Path
import shutil
import unittest
import bridge_process as bridge
import actions_process as helpers
from runtime_process import DEFAULT

class SettingsHost(bridge.Bridge):
    def probe(self, action, **fields):
        value=self.rpc(dict(op='native.test',surface='settings',action=action,**fields))
        if not value.get('ok'):raise AssertionError(value)
        return value['result']
    def settled(self, predicate=lambda value:True):
        return self.wait(lambda s:not s.get('configurationUpdating',False) and s.get('managementUI',{}).get('revision',0)>0 and s['managementUI']['revision']==s['managementUI']['appliedRevision'] and predicate(s))

class Palette:
    def __init__(self, host):self.host=host
    def __getattr__(self, key):return getattr(self.host,key)
    def probe(self, action, **fields):return bridge.Bridge.probe(self.host,action,**fields)
    def settled(self,predicate=lambda value:True):return bridge.Bridge.settled(self.host,predicate)

class Library(unittest.TestCase):
    view=helpers.ActionsProcesses.view
    key=helpers.ActionsProcesses.key
    saved=helpers.ActionsProcesses.saved
    action=helpers.ActionsProcesses.action
    alias=helpers.ActionsProcesses.alias
    command=helpers.ActionsProcesses.command
    def host(self):
        host=SettingsHost();self.addCleanup(host.close);host.settled();return host
    def show(self,host,query='finder'):
        self.assertTrue(host.rpc(dict(op='show',mode='settings',query=query))['ok']);host.settled(lambda s:s['managementUI']['visible']);self.view(host,lambda v:v['input']==query)
    def fixture(self,host,commands):
        config=copy.deepcopy(DEFAULT);config['commands']=commands;host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok']);host.settled()
    def capture(self,host,name):
        value=host.probe('screenshot');image=host.profile/'native-settings-probe.png';self.assertTrue(image.read_bytes().startswith(b'\x89PNG\r\n\x1a\n'))
        if evidence:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            output=Path(evidence).parent/'library';output.mkdir(parents=True,exist_ok=True);shutil.copy2(image,output/(name+'.png'));(output/(name+'.json')).write_text(json.dumps(value,ensure_ascii=False,indent=2))
        return value
    def rows(self,host):return host.probe('library-read')['rows']
    def nav(self,host,action,**fields):host.probe(action,**fields);host.settled();return host.probe('library-read')
