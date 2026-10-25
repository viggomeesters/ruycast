#!/usr/bin/env python3
"""Import enabled static quicklinks and app aliases from a decrypted Raycast export.

Output remains private. Existing Raycast data and shortcuts are never modified.
Use --include-templates and --include-snippets for Ruycast 0.2+ content support.
Raycast-dependent commands are not invoked or modified.
"""
import argparse
import json
import hashlib
import re
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile
from urllib.parse import unquote, urlsplit

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('export', type=Path)
p.add_argument('--config', type=Path, help='Explicit offline config; defaults to the active CLI configuration')
p.add_argument('--cli', type=Path, default=Path.home() / '.local/bin/ruycast')
p.add_argument('--apply', action='store_true')
p.add_argument('--include-snippets', action='store_true')
p.add_argument('--include-templates', action='store_true')
args = p.parse_args()
if args.config is None:
    location = json.loads(subprocess.check_output([str(args.cli), 'config', 'path']))
    if location.get('ok') is not True:
        raise SystemExit('Cannot resolve the active configuration.')
    args.config = Path(location['result'])
data = json.loads(args.export.read_text())
config_bytes = args.config.read_bytes()
config = json.loads(config_bytes)
roots = {r['key']: r for r in data['builtin_package_rootSearch']['rootSearch']}
info = {r['key']: r for r in data['raycast_user_activity']['activityInfo']}
commands = config['commands']
names = {n.lower() for c in commands for n in [c['id'], *c.get('aliases', [])]}
by_app = {c['target']: c for c in commands if c['kind'] == 'app'}
inventory = {}
for directory in ['/Applications', '/System/Applications', '/System/Library/CoreServices', str(Path.home() / 'Applications')]:
    for app in Path(directory).glob('*.app'):
        try:
            meta = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
            inventory[meta['CFBundleIdentifier']] = (app, meta)
        except (OSError, KeyError, ValueError):
            pass
counts = dict(appAliases=0, apps=0, quicklinks=0, parameterizedSkipped=0, disabledSkipped=0, unsupportedSkipped=0)


def aliases(key):
    alias = roots.get(key, {}).get('alias', '').strip()
    if alias and alias.lower() not in names:
        names.add(alias.lower())
        return [alias]
    return []


for row in roots.values():
    if row.get('type') != 'systemApp' or not row.get('alias'):
        continue
    bid = row['key']
    if bid not in inventory:
        counts['unsupportedSkipped'] += 1
        continue
    path, bundle = inventory[bid]
    extra = aliases(row['key'])
    if not extra:
        continue
    if bid in by_app:
        by_app[bid].setdefault('aliases', []).extend(extra)
    else:
        cid = 'app.' + bid
        if cid.lower() in names:
            continue
        title = bundle.get('CFBundleDisplayName', bundle.get('CFBundleName', path.stem))
        command = dict(id=cid, title=title, kind='app', target=bid, aliases=extra)
        commands.append(command)
        names.add(cid.lower())
        by_app[bid] = command
        counts['apps'] += 1
    counts['appAliases'] += len(extra)

for link in data['builtin_package_quicklinks']['quicklinks']:
    if not link.get('isEnabled', True):
        counts['disabledSkipped'] += 1
        continue
    url = link['url']
    if not args.include_templates and ('{' in url or '}' in url):
        counts['parameterizedSkipped'] += 1
        continue
    cid = 'link.' + link['uuid'].lower()
    if cid.lower() in names:
        continue
    parsed = urlsplit(url)
    if re.fullmatch(r'[A-Za-z][A-Za-z0-9+.-]*', parsed.scheme) and parsed.scheme.lower() not in ['javascript', 'data', 'file']:
        kind, target = 'url', url
    elif parsed.scheme == 'file' and parsed.netloc in ['', 'localhost']:
        kind, target = 'path', unquote(parsed.path)
    elif url.startswith('/') or url.startswith('~/'):
        kind, target = 'path', url
    else:
        counts['unsupportedSkipped'] += 1
        continue
    names.add(cid.lower())
    commands.append(dict(id=cid, title=link['name'], kind=kind, target=target,
                         aliases=aliases('raycastQuicklink_' + link['uuid'])))
    counts['quicklinks'] += 1

if args.include_snippets:
    counts['snippets'] = 0
    keywords = {c.get('keyword') for c in commands if c.get('keyword')}
    for snippet in data.get('builtin_package_snippets', {}).get('snippets', []):
        keyword = snippet.get('alias', '').strip()
        cid = 'snippet.' + hashlib.sha256((snippet['name']+'\0'+keyword).encode()).hexdigest()[:20]
        if cid.lower() in names:
            continue
        command = dict(id=cid, title=snippet['name'], kind='snippet', target=snippet['text'])
        if keyword and keyword not in keywords:
            command['keyword'] = keyword
            keywords.add(keyword)
        commands.append(command)
        names.add(cid.lower())
        counts['snippets'] += 1

if args.apply:
    # The Rust CLI validates, locks, detects edits, writes atomically and reloads.
    descriptor, path = tempfile.mkstemp(prefix='.raycast-import-', suffix='.json', dir=args.config.parent)
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, 'w') as f:
            json.dump(commands, f, ensure_ascii=False)
        result = subprocess.run([str(args.cli), 'config', 'set', 'commands', '@' + path, '--expect-sha256', hashlib.sha256(config_bytes).hexdigest()], capture_output=True, text=True)
        if result.returncode:
            raise SystemExit(result.stdout + result.stderr)
    finally:
        os.unlink(path)
print(json.dumps(dict(applied=args.apply, imported=counts, totalCommands=len(commands), hotkeysImported=0), sort_keys=True))
