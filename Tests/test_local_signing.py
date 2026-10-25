"""Fail closed without real keychain, trust or permission changes."""
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'scripts'))
import local_signing as signing


class SigningTests(unittest.TestCase):
    def setUp(self):
        base=Path(os.environ.get('VIGGO_TEST_ROOT',tempfile.gettempdir())).resolve()
        base.mkdir(parents=True,exist_ok=True)
        self.tmp=tempfile.TemporaryDirectory(dir=base)
        self.root=Path(self.tmp.name)/'signing'
        self.root.mkdir(mode=0o700)
        for name in ['certificate.pem','identity.keychain-db','keychain-password']:
            (self.root/name).write_text('private fixture')
            (self.root/name).chmod(0o600)
        self.value={'schema':signing.SCHEMA,'certificate_sha1':'A'*40,
                    'certificate_sha256':hashlib.sha256((self.root/'certificate.pem').read_bytes()).hexdigest(),
                    'requirement':signing.requirement('A'*40)}
        self.manifest()
    def tearDown(self):self.tmp.cleanup()
    def manifest(self):
        p=self.root/'identity.json';p.write_text(json.dumps(self.value));p.chmod(0o600)
    def test_repeat_prepare_reuses_without_generating_or_touching_keychain(self):
        with patch.object(signing,'run') as run:
            self.assertEqual(signing.prepare(self.root),self.value)
            run.assert_not_called()
        (self.root/'certificate.pem').write_text('changed certificate')
        with self.assertRaisesRegex(RuntimeError,'certificate changed'):signing.prepare(self.root)
        self.assertEqual(json.loads((self.root/'identity.json').read_text()),self.value)
    def test_missing_private_state_never_rotates_identity(self):
        (self.root/'identity.keychain-db').unlink()
        with self.assertRaisesRegex(RuntimeError,'Missing or unsafe'):signing.prepare(self.root)
        (self.root/'identity.json').unlink()
        with self.assertRaisesRegex(RuntimeError,'Incomplete signing state'):signing.prepare(self.root)
        self.assertTrue((self.root/'keychain-password').exists())
    def test_unsafe_private_modes_and_symlinks_are_refused(self):
        p=self.root/'keychain-password';p.chmod(0o644)
        with self.assertRaisesRegex(RuntimeError,'unsafe'):signing.load(self.root)
        p.chmod(0o600)
        alias=self.root.parent/'alias';alias.symlink_to(self.root)
        with self.assertRaisesRegex(RuntimeError,'symlinked'):signing.load(alias)
        self.root.chmod(0o755)
        with self.assertRaisesRegex(RuntimeError,'private'):signing.load(self.root)
    def test_requirement_cannot_be_weakened(self):
        self.value['requirement']='identifier "'+signing.BUNDLE_ID+'"';self.manifest()
        with self.assertRaisesRegex(RuntimeError,'requirement changed'):signing.load(self.root)
    def test_signer_failure_never_uses_adhoc_or_modifies_trust(self):
        calls=[]
        def run(*args,**kwargs):
            calls.append(args)
            if args[0]=='/usr/bin/codesign':raise RuntimeError('signer unavailable')
            return ''
        with patch.object(signing,'run',side_effect=run),patch.object(signing,'security_private'):
            with self.assertRaisesRegex(RuntimeError,'signer unavailable'):signing.sign(self.root/'App.app',self.root)
        self.assertEqual(len([c for c in calls if c[0]=='/usr/bin/codesign']),1)
        self.assertFalse(any('add-trusted-cert' in c or ('--sign' in c and c[c.index('--sign')+1]=='-') for c in calls))
    def test_success_signs_and_verifies_exact_pinned_requirement(self):
        calls=[]
        with patch.object(signing,'run',side_effect=lambda *args,**kw:calls.append(args) or ''),patch.object(signing,'security_private'):
            result=signing.sign(self.root/'App.app',self.root)
        self.assertEqual(result['requirement'],self.value['requirement'])
        self.assertIn('=designated => '+self.value['requirement'],calls[-2])
        self.assertEqual(calls[-1][-2],'='+self.value['requirement'])
    def test_unexpected_installed_certificate_is_refused(self):
        app=self.root/'App.app';app.mkdir()
        def run(*args,**kw):
            if '--display' in args:return 'Identifier='+signing.BUNDLE_ID+'\nAuthority=Other certificate\n'
            if '-R' in args:raise RuntimeError('requirement mismatch')
            return ''
        with patch.object(signing,'run',side_effect=run):
            with self.assertRaisesRegex(RuntimeError,'requirement mismatch'):signing.check_installed(app,self.root)
    def test_one_time_adhoc_migration_checks_bundle_identity(self):
        app=self.root/'App.app';app.mkdir();calls=[]
        def run(*args,**kw):
            calls.append(args)
            if '--display' in args:return 'Identifier='+signing.BUNDLE_ID+'\nSignature=adhoc\n'
            return ''
        with patch.object(signing,'run',side_effect=run):signing.check_installed(app,self.root)
        self.assertFalse(any('-R' in c for c in calls))
    def test_signing_keychain_preserves_existing_search_entries(self):
        calls=[]
        def run(*args,**kw):
            calls.append(args)
            return '\"/existing/login.keychain-db\"\n' if 'list-keychains' in args else ''
        with patch.object(signing,'run',side_effect=run):signing.ensure_searchable(self.root)
        self.assertEqual(calls[-1],('/usr/bin/security','list-keychains','-d','user','-s','/existing/login.keychain-db',str(self.root/'identity.keychain-db')))

    def test_adhoc_is_refused_after_first_certificate_install(self):
        app=self.root/'App.app';app.mkdir()
        def run(*args,**kw):
            if '--display' in args:return 'Identifier='+signing.BUNDLE_ID+'\nSignature=adhoc\n'
            return ''
        with patch.object(signing,'run',side_effect=run):
            signing.check_installed(app,self.root)
            signing.record_install(app,self.root)
            with self.assertRaisesRegex(RuntimeError,'downgrade refused'):signing.check_installed(app,self.root)
        marker=self.root/'installed-identity.json'
        before=marker.read_bytes()
        with patch.object(signing,'verify'):signing.record_install(app,self.root)
        self.assertEqual(marker.read_bytes(),before)
        marker.write_text('{}')
        with self.assertRaisesRegex(RuntimeError,'marker changed'):signing.enrolled(self.root,self.value)
    def test_private_keychain_password_is_only_sent_via_stdin(self):
        import subprocess
        fixture='not-a-real-password'
        with patch.object(signing.subprocess,'run',return_value=subprocess.CompletedProcess([],0,'','')) as run:
            signing.security_private('unlock-keychain','-p',fixture,str(self.root/'identity.keychain-db'))
        args,kwargs=run.call_args
        self.assertEqual(args[0],['/usr/bin/security','-i'])
        self.assertNotIn(fixture,' '.join(args[0]))
        self.assertIn(fixture,kwargs['input'])
        with patch.object(signing.subprocess,'run',return_value=subprocess.CompletedProcess([],1,fixture,fixture)):
            with self.assertRaises(RuntimeError) as error:signing.security_private('unlock-keychain','-p',fixture,'fixture')
        self.assertNotIn(fixture,str(error.exception))
