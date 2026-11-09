"""Dedicated own hidden choice window. No OS popup tracking/focus/effects."""
import hashlib,json,os,shutil,subprocess,tempfile,unittest
from pathlib import Path
import keyword_process as keyword
ROOT=keyword.ROOT;BIN=ROOT/'target/debug/native-snippet-choice-probe';SHIM=ROOT/'target/native-snippet-choice-guard.dylib'
class NativeSnippetChoice(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run(['cc','-Wall','-Wextra','-Werror','-fobjc-arc','-fblocks','-dynamiclib','-framework','AppKit',str(ROOT/'tests/snippet_choice_guard.m'),'-o',str(SHIM)],check=True,capture_output=True)
        if folder:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            out=Path(folder);out.mkdir(parents=True,exist_ok=True);source=ROOT/'tests/snippet_choice_guard.m';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();(out/'native-snippet-choice-bindings.json').write_text(json.dumps(dict(sources={str(source.relative_to(ROOT.parent)):sha(source)},artifacts={str(SHIM.relative_to(ROOT.parent)):sha(SHIM)}),indent=2))
    def invoke(self,text='{field name="name" default="Own"}',steps=(),*,theme='dark',name=None,lib=True,**extra):
        own=tempfile.TemporaryDirectory(prefix='own-snippet-choice-',dir=ROOT/'target');self.addCleanup(own.cleanup);base=Path(own.name);(base/'.ruycast-choice-fixture').write_bytes(b'own-choice-v1')
        env={k:v for k,v in os.environ.items() if not k.startswith(('RUYCAST_','RUSTCAST_')) and k!='DYLD_INSERT_LIBRARIES'}
        if lib:env['DYLD_INSERT_LIBRARIES']=str(SHIM)
        title=extra.pop('title','Own snippet')
        case=dict(command=keyword.command('own.choice',';own',text),steps=list(steps),theme=theme,**extra);case['command']['title']=title
        p=subprocess.run([str(BIN),str(base)],input=json.dumps(case),capture_output=True,text=True,env=env,timeout=15);self.assertEqual(p.returncode,0,p.stderr);v=json.loads(p.stdout)
        if not lib:self.assertFalse(v['ok']);return v
        if extra.get('dropDuring'):
            self.assertTrue(v['ok'],v);self.assertTrue(v['rootDropped']);self.assertTrue(v['ownerRetired']);self.assertTrue(v['panelRetired']);self.assertEqual(v['events'],1);return v
        self.assertTrue(v['ok'],v);self.assertFalse(v['visible']);self.assertTrue(v['prohibited']);self.assertFalse(v['canBecomeKey']);self.assertFalse(v['canBecomeMain']);self.assertTrue(v['nonactivating']);self.assertTrue(v['ownerRetired']);self.assertTrue(v['generalBoardSubstitute'])
        if name and (folder:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR')):
            out=Path(folder).parent/'native-snippet-choice';out.mkdir(parents=True,exist_ok=True);shutil.copy2(base/'choice.png',out/(name+'.png'));(out/(name+'.json')).write_text(json.dumps(v,ensure_ascii=False,indent=2))
        return v
    def test_fixture_requires_complete_guard(self):self.assertIn('Complete owned clipboard substitute',self.invoke(lib=False)['error'])
    def test_required_field_does_not_read_clipboard_until_eligible(self):
        v=self.invoke('{name} {clipboard}',[dict(op='key',code=36)],clipboard='OWN CLIP');self.assertEqual(v['clipboardReads'],0);self.assertEqual(v['events'],[]);self.assertTrue(v['live'])
        v=self.invoke('{name} {clipboard}',[dict(op='set',field=0,value='Own'),dict(op='key',code=36)],clipboard='OWN CLIP');self.assertEqual(v['clipboardReads'],1);self.assertEqual(v['events'],[dict(submit=[['name','Own']])]);self.assertFalse(v['live'])
    def test_field_controls_buttons_and_hidden_renders(self):
        for theme in ['light','dark']:
            v=self.invoke('{field name="name" default="Own"} {field name="kind" options="One,Two"}',[dict(op='render'),dict(op='show')],theme=theme,name='fields-'+theme);self.assertEqual(v['width'],456);self.assertEqual(v['height'],270);self.assertEqual([c['label'] for c in v['controls']],['name','kind']);self.assertEqual(v['steps'],[dict(show=False)])
    def test_real_buttons_single_use_and_callback_reentry(self):
        v=self.invoke(steps=[dict(op='insert'),dict(op='insert'),dict(op='cancel')],reenter=True);self.assertEqual(v['events'],[dict(submit=[['name','Own']])]);self.assertFalse(v['failed'])
    def test_cancel_and_external_hide_have_distinct_restore_policy(self):
        for step,restore in [(dict(op='cancel'),True),(dict(op='key',code=53),True),(dict(op='hide'),False),(dict(op='resign'),False)]:
            v=self.invoke(steps=[step,dict(op='insert')]);self.assertEqual(v['events'],[dict(cancel=True,restore=restore)])
    def test_repeat_modifiers_and_marked_text_do_not_submit(self):
        for key in [dict(op='key',code=36,repeat=True),dict(op='route',code=36,flags=1<<20)]:
            v=self.invoke(steps=[key]);self.assertEqual(v['events'],[])
        v=self.invoke(steps=[dict(op='focus',field=0),dict(op='marked'),dict(op='equivalent',code=36)]);self.assertEqual(v['events'],[]);self.assertEqual(v['steps'][-1],dict(handled=False))
    def test_option_selection_and_arrow_clamping_without_opening_os_menu(self):
        v=self.invoke('{field name="kind" options="One,Two,Three"}',[dict(op='choose',field=0,index=1),dict(op='route',code=125),dict(op='route',code=125),dict(op='insert')]);self.assertEqual(v['events'],[dict(submit=[['kind','Three']])])
    def test_callback_panic_fails_closed(self):
        v=self.invoke(steps=[dict(op='insert'),dict(op='insert')],panic=True);self.assertTrue(v['failed']);self.assertFalse(v['live']);self.assertEqual(len(v['events']),1)
    def test_undo_redo_and_leaving_editor_submit_actual_values(self):
        for operation,expected,leave in [(dict(op='undo'),'Own',False),(dict(op='edit-key'),'Own',False),(dict(op='edit-key',redo=True),'OwnX',False),(dict(op='undo'),'Own',True)]:
            steps=[dict(op='focus',field=0),dict(op='type',value='X')]
            if operation.get('redo'):steps+=[dict(op='undo')]
            steps+=[operation]
            if leave:steps+=[dict(op='focus',field=1)]
            steps+=[dict(op='insert')]
            v=self.invoke('{field name="name" default="Own"} {other default="Other"}',steps);self.assertEqual(v['events'][0]['submit'][0],['name',expected]);self.assertTrue(v['steps'][1]['canUndo'])
    def test_popup_tracking_is_contained_in_fixture(self):
        v=self.invoke('{field name="kind" options="One,Two"}',[dict(op='popup-track')]);self.assertEqual(v['events'],[]);self.assertGreater(v['controls'][0]['blockedTracking'],0)
    def test_callback_drops_last_window_during_actual_native_lifecycle(self):
        for phase in ['close','hide','resign']:
            with self.subTest(phase=phase):self.invoke(dropDuring=phase)
    def test_scrollable_long_fields_and_two_line_title(self):
        text='{field name="A longer label that needs more than one line at the available field width to remain legible" default="Own"} '+' '.join('{'+f'field name="item{i}" default="own{i}"'+'}' for i in range(8))
        for theme in ['light','dark']:
            v=self.invoke(text,[dict(op='render')],theme=theme,name='long-'+theme,title='A longer snippet title that uses two lines while preserving the form and its actions');self.assertEqual(v['height'],440);self.assertEqual(len(v['controls']),9)
    def test_canonical_default_representation_is_preserved_on_submit(self):
        v=self.invoke('{field name="name" options="é,Other" default="é"}',[dict(op='insert')]);self.assertEqual(v['events'],[dict(submit=[['name','é']])])
    def test_undo_then_real_tab_keeps_restored_value(self):
        v=self.invoke('{field name="name" default="Own"} {other}',[dict(op='focus',field=0),dict(op='type',value='X'),dict(op='undo'),dict(op='key',code=48),dict(op='set',field=1,value='Other'),dict(op='insert')]);self.assertEqual(v['events'][0]['submit'][0],['name','Own'])
if __name__=='__main__':unittest.main(verbosity=2)
