"""Actual AnalyticsStore Swift model/JSON codec versus Rust; disposable files only.
The Swift save stub is a disposable-file oracle, not private-storage evidence.
"""
import copy,hashlib,json,os,shutil,subprocess,tempfile,time,unittest
from pathlib import Path
import runtime_process as rt

DIR=rt.ROOT/'target/analytics-reference'
PROBE=rt.ROOT/'target/debug/analytics-probe'

SWIFT=r'''
import Foundation
import Combine
import CryptoKit
import Darwin
struct LauncherError:Error {let text:String;init(_ text:String){self.text=text}}
enum ProductVersion {static let current="fixture"}
enum Files {
  __ENCODE__
  __DECODE__
  static func savePrivate(_ bytes:Data,to url:URL)throws {try bytes.write(to:url,options:.atomic)}
}
func jsonValue<T:Encodable>(_ value:T)->Any {try! JSONSerialization.jsonObject(with:Files.encode(value))}
final class Clock:@unchecked Sendable {private let lock=NSLock();private var value:Double;init(_ v:Double){value=v};func set(_ v:Double){lock.lock();value=v;lock.unlock()};func date()->Date {lock.lock();defer{lock.unlock()};return Date(timeIntervalSince1970:value)}}
__ACTUAL_SOURCE__
let input=try JSONSerialization.jsonObject(with:FileHandle.standardInput.readDataToEndOfFile()) as! [String:Any]
let root=URL(fileURLWithPath:input["root"] as! String)
guard FileManager.default.fileExists(atPath:root.appendingPathComponent(".ruycast-analytics-fixture").path) else {fatalError("Own fixture required")}
let clock=Clock(input["now"] as? Double ?? 1790000000)
var calendar=Calendar(identifier:input["calendar"] as? String == "buddhist" ? .buddhist:.gregorian)
calendar.timeZone=TimeZone(identifier:input["zone"] as? String ?? "UTC")!
let store=AnalyticsStore(root:root,saveDelay:60,now:{clock.date()},calendar:calendar)
var tickets:[String:AnalyticsAttempt]=[:],results:[Any]=[]
for a in input["actions"] as! [[String:Any]] {
 if let now=a["now"] as? Double {clock.set(now)}
 switch a["op"] as! String {
 case "attempt":if let t=store.recordAttempt(a["id"] as? String ?? "own",source:a["source"] as? String ?? "cli") {tickets[a["ticket"] as? String ?? "t"]=t}
 case "complete":if let t=tickets.removeValue(forKey:a["ticket"] as? String ?? "t") {store.recordCompletion(t,success:a["ok"] as? Bool ?? false,ms:a["ms"] as? Double ?? 0,code:a["code"] as? String ?? "action-failed",exitStatus:a["exit"] as? Int)}
 case "timing":store.recordTiming(a["name"] as? String ?? "",ms:a["ms"] as? Double ?? -1)
 case "error":store.recordError(a["category"] as? String ?? "",code:a["code"] as? String ?? "")
 case "configure":try store.configure(enabled:a["enabled"] as? Bool,retentionDays:a["retentionDays"] as? Int)
 case "reset":store.reset()
 case "import":store.importLegacy(a["counts"] as! [String:Int])
 case "snapshot":results.append(["document":jsonValue(store.snapshot),"storageError":store.storageError as Any? ?? NSNull()])
 case "export":results.append(store.exportSummary())
 case "hourly":results.append(store.snapshot.hourlyActivity(now:clock.date()).map{[$0.date.timeIntervalSince1970,$0.activity.map{jsonValue($0)} ?? NSNull()]})
 case "casts":results.append(["bool":(a["value"] as? Bool) as Any? ?? NSNull(),"int":(a["value"] as? Int) as Any? ?? NSNull()])
 case "flush":store.flushSynchronously()
 default:fatalError("Unknown action")
 }
}
store.flushSynchronously()
let result:[String:Any] = ["ok":true,"results":results,"document":jsonValue(store.snapshot),"storageError":store.storageError as Any? ?? NSNull()]
print(String(data:try JSONSerialization.data(withJSONObject:result,options:[.sortedKeys]),encoding:.utf8)!)
'''

