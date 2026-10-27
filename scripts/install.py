#!/usr/bin/env python3
"""Install Ruycast and migrate this launcher's legacy identity without losing state."""
import datetime
import fcntl
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import time
import tempfile
import atexit
import local_signing
import build_receipt
from install_support import NEW_ID,OLD_ID,require_owned_bundle,profile_paths,check_profile_paths,migrate_profile,data_hashes,data_files,configuration_path,set_cli_link,require_owned_link,preserve_personal_bridge

REPO=Path(__file__).resolve().parents[1]
with build_receipt.artifact_lock(REPO):
    receipt=build_receipt.verify(REPO) # Fail before staging or stopping the current app.
    HOME_DIR=Path.home()
    DOMAIN='gui/%s'%os.getuid()
    app_dir=Path(os.environ.get('RUYCAST_APP_DIR',os.environ.get('RUSTCAST_APP_DIR','/Applications' if os.access('/Applications',os.W_OK) else str(HOME_DIR/'Applications'))))
    APP=app_dir/'Ruycast.app'
    OLD_APP=app_dir/'Rustcast.app'
    app_dir.mkdir(parents=True,exist_ok=True)
    if not os.access(app_dir,os.W_OK):raise SystemExit('Install destination is not writable: '+str(app_dir))
    require_owned_bundle(APP,[NEW_ID])
    legacy_owned=False
    if OLD_APP.exists():
        try:require_owned_bundle(OLD_APP,[OLD_ID]);legacy_owned=True
        except RuntimeError:pass # Other launchers with this name are unrelated products.
    legacy_cli=HOME_DIR/'.local/bin/rustcast'
    legacy_owned=legacy_owned or (not OLD_APP.exists() and not OLD_APP.is_symlink() and APP.exists() and legacy_cli.is_symlink() and legacy_cli.resolve()==APP/'Contents/Resources/ruycast')
    local_signing.check_installed(APP)
    links=[HOME_DIR/'.local/bin/ruycast']+([legacy_cli] if legacy_owned else [])
    known_cli_targets=[OLD_APP/'Contents/Resources/rustcast',APP/'Contents/Resources/ruycast']
    for link in links:require_owned_link(link,known_cli_targets)
    previous_links={str(link):os.readlink(link) if link.is_symlink() else None for link in links}
    old_profile,new_profile=profile_paths(HOME_DIR,os.environ.get('RUYCAST_HOME') or os.environ.get('RUSTCAST_HOME'))
    if not legacy_owned:old_profile=new_profile # Never adopt another launcher's profile.
    active=check_profile_paths(old_profile,new_profile)
    active.mkdir(parents=True,exist_ok=True,mode=0o700)
    active.chmod(0o700)
    plists={label:HOME_DIR/'Library/LaunchAgents'/(label+'.plist') for label in [OLD_ID,NEW_ID]}
    for label,path in plists.items():
        if label==OLD_ID and not legacy_owned:continue
        if path.exists():
            meta=plistlib.loads(path.read_bytes())
            expected=OLD_APP if label==OLD_ID else APP
            if meta.get('Label')!=label or meta.get('ProgramArguments')!=[str(expected/'Contents/MacOS'/('RustcastApp' if label==OLD_ID else 'RuycastApp'))]:
                raise SystemExit('Unrelated login registration preserved: '+str(path))
    # Preserve an explicit config override across subsequent updates as well as
    # the first install. Validate registration ownership before trusting its env.
    previous_agent=plistlib.loads(plists[NEW_ID].read_bytes()) if plists[NEW_ID].exists() else {}
    configuration_override=os.environ.get('RUYCAST_CONFIG_DIR') or previous_agent.get('EnvironmentVariables',{}).get('RUYCAST_CONFIG_DIR')
    if configuration_override:
        if not isinstance(configuration_override,str):raise RuntimeError('Invalid installed configuration override.')
        configuration_override=str(Path(configuration_override).expanduser())
        os.environ['RUYCAST_CONFIG_DIR']=configuration_override

    DIST=REPO/'.dist';DIST.mkdir(exist_ok=True)
    stage_directory=tempfile.TemporaryDirectory(prefix='ruycast-install-',dir=DIST)
    atexit.register(stage_directory.cleanup)
    stage=Path(stage_directory.name)/'Ruycast.app'
    for directory in ['Contents/MacOS','Contents/Resources']:(stage/directory).mkdir(parents=True,exist_ok=True)
    shutil.copy2(REPO/'.build/swift/release/RuycastApp',stage/'Contents/MacOS/RuycastApp')
    shutil.copy2(REPO/'.build/rust/release/ruycast',stage/'Contents/Resources/ruycast')
    icon=REPO/'assets/AppIcon.icns'
    if icon.is_file():shutil.copy2(icon,stage/'Contents/Resources/AppIcon.icns')
    preserve_personal_bridge(REPO,APP,stage,os.environ.get("RUYCAST_PERSONAL_BRIDGE"))
    if build_receipt.sha(stage/'Contents/MacOS/RuycastApp')!=receipt['binaries']['.build/swift/release/RuycastApp'] or build_receipt.sha(stage/'Contents/Resources/ruycast')!=receipt['binaries']['.build/rust/release/ruycast']:
        raise RuntimeError('Staged binaries do not match the build receipt; current app preserved.')
