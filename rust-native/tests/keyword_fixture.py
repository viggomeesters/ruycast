"""Compile only the explicit main-owned typed notification fixture object."""
from pathlib import Path
import subprocess
ROOT=Path(__file__).resolve().parents[1]
def notification_object(directory):
    output=Path(directory)/'keyword-notification.o'
    subprocess.run(['clang','-Wall','-Wextra','-Werror','-Wno-objc-missing-super-calls','-fno-objc-arc','-c',str(ROOT/'tests/keyword_notification_shim.m'),'-o',str(output)],check=True,capture_output=True)
    return str(output)
