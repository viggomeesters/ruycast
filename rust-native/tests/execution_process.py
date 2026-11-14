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



if __name__=='__main__':unittest.main()
