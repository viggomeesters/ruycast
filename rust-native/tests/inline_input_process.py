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

    def delayed_control(self,host,**options):
        host.probe('schedule-inline-control',**options)
        stream=host.connect(timeout=4);self.addCleanup(stream.close);stream.sendall(b'{"op":"native.delay"}\n')
        self.file(host.profile/'inline-control-fired')
        with stream.makefile('rb') as reader:self.assertTrue(json.loads(reader.readline())['result']['newIntentDuringDelay'])
        host.settled()

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

if __name__=='__main__':unittest.main()
