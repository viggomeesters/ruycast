"""Actual AnalyticsStore Swift model/JSON codec versus Rust; disposable files only.
The Swift save stub is a disposable-file oracle, not private-storage evidence.
"""
import copy,hashlib,json,os,subprocess,tempfile,time,unittest
from concurrent.futures import ThreadPoolExecutor
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
    def test_private_serde_magic_keys_remain_literal_objects(self):
        objects=[{'$serde_json::private::RawValue':value} for value in ['true','42','null','"literal"','{"enabled":false}']]+[{'$serde_json::private::Number':'1'}]
        result=self.run_case([dict(op='casts',value=v) for v in objects]);self.assertEqual(result['results'],[{'bool':None,'int':None}]*len(objects))
    def test_actual_foundation_request_casts_are_scoped_to_analytics(self):
        values=[True,False,0,1,2,-1,0.0,1.0,1.5,'1',None,9223372036854775807,-9223372036854775808,9223372036854775808,1e19]
        self.run_case([dict(op='casts',value=v) for v in values])
    def test_attempt_completion_pause_reset_and_private_export(self):
        actions=[dict(op='attempt',id='own private URL token',source='snippet',ticket='a'),dict(op='configure',enabled=False),dict(op='attempt',id='ignored',ticket='ignored'),dict(op='complete',ticket='a',ok=True,ms=12),dict(op='snapshot'),dict(op='configure',enabled=True),dict(op='attempt',id='old',ticket='old'),dict(op='reset'),dict(op='complete',ticket='old',ok=False,ms=999),dict(op='attempt',id='e\u0301',source='unexpected'),dict(op='complete',ok=False,ms=3,code='private raw failure',exit=999),dict(op='snapshot'),dict(op='export')]
        out=self.run_case(actions);self.assertNotIn('own private URL token',json.dumps(out));self.assertEqual(out['document']['totalFailures'],1)
    def test_retention_utc_hours_and_calendar_days_across_dst(self):
        import datetime
        for iso in ['2026-03-29T12:30:00+00:00','2026-10-25T12:30:00+00:00']:
            now=datetime.datetime.fromisoformat(iso).timestamp();d=self.empty();d['retentionDays']=1;hour=int(now//3600*3600);d['hours']={str(hour-delta*3600):dict(uses=1,successes=0,failures=0) for delta in [23,24,25,26]}
            for zone in ['Europe/Amsterdam','America/New_York','UTC']:
                out=self.run_case([dict(op='snapshot'),dict(op='hourly')],initial=d,now=now,zone=zone);self.assertEqual(len(out['document']['hours']),2)
    def test_schema_one_missing_and_null_hours_are_upgraded(self):
        d=self.empty();d['schema']=1;d.pop('hourlyStartedAt')
        for hours in ['absent',None,{}]:
            fixture=copy.deepcopy(d)
            if hours=='absent':fixture.pop('hours')
            else:fixture['hours']=hours
            self.run_case([dict(op='snapshot'),dict(op='hourly')],initial=fixture)
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
    def test_calendar_year_and_local_day_are_foundation_values(self):
        for calendar in ['gregorian','buddhist']:
            for zone in ['UTC','Pacific/Kiritimati','America/Los_Angeles']:
                self.run_case([dict(op='attempt'),dict(op='complete',ok=True,ms=1),dict(op='snapshot')],calendar=calendar,zone=zone)
    @classmethod
    def tearDownClass(cls):
        if folder:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            out=Path(folder);out.mkdir(parents=True,exist_ok=True);sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();sources=[rt.ROOT.parent/'Sources/RuycastApp/AnalyticsStore.swift',rt.ROOT.parent/'Sources/LauncherCore/Files.swift',Path(__file__)];artifacts=[DIR/'reference',DIR/'main.swift'];(out/'analytics-reference-bindings.json').write_text(json.dumps(dict(cases=cls.cases,sources={str(p.relative_to(rt.ROOT.parent)):sha(p) for p in sources},artifacts={str(p.relative_to(rt.ROOT.parent)):sha(p) for p in artifacts}),indent=2))

class AnalyticsRuntime(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        DIR.mkdir(parents=True,exist_ok=True);cls.shim=DIR/'analytics-write-shim.dylib'
        subprocess.run(['clang','-Wall','-Wextra','-Werror','-dynamiclib',str(rt.ROOT/'tests/analytics_write_shim.c'),'-o',str(cls.shim)],check=True,capture_output=True)
    def host(self):
        h=rt.Host(start=False);self.addCleanup(h.close);h.env['DYLD_INSERT_LIBRARIES']=str(self.shim);h.env['RUYCAST_ANALYTICS_FIXTURE_HOME']=str(h.base);h.start();return h
    def entered(self,h):
        end=time.monotonic()+3
        while time.monotonic()<end:
            if (h.base/'analytics-write-entered').exists():return
            time.sleep(.01)
        self.fail('No actual analytics publication hold observed')
    def test_idle_cli_settings_flush_wakes_owner_and_preserves_existing_file(self):
        h=self.host();started=time.monotonic();paused=h.rpc(dict(op='analytics.settings',enabled=False));self.assertTrue(paused['ok'],paused);self.assertLess(time.monotonic()-started,2);self.assertFalse(paused['result']['enabled']);self.assertFalse(json.loads((h.profile/'analytics.json').read_text())['enabled']);self.assertTrue(h.cli('analytics','resume')['result']['enabled']);self.assertTrue(h.cli('analytics','reset','--confirm')['ok']);export=h.cli('analytics','export')['result'];self.assertNotIn('commands',export);self.assertEqual(export['attempts'],0)
    def test_actual_held_io_keeps_owner_responsive_and_reports_failed_flush(self):
        h=self.host();(h.base/'hold-analytics-write').touch();(h.base/'fail-analytics-write').touch()
        with ThreadPoolExecutor(1) as pool:
            pending=pool.submit(h.rpc,dict(op='analytics.settings',enabled=False),6);self.entered(h)
            try:
                self.assertFalse(pending.done());self.assertTrue(h.rpc(dict(op='status'))['ok']);self.assertFalse(h.rpc(dict(op='analytics'))['result']['enabled'])
            finally:(h.base/'analytics-write-release').touch()
            result=pending.result(timeout=4)
        self.assertFalse(result['ok']);self.assertIn('could not be saved',result['storageError']);self.assertTrue((h.base/'analytics-write-failed').exists());(h.base/'fail-analytics-write').unlink();result=h.rpc(dict(op='analytics.settings',enabled=True));self.assertTrue(result['ok']);self.assertTrue(json.loads((h.profile/'analytics.json').read_text())['enabled'])
    def test_full_flush_queue_does_not_hide_already_applied_setting(self):
        h=self.host();(h.base/'hold-analytics-write').touch()
        with ThreadPoolExecutor(1) as pool:
            first=pool.submit(h.rpc,dict(op='analytics.settings',enabled=False),9);self.entered(h)
            try:
                for retention in range(2,17):
                    stream=h.connect();stream.sendall((json.dumps(dict(op='analytics.settings',retentionDays=retention))+'\n').encode())
                    try:
                        end=time.monotonic()+2
                        while h.rpc(dict(op='analytics'))['result']['retentionDays']!=retention:
                            self.assertLess(time.monotonic(),end);time.sleep(.005)
                    finally:stream.close()
                result=h.rpc(dict(op='analytics.settings',retentionDays=20));self.assertFalse(result['ok']);self.assertTrue(result['applied']);self.assertEqual(result['persistence'],'unknown');self.assertIn('busy',result['reason']);self.assertEqual(h.rpc(dict(op='analytics'))['result']['retentionDays'],20)
            finally:(h.base/'analytics-write-release').touch()
            self.assertTrue(first.result(timeout=4)['ok'])
        self.assertTrue(h.rpc(dict(op='analytics.settings'))['ok']);self.assertEqual(json.loads((h.profile/'analytics.json').read_text())['retentionDays'],20)
    def test_raw_analytics_request_casts_match_foundation_without_config_coercion(self):
        h=self.host()
        for value,expected in [(0,False),(1,True),(2,True),(-1,True),(1.5,True),('1',True),(None,True)]:
            result=h.rpc(dict(op='analytics.settings',enabled=value));self.assertTrue(result['ok']);self.assertIs(result['result']['enabled'],expected)
        for value in [True,1.0,1.5,'7']:
            result=h.rpc(dict(op='analytics.settings',retentionDays=value));self.assertTrue(result['ok']);self.assertEqual(result['result']['retentionDays'],1)
        self.assertFalse(h.rpc(dict(op='analytics.settings',retentionDays=False))['ok']);self.assertFalse(h.rpc(dict(op='analytics.reset',confirmed=0))['ok']);self.assertTrue(h.rpc(dict(op='analytics.reset',confirmed=1))['ok'])
    def test_shutdown_does_not_confirm_a_flush_that_is_still_held(self):
        h=self.host();(h.base/'hold-analytics-write').touch();(h.base/'fail-analytics-write').touch()
        with ThreadPoolExecutor(1) as pool:
            pending=pool.submit(h.rpc,dict(op='analytics.settings',enabled=False),9);self.entered(h)
            try:
                self.assertTrue(h.rpc(dict(op='quit'))['ok']);result=pending.result(timeout=5);self.assertFalse(result['ok']);self.assertTrue(result['applied']);self.assertEqual(result['persistence'],'unknown');self.assertIsNone(h.process.poll());second=subprocess.run([str(rt.HOST),'--headless-test'],env=h.env,capture_output=True,text=True,timeout=3);self.assertNotEqual(second.returncode,0);self.assertIn('already running',second.stdout)
            finally:(h.base/'analytics-write-release').touch()
        out,err=h.process.communicate(timeout=5);self.assertEqual(h.process.returncode,0,out.decode()+err.decode());self.assertTrue((h.base/'analytics-write-failed').exists())
    @classmethod
    def tearDownClass(cls):
        if folder:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            p=Path(folder)/'analytics-reference-bindings.json';d=json.loads(p.read_text());sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();source=rt.ROOT/'tests/analytics_write_shim.c';d['sources'][str(source.relative_to(rt.ROOT.parent))]=sha(source);d['artifacts'][str(cls.shim.relative_to(rt.ROOT.parent))]=sha(cls.shim);p.write_text(json.dumps(d,indent=2))

if __name__=='__main__':unittest.main(verbosity=2)
