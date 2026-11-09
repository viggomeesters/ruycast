#!/usr/bin/env python3
"""Compatibility entrypoint; installation behavior is owned by ruycast-install."""
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]

if __name__ == '__main__':
    tool = os.environ.get('RUYCAST_INSTALL_TOOL')
    if tool:
        os.execv(tool, [tool, 'install', '--repo', str(ROOT), *sys.argv[1:]])
    manifest = ROOT / 'rust-native/Cargo.toml'
    if not manifest.is_file() or not (ROOT / 'rust-native/src/bin/ruycast-install.rs').is_file():
        raise SystemExit('Rust installer source is unavailable; no installation attempted.')
    os.execvp('cargo', ['cargo', 'run', '--locked', '--offline', '--release', '--manifest-path', str(manifest),
                       '--target-dir', str(ROOT / '.build/installer-tools'), '--bin', 'ruycast-install',
                       '--', 'install', '--repo', str(ROOT), *sys.argv[1:]])
