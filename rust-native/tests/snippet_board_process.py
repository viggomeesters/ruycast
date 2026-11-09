#!/usr/bin/env python3
"""Actual PreparedItem reads, one own named board + UTF-16 fault provider."""
import json
import hashlib
import os
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
SHIM = ROOT / 'target/snippet-board-shim.dylib'

class SnippetBoard(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run(['cc','-Wall','-Wextra','-Werror','-fobjc-arc','-dynamiclib',
                        '-framework','AppKit',str(ROOT/'tests/snippet_board_shim.m'),
                        '-o',str(SHIM)], check=True, capture_output=True)
        if directory := os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            output=Path(directory);output.mkdir(parents=True,exist_ok=True)
            source=ROOT/'tests/snippet_board_shim.m'
            digest=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
            (output/'snippet-board-bindings.json').write_text(json.dumps(dict(
                sources={str(source.relative_to(ROOT.parent)):digest(source)},
                artifacts={str(SHIM.relative_to(ROOT.parent)):digest(SHIM)}),indent=2))

    def case(self, mode, full=False, partial=False):
        env=dict(os.environ, DYLD_INSERT_LIBRARIES=str(SHIM))
        run=subprocess.run([str(ROOT/'target/debug/snippet-probe'),'owned-board',str(mode)],
                           capture_output=True,env=env,timeout=10)
        self.assertEqual(run.returncode,0,run.stderr.decode(errors='replace'))
        result=json.loads(run.stdout)
        self.assertEqual(result,dict(original=True,full=full,partial=partial,
                                     restored=True,generalBoardAbortGuard=True))

    def test_equal_native_text_html_and_unordered_types(self): self.case(0,True,True)
    def test_lone_high_surrogate_text(self): self.case(1)
    def test_lone_low_surrogate_html(self): self.case(2)
    def test_lone_surrogate_type(self): self.case(3)
    def test_partial_text_with_missing_html(self): self.case(4,False,True)
    def test_foreign_html_is_not_owned(self): self.case(5)
    def test_extra_type_is_not_owned(self): self.case(6)
    def test_difference_after_embedded_nul(self): self.case(7)
    def test_difference_after_utf16_chunk_boundary(self): self.case(8)
    def test_equal_multiple_utf16_chunks(self): self.case(9,True,True)

if __name__=='__main__': unittest.main(verbosity=2)
