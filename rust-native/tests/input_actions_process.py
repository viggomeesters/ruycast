"""Pinned configuration actions suspend one private Input draft."""
import json
import os
import time
import struct
import zlib
import unittest
import tempfile
import subprocess
from pathlib import Path
import bridge_process as bridge
import inline_input_process as inline
CMD=inline.CMD

def png_text_regions(path):
    """Decode own bounded RGBA8 fixture PNG; inspect stable title/footer pixels.

    Unlike native metadata this detects controls actually overwritten during draw.
    No OCR, extra screenshot, display flush or synchronization is used.
    """
    data=path.read_bytes();assert data[:8]==b'\x89PNG\r\n\x1a\n';pos=8;chunks=[]
    while pos<len(data):
        size=struct.unpack_from('>I',data,pos)[0];kind=data[pos+4:pos+8];value=data[pos+8:pos+8+size];pos+=12+size
        if kind==b'IHDR':w,h,depth,color,compression,filtering,interlace=struct.unpack('>IIBBBBB',value)
        if kind==b'IDAT':chunks.append(value)
    assert (depth,color,compression,filtering,interlace)==(8,6,0,0,0) and w<=4096 and h<=4096
    raw=zlib.decompress(b''.join(chunks));stride=w*4;assert len(raw)==h*(stride+1)
    previous=bytearray(stride);colors={'heading':set(),'footer':set()}
    for y in range(h):
        offset=y*(stride+1);kind=raw[offset];row=bytearray(raw[offset+1:offset+1+stride]);assert kind<=4
        for x in range(stride):
            a=row[x-4] if x>=4 else 0;b=previous[x];c=previous[x-4] if x>=4 else 0
            if kind==1:prediction=a
            elif kind==2:prediction=b
            elif kind==3:prediction=(a+b)//2
            elif kind==4:
                p=a+b-c;pa,pb,pc=abs(p-a),abs(p-b),abs(p-c);prediction=a if pa<=pb and pa<=pc else b if pb<=pc else c
            else:prediction=0
            row[x]=(row[x]+prediction)&255
        for name,top,bottom in [('heading',86,106),('footer',454,476)]:
            if top*h/490<=y<bottom*h/490:
                colors[name].update(bytes(row[x:x+3]) for x in range(int(24*w/760)*4,int(400*w/760)*4,4))
        previous=row
    return {name:len(values) for name,values in colors.items()}

