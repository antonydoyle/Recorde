"""Verify timeout and orphan cleanup without touching any user processes."""
import json
from pathlib import Path
import socket
import subprocess
import sys
import tempfile


def main():
    artifacts = Path(sys.argv[1]).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    runner = Path(__file__).with_name('linux_task.py')
    with tempfile.TemporaryDirectory(prefix='fixture-', dir=artifacts) as temporary:
        root = Path(temporary)
        listener = root / 'listener.py'
        listener.write_text('''import ctypes, json, os, pathlib, socket, sys, time
s = socket.socket()
s.bind(('127.0.0.1', 0))
s.listen()
if sys.argv[2] == 'private': ctypes.CDLL(None).prctl(4, 0, 0, 0, 0)
pathlib.Path(sys.argv[1]).write_text(json.dumps({'pid': os.getpid(), 'port': s.getsockname()[1]}))
if sys.argv[2] == 'zombie': sys.exit(0)
time.sleep(60)
''')
        parent = root / 'parent.py'
        parent.write_text('''import os, pathlib, subprocess, sys, time
subprocess.Popen([sys.executable, sys.argv[1], sys.argv[2], sys.argv[3]], start_new_session=True)
deadline = time.monotonic() + 5
while not pathlib.Path(sys.argv[2]).exists():
    if time.monotonic() > deadline: raise RuntimeError('Listener did not start')
    time.sleep(0.01)
if sys.argv[3] == 'timeout': time.sleep(60)
if sys.argv[3] == 'zombie':
    time.sleep(0.2)
    os._exit(0)  # Leave the exited child for the supervisor's subreaper.
''')
        for mode in ('timeout', 'orphan', 'private', 'zombie'):
            marker = root / f'{mode}.json'
            logs = artifacts / mode
            before = set(logs.glob('*/result.json'))
            completed = subprocess.run([
                sys.executable, str(runner), '--purpose', f'Linux {mode} cleanup self-test',
                '--artifacts', str(logs), '--timeout', '2', '--', sys.executable,
                str(parent), str(listener), str(marker), mode,
            ], timeout=15)
            assert completed.returncode == (1 if mode == 'timeout' else 0)
            results = set(logs.glob('*/result.json')) - before
            assert len(results) == 1
            result_file = results.pop()
            result = json.loads(result_file.read_text())
            assert not result['remaining_processes'] and not result['remaining_ports'], result
            process = json.loads(marker.read_text())
            recorded = json.loads(result_file.with_name('before-cleanup.json').read_text())
            if mode == 'zombie':
                assert recorded['processes'][str(process['pid'])]['state'] == 'Z'
                assert not recorded['ports'], 'Exited children cannot retain listeners'
                assert not Path(f'/proc/{process["pid"]}').exists(), 'Adopted zombie was not reaped'
            else:
                assert any(p['pid'] == process['pid'] and
                           (p.get('port') == process['port'] or (mode == 'private' and 'inspection_error' in p))
                           for p in recorded['ports'])
            with socket.socket() as probe:
                assert probe.connect_ex(('127.0.0.1', process['port'])) != 0
            print(f'PASS: {mode} cleanup stopped separate-session grandchild and released its port')


if __name__ == '__main__':
    main()
