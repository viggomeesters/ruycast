"""Strip release-local symbols while retaining exact code and private diagnostics."""
import hashlib,json,os,re,shutil,struct,subprocess,tempfile
from pathlib import Path

HOST='.build/swift/release/RuycastApp'
CLI='.build/rust/release/ruycast'

def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def uuid(path):
    result=subprocess.run(['xcrun','dwarfdump','--uuid',str(path)],text=True,capture_output=True,check=True)
    values=re.findall(r'UUID: ([A-Fa-f0-9-]+) ',result.stdout)
    if len(values)!=1:raise RuntimeError('Expected one native architecture and matching debug UUID: '+str(path))
    return values[0].upper()
def sections(path):
    data=Path(path).read_bytes()
    if len(data)<32 or struct.unpack_from('<I',data)[0]!=0xfeedfacf:raise RuntimeError('Expected a native 64-bit Mach-O release executable.')
    offset=32;result={}
    for _ in range(struct.unpack_from('<I',data,16)[0]):
        cmd,size=struct.unpack_from('<II',data,offset)
        if size<8 or offset+size>len(data):raise RuntimeError('Invalid Mach-O load command.')
        if cmd==0x19:
            count=struct.unpack_from('<I',data,offset+64)[0]
            if 72+count*80>size:raise RuntimeError('Invalid Mach-O sections.')
            for i in range(count):
                o=offset+72+i*80;name=data[o:o+16].split(b'\0')[0].decode();segment=data[o+16:o+32].split(b'\0')[0].decode()
                length=struct.unpack_from('<Q',data,o+40)[0];start=struct.unpack_from('<I',data,o+48)[0];flags=struct.unpack_from('<I',data,o+64)[0]
                if length and start and flags&0xff not in [1,12,18]:
                    if start+length>len(data):raise RuntimeError('Invalid Mach-O section bounds.')
                    result[segment+','+name]=hashlib.sha256(data[start:start+length]).hexdigest()
        offset+=size
    if not result:raise RuntimeError('Release executable has no backed sections.')
    return result

def verify_artifact_symbols(root,name,row):
    root=Path(root);symbol=root/row['unstrippedPath']
    if not symbol.is_file() or sha(symbol)!=row['unstrippedSHA256']:raise RuntimeError('Retained release symbols changed: '+name)
    if uuid(root/name)!=row['uuid'] or uuid(symbol)!=row['uuid']:raise RuntimeError('Retained release UUID changed: '+name)
    if name==HOST:
        bundle=root/row['dSYMPath'];expected=row['dSYMSHA256']
        actual={str(p.relative_to(bundle)):sha(p) for p in bundle.rglob('*') if p.is_file()}
        if actual!=expected or uuid(bundle)!=row['uuid']:raise RuntimeError('Retained host dSYM changed.')

def verify_symbols(root,record):
    for name,row in record['artifacts'].items():verify_artifact_symbols(root,name,row)

def atomic_json(path,value):
    path.parent.mkdir(parents=True,exist_ok=True);temporary=path.with_suffix('.new')
    with temporary.open('w') as handle:
        handle.write(json.dumps(value,indent=2)+'\n');handle.flush();os.fsync(handle.fileno())
    os.replace(temporary,path)
    descriptor=os.open(path.parent,os.O_RDONLY)
    try:os.fsync(descriptor)
    finally:os.close(descriptor)

def recover(root):
    root=Path(root);pending=root/'.build/receipts/release-preparation.pending.json'
    if not pending.is_file():return
    intent=json.loads(pending.read_text());receipt=root/'.build/receipts/release-preparation.json'
    if intent.get('schema')!='ruycast.release-preparation-intent.v1':raise RuntimeError('Unknown release preparation recovery intent.')
    record=intent['preparation']
    if receipt.is_file() and json.loads(receipt.read_text())==record and all(sha(root/n)==h for n,h in record['binaries'].items()):
        verify_symbols(root,record);pending.unlink();return
    changes=intent['changes']
    # Validate every recovery source before touching either executable. A process
    # interruption may leave either known state; unknown edits fail closed.
    for name,before in changes.items():
        if name not in [HOST,CLI]:raise RuntimeError('Unknown recovery artifact.')
        if sha(root/name) not in [before,record['binaries'][name]]:raise RuntimeError('Release artifact changed during interrupted preparation.')
        original=root/record['artifacts'][name]['unstrippedPath']
        if sha(original)!=before:raise RuntimeError('Release recovery original changed.')
    with tempfile.TemporaryDirectory(prefix='recover-',dir=root/'.build') as folder:
        staged=[]
        for name in changes:
            original=root/record['artifacts'][name]['unstrippedPath'];candidate=Path(folder)/Path(name).name
            shutil.copy2(original,candidate);staged.append((candidate,root/name))
        for candidate,destination in staged:os.replace(candidate,destination)
    pending.unlink()

