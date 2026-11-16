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
import subprocess
import sys
import time

REPO=Path(__file__).resolve().parents[2]

GROUPS_BYTES=(REPO/'rust-native/resources/parity-families.json').read_bytes()
GROUPS=json.loads(GROUPS_BYTES)

_AST_PARSER=None
def ast_parser():
    global _AST_PARSER
    command=['cargo','run','--quiet','--locked','--offline','--manifest-path',str(REPO/'rust-native/Cargo.toml'),'--target-dir',str(REPO/'.build/delivery-tools'),'--bin','ruycast-build','--','parity-parser','--repo',str(REPO)]
    run=subprocess.run(command,input=json.dumps(dict(binding=_AST_PARSER,deadlineNanos=time.clock_gettime_ns(time.CLOCK_MONOTONIC)+420_000_000_000)),cwd=REPO,capture_output=True,text=True,timeout=420,env={**os.environ,'CARGO_INCREMENTAL':'0'})
    if run.returncode:raise RuntimeError(run.stderr)
    _AST_PARSER=json.loads(run.stdout)
    return _AST_PARSER

def parse_module(module,paths,sdk,plugins):
    binding=ast_parser()
    command=['cargo','run','--quiet','--locked','--offline','--manifest-path',str(REPO/'rust-native/Cargo.toml'),'--target-dir',str(REPO/'.build/delivery-tools'),'--bin','ruycast-build','--','parity-compile','--repo',str(REPO)]
    payload=dict(module=module,paths=sorted(str(path.relative_to(REPO)) for path in paths),sdk=sdk,plugins=plugins,parser=binding)
    run=subprocess.run(command,input=json.dumps(payload),cwd=REPO,capture_output=True,text=True,timeout=180,env={**os.environ,'CARGO_INCREMENTAL':'0'})
    if run.returncode:raise RuntimeError(run.stderr)
    value=json.loads(run.stdout)
    return value['declarations'],value['context']
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

def rust_toolchain(stage='discover'):
    command=['cargo','run','--quiet','--locked','--offline','--manifest-path',str(REPO/'rust-native/Cargo.toml'),'--target-dir',str(REPO/'.build/delivery-tools'),'--bin','ruycast-build','--','parity-toolchain','--repo',str(REPO),'--stage',stage]
    run=subprocess.run(command,input=json.dumps(dict(deadlineNanos=time.clock_gettime_ns(time.CLOCK_MONOTONIC)+180_000_000_000)),cwd=REPO,capture_output=True,text=True,timeout=180,env={**os.environ,'CARGO_INCREMENTAL':'0'})
    if run.returncode:raise RuntimeError(run.stderr)
    sys.stderr.write(run.stderr)
    return json.loads(run.stdout)

def rust_report(reference,out,modules,contexts,tests,groups,wire):
    command=['cargo','run','--quiet','--locked','--offline','--manifest-path',str(REPO/'rust-native/Cargo.toml'),'--target-dir',str(REPO/'.build/delivery-tools'),'--bin','ruycast-build','--','parity-report','--repo',str(REPO)]
    payload=dict(reference=str(reference.resolve()),out=str(out.absolute()),deadlineNanos=time.clock_gettime_ns(time.CLOCK_MONOTONIC)+180_000_000_000,sourceFiles=modules,compilerContext=contexts,testInventory=tests,contractFamilies=groups,wireInventory=wire)
    run=subprocess.run(command,input=json.dumps(payload),cwd=REPO,capture_output=True,text=True,timeout=180,env={**os.environ,'CARGO_INCREMENTAL':'0'})
    if run.returncode:raise RuntimeError(run.stderr)
    sys.stderr.write(run.stderr)
    return json.loads(run.stdout)

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--reference',type=Path,required=True);parser.add_argument('--out',type=Path,required=True);args=parser.parse_args()
    source_files=rust_reference_sources(args.reference)
    paths=[REPO/path for path in source_files]
    ast_parser()
    if rust_reference_sources(args.reference)!=source_files:raise RuntimeError('Production/reference sources changed during parser bootstrap')
    toolchain=rust_toolchain();sdk=toolchain['sdk'];plugins=toolchain['plugins']
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
    print(json.dumps(rust_report(args.reference,args.out,modules,compiler_context,test_inventory,groups,wire_inventory)))
if __name__=='__main__':main()
