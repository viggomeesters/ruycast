#!/usr/bin/env python3
"""Isolated headless fixture host: cold readiness, real CLI, active/idle sampling.
Never uses the user's profile, general clipboard, global input or visible UI.
"""
import argparse, hashlib, json, math, os, socket, subprocess, tempfile, time
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--host',type=Path,required=True);p.add_argument('--cli',type=Path,required=True);p.add_argument('--probe',type=Path,required=True);p.add_argument('--out',type=Path,required=True);p.add_argument('--active-seconds',type=float,default=600);p.add_argument('--idle-seconds',type=float,default=600);p.add_argument('--cold-starts',type=int,default=30);p.add_argument('--samples',type=int,default=1000);p.add_argument('--operation-timeout',type=float,default=5);a=p.parse_args()
if a.cold_starts<1 or a.samples<1 or any(not math.isfinite(v) or v<0 for v in [a.active_seconds,a.idle_seconds]) or not math.isfinite(a.operation_timeout) or a.operation_timeout<=0:p.error('counts must be positive; durations finite and nonnegative; operation timeout finite and positive')
a.host=a.host.resolve();a.cli=a.cli.resolve();a.probe=a.probe.resolve()
host_sha=hashlib.sha256(a.host.read_bytes()).hexdigest();cli_sha=hashlib.sha256(a.cli.read_bytes()).hexdigest();probe_sha=hashlib.sha256(a.probe.read_bytes()).hexdigest()
def verify_binaries():
 for path,digest,label in [(a.host,host_sha,'Host'),(a.cli,cli_sha,'CLI'),(a.probe,probe_sha,'Probe')]:
  if hashlib.sha256(path.read_bytes()).hexdigest()!=digest:raise RuntimeError(label+' binary changed during measurement')
