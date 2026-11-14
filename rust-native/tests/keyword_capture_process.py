"""Complete owned native keyword filter/effects; no real apps/input/clipboard."""
import copy,json,os,subprocess,unittest
import shutil
from pathlib import Path
import runtime_process as rt
ROOT=Path(__file__).resolve().parents[1];BIN=Path(os.environ.get('RUYCAST_KEYWORD_CAPTURE_PROBE',ROOT/'target/debug/keyword-capture-probe'));SDK=ROOT/'target/keyword-capture-fixture'
def config(text='HELLO',keyword='abc',**extra):
    c=copy.deepcopy(rt.DEFAULT);c['commands']=[dict(id='own.snippet',title='Own snippet',kind='snippet',target=text,keyword=keyword,**extra)];c['snippetExpansionEnabled']=True;return c
def sdk(control='state',**kw):return dict(op='sdk',control=control,**kw)
def event(text='',**kw):return sdk('event',text=text,**kw)
def keys(text):return [event(c) for c in text]
class KeywordCapture(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        SDK.mkdir(parents=True,exist_ok=True)
        target=Path(os.environ.get('CARGO_TARGET_DIR',ROOT.parent/'.build/full-rust-release/native'))
        for name,variable,library in [('complete','RUYCAST_TEST_KEYWORD_SHIM','libkeyword_fixture.dylib'),('missing','RUYCAST_TEST_KEYWORD_PARTIAL','libkeyword_missing_value_type.dylib')]:
            artifact=Path(os.environ.get(variable,target/'release/examples'/library))
            if not artifact.is_absolute() or not artifact.is_file():
                raise RuntimeError('Build --features test-tools --example keyword-fixture --example keyword-missing-value-type first')
            shutil.copyfile(artifact,SDK/(name+'.dylib'))
    def invoke(self,ops,lib='complete',ok=True):
        env={k:v for k,v in os.environ.items() if not k.startswith(('RUYCAST_','RUSTCAST_')) and k!='DYLD_INSERT_LIBRARIES'}
        if lib:env['DYLD_INSERT_LIBRARIES']=str(SDK/(lib+'.dylib'))
        p=subprocess.run([str(BIN)],input=json.dumps(ops),capture_output=True,text=True,env=env,timeout=10);self.assertEqual(p.returncode,0,p.stderr);v=json.loads(p.stdout);self.assertEqual(v['ok'],ok,v)
        if ok:self.assertTrue(v['retired'],v);self.assertEqual(v['final']['allocated'],0,v);self.assertEqual(v['final']['apps'],0,v)
        return v
    def run_keys(self,text='abc',c=None,between=None):return self.invoke([dict(op='config',value=c or config()),dict(op='refresh'),*(between or []),*keys(text)])
    def inserted(self,state):
        units=[u for e in state['output'] if e['down'] and e['key']==0 for u in e['units']];return bytes(b for u in units for b in [u&255,u>>8]).decode('utf-16-le')
if __name__=='__main__':unittest.main(verbosity=2)
