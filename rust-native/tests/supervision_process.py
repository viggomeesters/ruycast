"""Independent Rust CLI lifetime, strict framing and parent cleanup faults."""
import json
import os
from pathlib import Path
import signal
import select
import subprocess
import tempfile
import time
import unittest
import runtime_process as rt

ROOT=rt.ROOT.parent
CLI=ROOT/'.build/rust/release/ruycast'
PROBE=rt.ROOT/'target/debug/supervision-probe'


class Supervision(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp=tempfile.TemporaryDirectory(prefix='ruycast-supervision-shim-')
        cls.addClassCleanup(cls.temp.cleanup)
        cls.shim=Path(cls.temp.name)/'fault.dylib'
        subprocess.run(['cc','-Wall','-Wextra','-Werror','-dynamiclib',str(rt.ROOT/'tests/supervision_fault_shim.c'),'-o',str(cls.shim)],check=True,capture_output=True)

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

    def direct(self,payload):
        child=subprocess.Popen([str(CLI),'__run-script-supervised'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=self.env)
        try:
            self.assertEqual(json.loads(self.line(child)),{'scriptSupervisor':3,'ready':True})
            out,err=child.communicate(payload,timeout=6)
            self.assertEqual(child.returncode,0,err.decode())
            envelope=json.loads(out);self.assertEqual(envelope['scriptSupervisor'],3);self.assertTrue(envelope['ok'])
            return envelope['response']
        finally:
            if child.poll() is None:child.kill();child.communicate()

    def direct_live(self,header):
        child=subprocess.Popen([str(CLI),'__run-script-supervised'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=self.env)
        try:
            json.loads(self.line(child));child.stdin.write(json.dumps(header).encode()+b'\n');child.stdin.flush()
            result=json.loads(self.line(child));child.stdin.close();child.stdin=None;child.communicate(timeout=3)
            self.assertEqual(child.returncode,0);return result['response']
        finally:
            if child.poll() is None:child.kill();child.communicate()

    def line(self,child,timeout=5):
        deadline=time.monotonic()+timeout;data=b''
        while time.monotonic()<deadline:
            if not select.select([child.stdout],[],[],max(0,deadline-time.monotonic()))[0]:break
            byte=os.read(child.stdout.fileno(),1)
            if not byte:return data
            data+=byte
            if byte==b'\n':return data
        raise AssertionError('Own helper protocol line timed out')

    def tree(self):
        pid=self.base/'script-pid';sentinel=self.base/'descendant'
        child='import time,signal,pathlib;signal.signal(signal.SIGTERM,signal.SIG_IGN);time.sleep(.8);pathlib.Path(%r).write_text("bad")'%str(sentinel)
        code='import os,time,signal,pathlib,sys,subprocess;pathlib.Path(%r).write_text(str(os.getpid()));signal.signal(signal.SIGTERM,signal.SIG_IGN);subprocess.Popen([sys.executable,"-c",%r]);time.sleep(3)'%(str(pid),child)
        return self.command(code),pid,sentinel

    def clean_tree(self,pid,sentinel):
        self.assertTrue(pid.exists(),'script did not start')
        with self.assertRaises(ProcessLookupError):os.kill(int(pid.read_text()),0)
        time.sleep(.85);self.assertFalse(sentinel.exists())

    def test_success_failure_and_signal_preserve_legacy_response(self):
        for code in ['print("literal Ω")','import sys;print("out");print("err",file=sys.stderr);raise SystemExit(7)','import os,signal;os.kill(os.getpid(),signal.SIGTERM)']:
            command=self.command(code)
            expected=subprocess.run([str(CLI),'__run-script'],input=json.dumps(command),capture_output=True,text=True,env=self.env,timeout=5)
            self.assertEqual(self.probe(command)['response'],json.loads(expected.stdout))

    def test_literal_null_stdin_and_explicit_home(self):
        script=self.base/'fixture'
        script.write_text('#!/bin/sh\nread line; printf "%s|%s" "$?" "$1"\n');script.chmod(0o700)
        command=self.command();command.update(target='~/fixture',arguments=['$(literal) Ω\nline'])
        self.assertEqual(self.probe(command)['response']['result']['stdout'],'1|$(literal) Ω\nline')

    def test_non_utf8_home_is_expanded_as_raw_path_bytes(self):
        home=os.fsencode(self.base)+b'/raw-\xff'
        try:
            subprocess.run([home+b'/fixture'],check=True,capture_output=True)
        except OSError as error:
            expected_errno=error.errno
        else:
            raise AssertionError('Nonexistent own fixture unexpectedly executed')
        command=self.command();command.update(target='~/fixture',arguments=[])
        result=self.direct_live(dict(command=command,homeBytes=list(home)))
        self.assertFalse(result['ok']);self.assertIn('os error '+str(expected_errno),result['error'])

    def test_output_bounds_survive_json_replacement_and_escaping(self):
        command=self.command('import os;os.write(1,bytes([255])*100000);os.write(2,bytes([0])*100000)')
        result=self.probe(command)['response']['result']
        self.assertEqual(len(result['stdout']),65536);self.assertEqual(len(result['stderr']),65536)
        self.assertEqual(set(result['stdout']),{'�'});self.assertEqual(set(result['stderr']),{'\0'})

    def test_large_encoded_header_preserves_payload_or_reports_os_limit(self):
        command=self.command();command.update(target='/usr/bin/printf',arguments=['%s','\x01'*400000])
        result=self.probe(command)['response']
        if result['ok']:self.assertEqual(result['result']['stdout'],'\x01'*65536)
        else:self.assertIn('Argument list too long',result['error'])

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

    def test_failed_helper_start_has_no_transient_child(self):
        for _ in range(8):self.assertIn('error',self.probe(helper=self.base/'missing'))

    def test_partial_header_eof_and_invalid_header_cannot_launch(self):
        marker=self.base/'must-not-launch'
        command=self.command('import pathlib;pathlib.Path(%r).write_text("bad")'%str(marker))
        header=json.dumps(dict(command=command,homeBytes=list(os.fsencode(self.base)))).encode()
        for payload in [header,header[:30],b'{invalid}\n',header+b'\nextra']:
            result=self.direct(payload);self.assertFalse(result['ok']);self.assertFalse(marker.exists())

    def test_oversized_header_and_invalid_home_fail_before_launch(self):
        self.assertFalse(self.direct(b'x'*(8*1024*1024+1)+b'\n')['ok'])
        for home in [[],list(b'relative'),list(b'/bad\0home')]:
            result=self.direct(json.dumps(dict(command=self.command(),homeBytes=home)).encode()+b'\n')
            self.assertFalse(result['ok']);self.assertIn('home',result['error'])

    def test_uncommitted_header_has_independent_startup_deadline(self):
        child=subprocess.Popen([str(CLI),'__run-script-supervised'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=self.env)
        try:
            json.loads(self.line(child));child.stdin.write(b'{');child.stdin.flush()
            result=json.loads(self.line(child));self.assertFalse(result['response']['ok'])
            self.assertIn('timed out',result['response']['error'])
            child.stdin.close();child.stdin=None;child.communicate(timeout=2)
        finally:
            if child.poll() is None:child.kill();child.communicate()

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
