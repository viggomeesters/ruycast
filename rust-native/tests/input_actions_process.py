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
if __name__=='__main__':unittest.main()
