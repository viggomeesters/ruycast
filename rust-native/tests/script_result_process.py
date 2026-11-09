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
    def test_private_serde_magic_keys_stay_literal_in_real_provider_results(self):
        host=self.host()
        for value in ['true','42','null','"literal"','{"enabled":false}']:
            payload=dict(ok=True,detail='Own literal report',extra={'$serde_json::private::RawValue':value})
            self.assertEqual(self.run_value(host,payload),dict(payload,exitCode=0))
        payload=dict(ok=True,detail='Own literal report',extra={'$serde_json::private::Number':'1'})
        self.assertEqual(self.run_value(host,payload),dict(payload,exitCode=0))
    def test_process_failure_overrides_provider_success_but_keeps_detail(self):
        host=self.host();response=self.run_value(host,dict(ok=True,choices=self.choices(),detail='PRIVATE failure report'),7)
        self.assertFalse(response['ok']);self.assertEqual(response['exitCode'],7);self.assertIn('status 7',response['error']);self.assertEqual(response['detail'],'PRIVATE failure report');self.assertEqual(response['choices'],self.choices())
        self.assertEqual(host.rpc({'op':'usage'})['result'],{});self.assertNotIn('PRIVATE',(host.profile/'command-log.json').read_text())
    def test_provider_failure_detail_and_message_survive_a_zero_process_exit(self):
        host=self.host();response=self.run_value(host,dict(ok=False,error='Provider asks attention',detail='PRIVATE partial result'))
        self.assertEqual(len(response.pop('diagnosticID')),36);self.assertEqual(response,dict(ok=False,error='Provider asks attention',detail='PRIVATE partial result',exitCode=0));self.assertEqual(host.rpc({'op':'usage'})['result'],{})
    def test_cli_does_not_apply_gui_choice_validation(self):
        host=self.host()
        for change in ['duplicate','missing','empty-title','wrong-value-type','canonical-duplicate']:
            choices=self.choices()
            if change=='duplicate':choices['items']*=2
            elif change=='missing':del choices['placeholder']
            elif change=='empty-title':choices['items'][0]['title']=''
            elif change=='wrong-value-type':choices['items'][0]['value']=42
            else:choices['items']=[dict(choices['items'][0],id='é'),dict(choices['items'][0],id='e\u0301')]
            response=self.run_value(host,dict(ok=True,choices=choices,detail='PRIVATE validation report'))
            self.assertEqual(response,dict(ok=True,choices=choices,detail='PRIVATE validation report',exitCode=0))
        self.assertEqual(host.rpc({'op':'usage'})['result']['script.fixture'],5)
    def test_failed_provider_does_not_validate_or_replace_its_choice_payload(self):
        host=self.host();native=dict(ok=False,error='Reload refused',choices={'not':'a valid list'},detail='Keep existing list')
        response=self.run_value(host,native);self.assertEqual(len(response.pop('diagnosticID')),36);self.assertEqual(response,dict(native,exitCode=0))
    def test_empty_choice_list_is_valid_and_metadata_extensions_stay_present(self):
        host=self.host();choices=self.choices();choices.update(items=[],future=True)
        self.assertEqual(self.run_value(host,dict(ok=True,choices=choices)),dict(ok=True,choices=choices,exitCode=0))
    def test_generic_ok_only_and_plain_json_keep_the_ordinary_runner_wrapper(self):
        host=self.host()
        for value in [dict(ok=True),dict(detail='no boolean discriminator'),dict(ok='true',choices=self.choices())]:
            response=self.run_value(host,value);self.assertTrue(response['ok']);self.assertEqual(json.loads(response['result']['stdout']),value)
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
