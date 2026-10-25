"""One app-specific signing identity. No silent rotation, ad-hoc fallback or trust edits."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shlex
import shutil
import subprocess
import tempfile

BUNDLE_ID = 'nl.viggomeesters.ruycast'
NAME = 'Ruycast Local Signing'
DEFAULT_ROOT = Path.home() / '.local/share/ruycast/signing'
SCHEMA = 'ruycast.local-signing.v1'


def run(*args, timeout=30):
    try:
        result = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        raise RuntimeError(Path(args[0]).name + ': operation timed out') from None
    if result.returncode:
        # Arguments and arbitrary command output can contain keychain credentials.
        reason = ' (certificate needs local code-signing authorization)' if 'CSSMERR_TP_NOT_TRUSTED' in result.stderr else ''
        raise RuntimeError(Path(args[0]).name + ': operation failed' + reason)
    return result.stdout + result.stderr


def security_private(*args):
    # security's interactive mode reads the command through stdin, not process argv.
    # One command per process preserves its exit status; never return its output.
    try:
        result=subprocess.run(['/usr/bin/security','-i'],input=shlex.join(map(str,args))+'\n',
                              capture_output=True,text=True,timeout=30)
    except subprocess.TimeoutExpired:
        raise RuntimeError('security: private keychain operation timed out') from None
    if result.returncode:
        raise RuntimeError('security: private keychain operation failed')


def write_private(path, value):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as handle:
        handle.write(value)


def requirement(fingerprint):
    return 'identifier "' + BUNDLE_ID + '" and certificate leaf = H"' + fingerprint + '"'


def validate_root(root):
    root = Path(root).expanduser().absolute()
    if any(c in str(root) for c in ['\n','\r']) or root.is_symlink() or root.resolve() != root:
        raise RuntimeError('Signing state may not use symlinked paths')
    if root.is_relative_to(Path(__file__).resolve().parents[1]):
        raise RuntimeError('Signing secrets must stay outside the code repository')
    if root.exists() and (not root.is_dir() or root.stat().st_mode & 0o077):
        raise RuntimeError('Signing directory must be private (0700)')
    return root


def load(root=DEFAULT_ROOT):
    root = validate_root(root)
    for name in ['identity.json', 'identity.keychain-db', 'keychain-password', 'certificate.pem']:
        path = root / name
        if not path.is_file() or path.is_symlink() or path.stat().st_mode & 0o077:
            raise RuntimeError('Missing or unsafe signing state: ' + name + '; preserve and repair it')
    value = json.loads((root / 'identity.json').read_text())
    fingerprint = value.get('certificate_sha1', '')
    if value.get('schema') != SCHEMA or len(fingerprint) != 40 or any(c not in '0123456789ABCDEF' for c in fingerprint):
        raise RuntimeError('Invalid pinned signing identity')
    if hashlib.sha256((root / 'certificate.pem').read_bytes()).hexdigest() != value.get('certificate_sha256'):
        raise RuntimeError('Signing certificate changed; automatic replacement is forbidden')
    if value.get('requirement') != requirement(fingerprint):
        raise RuntimeError('Signing requirement changed; preserve the pinned identity')
    return value


def ensure_searchable(root):
    keychain=str(root / 'identity.keychain-db')
    current=shlex.split(run('/usr/bin/security','list-keychains','-d','user'))
    if keychain not in current:
        run('/usr/bin/security','list-keychains','-d','user','-s',*current,keychain)


def prepare(root=DEFAULT_ROOT):
    root = validate_root(root)
    if (root / 'identity.json').exists():
        return load(root)
    if root.exists() and any(root.iterdir()):
        raise RuntimeError('Incomplete signing state; repair it instead of generating a new identity')
    root.mkdir(parents=True, mode=0o700, exist_ok=True)
    openssl = shutil.which('openssl', path='/opt/homebrew/opt/openssl@3/bin:/usr/bin')
    if not openssl:
        raise RuntimeError('OpenSSL is required to provision the signing identity')
    passphrase = secrets.token_hex(32)
    write_private(root / 'keychain-password', passphrase)
    keychain = root / 'identity.keychain-db'
    security_private('create-keychain', '-p', passphrase, str(keychain))
    run('/usr/bin/security', 'set-keychain-settings', '-lut', '600', str(keychain))
    security_private('unlock-keychain', '-p', passphrase, str(keychain))
    with tempfile.TemporaryDirectory(prefix='provision-', dir=root) as directory:
        work = Path(directory)
        run(openssl, 'req', '-x509', '-newkey', 'rsa:2048', '-sha256', '-noenc', '-days', '3650',
            '-subj', '/CN=' + NAME + '/O=Viggo Local Apps',
            '-addext', 'keyUsage=critical,digitalSignature',
            '-addext', 'extendedKeyUsage=critical,codeSigning',
            '-addext', 'basicConstraints=critical,CA:FALSE',
            '-keyout', str(work / 'private.pem'), '-out', str(root / 'certificate.pem'))
        run(openssl, 'pkcs12', '-export', '-name', NAME, '-keypbe', 'PBE-SHA1-3DES',
            '-certpbe', 'PBE-SHA1-3DES', '-macalg', 'sha1', '-inkey', str(work / 'private.pem'),
            '-in', str(root / 'certificate.pem'), '-out', str(work / 'identity.p12'),
            '-passout', 'file:' + str(root / 'keychain-password'))
        security_private('import', str(work / 'identity.p12'), '-f', 'pkcs12',
            '-k', str(keychain), '-P', passphrase, '-T', '/usr/bin/codesign')
        security_private('set-key-partition-list', '-S', 'apple-tool:,apple:,codesign:',
            '-s', '-k', passphrase, str(keychain))
    fingerprint = run(openssl, 'x509', '-in', str(root / 'certificate.pem'), '-noout', '-fingerprint', '-sha1').strip().split('=')[-1].replace(':', '')
    value = {'schema': SCHEMA, 'name': NAME, 'certificate_sha1': fingerprint,
             'certificate_sha256': hashlib.sha256((root / 'certificate.pem').read_bytes()).hexdigest(),
             'requirement': requirement(fingerprint)}
    keychain.chmod(0o600)
    (root / 'certificate.pem').chmod(0o600)
    write_private(root / 'identity.json', json.dumps(value, indent=2) + '\n')
    return load(root)


def verify(app, value):
    run('/usr/bin/codesign', '--verify', '--deep', '--strict', '-R', '=' + value['requirement'], str(app))


def sign(app, root=DEFAULT_ROOT):
    root = validate_root(root)
    value = load(root)
    ensure_searchable(root)
    passphrase = (root / 'keychain-password').read_text()
    security_private('unlock-keychain', '-p', passphrase, str(root / 'identity.keychain-db'))
    run('/usr/bin/codesign', '--force', '--sign', value['certificate_sha1'],
        '--keychain', str(root / 'identity.keychain-db'), '--timestamp=none',
        '--identifier', BUNDLE_ID, '--requirements', '=designated => ' + value['requirement'], str(app))
    verify(app, value)
    return {'method': 'persistent-local-certificate', 'certificate_sha1': value['certificate_sha1'],
            'requirement': value['requirement']}


def enrolled(root, value):
    marker=root / 'installed-identity.json'
    if not marker.exists() and not marker.is_symlink():
        return False
    if not marker.is_file() or marker.is_symlink() or marker.stat().st_mode & 0o077:
        raise RuntimeError('Unsafe installed signing marker; preserve and repair it')
    expected={'schema':'ruycast.installed-signing.v1','certificate_sha1':value['certificate_sha1'],
              'requirement':value['requirement']}
    if json.loads(marker.read_text())!=expected:
        raise RuntimeError('Installed signing marker changed; preserve the pinned identity')
    return True


def record_install(app, root=DEFAULT_ROOT):
    value=load(root)
    verify(app,value)
    if not enrolled(root,value):
        write_private(root / 'installed-identity.json',json.dumps({
            'schema':'ruycast.installed-signing.v1','certificate_sha1':value['certificate_sha1'],
            'requirement':value['requirement']},indent=2)+'\n')


def check_installed(app, root=DEFAULT_ROOT):
    """Allow the one-time ad-hoc transition; reject an unexpected certificate identity."""
    app = Path(app)
    if not app.exists():
        return
    value = load(root)
    was_enrolled=enrolled(root,value)
    run('/usr/bin/codesign', '--verify', '--deep', '--strict', str(app))
    details = run('/usr/bin/codesign', '--display', '--verbose=2', str(app))
    if 'Signature=adhoc' in details and ('Identifier=' + BUNDLE_ID + '\n') in details:
        if was_enrolled:
            raise RuntimeError('Ad-hoc downgrade refused after persistent signing enrollment')
        return
    verify(app, value)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('command', choices=['prepare', 'status', 'sign'])
    parser.add_argument('app', nargs='?', type=Path)
    parser.add_argument('--root', type=Path, default=DEFAULT_ROOT)
    args = parser.parse_args()
    if args.command == 'prepare':
        value = prepare(args.root)
    elif args.command == 'sign':
        if args.app is None:
            parser.error('sign requires an app path')
        value = sign(args.app, args.root)
    else:
        value = load(args.root)
    print(json.dumps(value, indent=2))


if __name__ == '__main__':
    try:
        main()
    except (RuntimeError, OSError, ValueError) as error:
        print(json.dumps({'ok':False,'error':str(error)}))
        raise SystemExit(1)
