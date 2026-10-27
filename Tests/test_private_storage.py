"""Observe actual private writes, including temporary bytes before rename."""
import ctypes, json, os, stat, subprocess, tempfile, time, unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
HOST=ROOT/'.build/swift/debug/LauncherCoreChecks'
LIBC=ctypes.CDLL(None,use_errno=True)
LIBC.acl_get_file.argtypes=[ctypes.c_char_p,ctypes.c_int];LIBC.acl_get_file.restype=ctypes.c_void_p
LIBC.acl_get_entry.argtypes=[ctypes.c_void_p,ctypes.c_int,ctypes.POINTER(ctypes.c_void_p)];LIBC.acl_get_entry.restype=ctypes.c_int
LIBC.acl_free.argtypes=[ctypes.c_void_p];LIBC.acl_free.restype=ctypes.c_int
def has_acl(path):
    acl=LIBC.acl_get_file(os.fsencode(path),0x100)
    if not acl:
        code=ctypes.get_errno()
        if code==2:return False # Darwin reports ENOENT when the extended ACL is absent.
        raise OSError(code,os.strerror(code))
    try:
        entry=ctypes.c_void_p()
        return LIBC.acl_get_entry(acl,0,ctypes.byref(entry))==0
    finally:LIBC.acl_free(acl)
