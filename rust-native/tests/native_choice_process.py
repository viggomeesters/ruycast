"""Live keyword request + original AX anchor + real hidden AppKit form.
Own typed app instances, private center/monitor adapter, synthetic SDK effects.
Never orders a window, changes focus, reads personal clipboard or posts keys.
"""
import copy,json,os,subprocess,time,unittest
from pathlib import Path
import native_keyword_process as keyword
import native_keyboard_process as keyboard
import keyword_capture_process as capture
import runtime_process as rt
import hashlib,shutil
import bridge_process as bridge

class NativeChoice(unittest.TestCase):
    def analytics(self,host):return host.rpc(dict(op='analytics'))['result']
    def wait_notice(self,host,text):
        end=time.monotonic()+3
        while time.monotonic()<end:
            notice=host.probe('notice-read')
            if notice.get('text')==text:return notice
            time.sleep(.01)
        raise AssertionError(notice)
    def test_actual_choice_analytics_records_once_only_at_apply_and_pause_is_preserved(self):
        host,_=self.visible();self.assertTrue(host.rpc(dict(op='analytics.reset',confirmed=True))['ok']);self.assertEqual(self.analytics(host)['totalAttempts'],0)
        self.choice(host,'insert');self.wait_choice(host,'None');self.wait_output(host);d=self.analytics(host);self.assertEqual((d['totalAttempts'],d['totalSuccesses'],d['totalFailures']),(1,1,0));self.assertEqual(next(iter(d['commands'].values()))['sources'],{'snippet':1});self.choice(host,'insert');self.assertEqual(self.analytics(host)['totalAttempts'],1)
        host,_=self.visible();self.assertTrue(host.rpc(dict(op='analytics.settings',enabled=False))['ok']);self.choice(host,'insert');self.wait_choice(host,'None');self.wait_output(host);self.assertEqual(self.analytics(host)['totalAttempts'],0)
    def test_actual_plain_keyword_analytics_and_rich_partial_failure_notice(self):
        host,_=self.host(body='Hello');self.assertEqual(self.type(host,'abc'),[False,False,True]);d=self.analytics(host);self.assertEqual((d['totalAttempts'],d['totalSuccesses']),(1,1))
        for theme in ['light','dark']:
            host,_=self.host(body='A{cursor}BC',keyword='x',snippetOutput='html',theme=theme);(host.base/'keyword-fail-board_partial').touch();self.assertEqual(self.type(host,'x'),[False]);d=self.analytics(host);self.assertEqual((d['totalAttempts'],d['totalFailures']),(1,1));self.assertEqual(d['events'][-1]['code'],'action-failed');text='Snippet insertion may be incomplete. Check the text before retrying.';notice=self.wait_notice(host,text);self.assertFalse(notice['windowShown']);self.assertTrue(notice['nonactivating']);self.assertTrue(notice['ignoresMouse']);self.assertTrue(notice['timer']);render=host.probe('notice-screenshot');self.assertEqual(render['text'],text);self.assertEqual(render['appearance'],'NSAppearanceNameAqua' if theme=='light' else 'NSAppearanceNameDarkAqua')
            if folder:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
                out=Path(folder).parent/'snippet-feedback';out.mkdir(parents=True,exist_ok=True);shutil.copy2(host.profile/'native-notice-probe.png',out/('partial-'+theme+'.png'));(out/('partial-'+theme+'.json')).write_text(json.dumps(render,indent=2))

    def test_actual_return_failures_show_feedback_without_false_attempt(self):
        host,_=self.visible();(host.base/'keyword-fail-app_activate').touch();self.choice(host,'insert');self.wait_choice(host,'None');self.wait_notice(host,'The app is unavailable. Type the keyword again.');self.assertEqual(self.analytics(host)['totalAttempts'],0)
        host,_=self.visible();self.kw(host,'text',value='changed',at=7);self.choice(host,'insert');self.wait_choice(host,'None');self.wait_notice(host,'The text field changed. Type the keyword again.');self.assertEqual(self.analytics(host)['totalAttempts'],0)
    def test_bounded_original_return_failure_has_feedback_and_no_attempt_or_output(self):
        host,_=self.visible();self.activate(host,'own');self.kw(host,'environment',pid='self',app=2**64-1);self.choice(host,'insert');self.wait_choice(host,'None');self.wait_notice(host,'The text field changed. Type the keyword again.');self.assertEqual(self.analytics(host)['totalAttempts'],0);self.assertEqual(self.kw(host)['posts'],0)
    @classmethod
    def setUpClass(cls):
        keyboard.NativeKeyboard.setUpClass.__func__(cls)
        cls.guard=Path(cls.sdk.name)/'choice-guard.dylib'
        target=Path(os.environ.get('CARGO_TARGET_DIR',rt.ROOT.parent/'.build/full-rust-release/native'))
        artifact=Path(os.environ.get('RUYCAST_TEST_CHOICE_SDK',target/'release/examples/libchoice_fixture.dylib'))
        if not artifact.is_absolute() or not artifact.is_file():
            raise RuntimeError('Build --features test-tools --example choice-fixture first')
        shutil.copyfile(artifact,cls.guard)
        assert hashlib.sha256(artifact.read_bytes()).digest()==hashlib.sha256(cls.guard.read_bytes()).digest()
        if folder:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            out=Path(folder);out.mkdir(parents=True,exist_ok=True);artifact=out/'native-choice-guard.dylib';shutil.copyfile(cls.guard,artifact);sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
            sources=[rt.ROOT/'tests/fixtures'/name for name in ['choice_board_sdk.rs','choice_monitor_sdk.rs','choice_fixture.rs']]
            sources.extend([rt.ROOT/'Cargo.toml',rt.ROOT/'Cargo.lock',rt.ROOT.parent/'rust/Cargo.toml',rt.ROOT.parent/'rust-toolchain.toml'])
            artifact_name=artifact.relative_to(rt.ROOT.parent) if artifact.is_relative_to(rt.ROOT.parent) else artifact
            (out/'native-choice-bindings.json').write_text(json.dumps(dict(sources={str(source.relative_to(rt.ROOT.parent)):sha(source) for source in sources},artifacts={str(artifact_name):sha(artifact)}),indent=2))

    @classmethod
    def tearDownClass(cls):keyboard.NativeKeyboard.tearDownClass.__func__(cls)
    wait_file=keyboard.NativeKeyboard.wait_file
    kw=keyword.NativeKeyword.kw
    type=keyword.NativeKeyword.type
    reset=keyword.NativeKeyword.reset
    def host(self,guard=True,barrier=False,body=None,theme=None,**extra):
        host=keyboard.NativeKeyboard.host(self,start=False)
        if guard:host.env['DYLD_INSERT_LIBRARIES']+=os.pathsep+str(self.guard)
        if barrier:host.env['RUYCAST_NATIVE_TEST_CONFIG_ADOPTION_BARRIER']=str(host.base)
        config=capture.config(body or 'Hello {field name="name" default="World"}{field name="kind" options="!,?" default="!"}',**extra)
        if theme:config['theme']=theme
        host.replace(config);host.start();host.settled(lambda s:s['snippetExpansion']=='On')
        self.kw(host,'text',value='abc')
        return host,config
    def choice(self,host,action='read-request',**values):return self.kw(host,'choice',choiceAction=action,**values)
    def wait_choice(self,host,phase):
        end=time.monotonic()+3
        while time.monotonic()<end:
            value=self.choice(host)
            if value['phase']==phase:return value
            time.sleep(.01)
        raise AssertionError(value)
    def visible(self,**extra):
        host,c=self.host(**extra);self.assertEqual(self.type(host,'abc'),[False,False,False]);v=self.wait_choice(host,'Visible');self.assertTrue(v['anchor']);self.assertTrue(v['window']);self.assertEqual(self.kw(host)['posts'],0);self.choice(host,'set',field=0,value='World');return host,c
    def wait_output(self,host):
        end=time.monotonic()+3
        while time.monotonic()<end:
            state=self.kw(host)
            if state['posts']>0:return state
            time.sleep(.01)
        raise AssertionError(state)
    def inserted(self,state):return capture.KeywordCapture.inserted(self,state)
    def capture(self,host,name):
        value=self.choice(host,'render');self.assertEqual(value['labels'],['name','kind']);self.assertFalse(value['nativeIsKey']);self.assertFalse(value['nativeVisible'])
        expected='NSAppearanceNameDarkAqua' if name.endswith('dark') else 'NSAppearanceNameAqua';self.assertEqual(value['appearance'],expected);self.assertEqual(value['explicitAppearance'],expected)
        if folder:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            out=Path(folder).parent/'native-choice';out.mkdir(parents=True,exist_ok=True);shutil.copy2(host.base/'choice.png',out/(name+'.png'));(out/(name+'.json')).write_text(json.dumps(value,indent=2))
        return value
    def activate(self,host,*values):return host.probe('keyboard-hid',control='activation-burst',values=list(values))
    def test_missing_complete_clipboard_substitute_refuses_before_ui_construction(self):
        host,_=self.host(guard=False);self.type(host,'abc');self.wait_choice(host,'None');self.assertEqual(self.kw(host)['posts'],0)
    def test_compatible_confirmed_configuration_keeps_user_draft(self):
        host,c=self.visible();self.choice(host,'set',field=0,value='Kept')
        c['commands'].append(dict(id='other',title='Other',kind='url',target='https://example.com'));c['theme']='light'
        before=host.rpc({'op':'status'})['result']['keyboardGeneration'];host.replace(c)
        host.wait(lambda s:s['keyboardGeneration']>before and s['keywordGeneration']==s['keyboardGeneration'] and not s['configurationUpdating'])
        self.assertEqual(self.choice(host)['phase'],'Visible');view=self.choice(host,'read');self.assertEqual(view['values'],['Kept','!']);self.assertEqual(view['appearance'],'NSAppearanceNameAqua')
        self.choice(host,'insert');self.assertEqual(self.inserted(self.wait_output(host)),'Hello Kept!')
    def test_changed_or_removed_original_command_cancels(self):
        for remove in [False,True]:
            host,c=self.visible();before=host.rpc({'op':'status'})['result']['keyboardGeneration']
            if remove:c['commands']=[]
            else:c['commands'][0]['title']='Changed'
            host.replace(c);host.wait(lambda s:s['keyboardGeneration']>before and not s['configurationUpdating']);self.wait_choice(host,'None');self.assertEqual(self.kw(host)['posts'],0)
    def test_pending_configuration_blocks_submit_and_retains_real_form_draft(self):
        for theme in ['light','dark']:
            host,c=self.visible(barrier=True,theme=theme);self.choice(host,'set',field=0,value='Kept');self.assertTrue(self.capture(host,'ready-'+theme)['insertEnabled']);before=host.rpc({'op':'status'})['result']['keyboardGeneration']
            marker=host.base/'config-adoption-entered';release=host.base/'config-adoption-release';(host.base/'hold-config-adoption').touch()
            c['commands'].append(dict(id='unrelated',title='Unrelated',kind='url',target='https://example.com'));host.replace(c)
            try:
                self.wait_file(marker);host.wait(lambda s:s['configurationUpdating'] and s['keywordGeneration']==0)
                result=self.choice(host,'insert');self.assertTrue(result['live']);self.assertFalse(result['insertEnabled']);self.assertTrue(self.choice(host,'submit-key')['live']);self.assertEqual(result['values'],['Kept','!']);self.assertEqual(self.choice(host)['phase'],'Visible');self.assertEqual(self.kw(host)['posts'],0);self.assertFalse(self.capture(host,'pending-'+theme)['insertEnabled'])
                self.activate(host,'own');self.assertEqual(self.choice(host)['phase'],'Visible')
            finally:release.touch()
            host.wait(lambda s:s['keywordGeneration']==s['keyboardGeneration'] and s['keyboardGeneration']>before and not s['configurationUpdating'])
            self.assertTrue(self.choice(host,'read')['insertEnabled']);self.choice(host,'insert');self.assertEqual(self.inserted(self.wait_output(host)),'Hello Kept!')
    def test_return_waits_for_original_and_new_input_cancels_while_returning(self):
        for cancel in [False,True]:
            host,_=self.visible();self.activate(host,'own');self.kw(host,'environment',pid='self',app=2**64-1);self.choice(host,'insert');self.wait_choice(host,'Returning')
            if cancel:
                self.kw(host,'event',text='x');self.wait_choice(host,'None');self.assertEqual(self.kw(host)['posts'],0)
            else:
                self.kw(host,'environment',app=1,pid=42);self.activate(host,'original');self.assertEqual(self.inserted(self.wait_output(host)),'Hello World!')
            self.assertEqual(self.kw(host)['activationRequests'],1)
    def test_lost_observation_and_shutdown_retire_native_request(self):
        host,_=self.visible();self.kw(host,'event',type=0xffffffff);self.wait_choice(host,'None');self.assertEqual(self.kw(host)['posts'],0)
        host,_=self.visible();self.assertTrue(host.rpc({'op':'quit'})['ok']);out,err=host.process.communicate(timeout=5);self.assertEqual(host.process.returncode,0,out.decode()+err.decode());self.assertFalse((host.profile/'control.sock').exists());self.assertEqual(json.loads((host.base/'keyword-retired.json').read_text())['state']['allocated'],0)
    def test_malformed_known_ax_value_cannot_authorize_live_choice(self):
        host,_=self.host();self.kw(host,'text',units=[0xd800,97,98,99],at=4);self.type(host,'abc');self.wait_choice(host,'None');self.assertEqual(self.kw(host)['posts'],0)
        host,_=self.visible();self.kw(host,'text',units=[97,98,99,0xd801],at=3);self.choice(host,'insert');self.wait_choice(host,'None');self.assertEqual(self.kw(host)['posts'],0)
    def test_rich_choice_uses_original_target_and_delayed_cursor(self):
        for mode in ['html','teams']:
            host,_=self.visible(snippetOutput=mode,body='Hello {field name="name" default="World"}{field name="kind" options="!,?" default="!"} {cursor}TAIL');self.choice(host,'insert');state=self.wait_output(host)
            end=time.monotonic()+2
            while sum(v['key']==123 and v['down'] for v in state['output'])<4 and time.monotonic()<end:time.sleep(.01);state=self.kw(host)
            self.assertEqual(bytes(state['pasteText']).decode(),'Hello World! TAIL');self.assertEqual(sum(v['key']==123 and v['down'] for v in state['output']),4);self.assertEqual(state['pasteIssues'],1)
    def test_quit_keeps_profile_through_held_native_monitor_callback_cleanup(self):
        host,_=self.visible();(host.base/'hold-choice-remove').touch();release=host.base/'choice-remove-release';self.kw(host,'choice-schedule-click');self.wait_file(host.base/'choice-remove-entered')
        try:
            self.assertTrue(host.rpc({'op':'quit'})['ok']);other=subprocess.run([str(bridge.TOOLS),'--native-hidden'],env=host.env,capture_output=True,text=True,timeout=3);self.assertNotEqual(other.returncode,0);self.assertIn('already running',other.stdout);self.assertIsNone(host.process.poll())
        finally:release.touch()
        out,err=host.process.communicate(timeout=5);self.assertEqual(host.process.returncode,0,out.decode()+err.decode());self.assertFalse((host.profile/'control.sock').exists())
if __name__=='__main__':unittest.main(verbosity=2)
