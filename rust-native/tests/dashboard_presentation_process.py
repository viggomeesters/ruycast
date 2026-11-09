"""Execute frozen Swift presentation expressions against the Rust surface.
Swift is a test oracle only. No native window or personal data is accessed.
"""
import datetime,hashlib,json,os,re,subprocess,tempfile,unittest
from pathlib import Path
import analytics_process as a

DIR=a.rt.ROOT/'target/dashboard-reference'

def body(source,marker):
    start=source.index(marker);start=source.index('{',start);depth=1;i=start+1
    while depth:
        depth+=(source[i]=='{')-(source[i]=='}');i+=1
    return source[start+1:i-1]

class Presentation(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        DIR.mkdir(parents=True,exist_ok=True)
        root=a.rt.ROOT.parent
        src=(root/'Sources/RuycastApp/AnalyticsDashboard.swift').read_text()
        store=(root/'Sources/RuycastApp/AnalyticsStore.swift').read_text()
        store='\n'.join(x for x in store.splitlines() if x not in ['import Foundation','import Combine','import CryptoKit','import Darwin','import LauncherCore','import LauncherModels'])
        codec=(root/'Sources/LauncherCore/Files.swift').read_text().splitlines()
        encode=next(x.strip().replace('public static','static',1) for x in codec if 'public static func encode<' in x)
        decode=next(x.strip().replace('public static','static',1) for x in codec if 'public static func decode<' in x)
        names=body(src,'private var names:')
        sort=next(x.strip().removeprefix('let rows=') for x in src.splitlines() if 'let rows=d.commands.sorted' in x)
        days=body(src,'private var dayKeys:').replace('let calendar=Calendar.current,today=calendar.startOfDay(for:Date())','let calendar=calendar,today=calendar.startOfDay(for:now)')
        helpers='\n'.join('func '+x.split('private func ',1)[1] for x in src.splitlines() if any('private func '+n in x for n in ['ms(','title(','reason(']))
        oracle=r'''
struct Command:Decodable {let id:String;let title:String}
func surface(_ d:AnalyticsDocument,_ commands:[Command],_ now:Date,_ calendar:Calendar)->[String:Any] {
 __HELPERS__
 let names:[String:String] = {__NAMES__}()
 let rows=__SORT__
 let dayKeys:[String]={__DAYS__}()
 let formatter=DateFormatter();formatter.locale=Locale.current;formatter.calendar=calendar;formatter.timeZone=calendar.timeZone
 func format(_ date:Date,_ pattern:String)->String {formatter.dateFormat=pattern;return formatter.string(from:date)}
 let hours=d.hourlyActivity(now:now).map {point -> [String:Any] in ["date":point.date.timeIntervalSince1970,"label":format(point.date,"HH:mm"),"accessibility":format(point.date,"MMM d, HH:mm z"),"uses":point.activity.map{$0.uses} as Any? ?? NSNull()]}
 let errors=Array(d.events.reversed().prefix(20)).map{event -> [String:Any] in ["name":event.commandID.flatMap{names[$0]} ?? event.category.capitalized,"reason":event.exitStatus.map{"Script exited with status \($0)"} ?? reason(event.code),"date":format(event.date,"MMM d, yyyy HH:mm")]}
 return ["uses":d.totalAttempts+d.priorSuccesses,"successes":d.totalSuccesses+d.priorSuccesses,"failures":d.totalFailures,"prior_successes":d.priorSuccesses,"omitted_commands":d.omittedCommands,"hours":hours,"days":d.days.isEmpty ? []:dayKeys.map{["label":$0,"uses":d.days[$0].map{$0.uses} as Any? ?? NSNull()]},"commands":Array(rows.prefix(30)).map{id,row in ["id":id,"name":names[id] ?? "Removed command · \(id.prefix(6))","uses":row.uses,"failures":row.failures,"p95":ms(row.timing.p95MS)]},"timings":d.timings.keys.sorted().map{[title($0),ms(d.timings[$0]?.p95MS)]},"errors":errors]
}
'''.replace('__HELPERS__',helpers).replace('__NAMES__',names).replace('__SORT__',sort).replace('__DAYS__',days)
        main=a.SWIFT.replace('__ACTUAL_SOURCE__',store+'\n'+oracle).replace('__ENCODE__',encode).replace('__DECODE__',decode)
        main=main.replace(' case "snapshot":',' case "dashboard":let commands=try JSONDecoder().decode([Command].self,from:JSONSerialization.data(withJSONObject:a["commands"]!));results.append(surface(store.snapshot,commands,clock.date(),calendar))\n case "snapshot":')
        (DIR/'main.swift').write_text(main)
        run=subprocess.run(['swiftc',str(DIR/'main.swift'),'-o',str(DIR/'reference')],capture_output=True,text=True);assert run.returncode==0,run.stderr
        cls.bindings={'sources':{str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [root/'Sources/RuycastApp/AnalyticsDashboard.swift',root/'Sources/RuycastApp/AnalyticsStore.swift',Path(__file__)]},'generatedSHA256':hashlib.sha256(main.encode()).hexdigest(),'cases':0}
    def compare(self,actions,now=1790000000,zone='UTC',calendar='gregorian'):
        result=[]
        for binary in [a.PROBE,DIR/'reference']:
            with tempfile.TemporaryDirectory(prefix='surface-',dir=DIR) as own:
                root=Path(own);(root/'.ruycast-analytics-fixture').touch()
                run=subprocess.run([str(binary)],input=json.dumps(dict(root=str(root),now=now,zone=zone,calendar=calendar,actions=actions)),capture_output=True,text=True,timeout=30)
                self.assertEqual(run.returncode,0,run.stderr);result.append(json.loads(run.stdout)['results'][-1])
        rust,swift=result
        for p in rust['days']:p.pop('date');p.pop('accessibility')
        for e in rust['errors']:e.pop('id')
        self.assertEqual({k:rust[k] for k in swift},swift)
        type(self).bindings['cases']+=1;return rust
    def test_populated_top30_hash_ties_last_wins_disabled_names_samples_errors_and_legacy(self):
        commands=[dict(id='é',title='Composed',kind='script',target='/usr/bin/true'),dict(id='e\u0301',title='Decomposed',kind='script',target='/usr/bin/true',enabled=False),dict(id='é',title='Last wins disabled',kind='script',target='/usr/bin/true',enabled=False),dict(id='builtin.dashboard',title='Ruycast Dashboard',kind='builtin',target='analytics'),dict(id='discovered.app',title='Discovered app',kind='app',target='/Own.app')]
        actions=[dict(op='import',counts={'legacy':14})]
        for i in range(40):actions += [dict(op='attempt',id=f'command-{i:02}'),dict(op='complete',ok=i%2==0,ms=i,code='timeout')]
        for id in ['é','e\u0301','builtin.dashboard','discovered.app']:
            for ms in [0.1,0.5,0.7,100]:actions += [dict(op='attempt',id=id),dict(op='complete',ok=False,ms=ms,code='permission',exit=2 if id=='discovered.app' else None)]
        for n in range(270):actions.append(dict(op='timing',name='search.model',ms=n/10))
        actions.append(dict(op='dashboard',commands=commands));r=self.compare(actions)
        self.assertEqual(len(r['commands']),30);self.assertEqual(len(r['errors']),20);self.assertEqual(r['prior_successes'],14)
        self.assertIn('Last wins disabled',[r['name'] for r in r['commands']]);self.assertIn('Decomposed',[r['name'] for r in r['commands']])
        self.assertEqual(next(r for r in r['commands'] if r['name']=='Last wins disabled')['p95'],'0.70 ms')
    def test_elapsed_utc_hours_unknown_calendar_start_retention_dst_and_year(self):
        for iso in ['2026-03-29T12:30:00+00:00','2026-10-25T12:30:00+00:00']:
            now=datetime.datetime.fromisoformat(iso).timestamp()
            for calendar in ['gregorian','buddhist']:
                for zone in ['Europe/Amsterdam','America/New_York','UTC']:
                    actions=[dict(op='configure',retentionDays=14),dict(op='attempt',id='own',now=now-4*86400),dict(op='complete',ok=True,ms=0.01),dict(op='attempt',id='own',now=now-25*3600),dict(op='complete',ok=False,ms=1),dict(op='dashboard',commands=[],now=now)]
                    r=self.compare(actions,now=now-4*86400,zone=zone,calendar=calendar);self.assertEqual(len(r['hours']),24);self.assertEqual(len(r['days']),5)
                    stamps=[p['date'] for p in r['hours']];self.assertTrue(all(b-a==3600 for a,b in zip(stamps,stamps[1:])))
                    self.assertTrue(all(p['uses'] is not None for p in r['hours']))
        r=self.compare([dict(op='dashboard',commands=[])]);self.assertEqual(r['commands'],[]);self.assertEqual(r['days'],[]);self.assertEqual(sum(p['uses'] is None for p in r['hours']),23)
    @classmethod
    def tearDownClass(cls):
        if path:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            p=Path(path);p.mkdir(parents=True,exist_ok=True);(p/'dashboard-presentation-bindings.json').write_text(json.dumps(cls.bindings,indent=2))
if __name__=='__main__':unittest.main(verbosity=2)
