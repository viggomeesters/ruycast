#!/usr/bin/env python3
"""Compare the Rust engine against actual current Swift template/model sources.
All values/date/locale/clipboard are synthetic arguments; no pasteboard or UI.
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
BIN = ROOT / 'target/debug/template-probe'
REFERENCE_DIR = ROOT / 'target/template-reference'
REFERENCE = REFERENCE_DIR / 'template-reference'


def fixture(target, *, kind='snippet', values=(), **extra):
    result = dict(command=dict(id='fixture', title='Fixture', kind=kind, target=target),
                  values=list(values), now=1761411300, locale='en_GB', timeZone='UTC')
    result.update(extra)
    return result


class Templates(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.comparisons = []
        REFERENCE_DIR.mkdir(exist_ok=True)
        commands = [
            ['swiftc', '-emit-library', '-emit-module', '-module-name', 'LauncherModels',
             '-o', str(REFERENCE_DIR / 'libLauncherModels.dylib'), '-emit-module-path',
             str(REFERENCE_DIR / 'LauncherModels.swiftmodule'), str(REPO / 'Sources/LauncherModels/Models.swift')],
            ['swiftc', '-parse-as-library', '-I', str(REFERENCE_DIR), '-L', str(REFERENCE_DIR),
             '-lLauncherModels', '-Xlinker', '-rpath', '-Xlinker', str(REFERENCE_DIR),
             str(REPO / 'Sources/LauncherCore/Templates.swift'), str(ROOT / 'tests/template_reference.swift'),
             '-o', str(REFERENCE)],
        ]
        for command in commands:
            run = subprocess.run(command, capture_output=True, text=True, timeout=90)
            if run.returncode:
                raise AssertionError(run.stderr)
        evidence = os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR')
        if evidence:
            path = Path(evidence) / 'template-reference-bindings.json'
            path.parent.mkdir(parents=True, exist_ok=True)
            sources = [REPO / 'Sources/LauncherModels/Models.swift', REPO / 'Sources/LauncherCore/Templates.swift', ROOT / 'tests/template_reference.swift', REPO / 'Sources/RuycastApp/AppState.swift']
            artifacts = [REFERENCE, REFERENCE_DIR / 'libLauncherModels.dylib']
            path.write_text(json.dumps(dict(sources={str(p.relative_to(REPO)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources}, artifacts={str(p.relative_to(REPO)): hashlib.sha256(p.read_bytes()).hexdigest() for p in artifacts}), indent=2))

    @classmethod
    def tearDownClass(cls):
        evidence = os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR')
        if evidence:
            path = Path(evidence) / 'template-comparisons.json'
            path.write_text(json.dumps(dict(comparedCases=sum(len(v['cases']) for v in cls.comparisons), comparisons=cls.comparisons), ensure_ascii=False, indent=2))

    def invoke(self, binary, cases):
        env = dict(os.environ)
        for key in list(env):
            if key.startswith(('RUYCAST_', 'RUSTCAST_')) or key == 'DYLD_INSERT_LIBRARIES':
                env.pop(key)
        run = subprocess.run([str(binary)], input=json.dumps(cases, ensure_ascii=False),
                             text=True, capture_output=True, timeout=15, env=env)
        self.assertEqual(run.returncode, 0, run.stderr + run.stdout)
        result = json.loads(run.stdout)
        self.assertTrue(result['ok'])
        return result['cases']

    def compare(self, cases):
        expected, actual = self.invoke(REFERENCE, cases), self.invoke(BIN, cases)
        for index, (case, before, after) in enumerate(zip(cases, expected, actual)):
            self.assertEqual(before, after, json.dumps(dict(index=index, case=case, swift=before, rust=after), ensure_ascii=False))
        self.assertEqual(len(actual), len(cases))
        self.comparisons.append(dict(test=self.id(), cases=cases, expected=expected, actual=actual))
        return actual

    def test_clipboard_demand_matches_current_appstate_character_semantics(self):
        source=(REPO/"Sources/RuycastApp/AppState.swift").read_text()
        self.assertIn('cmd.target.contains("{clipboard}") ? NSPasteboard.general.string(forType:.string):nil',source)
        cases=[fixture(value) for value in ["{clipboard}","{clipboard}\u0301","{clipboard}\ufe0f","{clipboard}\u200d", "{clipboard}\u0301/{clipboard}","{{clipboard}}", "{x name=\"clipboard\"}","\u0301{clipboard}", "\u200d{clipboard}", "plain", "{CLIPBOARD}","{clipboard }", "{clipboard}{clipboard}"]]
        actual=self.compare(cases)
        self.assertEqual([c["needsClipboard"] for c in actual[:5]], [True,False,False,False,True])

    def test_large_invalid_clipboard_demand_corpus(self):
        corpus="Ω{clipboard}\u0301"*4096
        actual=self.compare([fixture(corpus), fixture(corpus+"{clipboard}")])
        self.assertEqual([c["needsClipboard"] for c in actual],[False,True])

    def test_application_data_is_not_bounded_by_socket_or_fixture_limits(self):
        cases = [fixture('https://fixture.invalid/' + '{cursor}' * 16385, kind='url'),
                 fixture('https://fixture.invalid/{x}{x}', kind='url',
                         values=[['x', 'a' * 600000]])]
        actual = self.compare(cases)
        self.assertEqual(actual[0]['render']['text'], 'https://fixture.invalid/')
        self.assertEqual(len(actual[1]['render']['text']), 1200024)

    def test_field_order_defaults_options_and_prompt_ownership(self):
        cases = [fixture('{query}/{argument}/{x name="Place" default="Home"}/{x name="Place" default="Else"}'),
                 fixture('{x default="Ready"}'), fixture('{x default="Ready" options="Alpha,Beta"}'),
                 fixture('{x options=" A, B,A,, , B,é,e\u0301,K,K,1,１"}'),
                 fixture('{x options="A,\u0301B, C"}'), fixture('{x name="" default="yes"}'),
                 fixture('{x xname="Substring" xdefault="V" xoptions="One,Two"}'),
                 fixture('{clipboard}{cursor}{date}{time}{datetime}{day}{date format="yyyy"}{datetime:yyyy}')]
        for kind in ['app', 'path', 'script', 'url', 'snippet']:
            case = fixture('{input}{query}', kind=kind, values=[['input','I'],['Query','Q']])
            case['command']['inputPrompt'] = 'Prompt Ω'
            cases.append(case)
        self.compare(cases)

    def test_canonical_fields_options_and_ordered_value_insertion(self):
        cases = [fixture('{é}{e\u0301}', values=[['e\u0301','First'],['é','Last']]),
                 fixture('{K}{K}{K}', values=[['K','Value']]),
                 fixture('{x name="é" options="é,e\u0301,É,Ａ,A"}', values=[['e\u0301','Picked']]),
                 fixture('{x name="input"}{x name="INPUT"}', values=[['input','lower'],['INPUT','upper']])]
        cases[-1]['command']['inputPrompt'] = 'Existing input'
        self.compare(cases)

    def test_url_percent_encoding_is_only_for_user_fields(self):
        cases = [fixture('https://fixture.invalid/{query}?x={value}&c={clipboard}&d={date}', kind='url',
                         values=[['Query','a&b #/Ωé\u0301😀_-~.\n\0'],['value','  literal  ']], clipboard='raw / Ω'),
                 fixture('https://fixture.invalid/{query}', kind='url', values=[['Query','{}$(literal)']]),
                 fixture('{query}', kind='snippet', values=[['Query','{}$(literal)']])]
        self.compare(cases)

    def test_missing_blank_and_explicit_empty_values_preserve_reverse_error_order(self):
        cases = [fixture('{first}/{second}'), fixture('{clipboard}{missing}'), fixture('{missing}{clipboard}'),
                 fixture('{query default="fallback"}', values=[['Query','']]),
                 fixture('{x default="  \u2000\n"}'), fixture('{x}', values=[['x','\u0085\u2028']]),
                 fixture('{x}', values=[['x','\u200b']])]
        self.compare(cases)

    def test_utf16_cursor_multiple_marks_and_literal_replacements(self):
        cases = [fixture('😀{x}{cursor}tail{y}', values=[['x','Ω👨\u200d👩\u200d👧\u200d👦'],['y','z']]),
                 fixture('{x}{cursor}a{cursor}b{y}', values=[['x','é\u0301'],['y','end']]),
                 fixture('{cursor}{clipboard}{x}', values=[['x','{cursor}{missing}']], clipboard='{query}'),
                 fixture('a\0{cursor}b'), fixture('plain 😀 Ω')]
        actual = self.compare(cases)
        self.assertEqual(actual[1]['render']['cursor'], 2)

    def test_fixed_dates_locales_zones_dst_and_custom_patterns(self):
        cases = []
        target = '{date}|{time}|{datetime}|{day}|{date format="yyyy-MM-dd HH:mm:ss ZZZZZ"}|{datetime:yyyy/MM/dd}|{date bad}|{date 👨\u200d👩\u200d👧\u200d👦é\u0301abcdef}'
        for locale in ['en_GB', 'nl_NL', 'ar_EG', 'ja_JP', 'th_TH', 'en_US_POSIX']:
            for zone in ['UTC', 'Europe/Amsterdam', 'America/New_York', 'Asia/Kathmandu']:
                cases.append(fixture(target, locale=locale, timeZone=zone))
        for now in [-1, 0, 1711845000, 1729992600, 1761411300.125]:
            cases.append(fixture(target, now=now, timeZone='Europe/Amsterdam'))
        self.compare(cases)

    def test_current_locale_and_zone_defaults_without_global_mutation(self):
        self.compare([fixture('{date}|{time}|{datetime}|{day}', locale=None, timeZone=None)])

    def test_malformed_nested_newline_and_combining_brace_grammar(self):
        strings = ['{}', '{{query}}', '{a{b}c}', '{unclosed', 'close}', '{x\nname="Multi\nline" default="Y"}',
                   '{date\tformat="yyyy"}', '{datetime:}', '{date }', '{date x}', '{query}{query}',
                   '{\u0301x}', '\u0301{x}\u0301', '{x}\u200d😀', '{x name="a{b}c"}',
                   '{x default="unterminated}', '{x options="a,b" default=" x "}']
        self.compare([fixture(s, values=[['query','q'],['Query','q'],['x','X'],['b','B']]) for s in strings])

    def test_seeded_mixed_token_corpus(self):
        rng = random.Random(86031)
        atoms = ['plain', 'Ω', '😀', '\n', '\0', '{query}', '{argument}', '{x}', '{cursor}', '{clipboard}', '{date}',
                 '{x default="Yes"}', '{x name="é" default="D"}', '{x options="A,B,A"}', '{}', '{{x}}', '{', '}', '{date format="yyyy"}']
        cases = [fixture(''.join(rng.choices(atoms, k=rng.randint(1, 24))),
                         kind=rng.choice(['snippet','url']), clipboard=rng.choice([None,'Ω\n{cursor}']),
                         values=rng.choice([[], [['Query','a&b'],['x','X']], [['x',' '],['é','E']]])) for _ in range(300)]
        self.compare(cases)

    def test_repeated_large_values_and_cursor_parity(self):
        self.compare([fixture(('prefix{x}'*500)+'{cursor}end', values=[['x','Ω😀'*16]]),
                      fixture('{x options="'+(','.join(['A','é','e\u0301','B']*2500))+'"}', values=[['x','B']])])

    def test_combining_character_after_date_prefix_is_not_an_automatic_token(self):
        cases = []
        for prefix in ['date ', 'datetime:']:
            for mark in ['\u0301', '\u200d', '\ufe0f']:
                token = prefix + mark + 'yyyy'
                cases.append(fixture('{'+token+'}', values=[[token,'Value']]))
                cases.append(fixture('{'+token+' default="Default"}'))
        self.compare(cases)

    def test_explicit_budget_and_invalid_context_guards(self):
        cases = [fixture('12345', limits=dict(sourceBytes=4,tokens=4,outputBytes=8)),
                 fixture('{x}{y}', limits=dict(sourceBytes=100,tokens=1,outputBytes=100)),
                 fixture('{x}', values=[['x','123456789']], limits=dict(sourceBytes=100,tokens=4,outputBytes=8)),
                 fixture('{x}', values=[['x','a'*200]], limits=dict(sourceBytes=100,tokens=4,outputBytes=8)),
                 fixture('plain', timeZone='dev-fixture-invalid-zone')]
        result = self.invoke(BIN, cases)
        self.assertFalse(result[0]['ok'])
        self.assertFalse(result[1]['ok'])
        for case in result[2:]:
            self.assertTrue(case['ok'])
            self.assertFalse(case['render']['ok'])

    def test_output_bound_uses_final_size_not_intermediate_reverse_size(self):
        case = fixture('{cursor}{x}', values=[['x','12345678']], limits=dict(sourceBytes=100,tokens=4,outputBytes=8))
        result = self.invoke(BIN, [case])[0]
        self.assertEqual(result['render'], dict(ok=True, text='12345678', cursor=0))


if __name__ == '__main__':
    unittest.main(verbosity=2)
