"""Run one Linux validation task with a deadline, ownership ledger and cleanup.

Usage: python3 scripts/linux_task.py --purpose 'Tests' --artifacts /tmp/recorde-checks
       --timeout 300 -- cargo test --locked --lib
No shell interpolation. Logs and command lines may contain arguments: never pass secrets.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import uuid


def processes():
    result = {}
    for directory in Path('/proc').iterdir():
        if not directory.name.isdecimal():
            continue
        try:
            fields = (directory / 'stat').read_text().rsplit(')', 1)[1].split()
            result[int(directory.name)] = {
                'parent': int(fields[1]), 'start': fields[19], 'state': fields[0],
                'command': (directory / 'cmdline').read_bytes().replace(b'\0', b' ').decode(errors='replace'),
            }
        except (OSError, IndexError, ValueError):
            continue  # Processes may exit during inspection.
    return result


def descendants(snapshot):
    owned = {os.getpid()}
    while True:
        found = {pid for pid, info in snapshot.items() if info['parent'] in owned}
        if found <= owned:
            return {pid: snapshot[pid] for pid in owned if pid != os.getpid()}
        owned.update(found)


def ports(owned):
    sockets = {}
    unavailable = []
    for pid, info in owned.items():
        if info['state'] == 'Z':
            continue  # Exited children have no open descriptors; reap them below.
        try:
            for fd in Path(f'/proc/{pid}/fd').iterdir():
                try:
                    link = fd.readlink().as_posix()
                    if link.startswith('socket:['):
                        sockets[link[8:-1]] = pid
                except OSError:
                    pass
        except FileNotFoundError:
            pass
        except PermissionError as error:
            # Credential helpers can deliberately disable /proc inspection.
            # Record that limitation; ownership still comes from PID/ancestry.
            unavailable.append({'pid': pid, 'inspection_error': str(error)})
    found = unavailable
    for protocol in ('tcp', 'tcp6', 'udp', 'udp6'):
        for line in Path(f'/proc/net/{protocol}').read_text().splitlines()[1:]:
            fields = line.split()
            if fields[9] in sockets and (protocol.startswith('udp') or fields[3] == '0A'):
                found.append({'pid': sockets[fields[9]], 'protocol': protocol,
                              'address': fields[1], 'port': int(fields[1].split(':')[1], 16)})
    return found


def stop(owned):
    for pid, info in owned.items():
        try:
            # A pidfd prevents PID reuse between the ownership check and signal.
            handle = os.pidfd_open(pid)
            try:
                current = processes().get(pid)
                # An adopted setuid child may already be a root-owned zombie.
                # It needs waitpid, not a signal that this user cannot send.
                if current and current['start'] == info['start'] and current['state'] != 'Z':
                    signal.pidfd_send_signal(handle, signal.SIGKILL)
            finally:
                os.close(handle)
        except ProcessLookupError:
            pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--purpose', required=True)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--timeout', type=int, default=300, choices=range(1, 3601), metavar='SECONDS')
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    if not command:
        parser.error('A command is required')
    # Adopt orphaned grandchildren, including tools which create another session.
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(36, 1, 0, 0, 0) != 0:  # PR_SET_CHILD_SUBREAPER
        raise OSError(ctypes.get_errno(), 'Could not enable child cleanup')
    root = args.artifacts.resolve() / uuid.uuid4().hex
    root.mkdir(parents=True, mode=0o700)

    def write(name, data):
        (root / name).write_text(json.dumps(data, indent=2) + '\n')

    def interrupted(signum, frame):
        raise InterruptedError(f'Task runner received signal {signum}')

    for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(sig, interrupted)
    code, failure, worker = 125, None, None
    try:
        with (root / 'stdout.log').open('wb') as stdout, (root / 'stderr.log').open('wb') as stderr:
            worker = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=stdout,
                                      stderr=stderr, start_new_session=True)
            write('ownership.json', {'purpose': args.purpose, 'runner_pid': os.getpid(),
                                    'worker_pid': worker.pid, 'command': command, 'ports_requested': []})
            print(f'Started {args.purpose}; PID {worker.pid}; no ports requested; ledger: {root}', flush=True)
            deadline = time.monotonic() + args.timeout
            while worker.poll() is None:
                if time.monotonic() >= deadline:
                    raise TimeoutError(f'Task exceeded {args.timeout}s')
                if any((root / name).stat().st_size > 20 * 1024 * 1024 for name in ('stdout.log', 'stderr.log')):
                    raise RuntimeError('Task output exceeded 20 MiB')
                time.sleep(0.2)
            code = worker.returncode
            if any((root / name).stat().st_size > 20 * 1024 * 1024 for name in ('stdout.log', 'stderr.log')):
                raise RuntimeError('Task output exceeded 20 MiB')
    except Exception as error:
        failure = str(error)
    finally:
        for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
            signal.signal(sig, signal.SIG_IGN)
        owned = descendants(processes())
        try:
            write('before-cleanup.json', {'processes': owned, 'ports': ports(owned)})
        finally:
            # Repeat to include children adopted while their parents are stopped.
            deadline = time.monotonic() + 5
            while owned:
                stop(owned)
                if worker is not None:
                    worker.poll()
                try:
                    while os.waitpid(-1, os.WNOHANG)[0]:
                        pass
                except ChildProcessError:
                    pass
                owned = descendants(processes())
                if time.monotonic() >= deadline:
                    break
                time.sleep(0.05)
        remaining_ports = ports(owned)
        write('result.json', {'purpose': args.purpose, 'exit_code': code, 'failure': failure,
                             'remaining_processes': owned, 'remaining_ports': remaining_ports})
        print(f'Cleanup: {len(owned)} task processes, {len(remaining_ports)} listeners remain. Artifacts: {root}', flush=True)
    if failure or owned or remaining_ports:
        raise SystemExit(failure or 'Task cleanup failed')
    raise SystemExit(code if code >= 0 else 128 - code)


if __name__ == '__main__':
    main()
