#!/usr/bin/env python3
"""Read-only Raycast export normalization; apply only through Ruycast's live writer.

Dry run is the default. Never writes clipboard.json or accesses the pasteboard.
Reports contain counts and hashes, never clipboard bodies or source paths.
"""
import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import socket
import stat
import tempfile

DEFAULT_EXPORT = Path.home() / '.local/state/raycast-export-trial-20261020/post-import-20261021-083502.json'
MAX_WIRE = 1_000_000


def normalize(data, max_text_bytes=100_000):
    package = data.get('builtin_package_clipboardHistory', {})
    rows = package.get('clipboardHistoryRecords', [])
    if not isinstance(rows, list):
        raise ValueError('Invalid clipboard history export structure.')
    counts = dict(records=len(rows), text=0, files=0, missingImageAssets=0,
                  unsupportedImageAssets=0, missingFileReferences=0, unsupported=0, invalid=0, oversized=0, duplicate=0)
    clips, seen = [], set()
    for row in rows:
        try:
            if not isinstance(row, dict):
                raise ValueError()
            category = row.get('category')
            if category == 'image':
                path = Path(row.get('filePath', ''))
                # Exported image paths are outside the export; never invent pixels.
                key = 'unsupportedImageAssets' if path.is_absolute() and path.is_file() else 'missingImageAssets'
                counts[key] += 1
                continue
            if category not in ('text', 'link', 'file'):
                counts['unsupported'] += 1
                continue
            text, source = row.get('text'), row.get('applicationPath', '')
            if not isinstance(text, str) or not isinstance(source, str):
                raise ValueError()
            date = dt.datetime.fromisoformat(row['createdAt'].replace('Z', '+00:00'))
            if date.tzinfo is None:
                raise ValueError()
            canonical = json.dumps(row, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()
            identifier = 'raycast-' + hashlib.sha256(canonical).hexdigest()
            item = dict(id=identifier, text=text, source=source,
                        date=date.astimezone(dt.timezone.utc).isoformat(timespec='seconds').replace('+00:00', 'Z'))
            if category == 'file':
                path = row.get('filePath')
                if not isinstance(path, str) or not path.startswith('/') or '\x00' in path or '\n' in path:
                    raise ValueError()
                item.update(contentType='files', fileURLs=[Path(path).as_uri()], text='Files: ' + Path(path).name)
                if not Path(path).exists():
                    counts['missingFileReferences'] += 1
            if not item['text'] or len(item['text'].encode()) > max_text_bytes or len(source.encode()) > 16384:
                counts['oversized'] += 1
                continue
            if identifier in seen:
                counts['duplicate'] += 1
                continue
            seen.add(identifier)
            clips.append(item)
            counts['files' if category == 'file' else 'text'] += 1
        except (ValueError, KeyError, TypeError, OverflowError, OSError):
            counts['invalid'] += 1
    clips.sort(key=lambda item: (item['date'], item['id']), reverse=True)
    return clips, counts


def batches(clips, source_hash):
    batch = []
    for item in clips:
        proposed = batch + [item]
        if batch and (len(proposed) > 100 or len(json.dumps(request(proposed, source_hash)).encode()) >= MAX_WIRE):
            yield request(batch, source_hash)
            batch = []
        batch.append(item)
        if len(json.dumps(request(batch, source_hash)).encode()) >= MAX_WIRE:
            raise ValueError('A clipboard item exceeds the import transport limit.')
    if batch:
        yield request(batch, source_hash)


def request(items, source_hash):
    fingerprint = hashlib.sha256(json.dumps(items, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    return dict(op='clipboard.import', sourceSHA256=source_hash, batchID=fingerprint, items=items)


def send(path, payload):
    info = path.lstat()
    if not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.getuid():
        raise ValueError('Ruycast control socket is not an owned socket.')
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(30)
        connection.connect(str(path))
        connection.sendall(json.dumps(payload).encode() + b'\n')
        response = bytearray()
        while b'\n' not in response:
            part = connection.recv(min(65536, MAX_WIRE - len(response)))
            if not part:
                break
            response.extend(part)
            if len(response) >= MAX_WIRE:
                raise ValueError('Ruycast import response exceeds its limit.')
        reply = json.loads(response.split(b'\n', 1)[0])
        if reply.get('ok') is not True:
            # Server errors may contain private values; don't echo them.
            raise ValueError('Ruycast rejected clipboard import; original export is unchanged.')
        return reply.get('result', {})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('export', nargs='?', type=Path, default=DEFAULT_EXPORT)
    parser.add_argument('--socket', type=Path, default=Path.home() / '.config/ruycast/control.sock')
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--max-text-bytes', type=int, default=100_000)
    args = parser.parse_args()
    try:
        raw = args.export.read_bytes()
        source_hash = hashlib.sha256(raw).hexdigest()
        clips, counts = normalize(json.loads(raw), args.max_text_bytes)
        report = dict(sourceSHA256=source_hash, counts=counts, candidates=len(clips), applied=False,
                      policy='Host settings determine retention, exclusions, size limits and history capacity.')
        if args.apply:
            # Private normalized interchange file is short-lived and never a history writer.
            with tempfile.TemporaryDirectory(prefix='ruycast-clipboard-import-') as temporary:
                normalized = Path(temporary) / 'normalized.json'
                descriptor = os.open(normalized, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                with os.fdopen(descriptor, 'w') as stream:
                    json.dump(clips, stream)
                totals = {}
                count = 0
                # Newest batches first: the host's retention policy owns final capacity.
                for payload in batches(json.loads(normalized.read_text()), source_hash):
                    result = send(args.socket, payload)
                    count += 1
                    for key, value in result.items():
                        if isinstance(value, int) and not isinstance(value, bool):
                            totals[key] = value if key == "retained" else totals.get(key, 0) + value
                report.update(applied=True, batches=count, hostCounts=totals)
        print(json.dumps(report, sort_keys=True))
    except (OSError, ValueError, TypeError, KeyError):
        parser.exit(1, 'Clipboard import failed; source unchanged. Check export and running host import support.\n')


if __name__ == '__main__':
    main()
