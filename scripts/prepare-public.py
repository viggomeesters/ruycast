#!/usr/bin/env python3
"""Create a clean source artifact; never publish private development history."""
import argparse,json,os,subprocess
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
TOP=['Package.swift','Makefile','README.md','LICENSE','SECURITY.md','CONTRIBUTING.md','CHANGELOG.md','.gitignore','.github/pull_request_template.md','.github/ISSUE_TEMPLATE/bug_report.md','.github/ISSUE_TEMPLATE/feature_request.md']
SCRIPTS=['package-app.py','build-native-locked.sh','build-native.sh','build_receipt.py','prepare_release.py','check.sh','check-locked.sh','check-settings.sh','check-settings-locked.sh','check-performance.sh','check-performance-locked.sh','perf-probe.c','sample-performance.py','benchmark-host.py','install.sh','install.py','install_support.py','local_signing.py','check-signing.py','import-raycast.py','import-clipboard.py','prepare-public.py','product_version.py']
TESTS=['test_architecture.py','test_package_app.py','test_cli.py','test_config_activation.py','test_config_directory.py','test_clipboard_import.py','test_install_migration.py','test_local_signing.py','test_public_contract.py','test_build_pipeline.py','test_socket_recovery.py','test_private_storage.py','fixtures/private_fsync_shim.rs','fixtures/private_fsync_probe.rs','fixtures/search-quality.json','test_product_version.py','test_benchmark_host.py']
DOCS=['DIAGNOSTICS.md','INSTALLATION.md','GETTING-STARTED.md','CONFIG.md','ARCHITECTURE.md','ANALYTICS.md','PERFORMANCE.md','RELEASING.md','VISUALS.md','IMPLEMENTATION-REFERENCES.md','adr/0001-native-modular-core.md','public-readiness-review.md','vision.json','schemas/repo-vision-contract.schema.json']
def selected(root):
    return _rust(['list','--repo',str(Path(root).absolute())])['files']
def prepare(root,destination):
    return _rust(['export','--repo',str(Path(root).absolute()),'--out',str(Path(destination).absolute())])
def _rust(arguments):
    tool=os.environ.get('RUYCAST_SOURCE_TOOL')
    command=[tool] if tool else ['cargo','run','--quiet','--offline','--locked','--manifest-path',str(ROOT/'rust-native/Cargo.toml'),'--bin','ruycast-source','--']
    process=subprocess.run(command+arguments,capture_output=True,text=True)
    try:result=json.loads(process.stdout)
    except json.JSONDecodeError:raise RuntimeError(process.stderr.strip() or 'Rust source tool returned no JSON.')
    if process.returncode or not result.get('ok'):raise RuntimeError(result.get('error','Rust source export failed.'))
    return result
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--out',type=Path,default=ROOT/'.dist/ruycast-public-source');a=p.parse_args()
    try:print(json.dumps(prepare(ROOT,a.out),indent=2))
    except (RuntimeError,OSError) as e:raise SystemExit(str(e))