class InputActions(unittest.TestCase):
    host=inline.InlineInput.host;command=inline.InlineInput.command;show=inline.InlineInput.show;file=inline.InlineInput.file;view=inline.InlineInput.view;key=inline.InlineInput.key;replace_command=inline.InlineInput.replace_command;idle=inline.InlineInput.idle;capture=inline.InlineInput.capture
    open=inline.InlineInput.open;type=inline.InlineInput.type;text=inline.InlineInput.text;closed=inline.InlineInput.closed
    def fixture(self):
        host=self.host(native=True,commands=[self.command(inputPrompt='Message')]);self.open(host);self.type(host,'PRIVATE literal Ω');return host
    def action(self,host,title):
        self.key(host,40,'k',CMD);view=self.view(host,lambda v:v['actionsVisible']);index=view['actionLabels'].index(title);host.probe('action-button',index=index)
    def form(self,host):return self.view(host,lambda v:v['formVisible'] and not v['runInputInline'])
    def resumed(self,host):
        host.settled()
        return self.view(host,lambda v:v['runInputInline'] and not v['actionsVisible'])
    def saved_command(self,host):return next(c for c in json.loads(host.config.read_bytes())['commands'] if c['id']=='script.fixture')
    @classmethod
    def setUpClass(cls):
        cls.temp=tempfile.TemporaryDirectory(prefix='input-actions-shim-',dir=bridge.rt.ROOT/'target');cls.shim=Path(cls.temp.name)/'sync.dylib'
        target=Path(os.environ.get('CARGO_TARGET_DIR',bridge.rt.ROOT.parent/'.build/full-rust-release/native'))
        cls.shim=Path(os.environ.get('RUYCAST_TEST_FCNTL_SHIM',target/'release/examples/libfcntl_sync_shim.dylib'))
        if not cls.shim.is_absolute() or not cls.shim.is_file():
            raise RuntimeError('Build --features test-tools --example fcntl-sync-shim and set RUYCAST_TEST_FCNTL_SHIM first')
    @classmethod
    def tearDownClass(cls):cls.temp.cleanup()
    def test_menu_cancel_while_owner_busy_preserves_actual_new_typing(self):
        host=self.fixture();self.key(host,40,'k',CMD);self.view(host,lambda v:v['actionsVisible']);before=host.config.read_bytes()
        host.probe('schedule-input-action-cancel');stream=host.connect(timeout=4);self.addCleanup(stream.close);stream.sendall(b'{"op":"native.delay"}\n');self.file(host.profile/'input-action-cancel-fired')
        with stream.makefile('rb') as reader:self.assertTrue(json.loads(reader.readline())['result']['newIntentDuringDelay'])
        self.resumed(host);self.assertEqual(self.text(host),'PRIVATE literal Ω continued');self.assertEqual(host.config.read_bytes(),before);self.assertFalse(host.probe('read')['formVisible'])
    def test_cancel_published_but_never_presented_child_preserves_live_text_ime_and_undo(self):
        host=self.fixture();before=host.config.read_bytes();self.key(host,40,'k',CMD);v=self.view(host,lambda v:v['actionsVisible']);host.probe('defer-input-child',defer=True);host.probe('action-button',index=v['actionLabels'].index('Set Alias…'))
        end=time.monotonic()+4
        while not host.probe('defer-input-child',defer=True)['pendingChild']:
            if time.monotonic()>end:self.fail('Owner did not actually publish the deferred child')
            time.sleep(.01)
        self.key(host,53);self.type(host,' continued');host.probe('run-inline-mark',text='日本');marked=host.probe('run-inline-read');self.assertTrue(marked['marked'])
        host.probe('defer-input-child',defer=False);self.resumed(host);after=host.probe('run-inline-read');self.assertEqual(after['text'],marked['text']);self.assertEqual(after['editorAddress'],marked['editorAddress']);self.assertTrue(after['marked']);self.assertTrue(after['canUndo']);self.assertEqual(host.config.read_bytes(),before);self.assertEqual(host.rpc({'op':'logs'})['result'],[])
    def test_postpublication_directory_sync_warning_resumes_without_retry(self):
        host=bridge.Bridge(start=False);self.addCleanup(host.close);config=json.loads(host.config.read_bytes());config['commands'].append(self.command(inputPrompt='Message'));host.replace(config)
        host.env.update(DYLD_INSERT_LIBRARIES=str(self.shim),RUYCAST_TEST_FSYNC_DIRECTORY=str(host.profile),RUYCAST_TEST_FSYNC_MARKER=str(host.profile/'sync-injected'));host.start();host.settled()
        self.open(host);self.type(host,'PRIVATE literal Ω');self.action(host,'Set Alias…');self.form(host);host.probe('alias-input',text='published');host.probe('save-editor');view=self.resumed(host)
        self.assertTrue((host.profile/'sync-injected').exists());self.assertEqual(self.saved_command(host)['aliases'],['published']);self.assertEqual(self.text(host),'PRIVATE literal Ω');self.assertTrue(any('disk confirmation failed' in x.lower() for x in view['screenText']),view['screenText']);self.assertEqual(host.rpc({'op':'logs'})['result'],[]);self.capture(host,'warning-input');pixels=png_text_regions(host.profile/'native-probe.png');self.assertGreater(pixels['heading'],8,pixels);self.assertGreater(pixels['footer'],8,pixels)
    def test_disable_and_remove_postpublication_warning_do_not_offer_retry(self):
        for action in ['Disable','Remove…']:
            host=bridge.Bridge(start=False);self.addCleanup(host.close);config=json.loads(host.config.read_bytes());config['commands'].append(self.command(inputPrompt='Message'));host.replace(config)
            host.env.update(DYLD_INSERT_LIBRARIES=str(self.shim),RUYCAST_TEST_FSYNC_DIRECTORY=str(host.profile),RUYCAST_TEST_FSYNC_MARKER=str(host.profile/'sync-injected'));host.start();host.settled();self.open(host);self.type(host,'PRIVATE literal Ω');self.action(host,action)
            if action=='Remove…':self.form(host);host.probe('save-editor');view=self.closed(host);self.assertFalse(any(c['id']=='script.fixture' for c in json.loads(host.config.read_bytes())['commands']))
            else:view=self.resumed(host);self.assertFalse(self.saved_command(host)['enabled']);self.assertEqual(self.text(host),'PRIVATE literal Ω');self.assertIn('Disabled · Expand ⌘↵',view['screenText']);self.capture(host,'warning-disabled');pixels=png_text_regions(host.profile/'native-probe.png');self.assertGreater(pixels['heading'],8,pixels);self.assertGreater(pixels['footer'],8,pixels)
            self.assertTrue((host.profile/'sync-injected').exists());self.assertTrue(any('disk confirmation failed' in x.lower() for x in view['screenText']));self.assertEqual(host.rpc({'op':'logs'})['result'],[])
    def test_queued_child_save_cancel_resumes_without_publishing(self):
        host=self.fixture();before=host.config.read_bytes();self.action(host,'Set Alias…');child=self.form(host)['editorToken'];host.probe('alias-input',text='must-not-save');host.probe('schedule-save-cancel');self.assertTrue(host.rpc({'op':'native.delay'})['ok']);view=self.resumed(host)
        self.assertGreater(view['editorToken'],child);self.assertEqual(self.text(host),'PRIVATE literal Ω');self.assertEqual(host.config.read_bytes(),before);self.type(host,' still typing');self.assertEqual(self.text(host),'PRIVATE literal Ω still typing')
    def test_external_removal_before_edit_keeps_input_without_replacement(self):
        host=self.fixture();config=json.loads(host.config.read_bytes());config['commands']=[c for c in config['commands'] if c['id']!='script.fixture'];host.replace(config);self.action(host,'Set Alias…')
        self.view(host,lambda v:any('changed' in x.lower() for x in v['screenText']));self.assertEqual(self.text(host),'PRIVATE literal Ω');self.assertFalse(host.probe('read')['formVisible'])
    def test_relocated_config_before_edit_does_not_redirect_suspended_input(self):
        host=self.fixture();folder=host.base/'relocated';folder.mkdir();self.assertTrue(host.cli('config','directory',str(folder))['ok']);before=host.config.read_bytes();self.action(host,'Set Alias…')
        self.view(host,lambda v:any('folder changed' in x.lower() for x in v['screenText']));self.assertEqual(self.text(host),'PRIVATE literal Ω');self.assertEqual(host.config.read_bytes(),before)
    def test_light_dark_input_config_actions_and_resumed_capture(self):
        host=self.fixture()
        for theme in ['light','dark']:
            config=json.loads(host.config.read_bytes());config['theme']=theme;host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok']);host.settled();self.key(host,40,'k',CMD);self.view(host,lambda v:v['actionsVisible']);self.capture(host,'input-actions-menu-'+theme)
            view=host.probe('read');host.probe('action-button',index=view['actionLabels'].index('Set Alias…'));self.form(host);self.capture(host,'input-actions-alias-'+theme);host.probe('cancel-editor');self.resumed(host);self.capture(host,'input-actions-resumed-'+theme)
    def test_each_config_editor_cancel_resumes_fresh_input_and_never_writes(self):
        host=self.fixture();before=host.config.read_bytes()
        for action in ['Set Alias…','Set Hotkey…','Edit…','Remove…']:
            old=host.probe('run-inline-read')['token'];self.action(host,action);child=self.form(host)['editorToken'];self.assertGreater(child,old)
            host.probe('cancel-editor');view=self.resumed(host);self.assertGreater(view['editorToken'],child);self.assertEqual(self.text(host),'PRIVATE literal Ω');self.assertEqual(host.config.read_bytes(),before)
        self.assertEqual(host.rpc({'op':'logs'})['result'],[])
    def test_alias_save_resumes_same_input_and_hotkey_recorder_save_does_too(self):
        host=self.fixture();self.action(host,'Set Alias…');self.form(host);host.probe('alias-input',text='newalias');host.probe('save-editor');self.resumed(host)
        self.assertEqual(self.saved_command(host)['aliases'],['newalias']);self.assertEqual(self.text(host),'PRIVATE literal Ω')
        self.action(host,'Set Hotkey…');self.form(host);self.key(host,45,'n',CMD);self.resumed(host);self.assertEqual(self.saved_command(host)['hotkey'],'cmd+n');self.assertEqual(self.text(host),'PRIVATE literal Ω')
        self.assertNotIn('PRIVATE',host.config.read_text());self.assertEqual(host.rpc({'op':'logs'})['result'],[])
    def test_failed_alias_save_retains_both_drafts_until_cancel(self):
        host=self.fixture();before=host.config.read_bytes();self.action(host,'Set Alias…');self.form(host);host.probe('alias-input',text='safari');host.probe('save-editor')
        self.view(host,lambda v:any('duplicate' in s.lower() for s in v['screenText']));self.assertEqual(host.config.read_bytes(),before);self.assertEqual(host.probe('read')['aliasValue'],'safari')
        host.probe('cancel-editor');self.resumed(host);self.assertEqual(self.text(host),'PRIVATE literal Ω')
    def test_alias_save_does_not_adopt_external_timeout_or_prompt(self):
        host=self.fixture();self.action(host,'Set Alias…');self.form(host);config=json.loads(host.config.read_bytes());c=next(c for c in config['commands'] if c['id']=='script.fixture');c['timeout']=2;c['inputPrompt']='Other prompt';host.replace(config)
        host.probe('alias-input',text='mine');host.probe('save-editor');self.resumed(host);self.key(host,36,'\r')
        self.view(host,lambda v:any('changed' in s.lower() for s in v['screenText']));self.assertEqual(self.text(host),'PRIVATE literal Ω');self.assertEqual(host.rpc({'op':'logs'})['result'],[])
    def test_full_editor_own_arguments_are_adopted_after_save_without_running(self):
        host=self.fixture();out=host.base/'result';self.action(host,'Edit…');self.form(host)
        host.probe('command-argument',index=1,text='import pathlib,sys;pathlib.Path(%r).write_text(sys.argv[1])'%str(out));host.probe('add-argument');host.probe('command-argument',index=2,text='{input}');host.probe('save-editor');self.resumed(host)
        self.assertFalse(out.exists());self.assertEqual(self.text(host),'PRIVATE literal Ω');self.key(host,36,'\r');self.file(out);self.idle(host);self.closed(host);self.assertEqual(out.read_text(),'PRIVATE literal Ω')
    def test_disable_and_enable_keep_input_but_disabled_run_cannot_spawn(self):
        host=self.fixture();self.action(host,'Disable');self.resumed(host);self.assertFalse(self.saved_command(host)['enabled']);self.assertEqual(self.text(host),'PRIVATE literal Ω')
        self.key(host,36,'\r');self.view(host,lambda v:any('disabled' in s.lower() or 'changed' in s.lower() for s in v['screenText']));self.assertEqual(host.rpc({'op':'logs'})['result'],[])
        self.key(host,40,'k',CMD);v=self.view(host,lambda v:v['actionsVisible']);self.assertNotIn('Run',v['actionLabels']);self.assertIn('Enable',v['actionLabels']);host.probe('action-button',index=v['actionLabels'].index('Enable'));self.resumed(host);self.assertTrue(self.saved_command(host)['enabled']);self.assertEqual(self.text(host),'PRIVATE literal Ω')
    def test_remove_commit_discards_input_and_never_executes(self):
        host=self.fixture();self.action(host,'Remove…');self.form(host);host.probe('save-editor');self.closed(host);self.assertFalse(any(c['id']=='script.fixture' for c in json.loads(host.config.read_bytes())['commands']));self.assertEqual(host.rpc({'op':'logs'})['result'],[])
    def test_hiding_child_drops_suspended_input(self):
        host=self.fixture();self.action(host,'Set Alias…');self.form(host);host.probe('hide-palette');host.settled(lambda s:not s['nativeUI']['visible']);self.show(host);self.assertFalse(host.probe('read')['runInputInline']);self.assertFalse(host.probe('read')['formVisible'])
    def test_settings_peer_preserves_primary_input(self):
        host=self.fixture();before=host.config.read_bytes();self.action(host,'Settings');self.resumed(host);host.wait(lambda s:s['managementUI']['visible']);self.assertEqual(self.text(host),'PRIVATE literal Ω');self.assertEqual(host.config.read_bytes(),before);self.assertEqual(host.rpc({'op':'logs'})['result'],[])

if __name__=='__main__':unittest.main()
