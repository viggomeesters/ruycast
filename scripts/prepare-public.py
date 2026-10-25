#!/usr/bin/env python3
"""Create a clean source artifact; never publish private development history."""
import argparse,hashlib,json,shutil,tarfile
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
TOP=['Package.swift','Makefile','README.md','LICENSE','SECURITY.md','CONTRIBUTING.md','CHANGELOG.md','.gitignore','.github/pull_request_template.md','.github/ISSUE_TEMPLATE/bug_report.md','.github/ISSUE_TEMPLATE/feature_request.md']
SCRIPTS=['build-native-locked.sh','build-native.sh','build_receipt.py','check.sh','check-settings.sh','check-performance.sh','perf-probe.c','sample-performance.py','benchmark-host.py','install.sh','install.py','install_support.py','local_signing.py','check-signing.py','import-raycast.py','import-clipboard.py','prepare-public.py']
TESTS=['test_cli.py','test_config_activation.py','test_config_directory.py','test_clipboard_import.py','test_install_migration.py','test_local_signing.py','test_public_contract.py']
DOCS=['DIAGNOSTICS.md','INSTALLATION.md','GETTING-STARTED.md','CONFIG.md','ARCHITECTURE.md','ANALYTICS.md','PERFORMANCE.md','RELEASING.md','VISUALS.md','IMPLEMENTATION-REFERENCES.md','public-readiness-review.md','vision.json','schemas/repo-vision-contract.schema.json']
def selected(root):
    paths=set(TOP+['rust/Cargo.toml','rust/Cargo.lock'])
    for tree in ['Sources','rust/src','config','assets','Tests/LauncherCoreTests']:
        paths.update(str(p.relative_to(root)) for p in (root/tree).rglob('*') if p.is_file())
    paths.update('scripts/'+p for p in SCRIPTS);paths.update('Tests/'+p for p in TESTS);paths.update('docs/'+p for p in DOCS)
    return sorted(paths)
def prepare(root,destination):
    root,destination=Path(root).resolve(),Path(destination).absolute()
    if destination.exists() or destination.is_symlink():raise RuntimeError('Destination exists; choose a new export directory.')
    if root==destination or destination in root.parents:raise RuntimeError('Export may not replace its source.')
    archive=destination.with_name(destination.name+'.tar.gz')
    if archive.exists() or archive.is_symlink():raise RuntimeError('Archive exists; choose a new export destination.')
    files=selected(root)
    for name in files:
        path=root/name
        if not path.is_file() or any(p.is_symlink() for p in [path,*list(path.parents)[:len(Path(name).parts)-1]]):raise RuntimeError('Missing or linked public source: '+name)
    destination.mkdir(parents=True)
    manifest={}
    for name in files:
        path=destination/name;path.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(root/name,path);manifest[name]=hashlib.sha256(path.read_bytes()).hexdigest()
    receipt={'schema':'ruycast.public-source.v1','version':'0.7.2','files':manifest,'exclusions':['Development Git history and workflow state','Build output, profiles, analytics and clipboard data','Optional private adapters and their tests','Local signing material'],'publication':'export-only; see docs/RELEASING.md for release status'}
    (destination/'PUBLIC-SOURCE.json').write_text(json.dumps(receipt,indent=2)+'\n')
    with tarfile.open(archive,'x:gz') as tar:
        for name in sorted([*files,'PUBLIC-SOURCE.json']):
            path=destination/name;info=tar.gettarinfo(str(path),arcname='ruycast/'+name);info.uid=info.gid=0;info.uname=info.gname='';info.mtime=0
            with path.open('rb') as handle:tar.addfile(info,handle)
    return {'source':str(destination),'files':len(files),'archive':str(archive),'archiveSHA256':hashlib.sha256(archive.read_bytes()).hexdigest(),'published':False}
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--out',type=Path,default=ROOT/'.dist/ruycast-public-source');a=p.parse_args()
    try:print(json.dumps(prepare(ROOT,a.out),indent=2))
    except (RuntimeError,OSError) as e:raise SystemExit(str(e))
