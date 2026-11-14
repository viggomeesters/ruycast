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

if __name__=='__main__':unittest.main()
