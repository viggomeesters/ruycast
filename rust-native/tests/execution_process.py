"""Native/control execution on real isolated hosts with synthetic scripts."""
import copy
import json
import os
from pathlib import Path
import socket
import time
import unittest
import shutil
import runtime_process as rt
from bridge_process import Bridge

rt.HOST = rt.ROOT / 'target/headless-tools/debug/ruycast-native'


class Execution(unittest.TestCase):
    def command(self, code=None, **extra):
        return dict(id='script.fixture', title='Execution Fixture', aliases=['runfixture'], kind='script',
                    target='/usr/bin/python3', arguments=['-c', code or 'print("fixture result")'], timeout=3, **extra)

    def host(self, native=False, commands=None):
        host = Bridge(start=False) if native else rt.Host(start=False)
        self.addCleanup(host.close)
        value = copy.deepcopy(rt.DEFAULT)
        value['commands'].extend(commands or [self.command()])
        host.replace(value)
        host.start()
        if native:
            host.settled()
        return host

    def pending(self, host, **request):
        stream = host.connect(timeout=6)
        self.addCleanup(stream.close)
        stream.sendall(json.dumps(dict(op='execute', id='runfixture', **request)).encode()+b'\n')
        return stream

    def response(self, stream):
        with stream.makefile('rb') as reader:
            return json.loads(reader.readline())

    def file(self, path, timeout=3):
        end = time.monotonic()+timeout
        while time.monotonic()<end:
            if path.exists():
                return path.read_text()
            time.sleep(.01)
        raise AssertionError('Own fixture did not publish '+path.name)

    def dead(self, pid):
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid), 0)

    def replace_command(self, host, command):
        value = json.loads(host.config.read_bytes())
        value['commands'] = [c for c in value['commands'] if c['id'] != command['id']]+[command]
        host.replace(value)
        self.assertTrue(host.rpc({'op':'reload'})['ok'])

    def show(self, host, query='runfixture'):
        self.assertTrue(host.rpc(dict(op='show', query=query))['ok'])
        host.settled(lambda s:s['nativeUI']['visible'])
        self.assertFalse(host.probe('read')['windowShown'])

    def key(self, host, code, chars='', flags=0):
        return host.probe('key-event', code=code, flags=flags, chars=chars)

    def test_native_window_close_cancels_running_script(self):
        host=self.host(native=True);pid=host.base/'closed-pid'
        self.replace_command(host,self.command('import os,pathlib,time;pathlib.Path(%r).write_text(str(os.getpid()));time.sleep(10)'%str(pid)))
        self.show(host);self.key(host,36,'\r');self.file(pid);host.probe('window-close')
        host.wait(lambda s:s['scriptsRunning']==0 and not s['nativeUI']['visible']);self.dead(pid.read_text())

    def test_native_escape_cancels_running_script(self):
        host=self.host(native=True);pid=host.base/'native-pid'
        self.replace_command(host,self.command('import os,pathlib,time;pathlib.Path(%r).write_text(str(os.getpid()));time.sleep(10)'%str(pid)))
        self.show(host);self.key(host,36,'\r');self.file(pid)
        self.key(host,53,flags=1<<23)
        host.wait(lambda s:s['scriptsRunning']==0 and not s['nativeUI']['visible'])
        self.dead(pid.read_text());self.assertEqual(host.rpc({'op':'usage'})['result'],{})

    def test_native_composition_keeps_escape_until_committed(self):
        host=self.host(native=True);pid=host.base/'ime-pid'
        self.replace_command(host,self.command('import os,pathlib,time;pathlib.Path(%r).write_text(str(os.getpid()));time.sleep(10)'%str(pid)))
        self.show(host);self.key(host,36,'\r');self.file(pid)
        self.assertTrue(host.probe('execution-ime-escape')['imeEscapeIgnored'])
        self.assertEqual(host.rpc({'op':'status'})['result']['scriptsRunning'],1)
        self.key(host,53);host.wait(lambda s:s['scriptsRunning']==0);self.dead(pid.read_text())

    def test_native_query_change_does_not_cancel_or_hide_new_results(self):
        host=self.host(native=True);ready=host.base/'native-ready';marker=host.base/'native-done'
        code='import pathlib,time;pathlib.Path(%r).write_text("ready");time.sleep(.3);pathlib.Path(%r).write_text("done")'%(str(ready),str(marker))
        self.replace_command(host,self.command(code))
        self.show(host);self.key(host,36,'\r');self.file(ready)
        self.assertTrue(host.rpc(dict(op='show',query='finder'))['ok'])
        host.settled(lambda s:s['nativeUI']['selectedId']=='app.finder')
        self.file(marker);host.wait(lambda s:s['scriptsRunning']==0)
        status=host.rpc({'op':'status'})['result']['nativeUI']
        self.assertTrue(status['visible']);self.assertEqual(status['selectedId'],'app.finder')
        self.assertEqual(host.probe('read')['input'],'finder')

    def delayed_completion(self,mode=None,cli=False):
        other=self.command();other.update(id='script.other',title='Execution Fixture Other',aliases=['otherfixture'])
        host=self.host(native=True,commands=[self.command(),other])
        gate=host.base/'gate';ready=host.base/'ready'
        code='import pathlib,time;pathlib.Path(%r).write_text("ready");gate=pathlib.Path(%r)\nwhile not gate.exists():time.sleep(.01)\nprint("completed")'%(str(ready),str(gate))
        self.replace_command(host,self.command(code))
        if cli:stream=self.pending(host)
        else:
            self.show(host,'fixture' if mode=='selection' else 'runfixture')
            self.assertEqual(host.rpc({'op':'status'})['result']['nativeUI']['selectedId'],'script.fixture')
            self.key(host,36,'\r');stream=None
        self.file(ready)
        if mode:host.probe('schedule-execution-intent',mode=mode)
        delay=host.connect(timeout=5);self.addCleanup(delay.close)
        delay.sendall(b'{"op":"native.delay","executionBarrier":true}\n')
        self.file(host.profile/'owner-delay-started')
        return host,gate,delay,stream

    def test_pending_query_menu_editor_and_selection_survive_ready_completion(self):
        for mode in ['query','menu','editor','selection']:
            with self.subTest(mode=mode):
                host,gate,delay,_=self.delayed_completion(mode)
                gate.write_text('go');self.file(host.profile/'execution-intent-fired')
                evidence=self.response(delay)['result']
                self.assertTrue(evidence['newIntentDuringDelay']);self.assertGreater(evidence['executionReady'],0)
                host.wait(lambda s:s['scriptsRunning']==0)
                view=host.probe('read');status=host.rpc({'op':'status'})['result']['nativeUI']
                self.assertTrue(status['visible'])
                if mode=='query':self.assertEqual(view['input'],'finder')
                if mode=='menu':self.assertTrue(view['actionsVisible'])
                if mode=='editor':self.assertTrue(view['formVisible'])
                if mode=='selection':self.assertEqual(status['selectedId'],'script.other')
                host.close()

    def test_main_rejects_stale_completion_after_newer_local_intent(self):
        host=self.host(native=True);self.show(host)
        host.probe('reject-stale-execution-view');host.settled();self.assertTrue(host.probe('read')['actionsVisible'])

    def test_harvested_cli_completion_is_delivered_and_recorded_before_native_stop(self):
        host,gate,delay,stream=self.delayed_completion('stop',cli=True)
        gate.write_text('go');self.file(host.profile/'execution-intent-fired')
        evidence=self.response(delay)['result'];self.assertTrue(evidence['nativeStopDuringDelay'])
        self.assertGreater(evidence['executionReady'],0)
        self.assertTrue(self.response(stream)['ok'])
        out,err=host.process.communicate(timeout=5);self.assertEqual(host.process.returncode,0,out.decode()+err.decode())
        logs=json.loads((host.profile/'command-log.json').read_bytes())['entries']
        self.assertEqual([e['phase'] for e in logs],['command.start','command.finish'])
        self.assertEqual(json.loads((host.profile/'usage.json').read_bytes())['script.fixture'],1)

    def test_external_replacement_queued_with_completion_cannot_receive_usage(self):
        host,gate,delay,stream=self.delayed_completion(cli=True)
        replacement=self.command();replacement.update(target='/usr/bin/true',arguments=[])
        value=json.loads(host.config.read_bytes());value['commands']=[c for c in value['commands'] if c['id']!='script.fixture']+[replacement]
        host.replace(value) # no reload request; owner is deliberately occupied
        gate.write_text('go');self.assertGreater(self.response(delay)['result']['executionReady'],0)
        result=self.response(stream);self.assertTrue(result['ok']);self.assertIn('usageWarning',result)
        self.assertEqual(host.rpc({'op':'usage'})['result'],{})

    def test_ready_cli_completion_survives_queued_rpc_quit(self):
        host,gate,delay,stream=self.delayed_completion(cli=True)
        gate.write_text('go')
        quit_stream=host.connect(timeout=5);self.addCleanup(quit_stream.close)
        quit_stream.sendall(b'{"op":"quit"}\n')
        self.assertGreater(self.response(delay)['result']['executionReady'],0)
        self.assertTrue(self.response(stream)['ok']);self.assertTrue(self.response(quit_stream)['ok'])
        out,err=host.process.communicate(timeout=5);self.assertEqual(host.process.returncode,0,out.decode()+err.decode())
        self.assertEqual(json.loads((host.profile/'usage.json').read_bytes())['script.fixture'],1)

    def test_native_config_replacement_and_error_do_not_leave_running_label(self):
        for invalid in [False,True]:
            with self.subTest(invalid=invalid):
                host,gate,delay,_=self.delayed_completion()
                if invalid:host.replace(b'{invalid')
                else:
                    value=json.loads(host.config.read_bytes())
                    for command in value['commands']:
                        if command['id']=='script.fixture':command.update(target='/usr/bin/true',arguments=[])
                    host.replace(value)
                gate.write_text('go');self.assertGreater(self.response(delay)['result']['executionReady'],0)
                host.wait(lambda s:s['scriptsRunning']==0)
                view=host.probe('read');self.assertNotIn('Running…',view['screenText'])
                if invalid:self.assertTrue(any(text.startswith('Config:') for text in view['screenText']))
                self.assertTrue(host.rpc({'op':'status'})['result']['nativeUI']['visible'])
                self.assertEqual(host.rpc({'op':'usage'})['result'],{})
                host.close()

    def test_running_and_failure_copy_in_both_themes(self):
        for theme in ['light','dark']:
            host=self.host(native=True);value=json.loads(host.config.read_bytes());value['theme']=theme;host.replace(value);host.rpc({'op':'reload'})
            gate=host.base/'gate';ready=host.base/'ready'
            code='import pathlib,time;pathlib.Path(%r).write_text("ready");gate=pathlib.Path(%r)\nwhile not gate.exists():time.sleep(.01)\nraise SystemExit(7)'%(str(ready),str(gate))
            self.replace_command(host,self.command(code));self.show(host);self.key(host,36,'\r');self.file(ready)
            states=[]
            for state in ['running','failure']:
                if state=='failure':gate.write_text('go');host.wait(lambda s:s['scriptsRunning']==0)
                view=host.probe('screenshot');states.append(view)
                if state=='running':self.assertIn('Running…',view['screenText'])
                else:self.assertTrue(any('status 7' in text for text in view['screenText']))
                if evidence:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
                    folder=Path(evidence).parent/'execution';folder.mkdir(exist_ok=True)
                    shutil.copy2(host.profile/'native-probe.png',folder/(state+'-'+theme+'.png'))
                    (folder/(state+'-'+theme+'.json')).write_text(json.dumps(view,ensure_ascii=False,indent=2))
            self.assertFalse(any(v['windowShown'] for v in states));host.close()


if __name__=='__main__':unittest.main()
