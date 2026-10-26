import shutil
"""Purely filesystem-based migration helpers; no app or service is started here."""
from pathlib import Path
import hashlib
import json
import os
import plistlib

NEW_ID='nl.viggomeesters.ruycast'
OLD_ID='nl.viggomeesters.rustcast'

def require_owned_bundle(path,identifiers):
    path=Path(path)
    if not path.exists():
        if path.is_symlink():raise RuntimeError('Dangling app link preserved: '+str(path))
        return
    if path.is_symlink() or not path.is_dir():raise RuntimeError('Unrelated app path preserved: '+str(path))
    try:identity=plistlib.loads((path/'Contents/Info.plist').read_bytes()).get('CFBundleIdentifier')
    except (OSError,ValueError):raise RuntimeError('Unrecognized app preserved: '+str(path))
    if identity not in identifiers:raise RuntimeError('Unrelated app identity preserved: '+str(path))

def profile_paths(home,override=None):
    if override:return Path(override),Path(override)
    home=Path(home)
    return home/'.config/rustcast',home/'.config/ruycast'

def check_profile_paths(old,new):
    old,new=Path(old),Path(new)
    if old==new:return old
    if old.is_symlink():
        if old.resolve()!=new.resolve() or not new.is_dir():raise RuntimeError('Legacy profile link does not point to Ruycast; preserved.')
    if new.is_symlink():raise RuntimeError('Canonical profile may not be a symlink; preserved.')
    if old.exists() and new.exists() and old.resolve()!=new.resolve():raise RuntimeError('Both profiles exist; refusing to merge or overwrite either.')
    active=old if old.exists() and not new.exists() else new
    if active.exists() and not active.is_dir():raise RuntimeError('Profile is not a directory; preserved.')
    return active

def migrate_profile(old,new):
    old,new=Path(old),Path(new)
    check_profile_paths(old,new)
    new.parent.mkdir(parents=True,exist_ok=True)
    if old!=new and old.exists() and not old.is_symlink() and not new.exists():old.rename(new)
    new.mkdir(parents=True,exist_ok=True,mode=0o700)
    new.chmod(0o700)
    if old!=new and not old.is_symlink():
        if old.exists():raise RuntimeError('Legacy profile changed during migration; preserved.')
        old.symlink_to(new,target_is_directory=True)
    return new

def configuration_path(profile):
    profile=Path(profile)
    override=os.environ.get('RUYCAST_CONFIG_DIR')
    selector=profile/'config-location.json'
    if override:
        directory=Path(override).expanduser()
    elif selector.exists():
        try:directory=Path(json.loads(selector.read_text())['directory']).expanduser()
        except (OSError,ValueError,KeyError,TypeError):raise RuntimeError('Invalid configuration location: '+str(selector))
    else:return profile/'config.json'
    if not directory.is_absolute() or '..' in directory.parts:raise RuntimeError('Configuration folder must be absolute, without parent components.')
    return directory/'config.json'

def data_files(profile):
    profile=Path(profile)
    result={name:profile/name for name in ['config.json','config-location.json','clipboard.json','usage.json','paused.json','utilities.json'] if (profile/name).is_file()}
    selected=configuration_path(profile)
    if selected!=profile/'config.json':
        if not selected.is_file():raise RuntimeError('Selected configuration is unavailable: '+str(selected))
        result['selected-config.json']=selected
    return result

def data_hashes(profile):
    return {name:hashlib.sha256(path.read_bytes()).hexdigest() for name,path in data_files(profile).items()}

def require_owned_link(path,targets):
    path=Path(path)
    if path.is_symlink():
        if path.resolve() not in {Path(target).resolve() for target in targets}:raise RuntimeError('Unrelated CLI symlink preserved: '+str(path))
    elif path.exists():raise RuntimeError('Existing non-symlink CLI preserved: '+str(path))

def set_cli_link(path,target,allowed_targets=()):
    path,target=Path(path),Path(target)
    require_owned_link(path,[target,*allowed_targets])
    path.parent.mkdir(parents=True,exist_ok=True)
    # Replace a symlink atomically without a broken-link window.
    temporary=path.with_name('.'+path.name+'-install-'+str(os.getpid()))
    if temporary.exists() or temporary.is_symlink():raise RuntimeError('CLI staging link already exists: '+str(temporary))
    temporary.symlink_to(target)
    try:temporary.replace(path)
    finally:temporary.unlink(missing_ok=True)


def preserve_personal_bridge(repo, installed, stage):
    """Retain an existing local adapter when updating from public-only source."""
    source = Path(repo) / 'scripts/personal_bridge.py'
    if not source.exists() and not source.is_symlink():
        source = Path(installed) / 'Contents/Resources/personal_bridge.py'
    if source.is_symlink():
        raise RuntimeError('Linked personal adapter preserved; install a regular adapter file before updating.')
    if source.is_file():
        destination = Path(stage) / 'Contents/Resources/personal_bridge.py'
        shutil.copy2(source, destination)
        if destination.read_bytes() != source.read_bytes():
            raise RuntimeError('Personal adapter changed while staging; current installation preserved.')
