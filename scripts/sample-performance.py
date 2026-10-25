#!/usr/bin/env python3
"""Read-only interval sampling. No clipboard contents, global keys or app focus."""
import argparse,json,subprocess,time
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--pid',type=int,required=True);p.add_argument('--seconds',type=float,default=600);p.add_argument('--probe',type=Path,required=True);p.add_argument('--out',type=Path,required=True);p.add_argument('--label',required=True);a=p.parse_args()
if not (0<a.seconds<=86400):p.error("--seconds must be between0 and86400")
samples=[];started=time.monotonic()
try:
    while True:
        r=subprocess.run([str(a.probe),str(a.pid)],capture_output=True,text=True,check=True);row=json.loads(r.stdout);row['elapsedS']=time.monotonic()-started;samples.append(row)
        if row['elapsedS']>=a.seconds:break
        time.sleep(min(5,max(.01,a.seconds-row['elapsedS'])))
except Exception as exc:
    error=type(exc).__name__
else:error=None
if not samples:
    a.out.parent.mkdir(parents=True,exist_ok=True);a.out.write_text(json.dumps({'label':a.label,'error':error,'sampleCount':0})+'\n');raise SystemExit(1)
first,last=samples[0],samples[-1];duration=max(.001,last['elapsedS']-first['elapsedS'])
result={'label':a.label,'durationS':duration,'sampleCount':len(samples),'error':error,'scope':'passive observed host; user clipboard/commands and system load may vary; no energy or display-latency claim','cpuPercentOneCore':100*(last['cpuNS']-first['cpuNS'])/1e9/duration,'footprintStartMiB':first['footprintBytes']/1048576,'footprintEndMiB':last['footprintBytes']/1048576,'footprintMaxMiB':max(x['footprintBytes'] for x in samples)/1048576,'packageIdleWakeupsPerSecond':(last['packageIdleWakeups']-first['packageIdleWakeups'])/duration,'interruptWakeupsPerSecond':(last['interruptWakeups']-first['interruptWakeups'])/duration,'diskReadBytes':last['diskReadBytes']-first['diskReadBytes'],'diskWriteBytes':last['diskWriteBytes']-first['diskWriteBytes'],'logicalWriteBytes':last['logicalWriteBytes']-first['logicalWriteBytes'],'samples':samples}
a.out.parent.mkdir(parents=True,exist_ok=True);a.out.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({k:v for k,v in result.items() if k!='samples'}))
