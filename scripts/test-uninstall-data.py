#!/usr/bin/env python3
"""Mac ARM package uninstall acceptance; never changes the login Keychain.

Uses SecKeychainCreate directly so the private Keychain is not added to the
user search list. Fixture secrets contain no real credentials.
"""
import argparse, ctypes, hashlib, os, subprocess, sys, tempfile
from pathlib import Path

if sys.platform != "darwin":
    raise SystemExit("This check requires the macOS Keychain.")
parser = argparse.ArgumentParser(description="Exercise packaged uninstall with an isolated home and private Keychain.")
parser.add_argument("--package", required=True, type=Path)
parser.add_argument("--installer", required=True, type=Path)
args = parser.parse_args()
package = args.package.resolve()
installer = args.installer.resolve()
def run(args, env=None, expected=0, data=None):
    r = subprocess.run(list(map(str,args)), env=env, input=data, text=True, capture_output=True, timeout=120)
    if r.returncode != expected:
        raise AssertionError(f'{args}: {r.returncode}\n{r.stdout}\n{r.stderr}')
    return r

def snapshot():
    return [run(['security', x]).stdout for x in ['default-keychain','list-keychains']]
before=snapshot()
with tempfile.TemporaryDirectory(prefix='plexi-uninstall-native-', dir='/tmp') as temp:
    home=Path(temp).resolve()
    keychain=home/'private.keychain-db'
    sec=ctypes.CDLL('/System/Library/Frameworks/Security.framework/Security')
    sec.SecKeychainCreate.argtypes=[ctypes.c_char_p,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_bool,ctypes.c_void_p,ctypes.POINTER(ctypes.c_void_p)]
    ref=ctypes.c_void_p()
    password=b'isolated-uninstall-fixture'
    assert sec.SecKeychainCreate(os.fsencode(keychain),len(password),password,False,None,ctypes.byref(ref))==0
    env={k:v for k,v in os.environ.items() if not k.startswith('PLEXI_')}
    env.update(HOME=str(home), PLEXI_DISTRIBUTION_HOME=str(home/'registry'), PLEXI_KEYCHAIN_PATH=str(keychain), PLEXI_KEYCHAIN_PASSWORD=password.decode())
    profile=home/'.plexi-alpha'; other=home/'.plexi-beta'
    other.mkdir(); (other/'sentinel').write_text('keep')
    profile.mkdir(); (profile/'host').mkdir(); (profile/'sentinel').write_text('delete')
    def accounts(p):
        h=hashlib.sha256(str(p).encode()).hexdigest()
        hh=hashlib.sha256(str(p/'host').encode()).hexdigest()
        return ['plexi:host:permission-seal:'+h,'plexi:host:permission-audit-tip:'+h,'plexi:host:needs-you-journal-tip:'+hh]
    alpha=accounts(profile); beta=accounts(other)
    for a in alpha+beta+['permission-mac']:
        run(['security','add-generic-password','-s','plexi-host-seal','-a',a,'-w','fixture',keychain])
    def exists(a):
        r=subprocess.run(['security','find-generic-password','-s','plexi-host-seal','-a',a,str(keychain)],capture_output=True)
        assert r.returncode in (0,44), r.stderr
        return r.returncode==0
    def install():
        run([installer,'--channel','alpha','--package',package,'--install-only','--install-dir',home/'install','--bin-dir',home/'bin','--applications-dir',home/'Applications'],env)
    install()
    print('Installed isolated Alpha package', flush=True)
    binary=home/'bin/plexi-alpha'
    # Omitted choice keeps data, even with --yes.
    run([binary,'uninstall','--yes'],env)
    assert (profile/'sentinel').exists() and all(exists(a) for a in alpha)
    assert not binary.exists()
    print('PASS: default uninstall keeps profile and credentials', flush=True)
    install()
    # Cancelling an explicit deletion leaves everything intact.
    run([binary,'uninstall','--delete-data'],env,expected=1,data='n\n')
    assert binary.exists() and (profile/'sentinel').exists()
    # Interactive deletion removes profile and exactly its credential anchors.
    run([binary,'uninstall'],env,data='y\ny\n')
    assert not profile.exists() and not binary.exists()
    assert all(not exists(a) for a in alpha)
    assert all(exists(a) for a in beta+['permission-mac'])
    assert (other/'sentinel').read_text()=='keep'
    assert not (home/'install/alpha/transaction').exists()
    print('PASS: interactive deletion removes only selected profile and credentials', flush=True)
    install()
    # The explicit flag also succeeds when there are no old credential anchors.
    run([binary,'uninstall','--delete-data','--yes'],env)
    assert not profile.exists() and not binary.exists()
    assert all(exists(a) for a in beta+['permission-mac'])
    assert snapshot()==before, 'User Keychain defaults/search list changed'
    print('PASS: real packaged keep, cancel, interactive delete, clean reinstall/delete; other profile/shared key preserved; Keychain defaults unchanged')
