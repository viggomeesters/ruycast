#!/usr/bin/env python3
"""Compatibility dispatch; the full validation matrix and receipts are Rust."""
import os
from pathlib import Path
import sys

if __name__ == '__main__':
    root = Path(__file__).resolve().parents[2]
    os.execvp('cargo', ['cargo', 'run', '--quiet', '--locked', '--offline',
                      '--target-dir', str(root / '.build/delivery-tools'),
                      '--manifest-path', str(root / 'rust-native/Cargo.toml'), '--bin', 'ruycast-build',
                      '--', 'compat', 'native-check', '--repo', str(root), '--', *sys.argv[1:]])