def prepare(root):
    root=Path(root);recover(root);receipt=root/'.build/receipts/release-preparation.json'
    pending=root/'.build/receipts/release-preparation.pending.json'
    current={n:sha(root/n) for n in [HOST,CLI]}
    previous=json.loads(receipt.read_text()) if receipt.is_file() else {}
    if previous.get('binaries')==current:
        verify_symbols(root,previous);return previous
    record={'schema':'ruycast.release-preparation.v1','stripArguments':['-S','-x'],'artifacts':{},'binaries':{}}
    with tempfile.TemporaryDirectory(prefix='strip-',dir=root/'.build') as folder:
        staged=[];changes={}
        for name in [HOST,CLI]:
            # Cargo and SwiftPM cache independently. A fresh host must not turn
            # an unchanged, already stripped CLI into its own diagnostic original.
            if previous.get('binaries',{}).get(name)==current[name]:
                row=previous['artifacts'][name];verify_artifact_symbols(root,name,row)
                record['artifacts'][name]=row;record['binaries'][name]=current[name];continue
            source=root/name;identity=uuid(source);directory=root/'.build/symbols'/current[name]
            dsym=root/(HOST+'.dSYM')
            if name==HOST and uuid(dsym)!=identity:raise RuntimeError('Host dSYM does not match executable; rebuild before stripping.')
            directory.mkdir(parents=True,exist_ok=True);original=directory/source.name
            if original.exists():
                if sha(original)!=current[name]:raise RuntimeError('Existing retained executable differs.')
            else:shutil.copy2(source,original)
            with original.open('rb') as handle:os.fsync(handle.fileno())
            row={'uuid':identity,'unstrippedPath':str(original.relative_to(root)),'unstrippedSHA256':current[name],'unstrippedBytes':source.stat().st_size}
            if name==HOST:
                retained=directory/'RuycastApp.dSYM'
                if not retained.exists():shutil.copytree(dsym,retained)
                if uuid(retained)!=identity:raise RuntimeError('Retained dSYM does not match host.')
                row.update(dSYMPath=str(retained.relative_to(root)),dSYMSHA256={str(p.relative_to(retained)):sha(p) for p in retained.rglob('*') if p.is_file()})
            candidate=Path(folder)/source.name;shutil.copy2(source,candidate)
            subprocess.run(['strip','-S','-x',str(candidate)],check=True,capture_output=True)
            before=sections(source)
            if sections(candidate)!=before or uuid(candidate)!=identity:raise RuntimeError('Stripping changed executable/data sections or UUID; refusing release.')
            subprocess.run(['codesign','--verify','--strict',str(candidate)],check=True,capture_output=True)
            row.update(strippedBytes=candidate.stat().st_size,backedSectionsSHA256=before)
            record['artifacts'][name]=row;record['binaries'][name]=sha(candidate)
            with candidate.open('rb') as handle:os.fsync(handle.fileno())
            staged.append((candidate,source));changes[name]=current[name]
        # Persist enough information to restore an interrupted pair before the
        # next compiler invocation. The build/install artifact lock owns this step.
        atomic_json(pending,{'schema':'ruycast.release-preparation-intent.v1','changes':changes,'preparation':record})
        try:
            for candidate,source in staged:os.replace(candidate,source)
            verify_symbols(root,record);atomic_json(receipt,record);pending.unlink()
        except BaseException:
            recover(root);raise
    return record

if __name__=='__main__':
    import sys
    root=Path(__file__).resolve().parents[1]
    recover(root) if sys.argv[1:]==['--recover'] else prepare(root)
