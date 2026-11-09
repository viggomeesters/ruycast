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

    def test_cli_literal_input_uuid_usage_and_private_logs(self):
        command=self.command()
        command.update(target='/usr/bin/printf', arguments=['%s|%s|%s|%s', '{input}', 'prefix{input}', '{uuid}', '{uuid}'], inputPrompt='Text')
        host=self.host(commands=[command])
        secret='literal $(echo secret) Ω https://private.invalid/token'
        result=host.cli('run', 'runfixture', '--input', secret)['result']['stdout'].split('|')
        self.assertEqual(result[:2], [secret, 'prefix{input}'])
        self.assertEqual(len(result[2]),36);self.assertNotEqual(result[2], result[3])
        self.assertEqual(host.cli('usage')['result']['script.fixture'],1)
        logs=host.cli('logs')['result']
        self.assertEqual([e['phase'] for e in logs], ['command.finish','command.start'])
        self.assertEqual(logs[0]['source'],'cli')
        raw=(host.profile/'command-log.json').read_text()
        self.assertNotIn(secret,raw);self.assertNotIn('private.invalid',raw);self.assertNotIn('stdout',raw)
        self.assertEqual((host.profile/'command-log.json').stat().st_mode&0o777,0o600)
        self.assertEqual((host.profile/'usage.json').stat().st_mode&0o777,0o600)

    def test_dry_run_disabled_and_missing_prompt_do_not_spawn_or_log(self):
        command=self.command(inputPrompt='Text')
        host=self.host(commands=[command])
        self.assertTrue(host.cli('run','runfixture','--dry-run')['dryRun'])
        self.assertFalse(host.rpc(dict(op='execute',id='runfixture'))['ok'])
        command['enabled']=False;self.replace_command(host,command)
        self.assertFalse(host.rpc(dict(op='execute',id='runfixture',values={'input':'text'}))['ok'])
        self.assertEqual(host.rpc({'op':'logs'})['result'],[])
        self.assertEqual(host.rpc({'op':'usage'})['result'],{})

    def test_expanded_arguments_reach_actual_os_limit_with_private_failure_logs(self):
        command=self.command(inputPrompt='Text');command['arguments']=['{input}']*32
        host=self.host(commands=[command])
        result=host.rpc(dict(op='execute',id='runfixture',values={'input':'x'*65536}))
        self.assertFalse(result['ok']);self.assertIn('Argument list too long',result['error'])
        logs=host.rpc({'op':'logs'})['result']
        self.assertEqual([entry['phase'] for entry in logs],['command.finish','command.start'])
        self.assertFalse(logs[0]['success'])
        self.assertNotIn('x'*100,json.dumps(logs))
        self.assertEqual(host.rpc({'op':'usage'})['result'],{})

    def test_status_and_configuration_remain_responsive_during_script(self):
        host=self.host()
        pid=host.base/'leader'
        command=self.command('import os,time,pathlib;pathlib.Path(%r).write_text(str(os.getpid()));time.sleep(.5);print("done")'%str(pid))
        self.replace_command(host,command)
        stream=self.pending(host);self.file(pid)
        start=time.monotonic();self.assertEqual(host.rpc({'op':'status'})['result']['scriptsRunning'],1)
        self.assertLess(time.monotonic()-start,.5)
        self.assertTrue(host.rpc({'op':'reload'})['ok'])
        self.assertTrue(self.response(stream)['ok']);self.dead(pid.read_text())
        host.wait(lambda s:s['scriptsRunning']==0)

    def test_half_closed_client_receives_result(self):
        host=self.host(commands=[self.command('import time;time.sleep(.15);print("half-close")')])
        stream=self.pending(host);stream.shutdown(socket.SHUT_WR)
        self.assertEqual(self.response(stream)['result']['stdout'],'half-close\n')
        self.assertEqual(host.rpc({'op':'usage'})['result']['script.fixture'],1)

    def test_disconnect_cancels_process_group_and_recovers_capacity(self):
        host=self.host();pid=host.base/'leader';sentinel=host.base/'orphan'
        child='import signal,time,pathlib;signal.signal(signal.SIGTERM,signal.SIG_IGN);time.sleep(.8);pathlib.Path(%r).write_text("bad")'%str(sentinel)
        code='import os,time,pathlib,subprocess,sys,signal;pathlib.Path(%r).write_text(str(os.getpid()));signal.signal(signal.SIGTERM,signal.SIG_IGN);subprocess.Popen([sys.executable,"-c",%r]);time.sleep(10)'%(str(pid),child)
        self.replace_command(host,self.command(code))
        stream=self.pending(host);self.file(pid);stream.close()
        host.wait(lambda s:s['scriptsRunning']==0);self.dead(pid.read_text())
        time.sleep(.85);self.assertFalse(sentinel.exists())
        self.assertEqual(host.rpc({'op':'usage'})['result'],{})
        logs=host.rpc({'op':'logs'})['result'];self.assertFalse(logs[0]['success'])

    def test_old_command_completion_is_not_attributed_to_replacement(self):
        host=self.host();pid=host.base/'leader'
        self.replace_command(host,self.command('import time,pathlib;pathlib.Path(%r).write_text("ready");time.sleep(.4);print("original")'%str(pid)))
        stream=self.pending(host);self.file(pid)
        replacement=self.command();replacement.update(target='/usr/bin/true',arguments=[])
        self.replace_command(host,replacement)
        result=self.response(stream)
        self.assertTrue(result['ok']);self.assertEqual(result['result']['stdout'],'original\n')
        self.assertIn('usageWarning',result);self.assertEqual(host.rpc({'op':'usage'})['result'],{})

    def test_usage_failure_reports_completed_action_without_retry(self):
        host=self.host();gate=host.base/'gate';ready=host.base/'ready';counter=host.base/'counter'
        code='import time,pathlib;pathlib.Path(%r).write_text("ready");p=pathlib.Path(%r)\nwhile not p.exists():time.sleep(.01)\npathlib.Path(%r).write_text("once")'%(str(ready),str(gate),str(counter))
        self.replace_command(host,self.command(code))
        stream=self.pending(host);self.file(ready)
        (host.profile/'usage.json').mkdir();gate.write_text('go')
        result=self.response(stream)
        self.assertFalse(result['ok']);self.assertTrue(result['actionCompleted'])
        self.assertEqual(counter.read_text(),'once');self.assertEqual(host.rpc({'op':'status'})['result']['scriptsRunning'],0)

    def test_invalid_existing_log_is_preserved_until_confirmed_clear(self):
        host=rt.Host(start=False);self.addCleanup(host.close)
        original=b'invalid preserved log'
        (host.profile/'command-log.json').write_bytes(original)
        value=copy.deepcopy(rt.DEFAULT);value['commands'].append(self.command());host.replace(value);host.start()
        self.assertTrue(host.cli('run','runfixture')['ok'])
        self.assertEqual((host.profile/'command-log.json').read_bytes(),original)
        self.assertIsNotNone(host.rpc({'op':'status'})['result']['commandLogError'])
        self.assertFalse(host.rpc({'op':'logs.clear'})['ok'])
        self.assertTrue(host.rpc({'op':'logs.clear','confirmed':True})['ok'])
        self.assertEqual(json.loads((host.profile/'command-log.json').read_bytes())['entries'],[])

    def test_cli_result_envelope_preserves_the_existing_provider_contract(self):
        host=self.host(commands=[self.command('print(\'{"ok":true,"detail":"fixture detail"}\')')])
        result=host.rpc(dict(op='execute',id='runfixture'))
        self.assertTrue(result['ok']);self.assertEqual(result['detail'],'fixture detail');self.assertEqual(result['exitCode'],0)

    def test_saturated_scripts_leave_status_and_quit_admission(self):
        host=self.host();pids=host.base/'pids';pids.mkdir()
        command=self.command('import sys,os,time,pathlib;pathlib.Path(sys.argv[1]).write_text(str(os.getpid()));time.sleep(10)')
        command['arguments'].append('{input}');command['timeout']=15
        self.replace_command(host,command)
        streams=[self.pending(host,values={'input':str(pids/str(i))}) for i in range(15)]
        host.wait(lambda s:s['scriptsRunning']==15)
        for i in range(15):self.file(pids/str(i))
        busy=host.rpc(dict(op='execute',id='runfixture',values={'input':str(pids/'extra')}))
        self.assertFalse(busy['ok']);self.assertIn('busy',busy['error'])
        start=time.monotonic();self.assertTrue(host.rpc({'op':'status'})['ok']);self.assertLess(time.monotonic()-start,.5)
        self.assertTrue(host.rpc({'op':'quit'})['ok'])
        out,err=host.process.communicate(timeout=5);self.assertEqual(host.process.returncode,0,out.decode()+err.decode())
        for path in pids.iterdir():self.dead(path.read_text())
        self.assertFalse((host.profile/'control.sock').exists())
        for stream in streams:stream.close()

    def test_native_return_runs_real_script_and_records_launcher_ranking(self):
        host=self.host(native=True);marker=host.base/'native-result'
        self.replace_command(host,self.command('import pathlib;pathlib.Path(%r).write_text("native")'%str(marker)))
        self.show(host);self.key(host,36,'\r');self.assertEqual(self.file(marker),'native')
        host.wait(lambda s:s['scriptsRunning']==0 and not s['nativeUI']['visible'])
        self.assertEqual(host.rpc({'op':'usage'})['result']['script.fixture'],1)
        ranking=json.loads((host.profile/'search-ranking.json').read_bytes())
        self.assertIn('script.fixture',ranking['entries'])
        self.assertEqual(host.rpc({'op':'logs'})['result'][0]['source'],'launcher')
        self.assertFalse(host.probe('read')['windowShown'])

    def test_native_actions_open_uses_same_execution_owner(self):
        host=self.host(native=True);marker=host.base/'native-actions'
        self.replace_command(host,self.command('import pathlib;pathlib.Path(%r).write_text("actions")'%str(marker)))
        self.show(host);self.key(host,40,'k',1<<20)
        end=time.monotonic()+3
        while time.monotonic()<end:
            view=host.probe('read')
            if view['actionsVisible']:break
            time.sleep(.01)
        index=next(i for i,label in enumerate(view['actionLabels']) if label.startswith('Run'))
        host.probe('action-button',index=index)
        self.assertEqual(self.file(marker),'actions')
        host.wait(lambda s:s['scriptsRunning']==0)

    def test_native_click_runs_the_clicked_script(self):
        host=self.host(native=True);marker=host.base/'clicked-script'
        self.replace_command(host,self.command('import pathlib;pathlib.Path(%r).write_text("clicked")'%str(marker)))
        self.show(host);host.probe('click',row=0)
        self.assertEqual(self.file(marker),'clicked');host.wait(lambda s:s['scriptsRunning']==0)

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
