"""Exercise real migration/rollback filesystem writes in a fake home and app tree."""
import contextlib
import io
import json
import os
from pathlib import Path
import plistlib
import runpy
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

REPO=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(REPO/'scripts'))
import local_signing
import build_receipt
from install_support import NEW_ID,OLD_ID,profile_paths,migrate_profile,check_profile_paths,data_hashes,set_cli_link,require_owned_bundle,require_owned_link

class MigrationTests(unittest.TestCase):
    def setUp(self):
        base=Path(os.environ.get('VIGGO_TEST_ROOT',REPO/'.build/test-profiles'));base.mkdir(parents=True,exist_ok=True)
        self.tmp=tempfile.TemporaryDirectory(dir=base);self.root=Path(self.tmp.name)
    def tearDown(self):self.tmp.cleanup()
    def test_external_local_adapter_stages_without_entering_public_source(self):
        from install_support import preserve_personal_bridge
        repo=self.root/'public';installed=self.root/'installed';stage=self.root/'stage'
        (stage/'Contents/Resources').mkdir(parents=True)
        adapter=self.root/'local-adapter.py';adapter.write_bytes(b'fixture local adapter')
        preserve_personal_bridge(repo,installed,stage,str(adapter))
        self.assertEqual((stage/'Contents/Resources/personal_bridge.py').read_bytes(),adapter.read_bytes())
        self.assertFalse((repo/'scripts/personal_bridge.py').exists())
        with self.assertRaisesRegex(RuntimeError,'absolute regular'):
            preserve_personal_bridge(repo,installed,stage,'relative.py')

    def test_public_source_update_preserves_existing_local_adapter(self):
        from install_support import preserve_personal_bridge
        repo=self.root/'public';installed=self.root/'Ruycast.app';stage=self.root/'stage'
        resource=installed/'Contents/Resources';resource.mkdir(parents=True)
        (stage/'Contents/Resources').mkdir(parents=True)
        adapter=resource/'personal_bridge.py';adapter.write_bytes(b'private adapter fixture')
        preserve_personal_bridge(repo,installed,stage)
        self.assertEqual((stage/'Contents/Resources/personal_bridge.py').read_bytes(),adapter.read_bytes())
        adapter.unlink();adapter.symlink_to(self.root/'external')
        with self.assertRaisesRegex(RuntimeError,'Linked personal adapter'):
            preserve_personal_bridge(repo,installed,stage)

    def test_profile_preserves_bytes_and_idempotent_alias(self):
        old,new=profile_paths(self.root);old.mkdir(parents=True)
        for name in ['config.json','clipboard.json','usage.json','paused.json']:(old/name).write_bytes(('private fixture Ω '+name).encode())
        before=data_hashes(old)
        migrate_profile(old,new)
        self.assertTrue(old.is_symlink());self.assertEqual(old.resolve(),new)
        self.assertEqual(data_hashes(new),before)
        migrate_profile(old,new);self.assertEqual(data_hashes(old),before)
    def test_custom_config_is_backed_up_and_checked_separately_from_runtime(self):
        from install_support import data_files
        profile=self.root/'runtime';profile.mkdir()
        custom=self.root/'synced'/'config';custom.mkdir(parents=True)
        selected=custom/'config.json';selected.write_text('{"private":"fixture"}')
        (profile/'config-location.json').write_text(json.dumps({'directory':str(custom)}))
        (profile/'clipboard.json').write_text('[]')
        files=data_files(profile);self.assertEqual(files['selected-config.json'],selected)
        before=data_hashes(profile);selected.write_text('{"private":"changed"}')
        self.assertNotEqual(data_hashes(profile)['selected-config.json'],before['selected-config.json'])
        self.assertEqual(data_hashes(profile)['clipboard.json'],before['clipboard.json'])

    def test_conflicting_profiles_and_links_are_preserved(self):
        old,new=profile_paths(self.root);old.mkdir(parents=True);new.mkdir()
        (old/'config.json').write_text('old');(new/'config.json').write_text('new')
        with self.assertRaisesRegex(RuntimeError,'Both profiles'):migrate_profile(old,new)
        self.assertEqual((old/'config.json').read_text(),'old');self.assertEqual((new/'config.json').read_text(),'new')
        link=self.root/'ruycast';link.write_text('user executable')
        with self.assertRaisesRegex(RuntimeError,'non-symlink'):set_cli_link(link,'/some/target')
        self.assertEqual(link.read_text(),'user executable')
    def test_unrelated_cli_symlink_is_not_hijacked(self):
        other=self.root/'other-tool';other.write_text('user executable')
        link=self.root/'ruycast';link.symlink_to(other)
        with self.assertRaisesRegex(RuntimeError,'Unrelated CLI symlink'):set_cli_link(link,self.root/'Ruycast.app/Contents/Resources/ruycast')
        self.assertEqual(link.resolve(),other)
        self.assertEqual(other.read_text(),'user executable')

    def test_wrong_bundle_identity_refused(self):
        app=self.root/'Rustcast.app';(app/'Contents').mkdir(parents=True)
        (app/'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleIdentifier':'org.unrelated.rustcast'}))
        with self.assertRaisesRegex(RuntimeError,'Unrelated app identity'):require_owned_bundle(app,[OLD_ID])
        self.assertTrue(app.exists())
    def installer_fixture(self,fail=False,signing_fail=False,unrelated=False,receipt_fault=None,existing_alias=False,config_override=False):
        repo=self.root/'repo';scripts=repo/'scripts';scripts.mkdir(parents=True)
        for name in ['install.py','install_support.py','local_signing.py','build_receipt.py']:shutil.copy2(REPO/'scripts'/name,scripts/name)
        for name in ['.build/swift/release/RuycastApp','.build/rust/release/ruycast']:
            path=repo/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_text('binary fixture')
        (repo/'Sources').mkdir();(repo/'Sources/Fixture.swift').write_text('source fixture')
        build_receipt.record(repo,build_receipt.fingerprint(repo))
        if receipt_fault=='source':(repo/'Sources/Fixture.swift').write_text('changed source')
        if receipt_fault=='binary':(repo/'.build/swift/release/RuycastApp').write_text('changed binary')
        if receipt_fault=='missing':(repo/'.build/receipts/release.json').unlink()
        home=self.root/'home';apps=home/'Applications';oldapp=apps/'Rustcast.app'
        (oldapp/'Contents/MacOS').mkdir(parents=True);(oldapp/'Contents/Resources').mkdir()
        (oldapp/'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleIdentifier':'org.other.rustcast' if unrelated else OLD_ID}))
        (oldapp/'Contents/MacOS/RustcastApp').write_text('old host')
        (oldapp/'Contents/Resources/rustcast').write_text('old cli')
        profile=home/'.config/rustcast';profile.mkdir(parents=True)
        for name in ['config.json','clipboard.json','usage.json','paused.json']:(profile/name).write_text('{}')
        before=data_hashes(profile)
        agents=home/'Library/LaunchAgents';agents.mkdir(parents=True)
        oldplist=agents/(OLD_ID+'.plist');oldplist.write_bytes(plistlib.dumps({'Label':OLD_ID,'ProgramArguments':[str(oldapp/'Contents/MacOS/RustcastApp')]}))
        oldcli=home/'.local/bin/rustcast';oldcli.parent.mkdir(parents=True);oldcli.symlink_to(oldapp/'Contents/Resources/rustcast')
        if existing_alias:
            app=apps/'Ruycast.app';(app/'Contents/MacOS').mkdir(parents=True);(app/'Contents/Resources').mkdir()
            (app/'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleIdentifier':NEW_ID,'CFBundleVersion':'17'}))
            (app/'Contents/MacOS/RuycastApp').write_text('current host');(app/'Contents/Resources/ruycast').write_text('current cli')
            oldcli.unlink();oldcli.symlink_to(app/'Contents/Resources/ruycast')
        preserved_cli=oldcli.resolve()
        # A same-named unknown stage is never touched by the installer.
        unknown=repo/'.dist/Ruycast.app';unknown.mkdir(parents=True);(unknown/'user-file').write_text('keep me')
        calls=[]
        def run(args,**kwargs):
            args=list(map(str,args));calls.append(args)
            if fail and args[:2]==['launchctl','bootstrap'] and args[-1].endswith(NEW_ID+'.plist'):
                raise subprocess.CalledProcessError(1,args)
            output=json.dumps({'ok':True,'result':{'name':'Ruycast','version':'0.2.1','configPath':str(custom/'config.json') if config_override else str(home/'.config/ruycast/config.json')}}) if args[-1]=='status' else ''
            return subprocess.CompletedProcess(args,0,output,'')
        env={'HOME':str(home),'RUYCAST_APP_DIR':str(apps)}
        custom=home/'synced configuration';custom.mkdir();(custom/'config.json').write_text('{}')
        env['RUYCAST_CONFIG_DIR']=str(custom) if config_override else ''
        for key in ['RUYCAST_HOME','RUSTCAST_HOME','RUSTCAST_APP_DIR']:env[key]=''
        with patch.dict(os.environ,env),patch('subprocess.run',side_effect=run),contextlib.redirect_stdout(io.StringIO()),patch.object(local_signing,'check_installed'),patch.object(local_signing,'verify'),patch.object(local_signing,'record_install'),patch.object(local_signing,'sign',side_effect=RuntimeError('signer unavailable') if signing_fail else None,return_value={'method':'persistent-local-certificate','requirement':'fixture'}):
            if receipt_fault:
                with self.assertRaisesRegex(RuntimeError,'receipt|binaries changed'):runpy.run_path(str(scripts/'install.py'),run_name='__main__')
                self.assertFalse(calls);self.assertTrue(oldapp.exists());self.assertFalse((apps/'Ruycast.app').exists());self.assertEqual(data_hashes(profile),before)
                return
            if signing_fail:
                with self.assertRaisesRegex(RuntimeError,'signer unavailable'):runpy.run_path(str(scripts/'install.py'),run_name='__main__')
                self.assertTrue(oldapp.exists())
                self.assertFalse((apps/'Ruycast.app').exists())
                self.assertEqual(data_hashes(profile),before)
                self.assertFalse(profile.is_symlink())
                self.assertFalse(any(c[0]=='launchctl' or c[-1]=='quit' for c in calls))
                return
            if fail:
                with self.assertRaises(subprocess.CalledProcessError):runpy.run_path(str(scripts/'install.py'),run_name='__main__')
            else:runpy.run_path(str(scripts/'install.py'),run_name='__main__')
        self.assertEqual((unknown/'user-file').read_text(),'keep me')
        if unrelated:
            self.assertEqual(data_hashes(profile),before);self.assertFalse(profile.is_symlink());self.assertTrue(oldapp.exists());self.assertEqual(oldcli.resolve(),preserved_cli);self.assertTrue(oldplist.exists())
            self.assertFalse(any(c[:2]==['launchctl','bootout'] and c[-1].endswith(OLD_ID) for c in calls))
            return home,apps,calls
        self.assertEqual(data_hashes(home/'.config/ruycast'),before)
        self.assertTrue(profile.is_symlink())
        return home,apps,calls
    def test_installer_pins_explicit_configuration_for_login_startup(self):
        home,apps,calls=self.installer_fixture(config_override=True)
        agent=plistlib.loads((home/'Library/LaunchAgents'/(NEW_ID+'.plist')).read_bytes())
        self.assertEqual(agent['EnvironmentVariables']['RUYCAST_CONFIG_DIR'],str(home/'synced configuration'))
        self.assertEqual((home/'synced configuration/config.json').read_text(),'{}')

    def test_installer_migrates_single_service_and_both_cli_names(self):
        home,apps,calls=self.installer_fixture()
        self.assertFalse((apps/'Rustcast.app').exists());self.assertTrue((apps/'Ruycast.app').exists())
        self.assertFalse((home/'Library/LaunchAgents'/(OLD_ID+'.plist')).exists())
        self.assertTrue((home/'Library/LaunchAgents'/(NEW_ID+'.plist')).exists())
        for name in ['ruycast','rustcast']:self.assertEqual((home/'.local/bin'/name).resolve(),apps/'Ruycast.app/Contents/Resources/ruycast')
    def test_failed_bootstrap_restores_old_app_cli_and_registration(self):
        home,apps,calls=self.installer_fixture(fail=True)
        self.assertTrue((apps/'Rustcast.app').exists());self.assertFalse((apps/'Ruycast.app').exists())
        self.assertEqual((home/'.local/bin/rustcast').resolve(),apps/'Rustcast.app/Contents/Resources/rustcast')
        self.assertFalse((home/'.local/bin/ruycast').exists())
        self.assertTrue(any(c[:2]==['launchctl','bootstrap'] and c[-1].endswith(OLD_ID+'.plist') for c in calls))

    def test_signing_failure_keeps_existing_host_and_profile_untouched(self):
        self.installer_fixture(signing_fail=True)

    def test_other_rustcast_app_cli_and_profile_are_preserved(self):
        self.installer_fixture(unrelated=True)

    def test_receipt_failure_never_stops_installed_app(self):
        for fault in ["source","binary","missing"]:
            with self.subTest(fault=fault):
                # Each fixture needs a fresh filesystem root.
                self.tearDown();self.setUp();self.installer_fixture(receipt_fault=fault)

    def test_existing_compatibility_alias_does_not_own_other_rustcast_app(self):
        self.installer_fixture(unrelated=True,existing_alias=True)
