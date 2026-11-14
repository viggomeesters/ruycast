"""Real native field-editor input: stable text, alias capture and menu transitions."""
import json
import time
import unittest
import input_process as inputs
CMD=inputs.CMD

class InlineInput(unittest.TestCase):
    host=inputs.NativeInput.host;command=inputs.NativeInput.command;show=inputs.NativeInput.show;file=inputs.NativeInput.file;view=inputs.NativeInput.view;key=inputs.NativeInput.key;replace_command=inputs.NativeInput.replace_command;idle=inputs.NativeInput.idle;capture=inputs.NativeInput.capture

    def open(self,host,tab=False):
        self.show(host);self.key(host,48 if tab else 36,'\t' if tab else '\r')
        return self.view(host,lambda v:v['runInputInline'])
    def type(self,host,text):host.probe('run-inline-type',text=text)
    def text(self,host):return host.probe('run-inline-read')['text']
    def expanded(self,host):return self.view(host,lambda v:v['formVisible'] and not v['runInputInline'])
    def closed(self,host):return self.view(host,lambda v:not v['runInputInline'] and not v['formVisible'])

    def test_same_token_reload_usage_and_theme_keep_text_editor_undo_and_composition(self):
        host=self.host(native=True,commands=[self.command(inputPrompt='Message')]);self.open(host);self.type(host,'before')
        initial=host.probe('run-inline-read');self.assertTrue(initial['canUndo'])
        config=json.loads(host.config.read_bytes());config['theme']='light';config['commands'].append(dict(id='script.other',title='Other',kind='script',target='/usr/bin/true',arguments=[]));host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok'])
        self.assertTrue(host.rpc(dict(op='execute',id='script.other'))['ok']);host.settled()
        after=host.probe('run-inline-read');self.assertEqual(after['text'],'before');self.assertEqual(after['editorAddress'],initial['editorAddress']);self.assertTrue(after['canUndo']);self.assertEqual(after['token'],initial['token'])
        host.probe('run-inline-mark',text='にほん');marked=host.probe('run-inline-read');self.assertTrue(marked['marked'])
        config['theme']='dark';host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok']);host.settled()
        after=host.probe('run-inline-read');self.assertTrue(after['marked']);self.assertEqual(after['text'],marked['text']);self.assertEqual(after['editorAddress'],marked['editorAddress']);self.assertEqual(after['selectedLocation'],marked['selectedLocation'])
        host.probe('run-inline-ime-guards');self.assertEqual(len(host.rpc({'op':'logs'})['result']),2)
        host.probe('run-inline-unmark');self.type(host,' continued');self.assertTrue(self.text(host).endswith(' continued'))

    def test_input_menu_escape_keeps_text_and_keyboard_expand_uses_owner_selection(self):
        host=self.host(native=True,commands=[self.command(inputPrompt='Message')]);self.open(host);self.type(host,'menu value')
        self.key(host,40,'k',CMD);view=self.view(host,lambda v:v['actionsVisible']);self.assertEqual(view['actionLabels'],['Run','Expand Input…','Cancel Input','Edit…','Set Alias…','Set Hotkey…','Disable','Remove…','Settings'])
        self.key(host,53);self.view(host,lambda v:not v['actionsVisible']);self.assertEqual(self.text(host),'menu value');self.type(host,' retained')
        self.key(host,40,'k',CMD);self.view(host,lambda v:v['actionsVisible']);self.key(host,125,'');self.key(host,36,'\r');self.expanded(host)
        self.assertEqual(host.probe('run-input-read')['text'],'menu value retained');self.assertEqual(host.rpc({'op':'logs'})['result'],[])

    def test_input_menu_clicked_expand_and_cancel(self):
        host=self.host(native=True,commands=[self.command(inputPrompt='Message')]);self.open(host);self.type(host,'clicked')
        self.key(host,40,'k',CMD);self.view(host,lambda v:v['actionsVisible']);host.probe('action-button',index=1);self.expanded(host);self.assertEqual(host.probe('run-input-read')['text'],'clicked')
        host.probe('cancel-editor');self.closed(host);self.open(host);self.type(host,'discard')
        self.key(host,40,'k',CMD);self.view(host,lambda v:v['actionsVisible']);host.probe('action-button',index=2);self.closed(host)
        self.assertEqual(host.rpc({'op':'logs'})['result'],[])

    def delayed_control(self,host,**options):
        host.probe('schedule-inline-control',**options)
        stream=host.connect(timeout=4);self.addCleanup(stream.close);stream.sendall(b'{"op":"native.delay"}\n')
        self.file(host.profile/'inline-control-fired')
        with stream.makefile('rb') as reader:self.assertTrue(json.loads(reader.readline())['result']['newIntentDuringDelay'])
        host.settled()

    def test_pending_menu_cancel_keeps_input_and_typing_while_owner_is_busy(self):
        host=self.host(native=True,commands=[self.command(inputPrompt='Message')]);self.open(host);self.type(host,'menu pending')
        token=host.probe('run-inline-read')['token'];self.delayed_control(host,operation='menu',cancel=True)
        self.assertTrue(host.probe('read')['runInputInline']);self.assertFalse(host.probe('read')['actionsVisible'])
        self.assertEqual(self.text(host),'menu pending continued');self.assertEqual(host.probe('run-inline-read')['token'],token);self.type(host,' now')
        self.assertEqual(host.rpc({'op':'logs'})['result'],[])

    def test_expired_expand_releases_input_without_losing_text(self):
        host=self.host(native=True,commands=[self.command(inputPrompt='Message')]);self.open(host);self.type(host,'expired expand')
        self.delayed_control(host,operation='expand',expire=True)
        self.view(host,lambda v:any('expired' in x for x in v['screenText']));self.assertTrue(host.probe('read')['runInputInline']);self.assertFalse(host.probe('read')['formVisible'])
        self.assertFalse(host.probe('run-inline-read')['pending']);self.assertEqual(self.text(host),'expired expand');self.type(host,' retained')
        self.assertEqual(host.rpc({'op':'logs'})['result'],[])

    def test_empty_menu_run_restores_direct_typing_and_selection(self):
        host=self.host(native=True,commands=[self.command(inputPrompt='Message')]);self.open(host)
        self.key(host,40,'k',CMD);self.view(host,lambda v:v['actionsVisible']);host.probe('action-button',index=0)
        self.view(host,lambda v:not v['actionsVisible'] and any('Enter Message' in x for x in v['screenText']));self.type(host,'repaired')
        self.assertEqual(self.text(host),'repaired');self.assertEqual(host.rpc({'op':'logs'})['result'],[])

    def test_old_menu_button_cannot_act_on_reopened_menu_or_new_input(self):
        host=self.host(native=True,commands=[self.command(inputPrompt='Message')]);self.open(host);self.type(host,'original')
        self.key(host,40,'k',CMD);self.view(host,lambda v:v['actionsVisible']);host.probe('retain-input-button',index=1);self.key(host,53);self.view(host,lambda v:not v['actionsVisible'])
        self.key(host,40,'k',CMD);self.view(host,lambda v:v['actionsVisible']);host.probe('old-input-button');self.assertTrue(host.probe('read')['runInputInline']);self.assertTrue(host.probe('read')['actionsVisible']);self.key(host,53);self.view(host,lambda v:not v['actionsVisible']);self.key(host,53);self.closed(host)
        self.open(host);self.type(host,'new');host.probe('old-input-button');self.assertEqual(self.text(host),'new');self.assertFalse(host.probe('read')['formVisible'])
        self.assertEqual(host.rpc({'op':'logs'})['result'],[])

    def test_native_expand_selectors_preserve_text_without_executing(self):
        host=self.host(native=True,commands=[self.command(inputPrompt='Message')])
        for selector in ['line-break','newline-ignore']:
            self.open(host);self.type(host,'selector value');host.probe('run-inline-selector',selector=selector);self.expanded(host)
            self.assertEqual(host.probe('run-input-read')['text'],'selector value');host.probe('cancel-editor');self.closed(host)
        self.assertEqual(host.rpc({'op':'logs'})['result'],[])

    def test_script_failure_restores_same_inline_text_cursor_and_typing(self):
        host=self.host(native=True,commands=[self.command('raise SystemExit(7)',inputPrompt='Message')]);self.open(host);self.type(host,'keep retry text')
        token=host.probe('run-inline-read')['token'];self.key(host,36,'\r');self.idle(host)
        self.view(host,lambda v:any('7' in x for x in v['screenText']));self.assertEqual(self.text(host),'keep retry text');self.assertEqual(host.probe('run-inline-read')['token'],token)
        self.type(host,' retained');self.assertEqual(self.text(host),'keep retry text retained');self.assertEqual(host.rpc({'op':'usage'})['result'],{})

    def test_cancel_running_inline_input_reaches_helper_and_does_not_reopen_old_draft(self):
        host=self.host(native=True);pid=host.base/'leader'
        code='import pathlib,os,time;pathlib.Path(%r).write_text(str(os.getpid()));time.sleep(2)'%str(pid)
        self.replace_command(host,self.command(code,inputPrompt='Message'));self.open(host);self.type(host,'cancel active');self.key(host,36,'\r');self.file(pid)
        self.key(host,53);self.closed(host);self.idle(host)
        with self.assertRaises(ProcessLookupError):__import__('os').kill(int(pid.read_text()),0)
        self.open(host);self.type(host,'new draft');self.assertEqual(self.text(host),'new draft');self.assertEqual(host.rpc({'op':'usage'})['result'],{})

    def test_newer_query_during_run_finishes_input_without_hiding_new_search(self):
        host=self.host(native=True);command,ready,gate,done=inputs.NativeInput.waiting_script(self,host);self.replace_command(host,command)
        self.open(host);self.type(host,'own fixture input');self.key(host,36,'\r');self.file(ready)
        host.probe('run-input-new-query',query='finder');host.settled();gate.write_text('go');self.file(done);self.idle(host)
        view=self.closed(host);self.assertEqual(view['input'],'finder');self.assertTrue(host.rpc({'op':'status'})['result']['nativeUI']['visible'])
        self.assertEqual(host.rpc({'op':'usage'})['result']['script.fixture'],1)

    def test_light_dark_inline_and_menu_captures(self):
        host=self.host(native=True,commands=[self.command(inputPrompt='Message')])
        for theme in ['light','dark']:
            config=json.loads(host.config.read_bytes());config['theme']=theme;host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok'])
            self.open(host);self.type(host,'Unicode büro 日本語');self.capture(host,'run-inline-'+theme)
            self.key(host,40,'k',CMD);self.view(host,lambda v:v['actionsVisible']);self.capture(host,'run-inline-menu-'+theme)
            self.key(host,53);self.view(host,lambda v:not v['actionsVisible']);self.key(host,53);self.closed(host)

if __name__=='__main__':unittest.main()
