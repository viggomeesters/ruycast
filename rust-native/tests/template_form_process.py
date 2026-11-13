"""Real own native virtual fields -> pinned actor -> fully fake SDK."""
import json
import time
import unittest
from urllib.parse import unquote
import native_open_process as native

class TemplateForms(unittest.TestCase):
    @classmethod
    def setUpClass(cls):native.fixture.Workspace.setUpClass()
    host=native.NativeOpen.host
    show=native.NativeOpen.show
    activate=native.NativeOpen.activate
    wait_file=native.NativeOpen.wait_file
    done=native.NativeOpen.done
    trace=native.NativeOpen.trace
    opens=native.NativeOpen.opens
    hides=native.NativeOpen.hides
    capture=native.NativeOpen.capture
    def fields(self,h,predicate=lambda value:True):
        end=time.monotonic()+5
        while time.monotonic()<end:
            h.settled();value=h.probe('template-read')
            if predicate(value):return value
            time.sleep(.01)
        self.fail('Own template field state did not settle: '+str(value))
    def key(self,h,code,chars='',flags=0):
        h.probe('key-event',code=code,chars=chars,flags=flags);h.settled()

    def test_native_form_can_open_expanded_output_above_one_mib(self):
        h=self.host(native.command('url','https://example.invalid/{x}{x}{y default="Z"}'))
        self.show(h);self.activate(h);self.fields(h,lambda v:v.get('fields')==2)
        h.probe('template-type',index=0,text='a'*600000)
        h.probe('template-type',index=0,text='a'*600000,append=True)
        h.probe('save-editor');self.wait_file(h.base/'open-started');self.done(h)
        self.assertEqual(self.opens(h)[0]['url'],'https://example.invalid/'+'a'*2400000+'Z')

    def test_native_inline_can_submit_complete_input_above_one_mib(self):
        h=self.host(native.command('url','https://example.invalid/{x}'))
        self.show(h);self.activate(h)
        h.probe('run-inline-type',text='a'*600000)
        h.probe('run-inline-type',text='a'*600000)
        self.key(h,36,'\r');self.wait_file(h.base/'open-started');self.done(h)
        self.assertEqual(self.opens(h)[0]['url'],'https://example.invalid/'+'a'*1200000)

    def form(self,h):
        self.show(h);self.activate(h);return self.fields(h,lambda v:v.get('fields') is not None)

    def test_late_metadata_is_cancelled_by_new_show_and_config_change(self):
        for reason in ['show','config','cancel']:
            h=self.host(native.command('url','https://example.invalid/{a}/{b}'),barrier=True)
            self.show(h);self.activate(h);self.wait_file(h.base/'template-fields-entered')
            self.assertTrue(h.rpc(dict(op='status'))['ok'])
            if reason=='show':self.show(h,'newer query')
            elif reason=='config':
                cfg=json.loads(h.config.read_text());cfg['commands'][-1]['target']='https://example.invalid/new/{x}/{y}'
                h.replace(cfg);h.cli('reload')
            else:h.probe('key-event',code=53,chars='',flags=0)
            (h.base/'template-fields-release').write_text('release')
            h.wait(lambda s:not s['nativeUI'].get('templatePreparing'));h.settled()
            self.assertIsNone(h.probe('template-read')['fields']);self.assertEqual(self.opens(h),[])
    def test_metadata_deadline_wakes_owner_and_early_return_uses_bound_values(self):
        h=self.host(native.command('url','https://example.invalid/{query}'),barrier=True)
        self.show(h,'nativeopen early&value');self.wait_file(h.base/'template-fields-entered')
        h.probe('key-event',code=36,chars='\r',flags=0)
        (h.base/'template-fields-release').write_text('release');self.wait_file(h.base/'open-started');self.done(h)
        self.assertIn('early%26value',self.opens(h)[0]['url'])
        h=self.host(native.command('url','https://example.invalid/{a}/{b}'),barrier=True)
        self.show(h);self.activate(h);self.wait_file(h.base/'template-fields-entered')
        h.wait(lambda s:not s['nativeUI'].get('templatePreparing'),timeout=7);h.settled()
        self.assertIn('preparation timed out','\n'.join(h.probe('read')['screenText']));self.assertEqual(self.opens(h),[])
        (h.base/'template-fields-release').write_text('release')

if __name__=='__main__':unittest.main(verbosity=2)
