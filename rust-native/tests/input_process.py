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

    def test_validation_literal_multiline_input_and_no_secret_logs(self):
        host=self.host(native=True);out=host.base/'result'
        code='import sys,pathlib;pathlib.Path(%r).write_text(sys.argv[1])'%str(out)
        command=self.command(code,inputPrompt='Message');command['arguments']+=['{input}'];self.replace_command(host,command)
        before=host.config.read_bytes();self.open(host)
        self.key(host,36,'\r',CMD)
        self.view(host,lambda v:any('Enter Message' in s for s in v['screenText']))
        state=host.probe('run-input-read');self.assertTrue(state['editable'] and state['focused']);self.assertFalse(out.exists())
        value='literal $(echo private) Ω\nsecond {uuid} https://private.invalid/token'
        self.type(host,value);self.assertEqual(host.probe('run-input-read')['text'],value)
        self.key(host,36,'\r',CMD);self.file(out);self.idle(host);self.view(host,lambda v:not v['formVisible'])
        self.assertEqual(out.read_text(),value);self.assertEqual(host.config.read_bytes(),before)
        self.assertEqual(host.rpc({'op':'usage'})['result']['script.fixture'],1)
        raw=(host.profile/'command-log.json').read_text();self.assertNotIn('private.invalid',raw);self.assertNotIn('literal $',raw)

    def test_failure_preserves_same_native_text_focus_typing_and_undo(self):
        host=self.host(native=True,commands=[self.command('import sys;print("failure",file=sys.stderr);raise SystemExit(7)',inputPrompt='Message')])
        token=self.open(host)['editorToken'];self.type(host,'Keep me')
        host.probe('save-editor');self.idle(host)
        self.view(host,lambda v:any('7' in s for s in v['screenText']))
        state=host.probe('run-input-read');self.assertTrue(state['editable'] and state['focused'] and state['canUndo']);self.assertEqual(state['text'],'Keep me')
        self.assertEqual(host.probe('read')['editorToken'],token)
        self.type(host,' and this');self.assertEqual(host.probe('run-input-read')['text'],'Keep me and this')
        self.key(host,6,'z',CMD);self.assertNotEqual(host.probe('run-input-read')['text'],'Keep me and this')
        self.assertEqual(host.rpc({'op':'usage'})['result'],{})

    def test_launch_failure_keeps_input_and_native_typing(self):
        command=self.command(inputPrompt='Message');command.update(target='/nonexistent-ruycast-fixture',arguments=['{input}'])
        host=self.host(native=True,commands=[command]);self.open(host);self.type(host,'launch draft');host.probe('save-editor')
        self.view(host,lambda v:any('such file' in s.lower() for s in v['screenText']));self.idle(host)
        self.assertEqual(host.probe('run-input-read')['text'],'launch draft');self.type(host,' retained')
        self.assertEqual(host.rpc({'op':'usage'})['result'],{})

    def test_timeout_keeps_input_and_native_typing(self):
        command=self.command('import time;time.sleep(1)',inputPrompt='Message');command['timeout']=.1
        host=self.host(native=True,commands=[command]);self.open(host);self.type(host,'timeout draft');host.probe('save-editor')
        self.view(host,lambda v:any('timed out' in s.lower() for s in v['screenText']));self.idle(host)
        self.assertEqual(host.probe('run-input-read')['text'],'timeout draft');self.type(host,' retained')
        self.assertEqual(host.rpc({'op':'usage'})['result'],{})

    def waiting_script(self,host):
        ready=host.base/'ready';gate=host.base/'gate';done=host.base/'done'
        code='import pathlib,time;pathlib.Path(%r).write_text("started");p=pathlib.Path(%r)\nwhile not p.exists():time.sleep(.01)\npathlib.Path(%r).write_text("completed")'%(str(ready),str(gate),str(done))
        return self.command(code,inputPrompt='Message'),ready,gate,done

    def test_unrelated_cli_success_and_configuration_refresh_cannot_strand_completion(self):
        for mode in ['usage','config']:
            with self.subTest(mode=mode):
                host=self.host(native=True);command,ready,gate,done=self.waiting_script(host);self.replace_command(host,command)
                config=json.loads(host.config.read_bytes());config['commands'].append(dict(id='script.other',title='Other fixture',kind='script',target='/usr/bin/true',arguments=[]));host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok'])
                self.open(host);self.type(host,'own fixture input');host.probe('save-editor');self.file(ready)
                if mode=='usage':
                    self.assertTrue(host.rpc(dict(op='execute',id='script.other'))['ok']);self.assertEqual(host.rpc({'op':'usage'})['result']['script.other'],1)
                else:
                    config['commands'][-1]['title']='Changed unrelated title';host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok'])
                gate.write_text('go');self.file(done);self.idle(host);self.view(host,lambda v:not v['formVisible'])
                self.assertEqual(host.rpc({'op':'usage'})['result']['script.fixture'],1);host.close()

    def test_newer_query_preserves_palette_but_finishes_the_owned_input(self):
        host=self.host(native=True);command,ready,gate,done=self.waiting_script(host);self.replace_command(host,command)
        self.open(host);self.type(host,'own fixture input');host.probe('save-editor');self.file(ready)
        host.probe('run-input-new-query',query='finder');host.settled()
        gate.write_text('go');self.file(done);self.idle(host)
        view=self.view(host,lambda v:not v['formVisible']);self.assertEqual(view['input'],'finder');self.assertTrue(host.rpc({'op':'status'})['result']['nativeUI']['visible'])
        self.assertEqual(host.rpc({'op':'usage'})['result']['script.fixture'],1)

    def test_completed_detail_replaces_input_without_reoffering_it(self):
        command=self.command('print(\'{"ok":true,"detail":"own fixture detail"}\')',inputPrompt='Message')
        host=self.host(native=True,commands=[command]);self.open(host);self.type(host,'own fixture input');host.probe('save-editor');self.idle(host)
        view=self.view(host,lambda v:not v['formVisible'] and v.get('scriptResult') is not None);self.assertIsNone(view['editorToken']);self.assertTrue(view['resultPanel']['detailVisible']);self.assertFalse(view['resultPanel']['editable']);self.assertEqual(view['resultPanel']['detail'],'own fixture detail');self.assertEqual(host.rpc({'op':'usage'})['result']['script.fixture'],1);host.probe('result-control',name='close');host.settled();view=host.probe('read');self.assertIsNone(view['editorToken']);self.assertFalse(view['formVisible']);self.assertIsNone(view['scriptResult'])

    def test_changed_command_after_completed_effect_closes_input_with_honest_warning(self):
        host=self.host(native=True);command,ready,gate,done=self.waiting_script(host);self.replace_command(host,command)
        self.open(host);self.type(host,'own fixture input');host.probe('save-editor');self.file(ready)
        changed=self.command(inputPrompt='Message');changed.update(target='/usr/bin/true',arguments=[]);self.replace_command(host,changed)
        gate.write_text('go');self.file(done);self.idle(host)
        self.view(host,lambda v:not v['formVisible'] and any('Action completed' in s for s in v['screenText']))
        self.assertEqual(host.rpc({'op':'usage'})['result'],{})

    def test_usage_write_failure_does_not_offer_completed_input_again(self):
        host=self.host(native=True);counter=host.base/'counter'
        code='import pathlib;pathlib.Path(%r).write_text("once")'%str(counter)
        self.replace_command(host,self.command(code,inputPrompt='Message'));self.open(host);self.type(host,'own fixture input')
        (host.profile/'usage.json').mkdir();host.probe('save-editor');self.file(counter);self.idle(host)
        self.view(host,lambda v:not v['formVisible'] and any('Action completed' in s for s in v['screenText']))
        host.probe('save-editor');self.assertEqual(counter.read_text(),'once');self.assertEqual(len(host.rpc({'op':'logs'})['result']),2)

    def test_cancel_running_input_does_not_close_a_new_draft(self):
        host=self.host(native=True);pid=host.base/'leader'
        code='import os,time,pathlib;pathlib.Path(%r).write_text(str(os.getpid()));time.sleep(2)'%str(pid)
        command=self.command(code,inputPrompt='Message');self.replace_command(host,command)
        old=self.open(host)['editorToken'];self.type(host,'cancel me');host.probe('save-editor');self.file(pid)
        host.probe('cancel-editor');self.view(host,lambda v:not v['formVisible']);self.idle(host);self.dead(pid.read_text())
        new=self.open(host)['editorToken'];self.assertNotEqual(old,new)
        self.type(host,'new draft');time.sleep(.1);self.assertEqual(host.probe('run-input-read')['text'],'new draft')
        self.assertEqual(host.rpc({'op':'usage'})['result'],{})

    def test_expired_submission_retains_text_and_releases_controls(self):
        host=self.host(native=True,commands=[self.command(inputPrompt='Message')]);self.open(host);self.type(host,'expired draft')
        host.probe('schedule-input-submit',expire=True)
        stream=host.connect(timeout=4);self.addCleanup(stream.close);stream.sendall(b'{"op":"native.delay"}\n')
        self.file(host.profile/'input-submit-fired')
        with stream.makefile('rb') as reader:self.assertTrue(json.loads(reader.readline())['result']['newIntentDuringDelay'])
        self.view(host,lambda v:any('expired' in s for s in v['screenText']))
        state=host.probe('run-input-read');self.assertFalse(state['pending']);self.assertTrue(state['editable'] and state['focused']);self.assertEqual(state['text'],'expired draft')
        self.type(host,' still editable');self.assertEqual(host.rpc({'op':'logs'})['result'],[])

    def test_cancel_queued_behind_busy_owner_never_starts(self):
        host=self.host(native=True,commands=[self.command(inputPrompt='Message')]);self.open(host);self.type(host,'cancel before admission')
        host.probe('schedule-input-submit',expire=False)
        stream=host.connect(timeout=4);self.addCleanup(stream.close);stream.sendall(b'{"op":"native.delay"}\n')
        self.file(host.profile/'input-submit-fired')
        with stream.makefile('rb') as reader:self.assertTrue(json.loads(reader.readline())['result']['newIntentDuringDelay'])
        self.view(host,lambda v:not v['formVisible']);self.assertEqual(host.rpc({'op':'logs'})['result'],[]);self.assertEqual(host.rpc({'op':'usage'})['result'],{})

    def test_light_dark_input_and_failure_capture(self):
        host=self.host(native=True,commands=[self.command(inputPrompt='Message')])
        for theme in ['light','dark']:
            config=json.loads(host.config.read_bytes());config['theme']=theme;host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok'])
            self.open(host);host.probe('save-editor');self.view(host,lambda v:any('Enter Message' in x for x in v['screenText']));self.capture(host,'input-validation-'+theme)
            host.probe('cancel-editor');self.view(host,lambda v:not v['formVisible']);self.open(host)
            self.type(host,'Unicode büro 日本語\nSecond line');self.capture(host,'input-'+theme)
            host.probe('cancel-editor');self.view(host,lambda v:not v['formVisible'])

if __name__=='__main__':unittest.main()
