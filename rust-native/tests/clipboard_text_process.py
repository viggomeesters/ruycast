"""Genuine native UTF16 read/prefill functions, own named-board fault provider."""
import hashlib,json,os,subprocess,unittest
from pathlib import Path
import bridge_process as bridge
ROOT=Path(__file__).resolve().parents[1]
REFERENCE=ROOT/'target/clipboard-text-reference'
SHIM=ROOT/'target/clipboard-text-shim.dylib'

class ClipboardText(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run(['cc','-Wall','-Wextra','-Werror','-fobjc-arc','-dynamiclib','-framework','AppKit',str(ROOT/'tests/snippet_board_shim.m'),'-o',str(SHIM)],check=True,capture_output=True)
        subprocess.run(['swiftc',str(ROOT/'tests/clipboard_text_reference.swift'),'-o',str(REFERENCE)],check=True,capture_output=True)
        if directory:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            output=Path(directory);output.mkdir(parents=True,exist_ok=True)
            digest=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
            sources=[ROOT/'tests/clipboard_text_reference.swift',ROOT/'tests/snippet_board_shim.m',ROOT.parent/'Sources/RuycastApp/CommandEditor.swift',ROOT.parent/'Sources/RuycastApp/AppState.swift']
            (output/'clipboard-text-reference-bindings.json').write_text(json.dumps(dict(sources={str(p.relative_to(ROOT.parent)):digest(p) for p in sources},artifacts={str(p.relative_to(ROOT.parent)):digest(p) for p in [REFERENCE,SHIM]}),indent=2))
    def case(self, reader, mode, expected):
        env={k:v for k,v in os.environ.items() if not k.startswith(('RUYCAST_','RUSTCAST_')) and k!='DYLD_INSERT_LIBRARIES'}
        env['DYLD_INSERT_LIBRARIES']=str(SHIM)
        r=subprocess.run([str(bridge.TOOLS),'--check-owned-clipboard',reader,str(mode)],capture_output=True,env=env,timeout=15)
        self.assertEqual(r.returncode,0,r.stderr.decode(errors='replace'))
        self.assertEqual(json.loads(r.stdout),dict(ok=True,value=expected,restored=True,generalBoardAbortGuard=True))
        oracle=subprocess.run([str(REFERENCE),reader,str(mode)],capture_output=True,env=env,timeout=15)
        self.assertEqual(oracle.returncode,0,oracle.stderr.decode(errors='replace'))
        self.assertEqual(json.loads(oracle.stdout),json.loads(r.stdout))
    def test_native_read_lone_high_surrogate(self): self.case('read',1,'�')
    def test_native_prefill_lone_high_surrogate(self): self.case('prefill',1,'�')
    def test_provider_is_required_before_any_native_board_creation(self):
        env={k:v for k,v in os.environ.items() if not k.startswith(('RUYCAST_','RUSTCAST_')) and k!='DYLD_INSERT_LIBRARIES'}
        r=subprocess.run([str(bridge.TOOLS),'--check-owned-clipboard','read','1'],env=env,capture_output=True,timeout=5)
        self.assertEqual(r.returncode,1);self.assertIn('Complete owned clipboard provider required',json.loads(r.stdout)['error'])
    def test_all_owned_native_read_and_prefill_cases(self):
        for mode,value,prefill in [(0,'A\0B😀','A\0B😀'),(10,'�','�'),(11,' �𐀀�\0\n','�𐀀�\0'),(12,'x'*4095+'😀','x'*4095+'😀'),(13,'x'*4095+'�A','x'*4095+'�A'),(14,' \n'+'x'*1000001+'\n ','x'*1000001),(15,' \n\u200b',None),(16,'',None),(17,None,None)]:
            for reader,expected in [('read',value),('prefill',prefill)]:
                with self.subTest(mode=mode,reader=reader):self.case(reader,mode,expected)

if __name__=='__main__': unittest.main(verbosity=2)