def normalized(v):
    if isinstance(v,list):return [normalized(x) for x in v]
    if isinstance(v,dict):return {k:normalized(x) for k,x in v.items() if k!='id'}
    return v

class Analytics(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        DIR.mkdir(parents=True,exist_ok=True)
        target=Path(os.environ.get('CARGO_TARGET_DIR',rt.ROOT.parent/'.build/full-rust-release/native'))
        artifact=Path(os.environ.get('RUYCAST_TEST_ANALYTICS_SHIM',target/'release/examples/libanalytics_write_shim.dylib'))
        if not artifact.is_absolute() or not artifact.is_file():
            raise RuntimeError('Build --features test-tools --example analytics-write-shim first')
        cls.shim=DIR/'analytics-write-shim.dylib';shutil.copyfile(artifact,cls.shim)
        original=(rt.ROOT.parent/'Sources/RuycastApp/AnalyticsStore.swift').read_text()
        source='\n'.join(line for line in original.splitlines() if line not in ['import Foundation','import Combine','import CryptoKit','import Darwin','import LauncherCore','import LauncherModels'])
        codec=(rt.ROOT.parent/'Sources/LauncherCore/Files.swift').read_text().splitlines()
        encode=next(x.strip().replace('public static','static',1) for x in codec if 'public static func encode<' in x)
        decode=next(x.strip().replace('public static','static',1) for x in codec if 'public static func decode<' in x)
        (DIR/'main.swift').write_text(SWIFT.replace('__ACTUAL_SOURCE__',source).replace('__ENCODE__',encode).replace('__DECODE__',decode))
        subprocess.run(['swiftc',str(DIR/'main.swift'),'-o',str(DIR/'reference')],check=True,capture_output=True)
        cls.cases=0
    def run_case(self,actions,initial=None,now=1790000000,zone='UTC',calendar='gregorian'):
        pair=[]
        for binary in [PROBE,DIR/'reference']:
            with tempfile.TemporaryDirectory(prefix='ruycast-analytics-',dir=DIR) as home:
                root=Path(home);(root/'.ruycast-analytics-fixture').touch()
                if initial is not None:(root/'analytics.json').write_text(json.dumps(initial))
                request=dict(root=str(root),now=now,zone=zone,calendar=calendar,actions=actions)
                p=subprocess.run([str(binary)],input=json.dumps(request),capture_output=True,text=True,timeout=30)
                self.assertEqual(p.returncode,0,p.stderr);pair.append(json.loads(p.stdout))
        self.assertEqual(normalized(pair[0]),normalized(pair[1]));type(self).cases+=1;return pair[0]
    def empty(self):return self.run_case([])['document']
    def test_existing_document_rejection_and_date_codec_match_actual_decoder(self):
        base=self.empty();variants=[]
        for key,value in [('schema',3),('retentionDays',0),('hourlyStartedAt',None),('hours',{'000':dict(uses=0,successes=0,failures=0)}),('timings',{'private':dict(count=0,totalMS=0,maxMS=0,samples=[])}),('totalAttempts',1000000000001)]:
            d=copy.deepcopy(base);d[key]=value;variants.append(d)
        for date in ['not a date','2026-11-01T00:00:00.123Z','2026-11-01T00:00:00Zjunk','2026-11-01T00:00:00Z\x00junk','2026-11-01T00:00:00+0000','2026-11-01T00:00:00Z']:
            d=copy.deepcopy(base);d['startedAt']=date;variants.append(d)
        for document in variants:
            pair=[]
            for binary in [PROBE,DIR/'reference']:
                with tempfile.TemporaryDirectory(prefix='analytics-load-',dir=DIR) as own:
                    root=Path(own);(root/'.ruycast-analytics-fixture').touch();original=json.dumps(document).encode();(root/'analytics.json').write_bytes(original)
                    run=subprocess.run([str(binary)],input=json.dumps(dict(root=str(root),now=1790000000,calendar='gregorian',zone='UTC',actions=[])),capture_output=True,text=True,timeout=20);self.assertEqual(run.returncode,0,run.stderr);value=json.loads(run.stdout);self.assertEqual((root/'analytics.json').read_bytes(),original)
                    pair.append(value['storageError'] if value['storageError'] else normalized(value['document']))
            self.assertEqual(pair[0],pair[1],document['startedAt']);type(self).cases+=1
    def test_actual_decoder_integer_spelling_at_all_stored_count_fields(self):
        document=self.run_case([dict(op='attempt',id='own',source='snippet'),dict(op='complete',ok=False,ms=1,exit=1)])['document']
        def floats(value):
            if type(value) is int:return float(value)
            if isinstance(value,dict):return {k:floats(v) for k,v in value.items()}
            if isinstance(value,list):return [floats(v) for v in value]
            return value
        self.run_case([dict(op='snapshot')],initial=floats(document))
        initial=json.dumps(self.empty());self.assertIn('"totalAttempts": 0',initial)
        for literal in ['0e99999','0e-99999','1.000','1.5','1000e-3','1000000000000.00001','999999999999.99999','1.0000000000000000001','1e-9999','-0.0']:
            raw=initial.replace('"totalAttempts": 0','"totalAttempts": '+literal);pair=[]
            for binary in [PROBE,DIR/'reference']:
                with tempfile.TemporaryDirectory(prefix='analytics-int-',dir=DIR) as own:
                    root=Path(own);(root/'.ruycast-analytics-fixture').touch();(root/'analytics.json').write_text(raw);run=subprocess.run([str(binary)],input=json.dumps(dict(root=str(root),now=1790000000,calendar='gregorian',zone='UTC',actions=[])),capture_output=True,text=True,timeout=20);self.assertEqual(run.returncode,0,run.stderr);result=json.loads(run.stdout);pair.append(result['storageError'] if result['storageError'] else normalized(result['document']))
            self.assertEqual(pair[0],pair[1],literal);type(self).cases+=1
    def test_sample_caps_error_whitelist_and_hashed_raw_utf8_identity(self):
        actions=[]
        for i in range(270):actions += [dict(op='attempt',id='é' if i%2 else 'e\u0301',source='snippet'),dict(op='complete',ok=i%3==0,ms=i,code='timeout' if i%2 else 'raw private text',exit=-1)]
        actions += [dict(op='timing',name='not-allowed',ms=2),dict(op='error',category='not-allowed',code='io'),dict(op='error',category='runtime',code='raw message'),dict(op='snapshot'),dict(op='export')]
        out=self.run_case(actions);self.assertEqual(len(out['document']['timings']['command.execute']['samples']),256);self.assertEqual(len(out['document']['events']),181)
        self.assertEqual(len(out['document']['commands']),2)
    def test_legacy_import_caps_and_second_import_do_not_duplicate(self):
        counts={f'own-{i}':i for i in range(-1,2051)}
        out=self.run_case([dict(op='import',counts=counts),dict(op='import',counts={'ignored':999}),dict(op='export')]);self.assertEqual(len(out['document']['commands']),2048)
    @classmethod
    def tearDownClass(cls):
        if folder:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            out=Path(folder);out.mkdir(parents=True,exist_ok=True);sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();sources=[rt.ROOT.parent/'Sources/RuycastApp/AnalyticsStore.swift',rt.ROOT.parent/'Sources/LauncherCore/Files.swift',Path(__file__),rt.ROOT/'tests/fixtures/analytics_write_shim.rs'];artifacts=[DIR/'reference',DIR/'main.swift',cls.shim];(out/'analytics-reference-bindings.json').write_text(json.dumps(dict(cases=cls.cases,sources={str(p.relative_to(rt.ROOT.parent)):sha(p) for p in sources},artifacts={str(p.relative_to(rt.ROOT.parent)):sha(p) for p in artifacts}),indent=2))

if __name__=='__main__':unittest.main(verbosity=2)