def stats(values):
 s=sorted(values);return {'samples':len(s),'medianMS':s[len(s)//2],'p95MS':s[int((len(s)-1)*.95)],'p99MS':s[int((len(s)-1)*.99)] if len(s)>=1000 else None,'maxMS':s[-1]}
def save_report(result):
 a.out.parent.mkdir(parents=True,exist_ok=True);temporary=None
 try:
  with tempfile.NamedTemporaryFile('w',dir=a.out.parent,prefix='.host-result-',delete=False) as f:
   temporary=Path(f.name);json.dump(result,f,indent=2);f.write('\n');f.flush();os.fsync(f.fileno())
  os.replace(temporary,a.out)
 finally:
  if temporary is not None:temporary.unlink(missing_ok=True)
with tempfile.TemporaryDirectory(prefix='rc-h-') as folder:
 root=Path(folder);env={k:v for k,v in os.environ.items() if k not in ['RUYCAST_HOME','RUSTCAST_HOME','RUYCAST_CONFIG_DIR','RUYCAST_APP_BINARY']};env['RUYCAST_HOME']=folder;host=None;last_shutdown=None;phase='init-config';cold=[];timings={};observations={}
 def rpc(op,**args):
  deadline=time.monotonic()+a.operation_timeout
  def remaining():
   value=deadline-time.monotonic()
   if value<=0:raise TimeoutError('RPC operation deadline exceeded: '+op)
   return value
  with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as s:
   s.settimeout(remaining());s.connect(str(root/'control.sock'));s.settimeout(remaining());s.sendall(json.dumps({'op':op,**args}).encode()+b'\n');chunks=[]
   while True:
    s.settimeout(remaining())
    chunk=s.recv(65536)
    if not chunk:raise RuntimeError('Socket closed before response')
    chunks.append(chunk)
    if chunk.endswith(b'\n'):break
   obj=json.loads(b''.join(chunks));assert obj['ok'],obj;return obj
 def fixture_status():
  status=rpc('status')['result']
  assert status['headless'] and Path(status['configPath']).resolve()==config_path.resolve() and status['commands']==500, 'Benchmark escaped fixture profile'
  return status
 def start():
  global host,last_shutdown
  verify_binaries()
  began=time.monotonic();host=subprocess.Popen([str(a.host),'--headless'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);last_shutdown=None
  while time.monotonic()-began<10:
   if host.poll() is not None:raise RuntimeError('Host exited during startup')
   try:
    fixture_status()
    return (time.monotonic()-began)*1000
   except (OSError,AssertionError,ValueError):time.sleep(.002)
  raise RuntimeError('Cold host startup timed out')
 def stop(require_graceful=True):
  global last_shutdown
  if host is None:return None
  if last_shutdown is not None and last_shutdown['pid']==host.pid:
   if require_graceful and not last_shutdown['graceful']:raise RuntimeError('Host did not shut down gracefully: '+json.dumps(last_shutdown))
   return last_shutdown
  result={'pid':host.pid,'graceful':False,'forcedCleanup':False,'method':None}
  if host.poll() is None:
   try:
    rpc('quit');host.wait(timeout=a.operation_timeout);result['graceful']=host.returncode==0
   except Exception as error:
    result['quitError']=type(error).__name__+': '+str(error)
    if host.poll() is None:
     result.update(forcedCleanup=True,method='terminate')
     try:host.terminate()
     except ProcessLookupError:pass
     try:host.wait(timeout=a.operation_timeout)
     except subprocess.TimeoutExpired:
      result['method']='kill'
      try:host.kill()
      except ProcessLookupError:pass
      try:host.wait(timeout=a.operation_timeout)
      except subprocess.TimeoutExpired:result['reapTimedOut']=True
  result['exitCode']=host.poll();last_shutdown=result
  if require_graceful and not result['graceful']:raise RuntimeError('Host did not shut down gracefully: '+json.dumps(result))
  return result
 try:
  subprocess.run([str(a.host),'--init-config'],env=env,capture_output=True,check=True,timeout=a.operation_timeout)
  config_path=root/'config.json';config=json.loads(config_path.read_text());config['commands']=[{'id':f'fixture.{i}','title':f'Open Project {i}','kind':'url','target':f'https://example.com/{i}','aliases':[f'p{i}']} for i in range(500)];config_path.write_text(json.dumps(config))
  history=[{'id':f'fixture-{i}','text':f'Fixture {i}: '+'native clipboard Ω '*400,'source':'fixture','date':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime(time.time()-i))} for i in range(200)]
  (root/'clipboard.json').write_text(json.dumps(history));cold=[];cold_shutdowns=[]
  for i in range(a.cold_starts):
   phase='cold-start';cold.append(start());phase='cold-shutdown';cold_shutdowns.append(stop())
  print(json.dumps({'phase':'cold','p95MS':stats(cold)['p95MS']}),flush=True)
  phase='warm-start';start();timings={}
  for name,args in [('status',['status']),('search',['search','Project 42'])]:
   phase='cli-'+name;verify_binaries();values=[]
   for i in range(a.samples+10):
    began=time.monotonic();result=subprocess.run([str(a.cli),*args],env=env,capture_output=True,check=True,timeout=a.operation_timeout);obj=json.loads(result.stdout);assert obj['ok'];elapsed=(time.monotonic()-began)*1000
    if i>=10:values.append(elapsed)
   verify_binaries();timings[name]=stats(values);print(json.dumps({'phase':name,**timings[name]}),flush=True)
  def sample():
   row=json.loads(subprocess.check_output([str(a.probe),str(host.pid)],timeout=a.operation_timeout));row['time']=time.monotonic();return row
  def observe(seconds,active):
   verify_binaries();record={'requestedDurationS':seconds,'samples':[],'requests':0,'complete':False};observations['active' if active else 'idle']=record;rows=record['samples'];rows.append(sample());began=time.monotonic();last_sample=began;last_monitor=began;requests=0
   while time.monotonic()-began<seconds:
    if active:
     query_index=requests%100
     rpc('search',query=f'Project {query_index}');requests+=1;record['requests']=requests
     rpc('clipboard.list',query=f'Fixture {query_index}');requests+=1;record['requests']=requests
     if time.monotonic()-last_monitor>=2:rpc('monitor');requests+=1;last_monitor=time.monotonic()
    record['requests']=requests
    time.sleep(min(.1,max(.001,seconds-(time.monotonic()-began))))
    if time.monotonic()-last_sample>=5:rows.append(sample());last_sample=time.monotonic()
   verify_binaries();rows.append(sample());first,last=rows[0],rows[-1];duration=last['time']-first['time']
   record.update({'complete':True,'durationS':duration,'requests':requests,'cpuPercentOneCore':100*(last['cpuNS']-first['cpuNS'])/1e9/duration,'packageIdleWakeupsPerSecond':(last['packageIdleWakeups']-first['packageIdleWakeups'])/duration,'interruptWakeupsPerSecond':(last['interruptWakeups']-first['interruptWakeups'])/duration,'footprintStartMiB':first['footprintBytes']/1048576,'footprintEndMiB':last['footprintBytes']/1048576,'footprintMaxMiB':max(r['footprintBytes'] for r in rows)/1048576,'diskReadBytes':last['diskReadBytes']-first['diskReadBytes'],'diskWriteBytes':last['diskWriteBytes']-first['diskWriteBytes'],'logicalWriteBytes':last['logicalWriteBytes']-first['logicalWriteBytes']});return record
  phase='active';active=observe(a.active_seconds,True) if a.active_seconds>0 else None
  if active:print(json.dumps({'phase':'active','durationS':active['durationS'],'requests':active['requests']}),flush=True)
  phase='idle';idle=observe(a.idle_seconds,False) if a.idle_seconds>0 else None
  phase='idle-endpoint';fixture_status()
  endpoint=rpc('search',query='Project 42')['result']
  assert isinstance(endpoint,list) and any(isinstance(row,dict) and row.get('id')=='fixture.42' for row in endpoint), 'Post-idle search lost fixture command'
  idle_endpoint={'statusVerified':True,'searchVerified':True,'commandID':'fixture.42','idleSeconds':a.idle_seconds}
  phase='final-shutdown';final_shutdown=stop()
  verify_binaries()
  result={'ok':True,'fixtureConfigurationIsolated':True,'scope':'500 commands /200 text fixtures, headless isolated host, no hotkey/discovery/display/pasteboard or physical energy claim; active includes monitor RPC every2seconds','coldReady':stats(cold),'warmCLI':timings,'active':active,'idle':idle,'idleEndpoint':idle_endpoint,'coldShutdowns':cold_shutdowns,'finalShutdown':final_shutdown,'hostSHA256':host_sha,'cliSHA256':cli_sha,'probeSHA256':probe_sha,'operationTimeoutS':a.operation_timeout,'onDiskHostChangedDuringRun':False,'onDiskCLIChangedDuringRun':False}
  save_report(result);print(json.dumps({'phase':'done','ok':True}),flush=True)
 except Exception as error:
  cleanup=stop(require_graceful=False)
  save_report({'ok':False,'phase':phase,'errorType':type(error).__name__,'error':str(error),'cleanup':cleanup,'coldReady':stats(cold) if cold else None,'warmCLI':timings,'active':observations.get('active'),'idle':observations.get('idle'),'operationTimeoutS':a.operation_timeout,'hostSHA256':host_sha,'cliSHA256':cli_sha,'probeSHA256':probe_sha})
  raise
 finally:
  if host is not None and host.poll() is None:stop(require_graceful=False)
