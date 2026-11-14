"""Actual Rust owner/master generation + keyword service, entirely own SDK."""
import copy,json,os,subprocess,time,unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import native_keyboard_process as keyboard
import keyword_capture_process as capture
import bridge_process as bridge
import actions_process as actions

class NativeKeyword(unittest.TestCase):
    @classmethod
    def setUpClass(cls):keyboard.NativeKeyboard.setUpClass.__func__(cls)
    @classmethod
    def tearDownClass(cls):keyboard.NativeKeyboard.tearDownClass.__func__(cls)
    key=actions.ActionsProcesses.key
    host=keyboard.NativeKeyboard.host;wait_file=keyboard.NativeKeyboard.wait_file;view=keyboard.NativeKeyboard.view
    def kw(self,host,control='state',**values):return host.probe('keyboard-keyword',control=control,**values)
    def type(self,host,text):return [self.kw(host,'event',text=c)['consumed'] for c in text]
    def configured(self,**values):
        host=self.host(start=False);c=capture.config();c.update(values);host.replace(c);host.start();host.wait(lambda s:s['keywordGeneration']==s['keyboardGeneration'] and s['snippetExpansion']=='On');return host,c
    def wait_generation(self,host,old):return host.wait(lambda s:s['keyboardGeneration']>old and s['keywordGeneration']==s['keyboardGeneration'] and not s['configurationUpdating'])
    def configured_rich(self,text='A{cursor}BC',mode='html'):
        host=self.host(start=False);c=capture.config(text,keyword='x',snippetOutput=mode);host.replace(c);host.start();host.wait(lambda s:s['keywordGeneration']==s['keyboardGeneration'] and s['snippetExpansion']=='On');return host,c
    def retire(self,host):
        self.assertTrue(host.rpc({'op':'quit'})['ok']);out,err=host.process.communicate(timeout=5);self.assertEqual(host.process.returncode,0,out.decode()+err.decode());return json.loads((host.base/'keyword-retired.json').read_text())
    def reload(self,host,c):
        before=host.rpc({'op':'status'})['result']['keyboardGeneration'];host.replace(c);reply=host.rpc({'op':'reload'});self.assertTrue(reply['ok'] or 'being updated' in reply.get('error',''),reply);return self.wait_generation(host,before)
    def reset(self,host):self.kw(host,'event',text='');self.kw(host,'clear')
if __name__=='__main__':unittest.main(verbosity=2)
