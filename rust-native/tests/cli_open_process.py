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

    def test_canonical_alias_and_disabled_name_guard_match_swift(self):
        host=self.host([command(aliases=['café'])])
        self.assertTrue(host.cli('run','cafe\u0301')['ok'])
        config=json.loads(host.config.read_text())
        config['commands'].append(dict(id='fixture.disabled',title='Disabled',kind='url',target='https://example.invalid',aliases=['cafe\u0301'],enabled=False))
        host.replace(config);host.cli('reload')
        before=len(self.opens(host))
        result=host.rpc(dict(op='execute',id='café'))
        self.assertFalse(result['ok']);self.assertIn('disabled',result['error'])
        self.assertEqual(len(self.opens(host)),before)

    def test_url_fields_encoding_open_with_and_literal_path(self):
        host=self.host([command('url','https://example.invalid/{query}?place={x name="Place"}',applicationID='dev.fixture.inline')])
        value=host.cli('run','fixture.open','--input',json.dumps(dict(Query='a&b /Ω',Place='New York')))
        self.assertEqual(value,dict(ok=True,result=dict(state='opened')))
        targets=[s for s in self.trace(host) if s['phase']=='open-with-targets'][0]['urls']
        self.assertIn('a%26b%20',targets[0])
        self.assertIn('New%20York',targets[0])
        path=host.base/'literal Ω\n$(not-executed)'
        path.write_text('fixture')
        config=json.loads(host.config.read_text());config['commands'][-1]=command('path',str(path));host.replace(config);host.cli('reload')
        self.assertEqual(host.cli('run','fixture.open'),dict(ok=True,result=dict(command='fixture.open',state='opened')))
        self.assertEqual(self.opens(host)[-1]['phase'],'open-default')
        self.assertIn('%0A',self.opens(host)[-1]['url'])

    def test_small_execute_packet_can_open_expanded_output_above_one_mib(self):
        host=self.host([command('url','https://example.invalid/{x}{x}')])
        result=host.rpc(dict(op='execute',id='fixture.open',values=dict(x='a'*600000)),timeout=10)
        self.assertTrue(result['ok'],result)
        self.assertEqual(self.opens(host)[0]['url'],'https://example.invalid/'+'a'*1200000)

    def test_missing_apps_paths_and_fields_do_not_open(self):
        for kind,target,expected in [('app','dev.fixture.missing','App is not installed:'),('path',None,'Path does not exist:'),('url','https://example.invalid/{query}','Enter Query.')]:
            host=self.host([command(kind,target or str(workspace_fixture.ROOT/'target/no-such-file'))])
            if target is None:
                config=json.loads(host.config.read_text());config['commands'][-1]['target']=str(host.base/'missing');host.replace(config);host.cli('reload')
            result=host.rpc(dict(op='execute',id='fixture.open'))
            self.assertFalse(result['ok'])
            self.assertIn(expected,result['error'])
            self.assertEqual(self.opens(host),[])
            host.close()

    def test_fixture_clipboard_without_explicit_named_enablement_is_never_read(self):
        host=self.host([command('url','https://example.invalid/{clipboard}')])
        result=host.rpc(dict(op='execute',id='fixture.open'))
        self.assertFalse(result['ok']);self.assertIn('fixture boundary',result['error'])
        self.assertEqual(self.trace(host),[self.trace(host)[0]])

    def test_named_clipboard_url_uses_main_thread_and_preserves_raw_substitution(self):
        host=self.host([command('url','https://example.invalid/?q={clipboard}')],clipboard='raw/Ω&x=1')
        value=host.cli('run','fixture.open')
        self.assertTrue(value['ok'],value)
        trace=self.trace(host)
        reads=[s for s in trace if s['phase']=='clipboard-read']
        self.assertEqual(len(reads),1);self.assertTrue(reads[0]['mainThread'])
        opened=self.opens(host)[0]
        self.assertFalse(opened['mainThread']);self.assertIn('&x=1',opened['url'])
        self.assertTrue((host.base/'clipboard-read').exists())
        self.assertEqual(host.cli('usage')['result']['fixture.open'],1)

    def test_missing_named_clipboard_and_nonliteral_demand_do_not_read_general(self):
        host=self.host([command('url','https://example.invalid/{clipboard}')],clipboard=None)
        value=host.rpc(dict(op='execute',id='fixture.open'))
        self.assertFalse(value['ok']);self.assertEqual(self.opens(host),[])
        self.assertTrue((host.base/'clipboard-read').exists())
        # Swift String.contains does not demand clipboard at a combined closing
        # brace, although the render regex still recognizes the token.
        config=json.loads(host.config.read_text());config['commands'][-1]['target']='https://example.invalid/{clipboard}\u0301'
        host.replace(config);host.cli('reload');(host.base/'clipboard-read').unlink()
        value=host.rpc(dict(op='execute',id='fixture.open'))
        self.assertFalse(value['ok']);self.assertFalse((host.base/'clipboard-read').exists())
        self.assertEqual(self.opens(host),[])

    def test_blocked_main_clipboard_does_not_block_owner_or_revive_expired_execution(self):
        host=self.host([command('url','https://example.invalid/{clipboard}')],dispatch=400,clipboard='hold-main')
        worker,result=self.asynchronous(host)
        self.wait_file(host.base/'clipboard-entered')
        self.assertTrue(host.rpc(dict(op='status'))['ok'])
        worker.join(2);self.assertFalse(worker.is_alive())
        self.assertFalse(result[0]['ok']);self.assertNotIn('actionCompleted',result[0])
        self.release(host)
        self.wait_file(host.base/'clipboard-read')
        host.wait(lambda s:s['liveExecutionWorkers']==0)
        self.assertEqual(self.opens(host),[])
        self.assertNotIn('fixture.open',host.cli('usage')['result'])

    def test_config_change_during_main_capture_retires_old_origin(self):
        host=self.host([command('url','https://example.invalid/{clipboard}')],clipboard='hold-main')
        worker,result=self.asynchronous(host)
        self.wait_file(host.base/'clipboard-entered')
        config=json.loads(host.config.read_text());config['commands'][-1]['target']='https://example.invalid/changed'
        host.replace(config);host.cli('reload')
        worker.join(2);self.assertFalse(worker.is_alive());self.assertFalse(result[0]['ok'])
        self.release(host);host.wait(lambda s:s['liveExecutionWorkers']==0)
        self.assertEqual(self.opens(host),[])
        self.assertTrue(host.cli('run','fixture.open')['ok'])

    def test_discovered_app_runs_from_shared_catalog_without_saved_presence(self):
        host=self.host(discovered=True)
        row=next(c for c in host.cli('search','Fixture Discovered')['result'] if c['id']=='discovered.dev.fixture.discovered')
        self.assertTrue(host.cli('run',row['id'],'--dry-run')['dryRun'])
        result=host.cli('run',row['id'])
        self.assertTrue(result['ok'])
        self.assertNotIn('usageWarning',result)
        self.assertEqual(host.cli('usage')['result'][row['id']],1)
        self.assertFalse(any(c['id']==row['id'] for c in host.cli('commands')['result']))

    def test_saved_disabled_target_suppresses_discovered_and_blocks_other_saved_copy(self):
        host=self.host([command(target='dev.fixture.discovered',enabled=False),dict(id='fixture.copy',title='Copy',kind='app',target='dev.fixture.discovered')],discovered=True)
        for identifier in ['fixture.open','fixture.copy','discovered.dev.fixture.discovered']:
            result=host.rpc(dict(op='execute',id=identifier));self.assertFalse(result['ok'])
        self.assertEqual(self.opens(host),[])

    def test_missing_interposition_is_fail_closed_in_headless_fixture(self):
        host=self.host(shim=False)
        result=host.rpc(dict(op='execute',id='fixture.open'))
        self.assertFalse(result['ok']);self.assertIn('fixture boundary',result['error'])
        self.assertFalse(self.trace(host))

    def test_blocked_lookup_deadline_is_retired_before_release_and_owner_remains_available(self):
        host=self.host([command(target='dev.fixture.hold-resolve')],dispatch=1000)
        worker,results=self.asynchronous(host)
        self.wait_file(host.base/'resolve-entered')
        status=host.cli('status')['result'];self.assertEqual(status['scriptsRunning'],0);self.assertEqual(status['openRequestsRunning'],1)
        worker.join(2);self.assertFalse(worker.is_alive());self.assertIsInstance(results[0],dict)
        self.assertFalse(results[0]['ok']);self.assertNotIn('actionCompleted',results[0])
        self.assertEqual(host.cli('status')['result']['liveExecutionWorkers'],1)
        self.assertEqual(self.opens(host),[])
        self.release(host)
        host.wait(lambda s:s['liveExecutionWorkers']==0)
        self.assertEqual(self.opens(host),[])

    def test_blocked_default_open_deadline_reports_unknown_before_transport_closes(self):
        host=self.host([command('url','ruycast-fixture://hold-sync/path')],dispatch=1000)
        worker,results=self.asynchronous(host)
        self.wait_file(host.base/'open-started')
        worker.join(2);self.assertFalse(worker.is_alive());self.assertIsInstance(results[0],dict)
        self.assertEqual(results[0]['code'],'open-outcome-unknown');self.assertTrue(results[0]['actionCompleted'])
        self.assertEqual(host.cli('status')['result']['openRequestsRunning'],0)
        self.release(host);host.wait(lambda s:s['liveExecutionWorkers']==0)
        self.assertNotIn('fixture.open',host.cli('usage')['result'])

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