previous_build=0
if APP.exists():
    prior=plistlib.loads((APP/'Contents/Info.plist').read_bytes()).get('CFBundleVersion','0')
    if not str(prior).isdigit():raise RuntimeError('Existing build number is not numeric; installation preserved.')
    previous_build=int(prior)
info={'CFBundleIdentifier':NEW_ID,'CFBundleName':'Ruycast','CFBundleDisplayName':'Ruycast',
      'CFBundleExecutable':'RuycastApp','CFBundlePackageType':'APPL','CFBundleShortVersionString':'0.8.4',
      'CFBundleVersion':str(max(7,previous_build+1)),'LSMinimumSystemVersion':'14.0','LSUIElement':True,'NSHighResolutionCapable':True}
if (stage/'Contents/Resources/AppIcon.icns').is_file():info['CFBundleIconFile']='AppIcon'
(stage/'Contents/Info.plist').write_bytes(plistlib.dumps(info))
signing=local_signing.sign(stage)
subprocess.run(['/usr/bin/codesign','--verify','--deep','--strict',str(stage)],check=True)
env=dict(os.environ,RUYCAST_HOME=str(active))
subprocess.run([str(stage/'Contents/MacOS/RuycastApp'),'--init-config'],env=env,check=True,capture_output=True)
# Everything required for rollback exists before the current service is stopped.
backup=HOME_DIR/'.local/state/ruycast/install-backups'/datetime.datetime.now().strftime('%Y%m%d-%H%M%S-%f')
backup.mkdir(parents=True,mode=0o700)
for bundle in [APP]+([OLD_APP] if legacy_owned else []):
    if bundle.exists():shutil.copytree(bundle,backup/bundle.name)
for label,path in plists.items():
    if label==OLD_ID and not legacy_owned:continue
    if path.exists():shutil.copy2(path,backup/path.name)
(backup/'cli-links.json').write_text(json.dumps(previous_links))
(backup/'cli-links.json').chmod(0o600)
candidate=APP.with_name('.Ruycast.new.app')
if candidate.exists() or candidate.is_symlink():raise SystemExit('Prior install candidate preserved: '+str(candidate))
shutil.copytree(stage,candidate)
profile_guard=None
replaced=False

def lock_profile(profile):
    guard=open(profile/'instance.lock','a');os.chmod(profile/'instance.lock',0o600)
    for _ in range(60):
        try:fcntl.flock(guard,fcntl.LOCK_EX|fcntl.LOCK_NB);return guard
        except BlockingIOError:time.sleep(.1)
    guard.close();raise RuntimeError('Existing launcher still owns the profile; installation preserved.')

