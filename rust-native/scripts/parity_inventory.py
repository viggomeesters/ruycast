#!/usr/bin/env python3
"""Reconcile the shipped Swift surface with Rust source/tests, without claiming
parity from filenames or mentions. This inventories compiler-parsed entrypoints;
semantic contracts and acceptance remain explicit review work.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import stat

REPO=Path(__file__).resolve().parents[2]

GROUPS_BYTES=(REPO/'rust-native/resources/parity-families.json').read_bytes()
GROUPS=json.loads(GROUPS_BYTES)

def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
_AST_PARSER=None
def parser_sha(path):
    prefix=REPO
    for part in path.relative_to(REPO).parts:
        prefix=prefix/part
        if stat.S_ISLNK(prefix.lstat().st_mode):raise RuntimeError('Linked AST parser input')
    descriptor=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK|os.O_CLOEXEC)
    with os.fdopen(descriptor,'rb') as stream:
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):raise RuntimeError('AST parser must be a regular file')
        return hashlib.sha256(stream.read()).hexdigest()
def ast_parser():
    global _AST_PARSER
    if _AST_PARSER is None:
        target=REPO/'.build/delivery-tools'
        sources=['rust-native/src/bin/parity_ast.rs','rust-native/src/bin/ruycast-build.rs','rust-native/src/bin/ruycast-build/validation.rs','rust-native/src/bin/ruycast-build/test_catalog.rs','rust-native/src/bin/ruycast-build/parity_wire.rs','rust-native/src/bin/ruycast-build/parity_families.rs','rust-native/src/bin/ruycast-build/parity_sources.rs','rust-native/resources/parity-families.json','rust-native/src/artifact_lease.rs','rust/src/private_storage.rs','rust-native/Cargo.toml','rust-native/Cargo.lock','rust/Cargo.toml','rust/Cargo.lock','rust-toolchain.toml','rust-native/scripts/parity_inventory.py']
        hashes={name:sha(REPO/name) for name in sources}
        command=['cargo','build','--quiet','--locked','--offline','--manifest-path',str(REPO/'rust-native/Cargo.toml'),'--target-dir',str(target),'--bin','parity-ast']
        run=subprocess.run(command,cwd=REPO,capture_output=True,text=True,timeout=180,env={**os.environ,'CARGO_INCREMENTAL':'0'})
        if run.returncode:raise RuntimeError(run.stderr)
        expected=parser_sha(target/'debug/parity-ast')
        command=['cargo','run','--quiet','--locked','--offline','--manifest-path',str(REPO/'rust-native/Cargo.toml'),'--target-dir',str(target),'--bin','ruycast-build','--','parity-ast-snapshot','--repo',str(REPO),'--sha256',expected]
        run=subprocess.run(command,cwd=REPO,capture_output=True,text=True,timeout=180,env={**os.environ,'CARGO_INCREMENTAL':'0'})
        if run.returncode:raise RuntimeError(run.stderr)
        binding=json.loads(run.stdout)
        if binding!={'artifact':f'.build/delivery-tools/parity-ast-artifacts/{expected}','sha256':expected}:raise RuntimeError('Invalid AST parser snapshot binding')
        binary=REPO/binding['artifact']
        if hashes!={name:sha(REPO/name) for name in sources} or parser_sha(binary)!=expected:raise RuntimeError('AST parser inputs changed during build/publication')
        binding['sources']=hashes
        _AST_PARSER=(binary,binding)
    binary,binding=_AST_PARSER
    if parser_sha(binary)!=binding['sha256'] or any(sha(REPO/name)!=value for name,value in binding['sources'].items()):raise RuntimeError('AST parser source/snapshot changed')
    return _AST_PARSER

def parse_module(module,paths,sdk,plugins):
    binary,binding=ast_parser()
    all_paths=sorted((REPO/'Sources'/module).glob('*.swift'))
    before={str(p.relative_to(REPO)):sha(p) for p in all_paths}
    cmd=['swiftc','-frontend','-dump-ast','-whole-module-optimization','-sdk',sdk,'-swift-version','6','-module-name',module,'-I','.build/swift/debug/Modules','-I','.build/swift/debug/CRuycastCore.build']
    cmd+=plugins
    if module!='RuycastApp':cmd+=['-parse-as-library']
    for p in all_paths:
        if p in paths:cmd+=['-primary-file',str(p.relative_to(REPO))]
        else:cmd+=[str(p.relative_to(REPO))]
    run=subprocess.run(cmd,cwd=REPO,capture_output=True,text=True,timeout=90)
    if run.returncode:raise RuntimeError(run.stderr)
    expected=sorted(str(p.relative_to(REPO)) for p in paths)
    if parser_sha(binary)!=binding['sha256']:raise RuntimeError('AST parser snapshot changed before execution')
    parser=subprocess.run([str(binary),*[part for name in expected for part in ['--source',name]]],input=run.stdout,cwd=REPO,capture_output=True,text=True,timeout=10)
    if parser.returncode:raise RuntimeError(parser.stderr)
    if parser_sha(binary)!=binding['sha256'] or any(sha(REPO/name)!=value for name,value in binding['sources'].items()):raise RuntimeError('AST parser source/artifact changed during inventory')
    parsed=json.loads(parser.stdout);declarations=parsed['declarations']
    context={str(p.relative_to(REPO)):sha(p) for p in all_paths}
    if context!=before:raise RuntimeError('Compiler source inputs changed during inventory')
    return declarations,dict(command=cmd,inputHashes=context,exitCode=run.returncode,diagnostics=run.stderr.strip(),sourceASTs=parsed['sourceASTs'],astParser=binding)
def rust_test_candidates():
    selectors=sorted({name for group in GROUPS for name in group['tests']})
    command=['cargo','run','--quiet','--locked','--offline','--manifest-path',str(REPO/'rust-native/Cargo.toml'),'--target-dir',str(REPO/'.build/delivery-tools'),'--bin','ruycast-build','--','parity-tests','--repo',str(REPO),'--selectors',json.dumps(selectors)]
    run=subprocess.run(command,cwd=REPO,capture_output=True,text=True,timeout=300,env={**os.environ,'CARGO_INCREMENTAL':'0'})
    if run.returncode:raise RuntimeError(run.stderr)
    return json.loads(run.stdout)

def rust_wire_inventory():
    command=['cargo','run','--quiet','--locked','--offline','--manifest-path',str(REPO/'rust-native/Cargo.toml'),'--target-dir',str(REPO/'.build/delivery-tools'),'--bin','ruycast-build','--','parity-wire','--repo',str(REPO),'--marker','    func handle(_ req:[String:Any],sink:ReplySink)']
    run=subprocess.run(command,cwd=REPO,capture_output=True,text=True,timeout=180,env={**os.environ,'CARGO_INCREMENTAL':'0'})
    if run.returncode:raise RuntimeError(run.stderr)
    return json.loads(run.stdout)

def rust_contract_families(modules,test_inventory):
    command=['cargo','run','--quiet','--locked','--offline','--manifest-path',str(REPO/'rust-native/Cargo.toml'),'--target-dir',str(REPO/'.build/delivery-tools'),'--bin','ruycast-build','--','parity-families','--repo',str(REPO)]
    payload=dict(sourceFamilies={path:value['contractFamily'] for path,value in modules.items()},testInventory=test_inventory)
    run=subprocess.run(command,input=json.dumps(payload),cwd=REPO,capture_output=True,text=True,timeout=180,env={**os.environ,'CARGO_INCREMENTAL':'0'})
    if run.returncode:raise RuntimeError(run.stderr)
    value=json.loads(run.stdout)
    if value['definitionsSHA256']!=hashlib.sha256(GROUPS_BYTES).hexdigest():raise RuntimeError('Contract family definitions changed since load')
    return value['contractFamilies']

def rust_reference_sources(reference):
    command=['cargo','run','--quiet','--locked','--offline','--manifest-path',str(REPO/'rust-native/Cargo.toml'),'--target-dir',str(REPO/'.build/delivery-tools'),'--bin','ruycast-build','--','parity-sources','--repo',str(REPO),'--reference',str(reference.resolve())]
    run=subprocess.run(command,cwd=REPO,capture_output=True,text=True,timeout=180,env={**os.environ,'CARGO_INCREMENTAL':'0'})
    if run.returncode:raise RuntimeError(run.stderr)
    value=json.loads(run.stdout)
    if value['definitionsSHA256']!=hashlib.sha256(GROUPS_BYTES).hexdigest():raise RuntimeError('Contract family definitions changed since load')
    return value['sourceFiles']

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--reference',type=Path,required=True);parser.add_argument('--out',type=Path,required=True);args=parser.parse_args()
    source_files=rust_reference_sources(args.reference)
    paths=[REPO/path for path in source_files]
    ast_parser()
    if rust_reference_sources(args.reference)!=source_files:raise RuntimeError('Production/reference sources changed during parser bootstrap')
    sdk=subprocess.check_output(['xcrun','--show-sdk-path'],text=True).strip()
    driver=subprocess.run(['swiftc','-###','-typecheck','Sources/RuycastApp/FeatureState.swift'],cwd=REPO,capture_output=True,text=True,check=True)
    driver_args=shlex.split(driver.stdout+driver.stderr);plugins=[]
    for i,arg in enumerate(driver_args):
        if arg in ['-plugin-path','-external-plugin-path','-load-plugin-library','-load-plugin-executable']:plugins+=driver_args[i:i+2]
    parsed={};compiler_context=[]
    for module in sorted({p.parent.name for p in paths}):
        declarations,context=parse_module(module,[p for p in paths if p.parent.name==module],sdk,plugins)
        parsed.update(declarations);compiler_context.append(context)
    modules=rust_reference_sources(args.reference)
    if modules!=source_files:raise RuntimeError('Production/reference sources changed during compiler inventory')
    for relative,value in modules.items():value['declarations']=parsed[relative]
    test_inventory=rust_test_candidates()
    groups=rust_contract_families(modules,test_inventory)
    wire_inventory=rust_wire_inventory()
    if wire_inventory['sources']['Sources/RuycastApp/AppState.swift']!=modules['Sources/RuycastApp/AppState.swift']['sha256']:raise RuntimeError('Wire source differs from compiler/reference source')
    wire=wire_inventory['wireOperations']
    head=subprocess.check_output(['git','-C',str(args.reference),'rev-parse','HEAD'],text=True).strip()
    result=dict(schema='ruycast.full-rust.behavior-matrix.candidate.v1',scope='Current accepted0.8.26 source crosswalk; production modules, compiler-parsed declarations, explicit semantic contract families, wire operations and candidate tests. Not final full behavior acceptance.',baseline=dict(version='0.8.26',build='66',referenceCommit=head,sourceOnlyComparison=True,releaseDeliveryEvidence='.go/evidence/native-edit-shortcuts/delivery-review.json'),compilerVersion=subprocess.check_output(['swiftc','--version'],text=True).strip(),compilerContext=compiler_context,testInventory=test_inventory,importedModuleHashes={p:sha(REPO/p) for p in ['.build/swift/debug/Modules/LauncherCore.swiftmodule','.build/swift/debug/Modules/LauncherModels.swiftmodule','.build/swift/debug/CRuycastCore.build/module.modulemap']},sourceFiles=modules,contractFamilies=groups,wireOperations=wire,wireInventory=wire_inventory,unclassifiedModules=[],allProductionSourcesMatchReference=True,acceptance='Pending independent semantic completeness review and criterion-level proof. No whole original checklist block is checked.',original12Retained=True,overallComplete=False)
    args.out.parent.mkdir(parents=True,exist_ok=True);args.out.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(dict(modules=len(modules),declarations=sum(len(m['declarations']) for m in modules.values()),families=len(groups),wireOperations=len(wire),unclassified=0,overallComplete=False)))
if __name__=='__main__':main()
