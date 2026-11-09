#!/usr/bin/env python3
"""Actual Swift snippet output/owned-board reference versus Rust. No paste,
keyboard, activation or general clipboard access. Oracle sources are production.
"""
import hashlib
import json
import os
from pathlib import Path
import random
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
REPO = ROOT.parent
REFERENCE_DIR = ROOT / 'target/snippet-reference'
REFERENCE = REFERENCE_DIR / 'snippet-reference'
BIN = ROOT / 'target/debug/snippet-probe'
SHIM = REFERENCE_DIR / 'snippet-faults.dylib'


def fixture(source, mode='html', **extra):
    return dict(source=source, mode=mode, **extra)


class SnippetOutput(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.comparisons = []
        cls.pathological = []
        REFERENCE_DIR.mkdir(parents=True, exist_ok=True)
        sources = [REPO / 'Sources/LauncherModels/Models.swift',
                   REPO / 'Sources/LauncherCore/Templates.swift',
                   REPO / 'Sources/LauncherCore/SnippetMatcher.swift',
                   REPO / 'Sources/LauncherCore/SnippetOutput.swift',
                   REPO / 'Sources/RuycastApp/SnippetClipboard.swift',
                   ROOT / 'tests/snippet_reference.swift']
        artifacts = [REFERENCE, REFERENCE_DIR / 'libLauncherModels.dylib',
                     REFERENCE_DIR / 'libLauncherCore.dylib', SHIM]
        commands = [
            ['swiftc', '-emit-library', '-emit-module', '-module-name', 'LauncherModels',
             '-o', str(artifacts[1]), '-emit-module-path', str(REFERENCE_DIR / 'LauncherModels.swiftmodule'), str(sources[0])],
            ['swiftc', '-emit-library', '-emit-module', '-module-name', 'LauncherCore',
             '-I', str(REFERENCE_DIR), '-L', str(REFERENCE_DIR), '-lLauncherModels',
             '-Xlinker', '-rpath', '-Xlinker', str(REFERENCE_DIR),
             '-o', str(artifacts[2]), '-emit-module-path', str(REFERENCE_DIR / 'LauncherCore.swiftmodule'),
             *map(str, sources[1:4])],
            ['swiftc', '-parse-as-library', '-I', str(REFERENCE_DIR), '-L', str(REFERENCE_DIR),
             '-lLauncherModels', '-lLauncherCore', '-Xlinker', '-rpath', '-Xlinker', str(REFERENCE_DIR),
             *map(str, sources[4:]), '-o', str(REFERENCE)],
            ['clang', '-dynamiclib', '-fobjc-arc', '-framework', 'AppKit',
             str(ROOT / 'tests/snippet_faults.m'), '-o', str(SHIM)],
        ]
        for command in commands:
            run = subprocess.run(command, capture_output=True, text=True, timeout=90)
            if run.returncode:
                raise AssertionError(run.stderr)
        if evidence := os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            p = Path(evidence) / 'snippet-reference-bindings.json'
            p.parent.mkdir(parents=True, exist_ok=True)
            sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
            p.write_text(json.dumps(dict(sources={str(p.relative_to(REPO)): sha(p) for p in sources + [ROOT / 'tests/snippet_faults.m']},
                                        artifacts={str(p.relative_to(REPO)): sha(p) for p in artifacts}), indent=2))

    @classmethod
    def tearDownClass(cls):
        if evidence := os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            (Path(evidence) / 'snippet-comparisons.json').write_text(json.dumps(dict(
                comparedCases=sum(len(v['cases']) for v in cls.comparisons), comparisons=cls.comparisons,
                pathological=cls.pathological), ensure_ascii=False, indent=2))

    def invoke(self, binary, cases, fault=None):
        env = {k: v for k, v in os.environ.items() if not k.startswith(('RUYCAST_', 'RUSTCAST_')) and k != 'DYLD_INSERT_LIBRARIES'}
        if fault:
            env.update(DYLD_INSERT_LIBRARIES=str(SHIM),RUYCAST_SNIPPET_FIXTURE='own-named-board',RUYCAST_SNIPPET_FAULT=fault)
        run = subprocess.run([str(binary)], input=json.dumps(cases, ensure_ascii=False),
                             capture_output=True, text=True, env=env, timeout=20)
        self.assertEqual(run.returncode, 0, run.stderr + run.stdout[:500])
        result = json.loads(run.stdout)
        self.assertTrue(result['ok'])
        return result['cases']

    def compare(self, cases):
        before, after = self.invoke(REFERENCE, cases), self.invoke(BIN, cases)
        self.assertEqual(len(before), len(after))
        for index, (a, b) in enumerate(zip(before, after)):
            self.assertEqual(a, b, json.dumps(dict(index=index, case=cases[index], swift=a, rust=b), ensure_ascii=False))
        self.comparisons.append(dict(test=self.id(), cases=cases, expected=before, actual=after))
        return after

    def test_plain_unknown_modes_and_complete_literal_text(self):
        self.compare([fixture('<img> [Guide](https://example.com)\nΩ\0', mode, cursor=500)
                      for mode in [None, '', 'plain', 'HTML', 'other']])

    def test_teams_paragraphs_html_breaks_and_safe_escaping(self):
        source = 'Hi 👋\n\nSee [Guide & tips](https://example.com/a_(b)?x=1&y=2)\nGoodbye\n'
        actual = self.compare([fixture(source, mode, cursor=len(source.encode('utf-16-le')) // 2)
                               for mode in ['teams', 'html']])
        self.assertNotIn('<table', actual[0]['payload']['html'])
        self.assertIn('<p style="margin:0"><br></p>', actual[0]['payload']['html'])
        self.assertIn('Guide &amp; tips</a>', actual[0]['payload']['html'])
        self.compare([fixture('<img src=x> [X](javascript:alert(1)) [Y](file:///etc/passwd) & " < >', mode)
                      for mode in ['teams', 'html']])

    def test_links_balanced_parentheses_escape_labels_and_invalid_syntax(self):
        sources = ['[A](https://example.com/a_(b))', '[A](http://a/(b(c)))',
                   '[line1\nline2](https://example.com)', '[line1\rline2](https://example.com)',
                   '[a\\]b](https://example.com)', '[a\\\\b](https://example.com)',
                   '[a\\qb](https://example.com)', '[](https://example.com)', '[A]()',
                   '[A](mailto:me@example.com)', '[A](mailto:)', '[A](https://)',
                   '[A](HTTPS://EXAMPLE.COM)', '[A](https://é.example/Ω)',
                   '[A](javascript:alert(1))', '[A](https://a/ b)',
                   '[A](https://example.com/a\u200bb)', '[A](https://example.com/%zz)',
                   '[A](https://example.com)\u0301', '[[A]](https://example.com)']
        self.compare([fixture(s) for s in sources])

    def test_url_validation_and_link_source_preserve_actual_bridge_behavior(self):
        urls = ['', 'mailto:', 'mailto:a', 'MAILTO:Ω', 'mailto:\u0301', 'mailto:\ufe0f', 'mailto:\u200d',
                'mailto:\u0301a', 'http://', 'https:///path',
                'https://example.com', 'HTTPS://EXAMPLE.COM/Ω', 'http://[::1]',
                'http://é.example/Ω', 'https://example.com/a(b)', 'file:///a',
                'javascript:alert(1)', 'https://example.com/%zz', 'http://user:pw@example.com',
                'https://example.com/\0']
        urls += ['https://example.com/' + ch + 'b' for ch in [' ', '\t', '\n', '\r', '\x85', '\u00a0', '\u200b', '\u200d', '\u2028', '\ufeff', '\u0301', '\u2060', ' \u0301', '\u00a0\u0301', '\r\n', '\n\u0301', '\t\u0301', '\r\n\u0301', '\u0600 ', '\u0600\u2000', '\u0600\t', '\u0600\u00a0\u0301']]
        self.compare([fixture('', label='[Guide]\\ Ω', url=url) for url in urls])

    def test_utf16_cursor_mapping_at_every_position_and_grapheme_boundary(self):
        sources = ['a[😀](https://example.com)b', '[a\\]b](https://example.com)',
                   '👩\u200d💻 e\u0301 [x](mailto:a)\r\n[🇳🇱](https://example.com)end',
                   '[\\[]](https://example.com)', 'a[hi](https://example.com)b',
                   'plain 😀e\u0301\r\n', '👩\u200d💻🇳🇱👨\u200d👩\u200d👧\u200d👦1\ufe0f\u20e3', '']
        for source in sources:
            count = len(source.encode('utf-16-le')) // 2
            self.compare([fixture(source, mode, cursor=c) for mode in ['html', 'teams', 'plain']
                          for c in [None, -1, *range(count + 2)]])

    def test_native_named_board_contains_one_complete_plain_or_rich_item(self):
        self.compare([fixture('Hi 👋\n\n[Guide](https://example.com/?a=1&b=2)\n', mode,
                              clipboard=True, seed=True, cursor=2) for mode in [None, 'plain', 'teams', 'html']])

    def test_grapheme_suffix_count_across_scripts_controls_and_joiners(self):
        sources=['क्ष','क्\u200dष','ক্\u200dষ','ශ්‍රී','각','اَلْعَرَبِيَّةُ',
                 '\u0600a','a\u034f\u0301','a\0\u0301',' \u200d','\t\u0301',
                 '\u2028\u0301','👩🏽\u200d💻','🇺🇳🇳🇱🇦','\r\n\r\n',
                 '\U00011f02\U00011f04\U00011f42\U00011f12',
                 '\U0001e4d0\U0001e4ec']
        self.compare([fixture(source,'plain',cursor=c) for source in sources
                      for c in range(len(source.encode('utf-16-le'))//2+2)])

    def test_real_sdk_item_failure_keeps_previous_board_but_write_failure_is_partial(self):
        positive=[fixture('Hi [Guide](https://example.com)',mode,clipboard=True,seed=True)
                  for mode in [None,'plain','teams','html']]
        before,after=self.invoke(REFERENCE,positive,'pass'),self.invoke(BIN,positive,'pass')
        self.assertEqual(before,after)
        self.assertTrue(all(c['clipboard']['success'] and c['clipboard']['items']==1 for c in after))
        self.comparisons.append(dict(test=self.id(),fault='pass-with-general-access-forbidden',
                                     cases=positive,expected=before,actual=after))
        cases=[fixture('Hi [Guide](https://example.com)', 'html', clipboard=True, seed=True)]
        for fault in ['string','html','write']:
            before,after=self.invoke(REFERENCE,cases,fault),self.invoke(BIN,cases,fault)
            self.assertEqual(before,after)
            value=after[0]['clipboard'];self.assertFalse(value['success'])
            if fault=='write':
                self.assertEqual((value['text'],value['html'],value['items']),(None,None,0))
                self.assertGreater(value['changes'],0)
            else:
                self.assertEqual((value['text'],value['html']),('Own previous text','<b>Own previous HTML</b>'))
                self.assertEqual(value['changes'],0)
            self.comparisons.append(dict(test=self.id(),fault=fault,cases=cases,expected=before,actual=after))

    def test_large_source_is_complete_without_an_application_quota(self):
        source = ('[Guide](https://example.com)\n' * 4000) + 'Ω & < end'
        actual = self.compare([fixture(source, 'teams', cursor=len(source.encode('utf-16-le')) // 2)])
        self.assertEqual(actual[0]['payload']['text'], ('Guide\n' * 4000) + 'Ω & < end')

    def test_generated_unicode_and_markup_corpus(self):
        rng = random.Random(86031)
        pieces = ['plain', '😀', 'e\u0301', '\r\n', '\u200b', '&<>"', '[a](https://a)',
                  '[b\\]c](mailto:a)', '[x](javascript:a)', '[x](', '\\', '[]()']
        cases = []
        for _ in range(120):
            # Newlines bound malformed pieces so the unchanged baseline's
            # nested quantifier cannot consume a later unrelated long URL.
            # That known pathological case has its own bounded test below.
            source = '\n'.join(rng.choice(pieces) for _ in range(rng.randrange(1, 14)))
            cases.append(fixture(source, rng.choice(['plain', 'teams', 'html']),
                                 cursor=rng.randrange(len(source.encode('utf-16-le')) // 2 + 1)))
        self.compare(cases)

    def test_invalid_long_url_does_not_repeat_legacy_exponential_match(self):
        source = '[A](https://example.com/a b)'
        cases = [fixture(source)]
        actual = self.invoke(BIN, cases)
        self.assertEqual(actual[0]['links'], [])
        self.assertEqual(actual[0]['payload']['text'], source)
        env = {k:v for k,v in os.environ.items() if not k.startswith(('RUYCAST_', 'RUSTCAST_')) and k != 'DYLD_INSERT_LIBRARIES'}
        try:
            run = subprocess.run([str(REFERENCE)], input=json.dumps(cases), text=True,
                                 capture_output=True, env=env, timeout=1)
        except subprocess.TimeoutExpired:
            self.pathological.append(dict(case=cases[0], actual=actual[0],
                                          actualSwiftOutcome='No response before 1-second fixture deadline; process killed/reaped.',
                                          differentialPass=False,
                                          semantics='(A+|B)* and (A|B)* have identical URL language/captures; whitespace makes this URL invalid.'))
        else:
            self.assertEqual(run.returncode, 0, run.stderr)
            self.assertEqual(json.loads(run.stdout)['cases'], actual)
            self.pathological.append(dict(case=cases[0], differentialPass=True))


if __name__ == '__main__':
    unittest.main(verbosity=2)