try:
    subprocess.run([str(REPO/'.build/rust/release/ruycast'),'quit'],env=env,capture_output=True)
    for label in ([NEW_ID,OLD_ID] if legacy_owned else [NEW_ID]):subprocess.run(['launchctl','bootout',DOMAIN+'/'+label],capture_output=True)
    profile_guard=lock_profile(active)
    before=data_hashes(active)
    for name,path in data_files(active).items():
        shutil.copy2(path,backup/name);(backup/name).chmod(0o600)
    PROFILE=migrate_profile(old_profile,new_profile)
    env=dict(os.environ,RUYCAST_HOME=str(PROFILE))
    (PROFILE/'control.sock').unlink(missing_ok=True)
    if data_hashes(PROFILE)!=before:raise RuntimeError("Profile data changed during migration.")
    if APP.exists():shutil.rmtree(APP)
    candidate.rename(APP);replaced=True
    plist=plists[NEW_ID];plist.parent.mkdir(parents=True,exist_ok=True)
    agent={'Label':NEW_ID,'ProgramArguments':[str(APP/'Contents/MacOS/RuycastApp')],
           'RunAtLoad':True,'KeepAlive':{'SuccessfulExit':False},'ThrottleInterval':10,
           'EnvironmentVariables':{'RUYCAST_HOME':str(PROFILE),**({'RUYCAST_CONFIG_DIR':configuration_override} if configuration_override else {})},
           'StandardOutPath':str(PROFILE/'launcher.log'),'StandardErrorPath':str(PROFILE/'launcher.log')}
    (PROFILE/'launcher.log').touch(mode=0o600,exist_ok=True);(PROFILE/'launcher.log').chmod(0o600)
    plist.write_bytes(plistlib.dumps(agent))
    for link in links:set_cli_link(link,APP/'Contents/Resources/ruycast',known_cli_targets)
    subprocess.run(['launchctl','enable',DOMAIN+'/'+NEW_ID],check=True)
    profile_guard.close();profile_guard=None
    subprocess.run(['launchctl','bootstrap',DOMAIN,str(plist)],check=True)
    for _ in range(80):
        status=subprocess.run([str(links[0]),'status'],env=env,capture_output=True,text=True)
        if status.returncode==0:break
        time.sleep(.1)
    else:raise RuntimeError('Startup could not be verified: '+status.stdout+status.stderr)
    state=json.loads(status.stdout)
    if state.get('result',{}).get('name')!='Ruycast':raise RuntimeError('Unexpected host identity after installation.')
    if Path(state.get('result',{}).get('configPath','')).resolve()!=configuration_path(PROFILE).resolve():raise RuntimeError('Installed host is using a different configuration folder.')
    after=data_hashes(PROFILE)
    # Config and authored library must be exact. Clipboard/usage can change after
    # startup during normal user activity; their migration was checked under lock.
    if any(before.get(name)!=after.get(name) for name in ["config.json","config-location.json","selected-config.json"]):raise RuntimeError("Configuration changed during installation.")
    local_signing.verify(APP,signing)
    subprocess.run(['launchctl','print',DOMAIN+'/'+NEW_ID],check=True,stdout=subprocess.DEVNULL)
    local_signing.record_install(APP)
except BaseException:
    if profile_guard:profile_guard.close()
    # Stop the candidate before restoring registrations. The legacy profile link
    # continues pointing at the one migrated data directory, including on rollback.
    if replaced:subprocess.run([str(APP/'Contents/Resources/ruycast'),'quit'],env=env,capture_output=True)
    subprocess.run(['launchctl','bootout',DOMAIN+'/'+NEW_ID],capture_output=True)
    rollback_profile=new_profile if new_profile.exists() else active
    rollback_guard=lock_profile(rollback_profile)
    (rollback_profile/'control.sock').unlink(missing_ok=True)
    if replaced:
        if APP.exists():shutil.rmtree(APP)
        if (backup/APP.name).exists():shutil.copytree(backup/APP.name,APP)
    for label,path in plists.items():
        if (backup/path.name).exists():shutil.copy2(backup/path.name,path)
        elif label==NEW_ID:path.unlink(missing_ok=True)
    for link in links:
        original=previous_links[str(link)]
        if original is not None:set_cli_link(link,original,known_cli_targets)
        elif link.is_symlink():link.unlink()
    restore=OLD_ID if (backup/plists[OLD_ID].name).exists() else NEW_ID
    rollback_guard.close()
    if (backup/plists[restore].name).exists():subprocess.run(['launchctl','bootstrap',DOMAIN,str(plists[restore])],capture_output=True)
    if candidate.exists():shutil.rmtree(candidate)
    raise
# Retire only the verified old product after the candidate is healthy.
lsregister=Path('/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister')
if legacy_owned and OLD_APP.exists():
    if lsregister.is_file():subprocess.run([str(lsregister),'-u',str(OLD_APP)],capture_output=True)
    shutil.rmtree(OLD_APP)
if legacy_owned:plists[OLD_ID].unlink(missing_ok=True)
if lsregister.is_file():subprocess.run([str(lsregister),'-f',str(APP)],capture_output=True)
print(json.dumps({'installed':str(APP),'cli':str(links[0]),'legacyCLI':str(links[1]) if len(links)>1 else None,
                  'config':str(configuration_path(PROFILE)),'legacyProfile':str(old_profile),
                  'loginAgent':str(plists[NEW_ID]),'signing':signing,'backup':str(backup),
                  'preservedDataHashes':before,'status':state}))

stage_directory.cleanup()
