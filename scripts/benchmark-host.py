#!/usr/bin/env python3
"""Isolated headless fixture host: cold readiness, real CLI, active/idle sampling.
Never uses the user's profile, general clipboard, global input or visible UI.
"""
import argparse, hashlib, json, os, socket, subprocess, tempfile, time
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--host',type=Path,required=True);p.add_argument('--cli',type=Path,required=True);p.add_argument('--probe',type=Path,required=True);p.add_argument('--out',type=Path,required=True);p.add_argument('--active-seconds',type=float,default=600);p.add_argument('--idle-seconds',type=float,default=600);p.add_argument('--cold-starts',type=int,default=30);p.add_argument('--samples',type=int,default=1000);a=p.parse_args()
a.host=a.host.resolve();a.cli=a.cli.resolve();a.probe=a.probe.resolve()
host_sha=hashlib.sha256(a.host.read_bytes()).hexdigest();cli_sha=hashlib.sha256(a.cli.read_bytes()).hexdigest()
def verify_binaries():
 for path,digest,label in [(a.host,host_sha,'Host'),(a.cli,cli_sha,'CLI')]:
  if hashlib.sha256(path.read_bytes()).hexdigest()!=digest:raise RuntimeError(label+' binary changed during measurement')
def stats(values):
 s=sorted(values);return {'samples':len(s),'medianMS':s[len(s)//2],'p95MS':s[int((len(s)-1)*.95)],'p99MS':s[int((len(s)-1)*.99)] if len(s)>=1000 else None,'maxMS':s[-1]}
with tempfile.TemporaryDirectory(prefix='rc-h-') as folder:
 root=Path(folder);env={k:v for k,v in os.environ.items() if k not in ['RUYCAST_HOME','RUSTCAST_HOME','RUYCAST_CONFIG_DIR','RUYCAST_APP_BINARY']};env['RUYCAST_HOME']=folder;host=None
 def rpc(op,**args):
  with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as s:
   s.settimeout(5);s.connect(str(root/'control.sock'));s.sendall(json.dumps({'op':op,**args}).encode()+b'\n');chunks=[]
   while True:
    chunk=s.recv(65536)
    if not chunk:raise RuntimeError('Socket closed before response')
    chunks.append(chunk)
    if chunk.endswith(b'\n'):break
   obj=json.loads(b''.join(chunks));assert obj['ok'],obj;return obj
 def start():
  global host
  verify_binaries()
  began=time.monotonic();host=subprocess.Popen([str(a.host),'--headless'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
  while time.monotonic()-began<10:
   if host.poll() is not None:raise RuntimeError('Host exited during startup')
   try:
    status=rpc('status')['result']
    assert status['headless'] and Path(status['configPath']).resolve()==config_path.resolve() and status['commands']==500, 'Benchmark escaped fixture profile'
    return (time.monotonic()-began)*1000
   except (OSError,AssertionError,ValueError):time.sleep(.002)
  raise RuntimeError('Cold host startup timed out')
 def stop():
  if host is not None and host.poll() is None:
   try:rpc('quit');host.wait(timeout=5)
   except Exception:host.terminate();host.wait(timeout=5)
 try:
  subprocess.run([str(a.host),'--init-config'],env=env,capture_output=True,check=True)
  config_path=root/'config.json';config=json.loads(config_path.read_text());config['commands']=[{'id':f'fixture.{i}','title':f'Open Project {i}','kind':'url','target':f'https://example.com/{i}','aliases':[f'p{i}']} for i in range(500)];config_path.write_text(json.dumps(config))
  history=[{'id':f'fixture-{i}','text':f'Fixture {i}: '+'native clipboard Ω '*400,'source':'fixture','date':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime(time.time()-i))} for i in range(200)]
  (root/'clipboard.json').write_text(json.dumps(history));cold=[]
  for i in range(a.cold_starts):cold.append(start());stop()
  print(json.dumps({'phase':'cold','p95MS':stats(cold)['p95MS']}),flush=True)
  start();timings={}
  for name,args in [('status',['status']),('search',['search','Project 42'])]:
   verify_binaries();values=[]
   for i in range(a.samples+10):
    began=time.monotonic();result=subprocess.run([str(a.cli),*args],env=env,capture_output=True,check=True);obj=json.loads(result.stdout);assert obj['ok'];elapsed=(time.monotonic()-began)*1000
    if i>=10:values.append(elapsed)
   verify_binaries();timings[name]=stats(values);print(json.dumps({'phase':name,**timings[name]}),flush=True)
  def sample():
   row=json.loads(subprocess.check_output([str(a.probe),str(host.pid)]));row['time']=time.monotonic();return row
  def observe(seconds,active):
   verify_binaries();rows=[sample()];began=time.monotonic();last_sample=began;last_monitor=began;requests=0
   while time.monotonic()-began<seconds:
    if active:
     rpc('search',query=f'Project {requests%100}');rpc('clipboard.list',query=f'Fixture {requests%100}');requests+=2
     if time.monotonic()-last_monitor>=2:rpc('monitor');requests+=1;last_monitor=time.monotonic()
    time.sleep(min(.1,max(.001,seconds-(time.monotonic()-began))))
    if time.monotonic()-last_sample>=5:rows.append(sample());last_sample=time.monotonic()
   verify_binaries();rows.append(sample());first,last=rows[0],rows[-1];duration=last['time']-first['time']
   return {'durationS':duration,'requests':requests,'cpuPercentOneCore':100*(last['cpuNS']-first['cpuNS'])/1e9/duration,'packageIdleWakeupsPerSecond':(last['packageIdleWakeups']-first['packageIdleWakeups'])/duration,'interruptWakeupsPerSecond':(last['interruptWakeups']-first['interruptWakeups'])/duration,'footprintStartMiB':first['footprintBytes']/1048576,'footprintEndMiB':last['footprintBytes']/1048576,'footprintMaxMiB':max(r['footprintBytes'] for r in rows)/1048576,'diskReadBytes':last['diskReadBytes']-first['diskReadBytes'],'diskWriteBytes':last['diskWriteBytes']-first['diskWriteBytes'],'logicalWriteBytes':last['logicalWriteBytes']-first['logicalWriteBytes'],'samples':rows}
  active=observe(a.active_seconds,True) if a.active_seconds>0 else None
  if active:print(json.dumps({'phase':'active','durationS':active['durationS'],'requests':active['requests']}),flush=True)
  idle=observe(a.idle_seconds,False) if a.idle_seconds>0 else None
  verify_binaries()
  result={'ok':True,'fixtureConfigurationIsolated':True,'scope':'500 commands /200 text fixtures, headless isolated host, no hotkey/discovery/display/pasteboard or physical energy claim; active includes monitor RPC every2seconds','coldReady':stats(cold),'warmCLI':timings,'active':active,'idle':idle,'hostSHA256':host_sha,'cliSHA256':cli_sha,'onDiskHostChangedDuringRun':False,'onDiskCLIChangedDuringRun':False}
  a.out.parent.mkdir(parents=True,exist_ok=True);a.out.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'phase':'done','ok':True}),flush=True)
 finally:stop()
