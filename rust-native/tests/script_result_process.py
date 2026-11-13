"""Visible shared execution helpers for Capture until its caller migrates."""
import execution_process as execution

class ScriptResults:
    host=execution.Execution.host;command=execution.Execution.command;file=execution.Execution.file
    replace_command=execution.Execution.replace_command
