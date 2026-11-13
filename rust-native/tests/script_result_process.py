"""Shared result interpretation through real CLI jobs and isolated profiles."""
import json
import unittest
import execution_process as execution
import input_process as inputs

class ScriptResults(unittest.TestCase):
    host=execution.Execution.host;command=execution.Execution.command;file=execution.Execution.file
    replace_command=execution.Execution.replace_command;pending=execution.Execution.pending;response=execution.Execution.response
    def choices(self):
        return dict(title='Project Koppelen',placeholder='Zoek project…',items=[dict(id='one',title='Café Project',subtitle='EX / PLAN',value='PRIVATE literal $(value) Ω',extra='unchanged')])
    def run_value(self,host,value,status=0):
        code='import sys;print(%r);raise SystemExit(%d)'%(json.dumps(value,ensure_ascii=False),status)
        self.replace_command(host,self.command(code));return host.rpc(dict(op='execute',id='runfixture'))
    def test_cli_preserves_choices_detail_unknown_fields_and_success_usage(self):
        host=self.host();native=dict(ok=True,choices=self.choices(),detail='PRIVATE report Ω',extra={'data':'PRIVATE extra'})
        response=self.run_value(host,native);self.assertEqual(response,dict(native,exitCode=0))
        self.assertEqual(host.rpc({'op':'usage'})['result']['script.fixture'],1)
        self.assertNotIn('PRIVATE',(host.profile/'command-log.json').read_text());self.assertNotIn('PRIVATE',(host.profile/'usage.json').read_text())
    show=inputs.NativeInput.show;view=inputs.NativeInput.view;key=inputs.NativeInput.key;type=inputs.NativeInput.type;idle=inputs.NativeInput.idle;capture=inputs.NativeInput.capture
    def test_native_provider_and_process_failure_details_keep_the_same_input_draft(self):
        for ok,status in [(False,0),(False,7),(True,7)]:
            payload=dict(ok=ok,error='Provider asks attention',detail='PRIVATE failure detail')
            code='print(%r);raise SystemExit(%d)'%(json.dumps(payload),status)
            host=self.host(native=True,commands=[self.command(code,inputPrompt='Message')]);token=inputs.NativeInput.open(self,host)['editorToken'];self.type(host,'PRIVATE keep input');host.probe('save-editor');self.idle(host)
            report=self.view(host,lambda v:v.get('scriptResult') is not None);self.assertTrue(report['resultPanel']['selectable']);self.assertFalse(report['resultPanel']['editable']);host.probe('result-control',name='close')
            expected='status 7' if ok else 'Provider asks attention'
            self.view(host,lambda v:v['formVisible'] and any(expected in t for t in v['screenText']));self.assertEqual(host.probe('read')['editorToken'],token);field=host.probe('run-input-read');self.assertEqual(field['text'],'PRIVATE keep input');self.assertTrue(field['focused'] and field['editable'] and field['canUndo']);self.type(host,' continued');self.assertEqual(host.probe('run-input-read')['text'],'PRIVATE keep input continued');self.assertEqual(host.rpc({'op':'usage'})['result'],{});self.assertNotIn('PRIVATE',(host.profile/'command-log.json').read_text());self.capture(host,'script-result-failure-%s-%s'%(ok,status))
    def test_usage_failure_preserves_completed_provider_result_and_does_not_retry(self):
        host=self.host();ready=host.base/'ready';gate=host.base/'gate';out=host.base/'out'
        native=dict(ok=True,choices=self.choices(),detail='PRIVATE completed report',extra=17)
        code='import time,pathlib;pathlib.Path(%r).write_text("ready");p=pathlib.Path(%r)\nwhile not p.exists():time.sleep(.01)\npathlib.Path(%r).write_text("once");print(%r)'%(str(ready),str(gate),str(out),json.dumps(native))
        self.replace_command(host,self.command(code));stream=self.pending(host);self.file(ready);(host.profile/'usage.json').mkdir();gate.write_text('go');response=self.response(stream)
        self.assertFalse(response['ok']);self.assertTrue(response['actionCompleted']);self.assertEqual(response['detail'],native['detail']);self.assertEqual(response['choices'],native['choices']);self.assertEqual(response['extra'],17);self.assertEqual(out.read_text(),'once')
        self.assertNotIn('PRIVATE',(host.profile/'command-log.json').read_text())

if __name__=='__main__':unittest.main()
