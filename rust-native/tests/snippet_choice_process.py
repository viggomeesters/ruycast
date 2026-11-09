"""Production Swift choice model comparison. No panels/global effects/clipboard."""
import copy,hashlib,json,os,subprocess,unittest
from pathlib import Path
import keyword_process as keyword
ROOT=keyword.ROOT;REPO=ROOT.parent;DIR=ROOT/'target/snippet-choice-reference';REFERENCE=DIR/'reference';BIN=ROOT/'target/debug/snippet-choice-probe'
class SnippetChoice(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.comparisons=[];DIR.mkdir(parents=True,exist_ok=True)
        sources=[REPO/'Sources/LauncherModels/Models.swift',REPO/'Sources/LauncherCore/Templates.swift',REPO/'Sources/LauncherCore/SnippetMatcher.swift',REPO/'Sources/RuycastApp/SnippetChoice.swift',ROOT/'tests/snippet_choice_reference.swift']
        models=DIR/'libLauncherModels.dylib';core=DIR/'libLauncherCore.dylib'
        commands=[['swiftc','-emit-library','-emit-module','-module-name','LauncherModels','-o',str(models),'-emit-module-path',str(DIR/'LauncherModels.swiftmodule'),str(sources[0])],['swiftc','-emit-library','-emit-module','-module-name','LauncherCore','-I',str(DIR),'-L',str(DIR),'-lLauncherModels','-Xlinker','-rpath','-Xlinker',str(DIR),'-o',str(core),'-emit-module-path',str(DIR/'LauncherCore.swiftmodule'),*map(str,sources[1:3])],['swiftc','-parse-as-library','-I',str(DIR),'-L',str(DIR),'-lLauncherModels','-lLauncherCore','-Xlinker','-rpath','-Xlinker',str(DIR),*map(str,sources[3:]),'-o',str(REFERENCE)]]
        for command in commands:
            p=subprocess.run(command,capture_output=True,text=True,timeout=90)
            if p.returncode:raise AssertionError(p.stderr)
        if directory:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            out=Path(directory);out.mkdir(parents=True,exist_ok=True);sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();(out/'snippet-choice-reference-bindings.json').write_text(json.dumps(dict(sources={str(p.relative_to(REPO)):sha(p) for p in sources},artifacts={str(p.relative_to(REPO)):sha(p) for p in [REFERENCE,models,core]}),indent=2))
    def compare(self,cases):
        env={k:v for k,v in os.environ.items() if not k.startswith(('RUYCAST_','RUSTCAST_')) and k!='DYLD_INSERT_LIBRARIES'};outputs=[]
        for binary in [REFERENCE,BIN]:
            p=subprocess.run([str(binary)],input=json.dumps(cases),capture_output=True,text=True,env=env,timeout=20);self.assertEqual(p.returncode,0,p.stderr);v=json.loads(p.stdout);self.assertTrue(v['ok']);outputs.append(v['cases'])
        self.assertEqual(*outputs);self.comparisons.append(dict(cases=cases,expected=outputs[0]))
    @classmethod
    def tearDownClass(cls):
        if directory:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):(Path(directory)/'snippet-choice-comparisons.json').write_text(json.dumps(cls.comparisons,ensure_ascii=False,indent=2))
    def start(self,text):return dict(op='start',command=keyword.command('own.choice',';own',text))
    def test_complete_command_equality_matches_actual_swift(self):
        base=keyword.command('own.é',';é','é');base.update(title='é',aliases=['é','Second'],arguments=['é','Other'],hotkey='é',inputPrompt='é',applicationID='own.é',snippetOutput='é',timeout=0.0,enabled=True)
        pairs=[]
        for field in ['id','title','kind','target','hotkey','inputPrompt','keyword','applicationID','snippetOutput']:
            value=copy.deepcopy(base);value[field]='é' if base[field]=='é' else base[field].replace('é','é');pairs.append((base,value))
            value=copy.deepcopy(base);value[field]+='changed';pairs.append((base,value))
        for field in ['aliases','arguments']:
            for values in [['é',base[field][1]],list(reversed(base[field])),[],base[field]+['new']]:
                value=copy.deepcopy(base);value[field]=values;pairs.append((base,value))
        for field in ['hotkey','timeout','inputPrompt','keyword','applicationID','snippetOutput','enabled']:
            for value in [None,False if field=='enabled' else (1.5 if field=='timeout' else '')]:
                other=copy.deepcopy(base);other[field]=value;pairs.append((base,other))
        value=copy.deepcopy(base);value['timeout']=-0.0;pairs.append((base,value))
        empty=keyword.command('own',';own','own');omitted=copy.deepcopy(empty);omitted.pop('aliases',None);omitted.pop('arguments',None);pairs.append((empty,omitted))
        self.compare([[dict(op='equal',left=a,right=b)] for a,b in pairs])
    def test_defaults_canonical_options_labels_and_empty_values(self):
        self.compare([[self.start(text)] for text in ['plain','{name}','{name default="own"}','{name options="One,Two" default="Two"}','{é options="é,é,Other" default="é"}','{name options="A,B" default="missing"}','{a label="Own label" default="x"} {b default="y"}','{name default="\u200b"}','{name default="\0😀"}']])
    def test_keyboard_modifiers_repeat_composition_and_field_editing(self):
        self.compare([[self.start('{name options="One,Two,Three"}'),dict(op='key',code=code,flags=flag,repeat=repeat,composing=composing,editingText=editing),dict(op='key',code=36),dict(op='key',code=36)] for code in [36,76,53,123,124,125,126,48] for flag in [0,1<<17,1<<18,1<<19,1<<20,1<<16] for repeat,composing,editing in [(False,False,False),(True,False,False),(False,True,False),(False,False,True)]])
    def test_trim_validation_correct_retry_and_single_use(self):
        self.compare([[self.start('{name}'),dict(op='submit'),dict(op='set',field=0,value=value),dict(op='submit'),dict(op='submit'),dict(op='cancel')] for value in ['',' \n','\u200b','\u00a0',' own ','😀\0']])
    def test_options_clamp_first_canonical_match_and_active_field(self):
        self.compare([[self.start('{one options="A,B,C"} {two options="é,é,Z" default="é"}'),*[dict(op='key',code=123) for _ in range(3)],*[dict(op='key',code=124) for _ in range(5)],dict(op='focus',field=1),dict(op='key',code=125),dict(op='set',field=1,value='missing'),dict(op='key',code=125),dict(op='cancel'),dict(op='key',code=124),dict(op='submit')]])
if __name__=='__main__':unittest.main(verbosity=2)
