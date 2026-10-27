"""Read the one compiled product version; reject anything outside its owned literal form."""
from pathlib import Path
import re
import os
import stat

VERSION_SOURCE=Path("Sources/LauncherCore/ProductVersion.swift")
_DECLARATION=re.compile(r'\s*public\s+enum\s+ProductVersion\s*\{\s*public\s+static\s+let\s+current\s*=\s*"((?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*))"\s*\}\s*')

def read_product_version(root):
    path=Path(root)/VERSION_SOURCE
    try:descriptor=os.open(path,os.O_RDONLY|os.O_NONBLOCK|os.O_NOFOLLOW)
    except OSError as error:raise RuntimeError("Cannot read canonical product version as a regular source file.") from error
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):raise RuntimeError("Canonical product version must be a regular source file.")
        with os.fdopen(descriptor,"r",encoding="utf-8") as reader:
            descriptor=-1
            source=reader.read()
    except (OSError,UnicodeError) as error:raise RuntimeError("Cannot read canonical product version.") from error
    finally:
        if descriptor>=0:os.close(descriptor)
    match=_DECLARATION.fullmatch(source)
    if match is None:raise RuntimeError("Expected one canonical ProductVersion.current semantic-version literal.")
    return match.group(1)
