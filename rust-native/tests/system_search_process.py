"""Owned real Unix socket + native hidden search/effect routes. No live apps."""
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import threading
import time
import unittest
import runtime_process as rt
import bridge_process as native
import actions_process as actions

TOOLS=rt.ROOT/'target/debug/ruycast-native'
rt.HOST=TOOLS
native.TOOLS=TOOLS
OUT=rt.ROOT/'target/system-search-evidence'

class SystemSearch(unittest.TestCase):
    key=actions.ActionsProcesses.key
    def host(self,gui=False,theme='light',width=760):
        h=(native.Bridge if gui else rt.Host)(start=False)
        self.addCleanup(h.close)
        h.env.update(RUYCAST_NATIVE_TEST_SYSTEM_SEARCH='owned',RUYCAST_NATIVE_TEST_MAIN_TABS='1',RUYCAST_NATIVE_TEST_WIDTH=str(width))
        config=copy.deepcopy(rt.DEFAULT)
        config['theme']=theme
        config['commands'].append(dict(id='fixture.alias',title='Local Exact Alias',kind='url',target='https://example.invalid/local',aliases=['owned']))
        h.replace(config)
        h.start()
        if gui:h.settled()
        return h
    def search(self,h,mode='commands',q=''):
        r=h.rpc(dict(op='search',mode=mode,query=q))
        self.assertTrue(r['ok'],r)
        return r
    def fixture(self,h,action='read',**fields):
        r=h.rpc(dict(op='system.fixture',action=action,**fields));self.assertTrue(r['ok'],r);return r['result']
    def effects(self,h):return self.fixture(h)['effects']
    def show(self,h,mode,query=''):
        self.assertTrue(h.rpc(dict(op='show',mode=mode,query=query))['ok'])
        h.settled(lambda s:s['nativeUI']['visible'] and not s['nativeUI']['systemSearchBusy'])
        return h.probe('read')
    def view(self,h,predicate):return actions.ActionsProcesses.view(self,h,predicate)
    def capture(self,h,name):
        h.probe('screenshot');OUT.mkdir(parents=True,exist_ok=True)
        shutil.copy2(h.profile/'native-probe.png',OUT/(name+'.png'))
        v=h.probe('read');(OUT/(name+'.json')).write_text(json.dumps(v,ensure_ascii=False,indent=2))
        return v
    def test_native_modes_return_and_hidden_renders(self):
        for theme in ['light','dark']:
            for width in [600,760]:
                h=self.host(True,theme,width)
                for scope,expected,source in [('tabs','Search Safari tabs…','https://example.invalid/owned'),('windows','Search windows…','Owned Editor'),('menus','Search app menu…','Owned Editor › File'),('shortcuts','Search Apple Shortcuts…','eeeeeeee')]:
                    with self.subTest(theme=theme,width=width,scope=scope):
                        v=self.show(h,scope);self.assertFalse(v['windowShown']);self.assertIn(expected,v['accessibilityText']);self.assertTrue(any(source in s for s in v['screenText']),v['screenText'])
                        if scope=='menus':self.assertTrue(any('Command + O' in s for s in v['accessibilityText']))
                        self.capture(h,f'{scope}-{theme}-{width}')
                h.close()
    def test_native_return_and_refresh_permission_recovery(self):
        h=self.host(True)
        for scope,expected in [('windows',['activate','AXRaise']),('menus',['activate','AXPress']),('tabs',['select-tab','front-window','activate']),('shortcuts',['shortcut:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee'])]:
            self.fixture(h,'clear-effects');self.show(h,scope)
            h.probe('key-event',code=36,chars='\r',flags=0)
            h.wait(lambda s:not s['nativeUI']['visible'] and not s['nativeUI']['systemSearchBusy'])
            self.assertEqual(self.effects(h),expected)
            self.assertFalse(h.probe('read')['windowShown'])
        self.fixture(h,'trust',value=False);v=self.show(h,'windows');self.assertTrue(any('Accessibility' in s for s in v['screenText']));self.capture(h,'windows-permission')
        self.fixture(h,'trust',value=True);actions.ActionsProcesses.action(self,h,'Refresh');h.settled(lambda s:not s['nativeUI']['systemSearchBusy']);self.assertEqual(h.probe('read')['rows'],2)
        self.show(h,'tabs');self.fixture(h,'clear-effects');actions.ActionsProcesses.action(self,h,'Allow Safari Access');h.settled(lambda s:not s['nativeUI']['systemSearchBusy']);self.assertEqual(self.effects(h),['consent'])
    def test_native_shortcut_alias_override_and_reopen(self):
        h=self.host(True);self.show(h,'shortcuts');actions.ActionsProcesses.action(self,h,'Set Alias')
        self.view(h,lambda v:v['formVisible']);h.probe('alias-input',text='savedshort');h.probe('save-editor');self.view(h,lambda v:not v['formVisible']);h.settled()
        saved=json.loads(h.config.read_text())['commands'][-1];self.assertEqual(saved['id'],'apple-shortcut.aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee');self.assertEqual(saved['aliases'],['savedshort'])
        self.assertEqual(self.search(h,'shortcuts','savedshort')['result'][0]['id'],saved['id'])
    def test_superseded_discovery_config_change_and_cli_disconnect_cannot_publish(self):
        h=self.host(True);self.fixture(h,'delay',milliseconds=150);self.assertTrue(h.rpc(dict(op='show',mode='windows'))['ok'])
        self.assertTrue(h.rpc(dict(op='show',mode='commands',query='missing-owned-term'))['ok']);self.fixture(h,'delay',milliseconds=0)
        h.settled(lambda s:not s['nativeUI']['systemSearchBusy']);self.assertEqual(h.probe('read')['rows'],0)
        self.fixture(h,'delay',milliseconds=200)
        stream=h.connect();stream.sendall(b'{"op":"search","mode":"windows"}\n');stream.close()
        config=json.loads(h.config.read_text());config['theme']='dark';h.replace(config);h.rpc(dict(op='reload'));self.fixture(h,'delay',milliseconds=0)
        self.assertTrue(h.rpc(dict(op='status'))['ok']);self.assertEqual(self.effects(h),[])

if __name__=='__main__':unittest.main()
