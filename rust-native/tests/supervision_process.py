"""Independent Rust CLI lifetime, strict framing and parent cleanup faults."""
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import unittest
import runtime_process as rt

ROOT=rt.ROOT.parent
CLI=Path(os.environ.get('RUYCAST_TEST_CLI',ROOT/'.build/rust/release/ruycast'))
PROBE=Path(os.environ.get('RUYCAST_SUPERVISION_PROBE',rt.ROOT/'target/debug/supervision-probe'))


class Supervision(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        target=Path(os.environ.get('CARGO_TARGET_DIR',ROOT/'.build/full-rust-release/native'))
        cls.shim=Path(os.environ.get('RUYCAST_TEST_SUPERVISION_SHIM',target/'release/examples/libsupervision_fault_shim.dylib'))
        if not cls.shim.is_absolute() or not cls.shim.is_file():
            raise RuntimeError('Build --features test-tools --example supervision-fault-shim and set RUYCAST_TEST_SUPERVISION_SHIM first')

    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix='ruycast-supervision-')
        self.addCleanup(self.temp.cleanup)
        self.base=Path(self.temp.name)
        self.env=dict(os.environ)
        for name in ['DYLD_INSERT_LIBRARIES','RUYCAST_HOME','RUSTCAST_HOME','RUYCAST_CONFIG_DIR','RUSTCAST_CONFIG_DIR']:
            self.env.pop(name,None)

    def command(self,code='print("literal Ω")',timeout=2):
        return dict(id='script.fixture',title='Fixture',kind='script',target='/usr/bin/python3',arguments=['-c',code],timeout=timeout)

    def probe(self,command=None,helper=CLI,fault=False,trace=False,**options):
        request=dict(helper=str(helper),command=command or self.command(),home=str(self.base),**options)
        env=dict(self.env)
        if fault or trace:env['DYLD_INSERT_LIBRARIES']=str(self.shim)
        if fault:env['RUYCAST_SUPERVISION_WAIT_MARKER']=str(self.base/'wait-injected')
        if trace:env['RUYCAST_SUPERVISION_KILL_TRACE']=str(self.base/'kill-trace')
        output=subprocess.run([str(PROBE)],input=json.dumps(request),capture_output=True,text=True,env=env,timeout=7)
        self.assertEqual(output.returncode,0,output.stdout+output.stderr)
        result=json.loads(output.stdout);self.assertTrue(result['noChildren'])
        return result['result']

    def fake(self,body):
        path=self.base/'fake-helper'
        path.write_text('#!/usr/bin/python3\nimport sys,json,os,time,pathlib\n'+body+'\n')
        path.chmod(0o700)
        return path

    def tree(self):
        pid=self.base/'script-pid';sentinel=self.base/'descendant'
        child='import time,signal,pathlib;signal.signal(signal.SIGTERM,signal.SIG_IGN);time.sleep(.8);pathlib.Path(%r).write_text("bad")'%str(sentinel)
        code='import os,time,signal,pathlib,sys,subprocess;pathlib.Path(%r).write_text(str(os.getpid()));signal.signal(signal.SIGTERM,signal.SIG_IGN);subprocess.Popen([sys.executable,"-c",%r]);time.sleep(3)'%(str(pid),child)
        return self.command(code),pid,sentinel

    def clean_tree(self,pid,sentinel):
        self.assertTrue(pid.exists(),'script did not start')
        with self.assertRaises(ProcessLookupError):os.kill(int(pid.read_text()),0)
        time.sleep(.85);self.assertFalse(sentinel.exists())

    def test_cancel_deadline_and_panic_keep_independent_group_cleanup(self):
        for option in ['cancelMs','deadlineMs','panicMs']:
            with self.subTest(option=option):
                command,pid,sentinel=self.tree()
                result=self.probe(command,**{option:200})
                if option=='panicMs':self.assertTrue(result['panicCaught'])
                else:self.assertFalse(result['response']['ok'])
                self.clean_tree(pid,sentinel)
                pid.unlink();sentinel.unlink(missing_ok=True)

    def test_real_parent_wait_error_retries_reap_without_child_leak(self):
        result=self.probe(fault=True)
        self.assertIn('error',result)
        self.assertEqual((self.base/'wait-injected').read_text(),'injected')

    def test_precommit_cleanup_wait_error_is_retried(self):
        result=self.probe(helper=self.fake('print("old runner",flush=True);time.sleep(.1)'),fault=True)
        self.assertIn('error',result);self.assertEqual((self.base/'wait-injected').read_text(),'injected')

    def test_bad_ready_or_no_ready_never_receives_a_command(self):
        for body in ['print("old runner",flush=True)','print(json.dumps({"scriptSupervisor":1,"ready":True}),flush=True)','time.sleep(1)']:
            helper=self.fake(body)
            self.assertIn('error',self.probe(helper=helper,deadlineMs=100))

    def test_partial_malformed_and_wrong_exit_response_are_unknown(self):
        ready='print(json.dumps({"scriptSupervisor":3,"ready":True}),flush=True);sys.stdin.readline()\n'
        for body in ['print("partial",end="",flush=True)','print("not json",flush=True)', 'print(json.dumps({"ok":True,"scriptSupervisor":3,"response":{"ok":True}}),flush=True)', 'print(json.dumps({"ok":True,"scriptSupervisor":3,"response":{"ok":False,"error":"failure"}}),flush=True);raise SystemExit(7)']:
            self.assertIn('error',self.probe(helper=self.fake(ready+body)))

    def test_oversized_final_response_closes_output_and_reaps_helper(self):
        body='print(json.dumps({"scriptSupervisor":3,"ready":True}),flush=True);sys.stdin.readline();os.write(1,b"x"*(3*1024*1024))'
        self.assertIn('error',self.probe(helper=self.fake(body)))

    def test_parent_panic_closes_output_before_waiting_for_helper(self):
        marker=self.base/'helper-cleaned'
        committed=self.base/'helper-committed'
        body='print(json.dumps({"scriptSupervisor":3,"ready":True}),flush=True);header=sys.stdin.readline();assert header.endswith("\\n");pathlib.Path(%r).write_text("committed");sys.stdin.read()\ntry:os.write(1,b"x"*1000000)\nexcept BrokenPipeError:pass\npathlib.Path(%r).write_text("closed");os._exit(0)'%(str(committed),str(marker))
        self.assertTrue(self.probe(helper=self.fake(body),panicMs=1000,trace=True)['panicCaught'])
        self.assertEqual(committed.read_text(),'committed')
        self.assertEqual(marker.read_text(),'closed')
        self.assertFalse((self.base/'kill-trace').exists(),'parent killed helper after commit')

    def test_stderr_cannot_block_helper_cleanup(self):
        body='print(json.dumps({"scriptSupervisor":3,"ready":True}),flush=True);sys.stdin.readline();os.write(2,b"x"*1000000);print(json.dumps({"ok":True,"scriptSupervisor":3,"response":{"ok":True,"result":{"exitCode":0,"stdout":"","stderr":""}},"capture":{"stdout":{"truncated":False,"eof":True,"validUtf8":True},"stderr":{"truncated":False,"eof":True,"validUtf8":True}}}),flush=True)'
        self.assertTrue(self.probe(helper=self.fake(body))['response']['ok'])


