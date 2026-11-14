"""Independent supervisor lifetime after real host death or suspension."""
import json
import os
import signal
import time
import unittest
import runtime_process as rt

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
