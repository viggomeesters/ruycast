"""Portable config uses the same location in the CLI, host and installer."""
import fcntl
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest

REPO = Path(__file__).resolve().parents[1]
CLI = REPO / '.build/rust/release/ruycast'
HOST = REPO / '.build/swift/debug/RuycastApp'

class ConfigDirectoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='rc-dir-', dir=os.environ.get('TMPDIR'))
        self.base = Path(self.temp.name)
        self.root = self.base/'local'
        self.root.mkdir()
        self.config = (REPO/'config/default.json').read_bytes()
        (self.root/'config.json').write_bytes(self.config)
        self.custom = self.base/'Synced vault with spaces'/'.appdata'/'ruycast'
        self.env = dict(os.environ, RUYCAST_HOME=str(self.root), RUYCAST_APP_BINARY=str(HOST))
        self.env.pop('RUYCAST_CONFIG_DIR', None)
        self.process = None

    def tearDown(self):
        if self.process is not None:
            self.process.terminate()
            self.process.wait(timeout=6)
        self.temp.cleanup()

    def cli(self, *args, ok=True, env=None):
        p = subprocess.run([str(CLI), *args], env=env or self.env, capture_output=True, text=True, timeout=10)
        response = json.loads(p.stdout)
        self.assertEqual(response['ok'], ok, response)
        self.assertEqual(p.returncode, 0 if ok else 1, response)
        return response

    def start(self):
        self.process = subprocess.Popen([str(HOST), '--headless'], env=self.env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(100):
            if (self.root/'control.sock').exists():
                response = subprocess.run([str(CLI),'status'],env=self.env,capture_output=True,text=True)
                if response.returncode == 0: return
            if self.process.poll() is not None: self.fail('Host exited during startup')
            time.sleep(.05)
        self.fail('Host did not start')

    def test_default_remains_compatible(self):
        self.assertEqual(self.cli('config','path')['result'],str(self.root/'config.json'))

    def test_persisted_directory_is_shared_and_survives_cli_restart(self):
        self.custom.mkdir(parents=True)
        (self.custom/'config.json').write_bytes(self.config)
        (self.root/'config-location.json').write_text(json.dumps({'directory':str(self.custom)}))
        self.assertEqual(self.cli('config','path')['result'],str(self.custom/'config.json'))
        self.start()
        status=self.cli('status')['result']
        self.assertEqual(status['configPath'],str(self.custom/'config.json'))
        self.assertEqual(status['profile'],str(self.root))

    def test_environment_overrides_persisted_location(self):
        (self.root/'config-location.json').write_text(json.dumps({'directory':'/missing/persisted'}))
        override=self.base/'environment';override.mkdir();(override/'config.json').write_bytes(self.config)
        env=dict(self.env,RUYCAST_CONFIG_DIR=str(override))
        self.assertEqual(self.cli('config','path',env=env)['result'],str(override/'config.json'))
        result=subprocess.run([str(HOST),'--init-config'],env=env,capture_output=True,text=True,check=True)
        self.assertEqual(json.loads(result.stdout)['path'],str(override/'config.json'))

    def test_offline_relocation_preserves_bytes_and_runtime_files(self):
        (self.root/'clipboard.json').write_text('[]')
        self.cli('config','directory',str(self.custom))
        self.assertEqual((self.custom/'config.json').read_bytes(),self.config)
        self.assertEqual(self.cli('config','path')['result'],str(self.custom/'config.json'))
        self.assertEqual(sorted(x.name for x in self.custom.iterdir()),['config.json'])
        self.assertTrue((self.root/'clipboard.json').exists())
        self.assertEqual((self.root/'config.json').read_bytes(),self.config)  # recovery copy
        self.cli('config','set','clipboard.limit','51')
        self.assertEqual(json.loads((self.custom/'config.json').read_text())['clipboard']['limit'],51)
        self.assertEqual((self.root/'config.json').read_bytes(),self.config)

    def test_existing_destination_is_not_overwritten(self):
        self.custom.mkdir(parents=True)
        target=self.custom/'config.json';target.write_bytes(self.config+b'\n')
        self.cli('config','directory',str(self.custom),ok=False)
        self.assertEqual(target.read_bytes(),self.config+b'\n')
        self.assertFalse((self.root/'config-location.json').exists())

    def test_invalid_selector_never_falls_back_to_stale_config(self):
        for content in ['{',json.dumps({'directory':'relative'}),json.dumps({'directory':'/missing/ruycast-config'})]:
            (self.root/'config-location.json').write_text(content)
            self.cli('config','get',ok=False)

    def test_live_relocation_keeps_host_and_observer_on_new_file(self):
        self.start()
        initial=self.cli('status')['result'];pid=initial['pid']
        self.cli('config','directory',str(self.custom))
        self.assertEqual(self.cli('status')['result']['pid'],pid)
        self.assertEqual(self.cli('status')['result']['configPath'],str(self.custom/'config.json'))
        self.cli('config','set','clipboard.limit','51')
        config=json.loads((self.custom/'config.json').read_text());config['commands'].append({'id':'fixture.relocated','title':'Relocated fixture','kind':'url','target':'https://example.com'})
        candidate=self.custom/'replacement.json';candidate.write_text(json.dumps(config));candidate.replace(self.custom/'config.json')
        for _ in range(100):
            if any(x['id']=='fixture.relocated' for x in self.cli('commands')['result']):break
            time.sleep(.06)
        else:self.fail('New location was not observed')
        self.assertFalse((self.custom/'control.sock').exists())
        self.assertTrue((self.root/'control.sock').exists())

    def test_busy_writer_cannot_be_relocated(self):
        with (self.root/'config.lock').open('w') as lock:
            fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            self.cli('config','directory',str(self.custom),ok=False)
        self.assertFalse(self.custom.exists())
        self.assertEqual(self.cli('config','path')['result'],str(self.root/'config.json'))

    def test_unavailable_persisted_config_is_not_initialized(self):
        (self.root/'config-location.json').write_text(json.dumps({'directory':str(self.custom)}))
        self.cli('config','init',ok=False)
        result=subprocess.run([str(HOST),'--init-config'],env=self.env,capture_output=True,text=True)
        self.assertNotEqual(result.returncode,0)
        self.assertFalse(self.custom.exists())

    def test_host_restart_uses_relocated_configuration(self):
        self.cli('config','directory',str(self.custom))
        self.start()
        self.assertEqual(self.cli('status')['result']['configPath'],str(self.custom/'config.json'))
        self.process.terminate();self.process.wait(timeout=6);self.process=None
        self.start()
        self.assertEqual(self.cli('status')['result']['configPath'],str(self.custom/'config.json'))

    def test_manual_selector_change_does_not_silently_activate_wrong_file(self):
        self.start()
        self.custom.mkdir(parents=True);target=self.custom/'config.json';target.write_bytes(self.config)
        (self.root/'config-location.json').write_text(json.dumps({'directory':str(self.custom)}))
        self.cli('config','set','clipboard.limit','51',ok=False)
        self.assertEqual(target.read_bytes(),self.config)
        self.assertEqual(self.cli('status')['result']['configPath'],str(self.root/'config.json'))

    def test_parent_component_after_symlink_is_rejected_by_cli_and_host(self):
        elsewhere=self.base/'elsewhere'/'child';elsewhere.mkdir(parents=True)
        link=self.base/'link';link.symlink_to(elsewhere,target_is_directory=True)
        path=str(link)+'/../ruycast'
        env=dict(self.env,RUYCAST_CONFIG_DIR=path)
        self.cli('config','path',env=env,ok=False)
        result=subprocess.run([str(HOST),'--init-config'],env=env,capture_output=True,text=True)
        self.assertNotEqual(result.returncode,0)
        self.cli('config','directory',path,ok=False)
        self.assertFalse((elsewhere.parent/'ruycast').exists())

    def test_raycast_import_defaults_to_active_cli_config(self):
        self.cli('config','directory',str(self.custom))
        self.cli('config','add',json.dumps({'id':'fixture.portable','title':'Portable','kind':'url','target':'https://example.com'}))
        export=self.base/'raycast-export.json'
        export.write_text(json.dumps({'builtin_package_rootSearch':{'rootSearch':[]},'raycast_user_activity':{'activityInfo':[]},'builtin_package_quicklinks':{'quicklinks':[]}}))
        result=subprocess.run(['python3',str(REPO/'scripts/import-raycast.py'),str(export),'--cli',str(CLI)],env=self.env,capture_output=True,text=True,timeout=10)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertEqual(json.loads(result.stdout)['totalCommands'],len(json.loads(self.config)['commands'])+1)
        self.assertEqual((self.root/'config.json').read_bytes(),self.config)

    def test_environment_pinned_directory_cannot_be_relocated(self):
        env=dict(self.env,RUYCAST_CONFIG_DIR=str(self.root))
        self.cli('config','directory',str(self.custom),env=env,ok=False)
        self.assertFalse(self.custom.exists())
