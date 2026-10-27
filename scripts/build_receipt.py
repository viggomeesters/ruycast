"""Bind a successful local build to unchanged production sources and binaries."""
import argparse,hashlib,json,os,fcntl,subprocess,sys,time
import prepare_release
from contextlib import contextmanager
from pathlib import Path

INPUTS=['Package.swift','rust/Cargo.toml','rust/Cargo.lock','scripts/build-native.sh','scripts/build-native-locked.sh','scripts/build_receipt.py','scripts/prepare_release.py']
TREES=['Sources','rust/src','config']
BINARIES=['.build/swift/release/RuycastApp','.build/rust/release/ruycast']

@contextmanager
def artifact_lock(root):
    path=Path(root)/'.build/artifacts.lock';path.parent.mkdir(parents=True,exist_ok=True)
    with path.open('a') as guard:
        fcntl.flock(guard,fcntl.LOCK_EX)
        # Close this descriptor on exit; an explicit LOCK_UN would also unlock
        # the inherited open-file description while a child still consumes it.
        yield guard.fileno()

def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def fingerprint(root):
    root=Path(root); files={name for name in INPUTS if (root/name).is_file()}
    for tree in TREES:
        files.update(str(p.relative_to(root)) for p in (root/tree).rglob('*') if p.is_file())
    rows=[]
    for name in sorted(files):
        path=root/name
        if path.is_symlink():raise RuntimeError('Build input may not be a symlink: '+name)
        rows.append([name,sha(path)])
    return hashlib.sha256(json.dumps(rows,separators=(',',':')).encode()).hexdigest()
def record(root,before):
    root=Path(root)
    if fingerprint(root)!=before:raise RuntimeError('Sources changed during build; rebuild before installing.')
    value={'schema':'ruycast.build-receipt.v1','sourcesSHA256':before,'binaries':{n:sha(root/n) for n in BINARIES}}
    preparation=root/'.build/receipts/release-preparation.json'
    if preparation.is_file():
        value['releasePreparation']=json.loads(preparation.read_text())
        if value['releasePreparation']['binaries']!=value['binaries']:raise RuntimeError('Release preparation differs from binaries.')
        prepare_release.verify_symbols(root,value['releasePreparation'])
    value['build']={'swiftArguments':json.loads(os.environ.get('RUYCAST_BUILD_SWIFT_ARGUMENTS_JSON','[]')),'toolchains':{cmd:subprocess.check_output(cmd.split(),text=True,stderr=subprocess.STDOUT).strip() for cmd in ['swiftc --version','rustc --version','cargo --version']},'environmentOverrides':{key:os.environ[key] for key in ['CARGO_PROFILE_RELEASE_LTO','CARGO_PROFILE_RELEASE_CODEGEN_UNITS','CARGO_PROFILE_RELEASE_OPT_LEVEL','CARGO_PROFILE_RELEASE_PANIC','RUSTFLAGS','TOOLCHAINS','DEVELOPER_DIR','SDKROOT'] if key in os.environ}}
    dest=root/'.build/receipts/release.json';dest.parent.mkdir(parents=True,exist_ok=True)
    temporary=dest.with_suffix('.new');temporary.write_text(json.dumps(value,indent=2)+'\n');temporary.replace(dest)
    return value

def verify(root):
    root=Path(root);path=root/'.build/receipts/release.json'
    if not path.is_file() or path.is_symlink():raise RuntimeError('Missing build receipt. Run bash scripts/build-native.sh -c release before installing.')
    value=json.loads(path.read_text())
    if value.get('schema')!='ruycast.build-receipt.v1' or value.get('sourcesSHA256')!=fingerprint(root):raise RuntimeError('Build receipt is stale; rebuild release before installing.')
    if value.get('binaries')!={n:sha(root/n) for n in BINARIES}:raise RuntimeError('Built binaries changed; rebuild release before installing.')
    if 'releasePreparation' in value:prepare_release.verify_symbols(root,value['releasePreparation'])
    return value
# Public entrypoints own one lease through their last artifact consumer. The
# descriptor is inherited by synchronous child commands, so a crashed controller
# cannot unlock artifacts while its verification shell is still using them.
PIPELINES={
    'check':('check-locked.sh',['-Xswiftc','-warnings-as-errors'],'debug'),
    'performance':('check-performance-locked.sh',['-c','release'],'release'),
    'settings':('check-settings-locked.sh',['-Xswiftc','-warnings-as-errors'],'debug'),
}

def verification_fingerprint(root):
    root=Path(root);rows=[['product',fingerprint(root)]]
    for path in sorted(root.iterdir()):
        if path.name=='.git':continue
        if path.is_symlink():raise RuntimeError('Verification input may not be a symlink: '+path.name)
        if path.is_file():rows.append([path.name,sha(path)])
    for tree in ['Tests','scripts','docs','assets','.github']:
        for path in sorted((root/tree).rglob('*')):
            if '__pycache__' in path.parts:continue
            if path.is_symlink():raise RuntimeError('Verification input may not be a symlink: '+str(path.relative_to(root)))
            if path.is_file():rows.append([str(path.relative_to(root)),sha(path)])
    return hashlib.sha256(json.dumps(rows,separators=(',',':')).encode()).hexdigest()

