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
    def test_long_choice_and_label_render_with_complete_accessibility_names(self):
        label='Place '+('long label '*12);value=('Long option '+('Ω value '*30)).rstrip()
        target='https://example.invalid/{a default="fixed"}/{b name="'+label+'" default="'+value+'" options="'+value+',Other"}'
        for theme in ['light','dark']:
            for width in [600,760]:
                h=self.host(native.command('url',target),theme=theme,width=width);self.form(h)
                view=self.capture(h,'choice-long-'+theme+'-'+str(width));self.assertIn(label,view['accessibilityText'])
                self.key(h,48,'\t');self.key(h,49,' ');view=self.capture(h,'choice-long-open-'+theme+'-'+str(width));self.assertIn(value,view['accessibilityText'])
                self.key(h,53);self.assertEqual(self.fields(h)['values'][1],value)
    def test_open_large_choice_virtualizes_last_option_and_retains_exact_value(self):
        options=','.join('Choice%d'%i for i in range(10000))
        h=self.host(native.command('url','https://example.invalid/{a default="x"}/{b name="Place" default="Choice0" options="'+options+'"}'));self.form(h)
        self.key(h,48,'\t');self.key(h,49,' ');v=self.fields(h)
        self.assertEqual(v['choice']['rows'],10000);self.assertLessEqual(v['choice']['visibleRows'],32);self.assertLess(v['choice']['reads'],64)
        h.probe('template-choice',command='select',index=9999);h.settled();v=self.fields(h)
        self.assertEqual(v['values'][1],'Choice0');self.assertLess(v['choice']['reads'],128)
        self.capture(h,'choice-last');self.key(h,36,'\r');self.assertEqual(self.fields(h)['values'][1],'Choice9999')
        self.key(h,36,'\r',1<<20);self.wait_file(h.base/'open-started');self.done(h);self.assertIn('Choice9999',unquote(self.opens(h)[0]['url']))

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
    def test_canonical_choice_default_and_narrow_light_dark_render(self):
        target='https://example.invalid/{a name="Text" default="fixed"}/{b name="Place" default="cafe\u0301" options="café,Other"}'
        for theme in ['light','dark']:
            h=self.host(native.command('url',target),theme=theme,width=600)
            self.form(h);self.capture(h,'template-'+theme)
            h.probe('save-editor');self.wait_file(h.base/'open-started');self.done(h)
            self.assertIn('café',__import__('unicodedata').normalize('NFC',unquote(self.opens(h)[0]['url'])))

    def test_many_fields_are_virtualized_and_last_value_remains_reachable(self):
        target='https://example.invalid/'+'/'.join('{f%d default="x"}'%i for i in range(5000))
        h=self.host(native.command('url',target));self.form(h)
        h.probe('template-type',index=4999,text='last-value');v=self.fields(h)
        self.assertEqual(v['fields'],5000);self.assertEqual(v['values'][-1],'last-value');self.assertLessEqual(v['liveRows'],32)
        h.probe('save-editor');self.wait_file(h.base/'open-started');self.done(h)
        self.assertTrue(unquote(self.opens(h)[0]['url']).endswith('last-value'))
    def test_large_choice_datasource_and_canonical_default(self):
        choices=','.join('Choice%d'%i for i in range(10000))
        target='https://example.invalid/{a name="Text" default="fixed"}/{b name="Mode" options="'+choices+'"}'
        h=self.host(native.command('url',target));self.form(h);v=self.fields(h)
        self.assertLess(v['optionReads'],64)
        h.probe('template-choose',index=1,selected=9999);self.assertEqual(self.fields(h)['values'][1],'Choice9999')
        h.probe('save-editor');self.wait_file(h.base/'open-started');self.done(h);self.assertIn('Choice9999',self.opens(h)[0]['url'])
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
