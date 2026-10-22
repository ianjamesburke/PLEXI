# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""macOS scratch-keychain CLI regression. Run with uv run and a binary path.

Pass --restart to also validate saved-workspace restoration in a scratch host.
No command prints a generated secret value or touches the login keychain.
"""
import json, os, pathlib, subprocess, sys, tempfile, uuid
if sys.platform != "darwin":
    raise SystemExit("This check uses the macOS throwaway-keychain backend.")
binary = str(pathlib.Path(sys.argv[1]).absolute())
def keychain_snapshot():
    return tuple(subprocess.run(["security", operation], capture_output=True, check=True).stdout
                 for operation in ["default-keychain", "list-keychains"])
before = keychain_snapshot()
with tempfile.TemporaryDirectory(prefix='plexi-secrets-', dir='/tmp') as scratch:
    root = pathlib.Path(scratch)
    env = {k:v for k,v in os.environ.items() if not k.startswith('PLEXI_')}
    env.update(HOME=scratch, PLEXI_KEYCHAIN_PATH=str(root/'test.keychain-db'), PLEXI_KEYCHAIN_PASSWORD=uuid.uuid4().hex, DEMO_TOKEN=uuid.uuid4().hex)
    folder = root/'workspace'; folder.mkdir(); child=folder/'child'; child.mkdir()
    def run(*args, ok=True, cwd=folder):
        proc=subprocess.run([binary,*args],cwd=cwd,env=env,capture_output=True,text=True,timeout=40)
        assert env['DEMO_TOKEN'] not in proc.stdout+proc.stderr, 'secret appeared in command output'
        assert (proc.returncode==0)==ok, f'Unexpected exit for {args}: {proc.returncode}'
        return proc.stdout if '--json' in args else proc.stdout+proc.stderr
    rows=json.loads(run('experiments','list','--json'))
    assert not next(row for row in rows if row['name']=='folder_secrets')['enabled']
    help=run('secret','set','--help')
    assert '--folder' in help and 'plexi experiments enable folder_secrets' in help
    refusal=run('secret','set','DEMO_TOKEN','--folder','.','--from-env',ok=False)
    assert 'plexi experiments enable folder_secrets' in refusal
    refusal=run('secret','set','DEMO_TOKEN','--folder',ok=False)
    assert 'plexi experiments enable folder_secrets' in refusal
    run('experiments','enable','folder_secrets')
    empty=json.loads(run('secret','list','--json'))
    assert not any(row['name']=='DEMO_TOKEN' for row in empty['entries'])
    missing=run('secret','set','DEMO_TOKEN','--folder',ok=False)
    assert '--folder .' in missing and 'Usage:' in missing
    run('secret','set','DEMO_TOKEN','--folder','.','--from-env')
    for cwd in [folder,child]:
        data=json.loads(run('secret','list','--json',cwd=cwd))
        assert any(row['name']=='DEMO_TOKEN' and row['scope'].startswith('folder:') for row in data['entries'])
    run('experiments','disable','folder_secrets')
    data=json.loads(run('secret','list','--json',cwd=child))
    assert not any(row['name']=='DEMO_TOKEN' for row in data['entries'])
    assert any(row['reason']=='experiment_disabled' and row['count']==1 for row in data['skipped'])
    assert '1 folder secret(s) hidden' in run('secret','list',cwd=child)
    env['DEMO_GLOBAL'] = env['DEMO_TOKEN']
    run('secret','set','DEMO_GLOBAL','--global','--from-env')
    env['PLEXI_CONTEXT_ROOT'] = str(folder)
    env['PLEXI_CONTEXT_NAME'] = 'scratch-context'
    outside = root/'outside'; outside.mkdir()
    partial = json.loads(run('secret','list','--json',cwd=outside))
    assert any(row['name']=='DEMO_GLOBAL' for row in partial['entries'])
    assert all(row['scope']=='global' for row in partial['entries'])
    assert partial['skipped'][0]['reason']=='outside_pane_context'
    assert '--context' in run('secret','list',cwd=outside)
    del env['PLEXI_CONTEXT_ROOT']; del env['PLEXI_CONTEXT_NAME']
    run('secret','delete','DEMO_GLOBAL','--global')
    run('experiments','enable','folder_secrets')
    run('secret','rm','DEMO_TOKEN','--folder','.')
    if '--restart' in sys.argv[2:]:
        try:
            run('host','start','--pane',f'cwd={folder},cmd=/bin/sleep 300','--background')
            old = json.loads(run('host','status','--json'))
            new = json.loads(run('host','restart','--json'))
            assert old['ready'] and new['ready']
            assert old['pid'] != new['pid'] and old['pane_count'] == new['pane_count']
            print(f"PASS: restart changed pid {old['pid']} to {new['pid']}, restored {new['pane_count']} pane(s)")
        finally:
            run('host','stop')
assert keychain_snapshot() == before, 'keychain selection changed'
print('PASS: scratch keychain exact argv, fresh-profile gate, help/errors, ancestor inventory and hidden-count JSON/human output')
