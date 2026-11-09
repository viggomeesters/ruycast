"""Configured native list: actual AppKit, disposable profile, no desktop effects."""
import copy
import os
import hashlib
import shutil
import json
from pathlib import Path
import subprocess
import unittest
from runtime_process import Host, HOST, DEFAULT, ROOT


class NativeProfiles(unittest.TestCase):
    def profile(self):
        host = Host(start=False)
        self.addCleanup(host.close)
        return host

    def render(self, host, query='', theme='dark', width='760', success=True):
        image = host.base / ('native-' + theme + '-' + width + '.png')
        value = subprocess.run([str(HOST), '--check-profile-native', str(image), theme, width, query],
                               env=host.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=10)
        self.assertEqual(value.returncode == 0, success, value.stdout + value.stderr)
        reply = json.loads(value.stdout)
        self.assertEqual(reply['ok'], success)
        if success:
            self.assertTrue(image.read_bytes().startswith(b'\x89PNG\r\n\x1a\n'))
            self.assertFalse(reply['windowShown'])
            self.assertFalse(reply['externalExecution'])
            self.assertFalse(reply['productionReady'])
            if evidence := os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
                folder=Path(evidence)
                folder.mkdir(parents=True,exist_ok=True)
                name=self._testMethodName+'-'+hashlib.sha256(query.encode()).hexdigest()[:8]+'-'+theme+'-'+width
                shutil.copy2(image, folder/(name+'.png'))
                (folder/(name+'.json')).write_text(json.dumps(reply,ensure_ascii=False,indent=2))
        self.assertFalse((host.profile / 'control.sock').exists())
        return reply

    def test_loaded_config_alias_unicode_disabled_and_native_labels(self):
        host = self.profile()
        config = copy.deepcopy(DEFAULT)
        config['commands'] = [dict(id='fixture.active', title='Café Ω notes', kind='url', target='https://example.com/',
                                   aliases=['café'], arguments=[], hotkey='right-cmd+n'),
                              dict(id='fixture.disabled', title='Hidden Ω', kind='url', target='https://example.com/',
                                   aliases=['hidden'], arguments=[], enabled=False)]
        host.config.write_text(json.dumps(config))
        before = host.config.read_bytes()
        for theme, width in [('light','600'),('dark','760')]:
            reply = self.render(host, 'café', theme, width)
            self.assertEqual(reply['configuredCommands'], 2)
            self.assertEqual(reply['nativeRows'], 1)
            self.assertEqual(reply['selectedId'], 'fixture.active')
            self.assertIn('Café Ω notes', reply['screenText'])
            self.assertIn('Quicklink', reply['screenText'])
            self.assertIn('R⌘ N', reply['screenText'])
            self.assertIn('Right Command + n', reply['accessibilityText'])
            self.assertIn('Open ↵', reply['screenText'])
            self.assertNotIn('Hidden Ω', reply['screenText'])
        self.assertEqual(host.config.read_bytes(), before)

    def test_different_command_kinds_keep_contextual_footer(self):
        host = self.profile()
        commands = [(c['kind'], copy.deepcopy(c)) for c in DEFAULT['commands']]
        fixtures = []
        for kind, expected in [('app','Open'),('snippet','Insert Snippet'),('script','Run'),('window','Run')]:
            command = next((c for k,c in commands if k == kind), None)
            if command is None:
                command = dict(id='fixture.' + kind, title=kind, kind=kind, target='fixture', aliases=[], arguments=[])
                if kind == 'script': command['target'] = '/usr/bin/true'
                if kind == 'window': command['target'] = 'left'
            command.update(id='fixture.' + kind, title='Configured ' + kind, aliases=['pick-' + kind])
            for key in ['hotkey','keyword','snippetOutput']: command.pop(key, None)
            fixtures.append(command)
        config = copy.deepcopy(DEFAULT)
        config['commands'] = fixtures
        host.config.write_text(json.dumps(config))
        for kind, expected in [('app','Open'),('snippet','Insert Snippet'),('script','Run'),('window','Run')]:
            reply = self.render(host, 'pick-' + kind)
            self.assertEqual(reply['selectedId'], 'fixture.' + kind)
            self.assertIn(expected + ' ↵', reply['screenText'])

    def test_empty_and_long_native_content(self):
        host = self.profile()
        config = copy.deepcopy(DEFAULT)
        config['commands'] = [dict(id='fixture.long',title='Extremely long configured command Ω ' * 24,
                                   kind='url',target='https://example.com/',aliases=['long'],arguments=[])]
        host.config.write_text(json.dumps(config))
        reply = self.render(host, 'long', 'light', '600')
        self.assertEqual(reply['nativeRows'], 1)
        reply = self.render(host, 'missing-zzzzzzzzzzzzzzzzzz')
        self.assertEqual(reply['nativeRows'], 0)
        self.assertIsNone(reply['selectedId'])
        self.assertIn('No commands found', reply['screenText'])
        self.assertNotIn('Open ↵', reply['screenText'])

    def test_invalid_configuration_releases_profile_lease(self):
        host = self.profile()
        host.config.write_text('broken')
        self.render(host, success=False)
        host.config.write_text(json.dumps(DEFAULT))
        self.render(host)

    def test_existing_owner_is_not_borrowed_for_native_probe(self):
        host = Host()
        self.addCleanup(host.close)
        image = host.base / 'must-not-render.png'
        process = subprocess.run([str(HOST),'--check-profile-native',str(image)],env=host.env,
                                 capture_output=True,text=True,timeout=5)
        self.assertNotEqual(process.returncode,0)
        self.assertFalse(image.exists())
        self.assertTrue(host.rpc({'op':'status'})['ok'])
        self.assertTrue((host.profile/'control.sock').exists())


if __name__ == '__main__':
    unittest.main(verbosity=2)
