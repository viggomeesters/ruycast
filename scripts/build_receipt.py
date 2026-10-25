"""Bind a successful local build to unchanged production sources and binaries."""
import argparse,hashlib,json,os,fcntl,subprocess,sys
from contextlib import contextmanager
from pathlib import Path

INPUTS=['Package.swift','rust/Cargo.toml','rust/Cargo.lock','scripts/build-native.sh','scripts/build-native-locked.sh','scripts/build_receipt.py']
TREES=['Sources','rust/src','config']
BINARIES=['.build/swift/release/RuycastApp','.build/rust/release/ruycast']

@contextmanager
def artifact_lock(root):
    path=Path(root)/'.build/artifacts.lock';path.parent.mkdir(parents=True,exist_ok=True)
    with path.open('a') as guard:
        fcntl.flock(guard,fcntl.LOCK_EX)
        try:yield
        finally:fcntl.flock(guard,fcntl.LOCK_UN)

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
    dest=root/'.build/receipts/release.json';dest.parent.mkdir(parents=True,exist_ok=True)
    temporary=dest.with_suffix('.new');temporary.write_text(json.dumps(value,indent=2)+'\n');temporary.replace(dest)
    return value

def verify(root):
    root=Path(root);path=root/'.build/receipts/release.json'
    if not path.is_file() or path.is_symlink():raise RuntimeError('Missing build receipt. Run bash scripts/build-native.sh -c release before installing.')
    value=json.loads(path.read_text())
    if value.get('schema')!='ruycast.build-receipt.v1' or value.get('sourcesSHA256')!=fingerprint(root):raise RuntimeError('Build receipt is stale; rebuild release before installing.')
    if value.get('binaries')!={n:sha(root/n) for n in BINARIES}:raise RuntimeError('Built binaries changed; rebuild release before installing.')
    return value
if __name__=='__main__':
    if len(sys.argv)>1 and sys.argv[1]=='build':
        root=Path(__file__).resolve().parents[1]
        with artifact_lock(root):
            raise SystemExit(subprocess.run(['bash','scripts/build-native-locked.sh',*sys.argv[2:]],cwd=root).returncode)
    p=argparse.ArgumentParser();p.add_argument('action',choices=['fingerprint','record','verify']);p.add_argument('--before');a=p.parse_args();root=Path(__file__).resolve().parents[1]
    try:
        if a.action=='fingerprint':print(fingerprint(root))
        elif a.action=='record':
            if not a.before:p.error('--before is required')
            print(json.dumps(record(root,a.before)))
        else:print(json.dumps(verify(root)))
    except (OSError,ValueError,RuntimeError) as e:raise SystemExit(str(e))