class HostLifecycle(unittest.TestCase):
    def host(self,commands):
        host=rt.Host(start=False);self.addCleanup(host.close)
        value=json.loads(host.config.read_bytes());value['commands'].extend(commands);host.replace(value);host.start();return host

    def wait_file(self,path):
        end=time.monotonic()+4
        while time.monotonic()<end:
            if path.exists():return json.loads(path.read_text())
            time.sleep(.01)
        raise AssertionError('own script did not start')

    def heartbeat_command(self,folder,index,timeout=.2):
        path=folder/str(index)
        code='import os,pathlib,time,json;end=time.monotonic()+1.2;p=pathlib.Path(%r);q=p.with_suffix(".tmp")\nwhile time.monotonic()<end:q.write_text(json.dumps({"pid":os.getpid(),"helper":os.getppid(),"time":time.monotonic()}));os.replace(q,p);time.sleep(.02)'%str(path)
        return dict(id='script.life'+str(index),title='Lifecycle fixture'+str(index),kind='script',target='/usr/bin/python3',arguments=['-c',code],timeout=timeout)

    def test_host_sigkill_keeps_supervisor_timeout_and_cancellation_alive(self):
        host=rt.Host(start=False);self.addCleanup(host.close)
        command=self.heartbeat_command(host.base,0)
        value=json.loads(host.config.read_bytes());value['commands'].append(command);host.replace(value);host.start()
        stream=host.connect();self.addCleanup(stream.close);stream.sendall(b'{"op":"execute","id":"script.life0"}\n')
        try:initial=self.wait_file(host.base/'0')
        except AssertionError:
            stream.settimeout(1)
            try:
                with stream.makefile('rb') as reader:reply=reader.readline(1_048_577).decode(errors='replace')
            except OSError as error:reply=type(error).__name__
            self.fail('Own heartbeat did not start before the host-kill point; script reply: '+reply)
        self.assertNotEqual(initial['helper'],host.process.pid)
        old_pid=host.process.pid;host.process.kill();host.stop(graceful=False)
        host.start();self.assertNotEqual(host.process.pid,old_pid)
        time.sleep(.5);last=json.loads((host.base/'0').read_text())
        self.assertLess(last['time']-initial['time'],.25)
        with self.assertRaises(ProcessLookupError):os.kill(initial['pid'],0)

    def test_unexpected_helper_death_reports_unknown_without_retry(self):
        host=rt.Host(start=False);self.addCleanup(host.close)
        value=json.loads(host.config.read_bytes());value['commands'].append(self.heartbeat_command(host.base,0,5));host.replace(value);host.start()
        stream=host.connect();self.addCleanup(stream.close);stream.sendall(b'{"op":"execute","id":"script.life0"}\n')
        initial=self.wait_file(host.base/'0')
        # Stop the owning parent first: its unreaped child PID cannot be
        # recycled between fixture identification and this controlled signal.
        host.process.send_signal(signal.SIGSTOP)
        try:os.kill(initial['helper'],signal.SIGKILL)
        finally:host.process.send_signal(signal.SIGCONT)
        with stream.makefile('rb') as reader:result=json.loads(reader.readline())
        self.assertFalse(result['ok']);self.assertIn('before retrying',result['error'])
        self.assertEqual(host.rpc({'op':'usage'})['result'],{})
        time.sleep(1.3);last=json.loads((host.base/'0').read_text());self.assertEqual(last['pid'],initial['pid'])
        with self.assertRaises(ProcessLookupError):os.kill(initial['pid'],0)

    def test_stopped_host_cannot_stop_its_independent_script_deadline(self):
        host=rt.Host(start=False);self.addCleanup(host.close)
        value=json.loads(host.config.read_bytes());value['commands'].append(self.heartbeat_command(host.base,0));host.replace(value);host.start()
        stream=host.connect();self.addCleanup(stream.close);stream.sendall(b'{"op":"execute","id":"script.life0"}\n')
        initial=self.wait_file(host.base/'0');host.process.send_signal(signal.SIGSTOP)
        try:
            time.sleep(.5);last=json.loads((host.base/'0').read_text());self.assertLess(last['time']-initial['time'],.3)
            with self.assertRaises(ProcessLookupError):os.kill(initial['pid'],0)
        finally:host.process.send_signal(signal.SIGCONT)
        with stream.makefile('rb') as reader:response=json.loads(reader.readline())
        self.assertFalse(response['ok']);self.assertIn('timed out',response['error'])

    def test_many_helpers_do_not_inherit_each_others_lifeline_writers(self):
        host=rt.Host(start=False);self.addCleanup(host.close)
        folder=host.base/'heartbeats';folder.mkdir()
        value=json.loads(host.config.read_bytes());value['commands'].extend(self.heartbeat_command(folder,i,5) for i in range(15));host.replace(value);host.start()
        streams=[]
        for i in range(15):
            stream=host.connect();self.addCleanup(stream.close);stream.sendall(json.dumps({'op':'execute','id':'script.life'+str(i)}).encode()+b'\n');streams.append(stream)
        initial=[self.wait_file(folder/str(i)) for i in range(15)]
        host.process.kill();host.stop(graceful=False);time.sleep(.5)
        for i,first in enumerate(initial):
            last=json.loads((folder/str(i)).read_text());self.assertLess(last['time']-first['time'],.5)
            with self.assertRaises(ProcessLookupError):os.kill(first['pid'],0)


if __name__=='__main__':unittest.main()