class PrivateStorageTests(unittest.TestCase):
    def observe_writes(self, mask, inherited_acl=False):
        with tempfile.TemporaryDirectory(prefix='private-storage-',dir=ROOT/'.build') as folder:
            root=Path(folder);root.chmod(0o755)
            parent_acl=None
            if inherited_acl:
                subprocess.run(['/bin/chmod','+a','everyone allow read,execute,readattr,readextattr,readsecurity,file_inherit,directory_inherit',str(root)],check=True,capture_output=True)
                self.assertTrue(has_acl(root),'Fixture must establish inherited ACLs')
                parent_acl=subprocess.check_output(['/bin/ls','-lde',str(root)],text=True).splitlines()[1:]
            process=subprocess.Popen([str(HOST),'--private-storage-probe',str(root)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,umask=mask)
            observations=0;unsafe=[];deadline=time.monotonic()+20
            try:
                while process.poll() is None and time.monotonic()<deadline:
                    for path in root.rglob("*"):
                        try:info=path.lstat()
                        except FileNotFoundError:continue
                        if stat.S_ISREG(info.st_mode) and info.st_size>0:
                            observations+=1
                            try:insecure_acl=inherited_acl and has_acl(path)
                            except FileNotFoundError:continue
                            if stat.S_IMODE(info.st_mode)&0o077 or insecure_acl:
                                unsafe.append((path.name,oct(stat.S_IMODE(info.st_mode))))
                    if unsafe:break
                if process.poll() is None and (unsafe or time.monotonic()>=deadline):process.terminate()
                out,err=process.communicate(timeout=5)
                self.assertFalse(unsafe, 'Private bytes readable by group/others during save: '+repr(unsafe[:4]))
                self.assertEqual(process.returncode,0,err.decode())
                self.assertGreater(observations,0,'Observer must see actual written bytes')
                self.assertEqual(stat.S_IMODE((root/'private.json').stat().st_mode),0o600)
                self.assertEqual(json.loads((root/'private.json').read_text())['iteration'],'19')
                self.assertEqual([p.name for p in root.iterdir()],['private.json'])
                self.assertEqual(stat.S_IMODE(root.stat().st_mode),0o755)
                if inherited_acl:
                    self.assertFalse(has_acl(root/'private.json'))
                    self.assertEqual(subprocess.check_output(['/bin/ls','-lde',str(root)],text=True).splitlines()[1:],parent_acl)
            finally:
                if process.poll() is None:process.kill()
                process.communicate(timeout=5)
    def test_private_throughout_write_with_typical_umask(self):self.observe_writes(0o022)
    def test_private_throughout_write_with_permissive_umask(self):self.observe_writes(0)
    def test_private_throughout_write_with_inherited_read_acl(self):self.observe_writes(0o022,inherited_acl=True)
    def test_owner_permissions_with_restrictive_umask(self):self.observe_writes(0o077)
    def test_unsearchable_creation_mask_fails_before_payload_and_preserves_old_file(self):
        with tempfile.TemporaryDirectory(prefix='private-storage-mask-',dir=ROOT/'.build') as folder:
            root=Path(folder);root.chmod(0o755)
            target=root/'private.json';target.write_bytes(b'old synthetic data');target.chmod(0o600)
            process=subprocess.run([str(HOST),'--private-storage-probe',str(root)],capture_output=True,umask=0o777,timeout=10)
            self.assertNotEqual(process.returncode,0)
            self.assertEqual(target.read_bytes(),b'old synthetic data')
            self.assertEqual([p.name for p in root.iterdir()],['private.json'])
    def cli_acl_fixture(self, action):
        with tempfile.TemporaryDirectory(prefix='private-cli-',dir=ROOT/'.build') as folder:
            base=Path(folder);profile=base/'profile';profile.mkdir(mode=0o700)
            portable=base/'portable';portable.mkdir(mode=0o755);portable.chmod(0o755)
            subprocess.run(['/bin/chmod','+a','everyone allow read,execute,readattr,readextattr,readsecurity,file_inherit,directory_inherit',str(portable)],check=True,capture_output=True)
            parent_acl=subprocess.check_output(['/bin/ls','-lde',str(portable)],text=True).splitlines()[1:]
            env=dict(os.environ,RUYCAST_HOME=str(profile),RUYCAST_CONFIG_DIR=str(portable));env.pop('RUSTCAST_HOME',None)
            target=portable/'config.json'
            if action=='edit':
                target.write_bytes((ROOT/'config/default.json').read_bytes());target.chmod(0o600)
                subprocess.run(['/bin/chmod','-N',str(target)],check=True,capture_output=True)
                arguments=['config','set','clipboard.limit','51']
            else:arguments=['config','init']
            result=subprocess.run([str(ROOT/'.build/rust/release/ruycast'),*arguments],env=env,capture_output=True,text=True,timeout=10)
            self.assertEqual(result.returncode,0,result.stdout+result.stderr)
            self.assertEqual(stat.S_IMODE(target.stat().st_mode),0o600)
            self.assertFalse(has_acl(target),'CLI payload inherited readable ACL')
            self.assertEqual(stat.S_IMODE(portable.stat().st_mode),0o755,'Selected folder mode must remain intact')
            self.assertEqual(subprocess.check_output(['/bin/ls','-lde',str(portable)],text=True).splitlines()[1:],parent_acl)
            if action=='edit':self.assertEqual(json.loads(target.read_text())['clipboard']['limit'],51)
            self.assertEqual([p.name for p in portable.iterdir()],['config.json'])
    def test_cli_init_with_inherited_read_acl(self):self.cli_acl_fixture('init')
    def test_cli_edit_with_inherited_read_acl(self):self.cli_acl_fixture('edit')
    def test_profile_acl_removed_and_legacy_alias_preserved(self):
        with tempfile.TemporaryDirectory(prefix='pr-',dir=ROOT/'.build') as folder:
            base=Path(folder);actual=base/'actual';actual.mkdir(mode=0o700)
            config=actual/'config.json';original=(ROOT/'config/default.json').read_bytes();config.write_bytes(original);config.chmod(0o600)
            alias=base/'alias';alias.symlink_to(actual,target_is_directory=True)
            actual.chmod(0o755)
            subprocess.run(['/bin/chmod','+a','everyone allow read,execute,readattr,readextattr,readsecurity,file_inherit,directory_inherit',str(actual)],check=True,capture_output=True)
            self.assertTrue(has_acl(actual))
            env=dict(os.environ,RUYCAST_HOME=str(alias));env.pop('RUYCAST_CONFIG_DIR',None);env.pop('RUSTCAST_HOME',None)
            app=Path(os.environ.get('RUYCAST_TEST_APP_HOST',ROOT/'.build/swift/debug/RuycastApp'))
            cli=ROOT/'.build/rust/release/ruycast'
            process=subprocess.Popen([str(app),'--headless'],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
            try:
                deadline=time.monotonic()+6
                while process.poll() is None and time.monotonic()<deadline:
                    result=subprocess.run([str(cli),'status'],env=env,capture_output=True,text=True,timeout=2)
                    if result.returncode==0:
                        response=json.loads(result.stdout);status=response.get('result',response)
                        if status.get('pid')==process.pid:break
                    time.sleep(.03)
                else:self.fail('Owned profile host did not start')
                self.assertTrue(status['headless'])
                self.assertFalse(has_acl(actual),'Profile ACL must be private before serving control requests')
                self.assertEqual(stat.S_IMODE(actual.stat().st_mode),0o700)
                self.assertTrue(alias.is_symlink(),'Supported legacy profile alias must remain intact')
                self.assertEqual(config.read_bytes(),original)
            finally:
                if process.poll() is None:process.terminate()
                process.communicate(timeout=6)
    def test_cli_failed_private_write_preserves_old_config(self):
        with tempfile.TemporaryDirectory(prefix='private-cli-fail-',dir=ROOT/'.build') as folder:
            base=Path(folder);profile=base/'profile';profile.mkdir(mode=0o700)
            portable=base/'portable';portable.mkdir(mode=0o755);portable.chmod(0o755)
            target=portable/'config.json';original=(ROOT/'config/default.json').read_bytes();target.write_bytes(original);target.chmod(0o600)
            env=dict(os.environ,RUYCAST_HOME=str(profile),RUYCAST_CONFIG_DIR=str(portable));env.pop('RUSTCAST_HOME',None)
            result=subprocess.run([str(ROOT/'.build/rust/release/ruycast'),'config','set','clipboard.limit','51'],env=env,capture_output=True,text=True,umask=0o777,timeout=10)
            self.assertNotEqual(result.returncode,0)
            self.assertEqual(target.read_bytes(),original)
            self.assertEqual([p.name for p in portable.iterdir()],['config.json'])
    def relocation_fsync_failure(self, mutation=None):
        with tempfile.TemporaryDirectory(prefix='fs-',dir=ROOT/'.build') as folder:
            base=Path(folder);profile=base/'profile';profile.mkdir(mode=0o700)
            original=(ROOT/'config/default.json').read_bytes();(profile/'config.json').write_bytes(original)
            destination=base/'portable';destination.mkdir(mode=0o755)
            shim=base/'failure.dylib';marker=base/'injected.marker'
            subprocess.run(['/usr/bin/clang','-dynamiclib','-Wall','-Wextra','-Werror',str(ROOT/'Tests/fixtures/private_fsync_shim.c'),'-o',str(shim)],capture_output=True,check=True,timeout=30)
            env=dict(os.environ,RUYCAST_HOME=str(profile),DYLD_INSERT_LIBRARIES=str(shim),RUYCAST_TEST_FSYNC_DIRECTORY=str(destination),RUYCAST_TEST_FSYNC_MARKER=str(marker))
            env.pop('RUYCAST_CONFIG_DIR',None);env.pop('RUSTCAST_HOME',None);env.pop('RUYCAST_TEST_FSYNC_MUTATE',None)
            if mutation:env['RUYCAST_TEST_FSYNC_MUTATE']=mutation
            result=subprocess.run([str(ROOT/'.build/swift/debug/RuycastApp'),'--set-config-directory',str(destination)],env=env,capture_output=True,text=True,timeout=10)
            self.assertNotEqual(result.returncode,0)
            self.assertEqual(marker.read_bytes(),b'injected','Must actually fail the post-publication directory fsync')
            self.assertEqual((profile/'config.json').read_bytes(),original)
            self.assertFalse((profile/'config-location.json').exists())
            target=destination/'config.json'
            if mutation:
                self.assertEqual(target.read_bytes(),b'external fixture edit')
                self.assertEqual(sorted(p.name for p in destination.iterdir()),['config.json','published.copy'] if mutation=='replace' else ['config.json'])
            else:self.assertEqual(list(destination.iterdir()),[],'Failed relocation must clean up its own unmodified publication')
    def test_relocation_directory_fsync_failure_rolls_back_own_publication(self):self.relocation_fsync_failure()
    def test_relocation_directory_fsync_failure_preserves_inplace_edit(self):self.relocation_fsync_failure('edit')
    def test_relocation_directory_fsync_failure_preserves_replacement(self):self.relocation_fsync_failure('replace')
