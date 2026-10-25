#!/usr/bin/env python3
"""Sign two distinct native fixtures and prove their pinned identity stays equal.

Uses an already provisioned certificate. Does not launch either fixture or touch
privacy permissions. Output is a public certificate/requirement proof only.
"""
import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import local_signing as signing


def check():
    identity=signing.load()
    base=Path(os.environ.get('VIGGO_TEST_ROOT',tempfile.gettempdir())).resolve()
    base.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(dir=base) as directory:
        work=Path(directory)
        signatures=[]
        digests=[]
        for variant in [1,2]:
            app=work/('Variant'+str(variant)+'.app')
            mac=app/'Contents/MacOS';mac.mkdir(parents=True)
            source=work/('variant'+str(variant)+'.c')
            source.write_text('int main(void) { return '+str(variant)+'; }\n')
            subprocess.run(['/usr/bin/clang',str(source),'-o',str(mac/'Proof')],check=True,capture_output=True)
            (app/'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleIdentifier':signing.BUNDLE_ID,'CFBundleExecutable':'Proof','CFBundlePackageType':'APPL','CFBundleVersion':str(variant)}))
            signing.sign(app)
            details=signing.run('/usr/bin/codesign','--display','--requirements','-',str(app))
            designated=next(line.removeprefix('# ').strip() for line in details.splitlines() if 'designated =>' in line)
            expected='designated => '+identity['requirement'].replace(identity['certificate_sha1'],identity['certificate_sha1'].lower())
            if designated!=expected:
                raise RuntimeError('Signed app does not carry the exact pinned requirement')
            signatures.append(designated)
            digests.append(hashlib.sha256((mac/'Proof').read_bytes()).hexdigest())
        if digests[0]==digests[1] or signatures[0]!=signatures[1]:
            raise RuntimeError('Changed native builds did not retain the same identity')
        return {'ok':True,'differentNativeBinaries':True,'sameDesignatedRequirement':True,
                'bothVerifyAgainstPinnedCertificate':True,'certificate_sha1':identity['certificate_sha1'],
                'requirement':identity['requirement'],'privacySettingsChanged':False,'fixturesLaunched':False}


if __name__=='__main__':
    try:print(json.dumps(check(),indent=2))
    except (RuntimeError,OSError,ValueError,subprocess.CalledProcessError) as error:
        print(json.dumps({'ok':False,'error':str(error)}));sys.exit(1)
