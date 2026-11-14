#!/usr/bin/env python3
"""Actual unchanged CLI/control owner -> native worker -> fake SDK, no real opens."""
import json
import os
from pathlib import Path
import plistlib
import socket
import threading
import time
import unittest
import workspace_process as workspace_fixture
from runtime_process import Host, DEFAULT


def command(kind='app', target='dev.fixture.inline', **extra):
    value=dict(id='fixture.open',title='Fixture',kind=kind,target=target,aliases=['fixture-alias'])
    value.update(extra)
    return value


class CliOpen(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        workspace_fixture.Workspace.setUpClass()

    def host(self, commands=None, *, dispatch=2000, shim=True, discovered=False, clipboard=False):
        host = Host(start=False)
        self.addCleanup(host.close)
        config = json.loads(json.dumps(DEFAULT))
        config['commands'] += commands or [command()]
        host.config.write_text(json.dumps(config))
        host.env.update(RUYCAST_WORKSPACE_FIXTURE_HOME=str(host.base), RUYCAST_NATIVE_TEST_DISPATCH_MS=str(dispatch))
        host.env["RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD"]="1"
        if clipboard is not False:
            host.env["RUYCAST_NATIVE_TEST_CLIPBOARD"]="named"
            if clipboard is not None: host.env["RUYCAST_NATIVE_TEST_CLIPBOARD_TEXT"]=clipboard
        if shim:
            host.env['DYLD_INSERT_LIBRARIES'] = str(workspace_fixture.SHIM)
        if discovered:
            bundle = host.applications / 'Fixture.app/Contents'
            bundle.mkdir(parents=True)
            identifier=discovered if isinstance(discovered,str) else 'dev.fixture.discovered'
            (bundle / 'Info.plist').write_bytes(plistlib.dumps(dict(CFBundleIdentifier=identifier,CFBundleName='Fixture Discovered')))
        host.start()
        # The unchanged CLI is a real protocol client, not an interposed process.
        # Only the already-started own host retains the synthetic SDK boundary.
        host.env.pop('DYLD_INSERT_LIBRARIES',None)
        host.env.pop('RUYCAST_WORKSPACE_FIXTURE_HOME',None)
        if discovered:
            host.wait(lambda s:not s['appDiscoveryPending'])
        return host

    def trace(self, host):
        path = host.base / 'workspace-trace.jsonl'
        return [json.loads(s) for s in path.read_text().splitlines()] if path.exists() else []

    def opens(self, host):
        return [s for s in self.trace(host) if s['phase'].startswith('open-')]

    def wait_file(self, path):
        end=time.monotonic()+3
        while not path.exists():
            self.assertLess(time.monotonic(),end,'Own SDK fixture did not enter')
            time.sleep(.005)

    def asynchronous(self, host, identifier='fixture.open'):
        result=[]
        def run():
            try: result.append(host.rpc(dict(op='execute',id=identifier),timeout=4))
            except BaseException as error:result.append(error)
        worker=threading.Thread(target=run)
        worker.start()
        self.addCleanup(worker.join,4)
        return worker,result

    def release(self,host):
        (host.base/'workspace-release').write_text('release')

    def test_changed_target_and_relocated_config_revoke_pending_before_commit(self):
        for change in ['replace','relocate','invalid']:
            host=self.host([command(target='dev.fixture.hold-resolve')])
            worker,results=self.asynchronous(host)
            self.wait_file(host.base/'resolve-entered')
            if change=='replace':
                config=json.loads(host.config.read_text());config['commands'][-1]['target']='dev.fixture.inline';host.replace(config);host.cli('reload')
            elif change=='relocate':
                self.assertTrue(host.rpc(dict(op='config.directory',directory=str(host.base/'portable')))['ok'])
            else:
                host.replace(b'broken configuration')
                self.assertFalse(host.rpc(dict(op='reload'))['ok'])
            worker.join(1);self.assertFalse(worker.is_alive());self.assertFalse(results[0]['ok']);self.assertNotIn('actionCompleted',results[0])
            self.release(host);host.wait(lambda s:s['liveExecutionWorkers']==0)
            self.assertEqual(self.opens(host),[])
            host.close()

    def test_discovered_origin_is_revoked_when_saved_binding_suppresses_it(self):
        target='dev.fixture.hold-resolve.discovered'
        host=self.host(discovered=target)
        identifier='discovered.'+target
        worker,results=self.asynchronous(host,identifier)
        self.wait_file(host.base/'resolve-entered')
        config=json.loads(host.config.read_text());config['commands'].append(dict(id='fixture.saved',title='Saved',kind='app',target=target))
        host.replace(config);host.cli('reload');worker.join(1)
        self.assertFalse(worker.is_alive());self.assertFalse(results[0]['ok']);self.assertNotIn('actionCompleted',results[0])
        self.release(host);host.wait(lambda s:s['liveExecutionWorkers']==0)
        self.assertEqual(self.opens(host),[])
        self.assertNotIn(identifier,host.cli('usage')['result'])

    def test_postcommit_replacement_is_unknown_and_not_attributed_to_new_target(self):
        host=self.host([command('url','ruycast-fixture://hold-sync/path')])
        worker,results=self.asynchronous(host);self.wait_file(host.base/'open-started')
        config=json.loads(host.config.read_text());config['commands'][-1]['target']='https://example.invalid/new'
        host.replace(config);host.cli('reload');worker.join(1)
        self.assertFalse(worker.is_alive());self.assertEqual(results[0]['code'],'open-outcome-unknown')
        self.release(host);host.wait(lambda s:s['liveExecutionWorkers']==0)
        self.assertNotIn('fixture.open',host.cli('usage')['result'])

    def test_alias_rebinding_revokes_the_original_pending_selection(self):
        host=self.host([command(target='dev.fixture.hold-resolve.alias')])
        worker,results=self.asynchronous(host,'fixture-alias');self.wait_file(host.base/'resolve-entered')
        config=json.loads(host.config.read_text());config['commands'][-1]['aliases']=[]
        config['commands'].append(dict(id='fixture.other',title='Other',kind='url',target='https://example.invalid',aliases=['fixture-alias']))
        host.replace(config);host.cli('reload');worker.join(1)
        self.assertFalse(worker.is_alive());self.assertFalse(results[0]['ok']);self.assertNotIn('actionCompleted',results[0])
        self.release(host);host.wait(lambda s:s['liveExecutionWorkers']==0);self.assertEqual(self.opens(host),[])

    def test_disconnect_and_quit_revoke_lookup_without_waiting_for_sdk(self):
        host=self.host([command(target='dev.fixture.hold-resolve')])
        stream=host.connect();stream.sendall(json.dumps(dict(op='execute',id='fixture.open')).encode()+b'\n')
        self.wait_file(host.base/'resolve-entered');stream.shutdown(socket.SHUT_RDWR);stream.close()
        host.wait(lambda s:s['openRequestsRunning']==0)
        self.assertEqual(host.cli('status')['result']['liveExecutionWorkers'],1)
        host.rpc(dict(op='quit'));out,err=host.process.communicate(timeout=2)
        self.assertEqual(host.process.returncode,0,err.decode());self.assertEqual(self.opens(host),[])

    def test_new_disabled_canonical_target_revokes_unchanged_saved_command(self):
        host=self.host([command(target='dev.fixture.hold-resolve.é')])
        worker,results=self.asynchronous(host)
        self.wait_file(host.base/'resolve-entered')
        config=json.loads(host.config.read_text())
        config['commands'].append(dict(id='fixture.block',title='Block',kind='app',target='dev.fixture.hold-resolve.e\u0301',enabled=False))
        host.replace(config);host.cli('reload')
        worker.join(1)
        self.assertFalse(worker.is_alive(),'Disabled target did not revoke the original pending command')
        self.assertFalse(results[0]['ok']);self.assertNotIn('actionCompleted',results[0])
        self.release(host);host.wait(lambda s:s['liveExecutionWorkers']==0)
        self.assertEqual(self.opens(host),[])

    def test_transport_timeout_retains_native_dispatch_truth_while_owner_busy(self):
        for mode,kind,barrier,committed in [('dev.fixture.hold-resolve','app','resolve-entered',False),('ruycast-fixture://hold-sync/path','url','open-started',True)]:
            host=self.host([command(kind,mode)],dispatch=500)
            worker,results=self.asynchronous(host)
            self.wait_file(host.base/barrier)
            busy=[]
            def delay():busy.append(host.rpc(dict(op='workspace.fixture.delay'),timeout=2))
            owner=threading.Thread(target=delay);owner.start();self.addCleanup(owner.join,3)
            self.wait_file(host.base/'owner-open-delay-started')
            worker.join(2);self.assertFalse(worker.is_alive());self.assertIsInstance(results[0],dict)
            self.assertTrue(results[0]['transportExpired'])
            self.assertFalse(results[0]['ok'])
            if committed:
                self.assertTrue(results[0]['actionCompleted']);self.assertEqual(results[0]['code'],'open-outcome-unknown')
            else:self.assertNotIn('actionCompleted',results[0])
            owner.join(2);self.release(host);host.wait(lambda s:s['liveExecutionWorkers']==0)
            self.assertEqual(len(self.opens(host)),int(committed))
            host.close()


if __name__=='__main__':unittest.main(verbosity=2)
