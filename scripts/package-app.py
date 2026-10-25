#!/usr/bin/env python3
"""Stage an unsigned public app candidate; never install, sign or publish it."""
import argparse
import json
import plistlib
import re
import shutil
import tempfile
from pathlib import Path

import build_receipt

ROOT = Path(__file__).resolve().parents[1]
BUNDLE_ID = 'nl.viggomeesters.ruycast'


def app_files(app):
    entries = list(app.rglob('*'))
    if app.is_symlink() or any(p.is_symlink() for p in entries):
        raise RuntimeError('Candidate app contains a symlink.')
    return {str(p.relative_to(app)): build_receipt.sha(p) for p in entries if p.is_file()}


def verify_candidate(root, destination):
    root, destination = Path(root).resolve(), Path(destination).absolute()
    receipt = destination / 'candidate.json'
    if destination.is_symlink() or receipt.is_symlink():
        raise RuntimeError('Linked candidate receipt is not supported.')
    value = json.loads(receipt.read_text())
    public_source(root)
    if (value.get('schema') != 'ruycast.unsigned-candidate.v1'
            or value.get('buildReceipt') != build_receipt.verify(root)
            or value.get('manifestSHA256') != build_receipt.sha(root / 'PUBLIC-SOURCE.json')
            or not value.get('appFiles') or value['appFiles'] != app_files(destination / 'Ruycast.app')):
        raise RuntimeError('Candidate changed; prepare a fresh candidate before signing.')
    return value


def public_source(root):
    manifest = root / 'PUBLIC-SOURCE.json'
    if (root / '.go').exists() or not manifest.is_file() or manifest.is_symlink():
        raise RuntimeError('Use the clean public checkout or an allowlisted source export.')
    value = json.loads(manifest.read_text())
    if value.get('schema') != 'ruycast.public-source.v1' or not re.fullmatch(r'\d+\.\d+\.\d+', value.get('version', '')):
        raise RuntimeError('Invalid public source manifest.')
    files = value.get('files')
    if not isinstance(files, dict) or not files:
        raise RuntimeError('Empty public source manifest.')
    for name, digest in files.items():
        path = Path(name)
        if path.is_absolute() or '..' in path.parts or any(part.startswith('.') for part in path.parts if part not in {'.gitignore', '.github'}):
            raise RuntimeError('Unsafe public source path.')
        source = root / path
        if any(p.is_symlink() for p in [source, *list(source.parents)[:len(path.parts)-1]]) or not source.is_file():
            raise RuntimeError('Missing or linked public source: ' + name)
        if build_receipt.sha(source) != digest:
            raise RuntimeError('Public source changed: ' + name + '; regenerate the reviewed manifest.')
    return value


def package(root, destination, build_number):
    root, destination = Path(root).resolve(), Path(destination).absolute()
    if not re.fullmatch(r'[1-9]\d*', str(build_number)):
        raise RuntimeError('Build number must be a positive integer.')
    if destination.exists() or destination.is_symlink():
        raise RuntimeError('Destination exists; choose a fresh candidate directory.')
    if destination == root or destination in root.parents:
        raise RuntimeError('Candidate directory may not replace its source.')
    with build_receipt.artifact_lock(root):
        manifest = public_source(root)
        receipt = build_receipt.verify(root)
        destination.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='.ruycast-candidate-', dir=destination.parent) as temporary:
            stage = Path(temporary) / 'candidate'
            app = stage / 'Ruycast.app'
            for folder in ['Contents/MacOS', 'Contents/Resources']:
                (app / folder).mkdir(parents=True, exist_ok=True)
            for source, target in [('.build/swift/release/RuycastApp', 'Contents/MacOS/RuycastApp'),
                                   ('.build/rust/release/ruycast', 'Contents/Resources/ruycast')]:
                shutil.copy2(root / source, app / target)
                if build_receipt.sha(app / target) != receipt['binaries'][source]:
                    raise RuntimeError('Candidate binary changed while staging.')
            icon = root / 'assets/AppIcon.icns'
            if icon.is_file():
                shutil.copy2(icon, app / 'Contents/Resources/AppIcon.icns')
            info = {'CFBundleIdentifier': BUNDLE_ID, 'CFBundleName': 'Ruycast', 'CFBundleDisplayName': 'Ruycast',
                    'CFBundleExecutable': 'RuycastApp', 'CFBundlePackageType': 'APPL',
                    'CFBundleShortVersionString': manifest['version'], 'CFBundleVersion': str(build_number),
                    'LSMinimumSystemVersion': '14.0', 'LSUIElement': True, 'NSHighResolutionCapable': True}
            if icon.is_file():
                info['CFBundleIconFile'] = 'AppIcon'
            (app / 'Contents/Info.plist').write_bytes(plistlib.dumps(info))
            # Repeat both bindings after copying; never expose a mixed candidate.
            public_source(root)
            if build_receipt.verify(root) != receipt:
                raise RuntimeError('Build receipt changed while staging.')
            value = {'schema': 'ruycast.unsigned-candidate.v1', 'version': manifest['version'],
                     'build': str(build_number), 'buildReceipt': receipt,
                     'manifestSHA256': build_receipt.sha(root / 'PUBLIC-SOURCE.json'),
                     'appFiles': app_files(app),
                     'signedForDistribution': False, 'notarized': False, 'publishable': False}
            (stage / 'candidate.json').write_text(json.dumps(value, indent=2) + '\n')
            # mkdir is the exclusive reservation; never replace an existing artifact.
            destination.mkdir()
            try:
                for child in stage.iterdir():
                    shutil.move(str(child), str(destination / child.name))
            except BaseException:
                shutil.rmtree(destination)
                raise
    return value


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument('--out', type=Path)
    group.add_argument('--verify', type=Path)
    parser.add_argument('--build-number')
    args = parser.parse_args()
    try:
        if args.verify:
            with build_receipt.artifact_lock(ROOT):
                result = verify_candidate(ROOT, args.verify)
        else:
            if not args.build_number:
                parser.error('--out requires --build-number')
            result = package(ROOT, args.out, args.build_number)
        print(json.dumps(result, indent=2))
    except (OSError, ValueError, RuntimeError) as error:
        raise SystemExit(str(error))
