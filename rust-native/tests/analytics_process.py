"""Actual AnalyticsStore Swift model/JSON codec versus Rust; disposable files only.
The Swift save stub is a disposable-file oracle, not private-storage evidence.
"""
import json,os,subprocess
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
