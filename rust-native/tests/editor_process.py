"""Actual native command editor: selected identity, literal fields and persistence."""
import copy
import json
import unittest
import actions_process as helpers
CMD=helpers.CMD
from runtime_process import DEFAULT

class CommandEditor(unittest.TestCase):
    host=helpers.ActionsProcesses.host;capture=helpers.ActionsProcesses.capture;view=helpers.ActionsProcesses.view;show=helpers.ActionsProcesses.show;key=helpers.ActionsProcesses.key;saved=helpers.ActionsProcesses.saved;action=helpers.ActionsProcesses.action;command=helpers.ActionsProcesses.command
    def edit(self,host):
        self.action(host,'Edit');return self.view(host,lambda v:v['formVisible'] and 'Name' in v['screenText'])
    def field(self,host,key,text):host.probe('command-input',key=key,text=text)
    def fixture(self,host,command):
        config=copy.deepcopy(DEFAULT);config['commands'].append(command);host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok']);self.show(host,command['id'])
