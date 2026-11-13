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


if __name__=='__main__':unittest.main(verbosity=2)