def checked_artifacts(root,kind):
    configuration=PIPELINES[kind][2]
    names=[f'.build/swift/{configuration}/RuycastApp','.build/rust/release/ruycast']
    if configuration=='debug':
        names.extend(['.build/swift/debug/LauncherCoreChecks','.build/swift/debug/CRuycastCore.build/module.modulemap'])
        modules=Path(root)/'.build/swift/debug/Modules'
        if not modules.is_dir():raise RuntimeError('Missing verification modules.')
        names.extend(str(p.relative_to(root)) for p in sorted(modules.rglob('*')) if p.is_file())
    rows={}
    for name in names:
        path=Path(root)/name
        if not path.is_file() or path.is_symlink():raise RuntimeError('Missing or linked verification artifact: '+name)
        rows[name]=sha(path)
    return rows

def command(root,args,lease,env=None):
    return subprocess.run(args,cwd=root,env=env,pass_fds=(lease,)).returncode

def build_locked(root,args,lease):
    env={**os.environ,'RUYCAST_BUILD_SWIFT_ARGUMENTS_JSON':json.dumps(args)}
    return command(root,['bash','scripts/build-native-locked.sh',*args],lease,env)

def run_pipeline(root,kind,lease,lock_wait=0):
    root=Path(root);script,args,_=PIPELINES[kind]
    receipt=root/'.build/receipts'/(kind+'.json')
    receipt.parent.mkdir(parents=True,exist_ok=True)
    # A failed current attempt must not leave an older pass looking current.
    receipt.unlink(missing_ok=True)
    before=verification_fingerprint(root);steps=[]
    def step(name,run):
        began=time.monotonic();result=run()
        steps.append({'stage':name,'elapsedSeconds':time.monotonic()-began,'exitCode':result})
        return result
    if kind=='check':
        result=step('rustTests',lambda:command(root,['cargo','test','--manifest-path','rust/Cargo.toml','--target-dir','.build/rust','--locked'],lease))
        if result:return result
    result=step('build',lambda:build_locked(root,args,lease))
    if result:return result
    artifacts=checked_artifacts(root,kind)
    result=step('verification',lambda:command(root,['bash','scripts/'+script],lease))
    if result:return result
    if verification_fingerprint(root)!=before:
        raise RuntimeError('Sources or test inputs changed during verification; rerun the complete pipeline.')
    if checked_artifacts(root,kind)!=artifacts:
        raise RuntimeError('Built artifacts changed during verification; rerun the complete pipeline.')
    value={'schema':'ruycast.verification-receipt.v1','pipeline':kind,'status':'passed',
           'verificationInputsSHA256':before,'binaries':artifacts,'steps':steps,'lockWaitSeconds':lock_wait}
    temporary=receipt.with_suffix('.new');temporary.write_text(json.dumps(value,indent=2)+'\n');temporary.replace(receipt)
    return 0

def verify_pipeline(root,kind):
    path=Path(root)/'.build/receipts'/(kind+'.json')
    if not path.is_file() or path.is_symlink():raise RuntimeError('Missing verification receipt; run '+kind+' again.')
    value=json.loads(path.read_text())
    if value.get('schema')!='ruycast.verification-receipt.v1' or value.get('pipeline')!=kind or value.get('status')!='passed':
        raise RuntimeError('Invalid verification receipt; run '+kind+' again.')
    if value.get('verificationInputsSHA256')!=verification_fingerprint(root) or value.get('binaries')!=checked_artifacts(root,kind):
        raise RuntimeError('Verification receipt is stale; run '+kind+' again.')
    return value

if __name__=='__main__':
    root=Path(__file__).resolve().parents[1]
    try:
        action=sys.argv[1] if len(sys.argv)>1 else ''
        if action=='build':
            with artifact_lock(root) as lease:
                raise SystemExit(build_locked(root,sys.argv[2:],lease))
        if action in PIPELINES:
            if len(sys.argv)!=2:raise RuntimeError('Verification pipelines do not accept positional arguments.')
            began=time.monotonic()
            with artifact_lock(root) as lease:
                raise SystemExit(run_pipeline(root,action,lease,time.monotonic()-began))
        if action.startswith('verify-') and action[7:] in PIPELINES:
            if len(sys.argv)!=2:raise RuntimeError('Verification receipt checks do not accept positional arguments.')
            with artifact_lock(root):print(json.dumps(verify_pipeline(root,action[7:])))
            raise SystemExit(0)
        p=argparse.ArgumentParser();p.add_argument('action',choices=['fingerprint','record','verify']);p.add_argument('--before');a=p.parse_args()
        if a.action=='fingerprint':print(fingerprint(root))
        elif a.action=='record':
            if not a.before:p.error('--before is required')
            print(json.dumps(record(root,a.before)))
        else:print(json.dumps(verify(root)))
    except (OSError,ValueError,RuntimeError) as e:raise SystemExit(str(e))
