"""Exact shipped Swift keyword semantics versus Rust; no real input or effects."""
import hashlib,json,os,random,subprocess,unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];REPO=ROOT.parent
DIR=ROOT/'target/keyword-reference';REFERENCE=DIR/'reference';BIN=ROOT/'target/debug/keyword-probe'
def command(id='one',keyword='abc',target='own text',**extra):
    return dict(id=id,title=id,kind='snippet',target=target,keyword=keyword,aliases=[],arguments=[],**extra)
def catalog(*commands):return dict(op='catalog',commands=list(commands))
def typed(text):return dict(op='type',text=text)
class Keywords(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        DIR.mkdir(parents=True,exist_ok=True);cls.comparisons=[]
        sources=[REPO/'Sources/LauncherModels/Models.swift',REPO/'Sources/LauncherCore/Templates.swift',REPO/'Sources/LauncherCore/SnippetMatcher.swift',ROOT/'tests/keyword_reference.swift']
        models=DIR/'libLauncherModels.dylib';core=DIR/'libLauncherCore.dylib'
        cmds=[['swiftc','-emit-library','-emit-module','-module-name','LauncherModels','-o',str(models),'-emit-module-path',str(DIR/'LauncherModels.swiftmodule'),str(sources[0])],
              ['swiftc','-emit-library','-emit-module','-module-name','LauncherCore','-I',str(DIR),'-L',str(DIR),'-lLauncherModels','-Xlinker','-rpath','-Xlinker',str(DIR),'-o',str(core),'-emit-module-path',str(DIR/'LauncherCore.swiftmodule'),*map(str,sources[1:3])],
              ['swiftc','-parse-as-library','-I',str(DIR),'-L',str(DIR),'-lLauncherModels','-lLauncherCore','-Xlinker','-rpath','-Xlinker',str(DIR),str(sources[3]),'-o',str(REFERENCE)]]
        for cmd in cmds:
            p=subprocess.run(cmd,capture_output=True,text=True,timeout=90)
            if p.returncode:raise AssertionError(p.stderr)
        if evidence:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            folder=Path(evidence);folder.mkdir(parents=True,exist_ok=True);sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
            (folder/'keyword-reference-bindings.json').write_text(json.dumps(dict(sources={str(p.relative_to(REPO)):sha(p) for p in sources},artifacts={str(p.relative_to(REPO)):sha(p) for p in [REFERENCE,models,core]}),indent=2))
    @classmethod
    def tearDownClass(cls):
        if evidence:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            (Path(evidence)/'keyword-comparisons.json').write_text(json.dumps(cls.comparisons,ensure_ascii=False,indent=2))
    def invoke(self,binary,cases):
        env={k:v for k,v in os.environ.items() if not k.startswith(('RUYCAST_','RUSTCAST_')) and k!='DYLD_INSERT_LIBRARIES'}
        p=subprocess.run([str(binary)],input=json.dumps(cases,ensure_ascii=False),capture_output=True,text=True,env=env,timeout=30)
        self.assertEqual(p.returncode,0,p.stderr);v=json.loads(p.stdout);self.assertTrue(v['ok'],v);return v['cases']
    def compare(self,cases):
        expected=self.invoke(REFERENCE,cases);actual=self.invoke(BIN,cases)
        self.assertEqual(len(expected),len(actual))
        for c,a,b in zip(cases,expected,actual):self.assertEqual(a,b,str(dict(case=c,swift=a,rust=b))[:6000])
        self.comparisons.append(dict(method=self.id(),cases=cases,expected=expected,actual=actual));return actual
    def test_all_valid_unicode_scalars_word_boundary_union(self):
        expected=self.invoke(REFERENCE,[[dict(op='classify-all')]])[0][0];actual=self.invoke(BIN,[[dict(op='classify-all')]])[0][0]
        a=bytes.fromhex(expected['bits']);b=bytes.fromhex(actual['bits']);differences=[hex(i) for i in range(0x110000) if bool(a[i//8]&(1<<(i%8)))!=bool(b[i//8]&(1<<(i%8)))]
        self.comparisons.append(dict(method=self.id(),validScalars=0x110000-0x800,swiftCount=expected['count'],rustCount=actual['count'],differences=differences,swiftSHA256=hashlib.sha256(a).hexdigest(),rustSHA256=hashlib.sha256(b).hexdigest()))
        self.assertEqual(differences,[],str(differences[:60]))
    def test_longest_ties_eligibility_and_catalog_changes(self):
        cmds=[command('missing',';x','{name}'),command('default',';x','{name default="own"}'),command('option',';y','{name options="A,B"}'),command('short','abc'),command('long',';abc'),command('same-first',';tie'),command('same-second',';tie'),command('disabled',';z',enabled=False),command('prompt',';p',inputPrompt='Own input')]
        wrong=command('wrong',';w');wrong['kind']='script';cmds.append(wrong)
        out=self.compare([[catalog(*cmds)]+[op for text in [';x',';y',';abc',';tie',';z',';p',';w'] for op in [dict(op='reset'),typed(text)]]])
        self.assertEqual([out[0][i]['match'] for i in range(2,len(out[0]),2)],['default','option','long','same-first',None,None,None])
        self.compare([[catalog(command('old',';old')),typed(';ne'),catalog(command('new',';new')),typed('w'),typed(';old')],
                      [catalog(command('empty','')),typed('x'),typed(''),typed('y')]])
    def test_boundaries_canonical_equivalence_and_graphemes(self):
        keys=['abc','é','e\u0301','Å','\u212b',';x','👍🏽','👩\u200d💻','🇳🇱','क्\u200dष','\r\n','1️⃣','\u0345\u0300','\u0300\u0345','\u200d','\x00']
        prefixes=['',' ',':','x','9','é','\u0345','\u0301','⅚','㊈','🇺🇸','क्\u200dष']
        cases=[[catalog(command('own',key)),typed(prefix+key)] for key in keys for prefix in prefixes]
        cases += [[catalog(command('canonical','é')),typed('e'),typed('\u0301')],[catalog(command('join','👩\u200d💻')),typed('👩'),typed('\u200d'),typed('💻')]]
        self.compare(cases)
    def test_suffix128_backspace_reset_and_segment_merges(self):
        cases=[]
        for filler in ['a','é','e\u0301','👍🏽','🇳🇱','क्\u200dष','\r\n','\u200d']:
            for n in [0,1,126,127,128,129,200]:cases.append([catalog(command('own',';abc')),typed(filler*n+';a'),typed('b'),dict(op='backspace'),typed('bc')])
        cases += [[catalog(command('own','abc')),typed('ab'),typed(''),typed('c')], [catalog(command('own','é')),typed('x'),dict(op='backspace'),typed('e\u0301')]]
        self.compare(cases)
    def test_replacement_utf16_chunking_cursor_and_erase_counts(self):
        texts=['','abc','a'*17,'😀'*10,'a'*15+'😀b','e\u0301'*17,'🇳🇱👩\u200d💻\r\nक्\u200dष','\u200dक\u0301\x00']
        cases=[[dict(op='replace',keyword='é👍🏽abc',finalInput=final,text=text,cursor=cursor)] for text in texts for cursor in [None,*range(len(text.encode('utf-16-le'))//2+3)] for final in ['c','xy','e\u0301','longerthanword']]
        for at in range(0,len(cases),500):self.compare(cases[at:at+500])
    def test_focus_secure_precedence(self):self.compare([[dict(op='focus',secure=secure,subrole=subrole)] for secure in [True,False] for subrole in [None,'','AXSecureTextField','axsecuretextfield','AXTextField','other']])
    def test_choice_single_use_retries_cancel_and_target_guards(self):
        cmd=command('choice',';c','Hello {name options="A,B"} {clipboard}{cursor}!')
        start=dict(op='choice-start',command=cmd,pid=42)
        good=dict(op='choice',values=[['name','Own']],pid=42,safe=True,unchanged=True,clipboard='Own previous')
        cases=[[start,good,good],[start,dict(op='choice-cancel'),good],[start,{**good,'values':[]},good],[start,{**good,'clipboard':None},good]]
        for values in [dict(pid=43),dict(safe=False),dict(unchanged=False)]:cases.append([start,{**good,**values},good])
        self.compare(cases)
    def test_seeded_mixed_transitions_and_catalog_scale(self):
        rng=random.Random(8611);alphabet=['a','b','é','e\u0301',';',' ', '👍🏽','\u0345','\r\n','🇳🇱'];cases=[]
        for _ in range(40):
            keys=[''.join(rng.choices(alphabet,k=rng.randrange(1,7))) for _ in range(12)]
            ops=[catalog(*(command(str(i),k) for i,k in enumerate(keys)))]
            for _ in range(70):
                n=rng.randrange(10);ops.append(dict(op='backspace') if n==0 else dict(op='reset') if n==1 else typed(''.join(rng.choices(alphabet,k=rng.randrange(0,5)))))
            cases.append(ops)
        self.compare(cases)
        cmds=[command(str(i),f';own{i:05d}') for i in range(10000)]
        self.compare([[catalog(*cmds),typed(';own09999'),typed('x;own00001'),dict(op='reset'),typed(';own00001')]])
if __name__=='__main__':unittest.main(verbosity=2)
