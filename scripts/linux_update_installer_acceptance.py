"""Privileged signed-update acceptance. ONLY for disposable GitHub-hosted runners.

Run this under sudo and scripts/linux_task.py, after ordinary installer acceptance
has removed Recorde. Never run it on a workstation or a self-hosted runner. It
does not exercise a desktop polkit prompt: root supplies PKEXEC_UID to exercise
the post-authorization boundary. No test key or bypass enters production source.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pwd
import shutil
import stat
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
import uuid


HELPER = Path('/usr/lib/recorde/recorde-update-installer')
STATE = Path('/var/lib/recorde-update')


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('previous', 'checksums', 'candidate', 'binary', 'helper', 'signer', 'artifacts'):
        parser.add_argument(f'--{name}', type=Path, required=True)
    args = parser.parse_args()
    if (os.environ.get('GITHUB_ACTIONS') != 'true'
            or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted'
            or os.getuid() != 0 or os.geteuid() != 0):
        raise SystemExit('Requires a disposable GitHub-hosted runner and supervised sudo invocation.')
    uid_text = os.environ.get('SUDO_UID', '')
    if not uid_text.isdecimal() or int(uid_text) == 0 or str(int(uid_text)) != uid_text:
        raise SystemExit('Requires the original, non-root hosted runner SUDO_UID.')
    uid = int(uid_text)
    account = pwd.getpwuid(uid)
    source = Path(__file__).resolve().parents[1]
    workspace = Path(os.environ['GITHUB_WORKSPACE']).resolve()
    runner_temp = Path(os.environ['RUNNER_TEMP']).resolve()
    if not source.is_relative_to(workspace) or not runner_temp.is_dir():
        raise SystemExit('Expected the checked-out source and disposable runner temporary directory.')
    for name in ('previous', 'checksums', 'candidate', 'binary', 'helper', 'signer'):
        setattr(args, name, getattr(args, name).resolve(strict=True))
    args.artifacts = args.artifacts.resolve()
    if args.artifacts.exists():
        raise SystemExit('Use a fresh external acceptance artifact directory.')
    if args.artifacts.is_relative_to(source):
        raise SystemExit('Acceptance artifacts must be outside the checkout.')
    args.artifacts.mkdir(parents=True, mode=0o700)
    wrapper = source / 'scripts/linux_task.py'
    sequence = 0
    results = {}

    def run(command, name, timeout=180, expected=0, as_user=False, extra_env=None):
        nonlocal sequence
        sequence += 1
        ledger = args.artifacts / f'{sequence:03d}-{name}'
        env = {'PATH': '/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin', 'HOME': '/root',
               'LANG': 'C.UTF-8', 'LC_ALL': 'C.UTF-8', 'DEBIAN_FRONTEND': 'noninteractive',
               'CARGO_BUILD_JOBS': '1', 'RUST_TEST_THREADS': '1'}
        env.update(extra_env or {})
        command = list(map(str, command))
        if as_user:
            command = ['/usr/sbin/runuser', '-u', account.pw_name, '--', '/usr/bin/env',
                       f'HOME={account.pw_dir}', f'PATH={account.pw_dir}/.cargo/bin:/usr/local/bin:/usr/bin:/bin',
                       *command]
        supervised = [sys.executable, str(wrapper), '--purpose', name, '--artifacts', str(ledger),
                      '--timeout', str(timeout), '--', *command]
        # The outer root linux_task wrapper owns this wrapper too, including its
        # descendants if an unexpected Python exception interrupts this wait.
        proc = subprocess.run(supervised, env=env, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, timeout=timeout + 15, check=False)
        (args.artifacts / f'{sequence:03d}-{name}-runner.log').write_bytes(proc.stdout + proc.stderr)
        sessions = list(ledger.iterdir())
        if len(sessions) != 1:
            raise RuntimeError(f'Missing supervised ledger: {name}')
        report = json.loads((sessions[0] / 'result.json').read_text())
        if report['failure'] or report['remaining_processes'] or report['remaining_ports']:
            raise RuntimeError(f'Process supervision failed: {name}')
        output = (sessions[0] / 'stdout.log').read_bytes()
        error = (sessions[0] / 'stderr.log').read_bytes()
        if (expected == 0 and report['exit_code'] != 0) or (expected != 0 and report['exit_code'] == 0):
            raise RuntimeError(f'Unexpected exit status for {name}: {report["exit_code"]}; see ledger')
        return output, error

    def field(package, name):
        return run(['/usr/bin/dpkg-deb', '-f', package, name], f'field-{name}')[0].decode().strip()

    def installed_version():
        return run(['/usr/bin/dpkg-query', '-W', '-f=${Version}', 'recorde'], 'installed-version')[0].decode()

    expected = [line.split()[0] for line in args.checksums.read_text().splitlines()
                if len(line.split()) == 2 and line.split()[1] == args.previous.name]
    if expected != [digest(args.previous)]:
        raise RuntimeError('Baseline DEB checksum mismatch.')
    if any(path.exists() or path.is_symlink() for path in (Path('/usr/bin/recorde'), HELPER, STATE)):
        raise RuntimeError('Refusing to alter a pre-existing Recorde installation or updater state.')
    run(['/usr/bin/dpkg-query', '-W', '-f=${Status}', 'recorde'], 'require-no-package', expected=1)
    if field(args.previous, 'Package') != 'recorde' or field(args.candidate, 'Package') != 'recorde':
        raise RuntimeError('Unexpected package identity.')
    old_version, new_version = field(args.previous, 'Version'), field(args.candidate, 'Version')
    run(['/usr/bin/dpkg', '--compare-versions', new_version, 'gt', old_version], 'require-upgrade')
    version = tomllib.loads((source / 'Cargo.toml').read_text())['package']['version']
    arch = {'amd64': 'x86_64', 'arm64': 'aarch64'}[field(args.candidate, 'Architecture')]
    floor = old_version.rsplit('-', 1)[0].replace('~', '-', 1)
    token = uuid.uuid4().hex
    marker = STATE / '.hosted-acceptance-owner'
    temp = Path(tempfile.mkdtemp(prefix='recorde-update-acceptance-', dir=runner_temp))
    os.chown(temp, uid, account.pw_gid)
    owned_installation = False
    userdata = Path(account.pw_dir) / '.config/recorde'
    sentinels = {path: digest(path) for path in
                 (userdata / 'config.toml', userdata / 'drafts/installer-test.txt', userdata / 'credentials.json')
                 if path.is_file() and not path.is_symlink()}

    def own_file(path, mode=0o600):
        os.chown(path, uid, account.pw_gid)
        path.chmod(mode)
        return path

    def signed(artifact, name, overrides=None):
        raw = temp / f'{name}.manifest.json'
        envelope = temp / f'{name}.envelope.json'
        now = int(time.time())
        manifest = {'schema': 1, 'key_id': 'hosted-fixture', 'channel': 'preview' if '-' in version else 'stable',
                    'version': version, 'issued_at': now - 60, 'expires_at': now + 3600,
                    'platform': 'linux', 'arch': arch, 'package_kind': 'deb',
                    'artifact_name': 'recorde-fixture.deb', 'size': artifact.stat().st_size,
                    'sha256': digest(artifact)}
        manifest.update(overrides or {})
        manifest['url'] = f'https://github.com/antonydoyle/Recorde/releases/download/v{manifest["version"]}/{manifest["artifact_name"]}'
        raw.write_text(json.dumps(manifest))
        own_file(raw)
        run([args.signer, 'sign', '--key-file', temp / 'private-key', '--manifest', raw,
             '--output', envelope], f'sign-{name}', as_user=True)
        return own_file(envelope)

    def helper(envelope, artifact, name, success=False):
        output, error = run([HELPER, '--envelope', envelope, '--artifact', artifact], name,
                            timeout=1000, expected=0 if success else 1,
                            extra_env={'PKEXEC_UID': str(uid)})
        if STATE.exists() and not marker.exists():
            marker.write_text(token)
            marker.chmod(0o600)
        results[name] = 'passed'
        return output, error

    def unchanged():
        if installed_version() != old_version:
            raise RuntimeError('Rejected update changed the installed version.')

    try:
        # Keys never enter command arguments/logs; the private temporary tree is
        # deleted even on failure and is not an uploaded artifact directory.
        run([args.signer, 'generate', '--key-id', 'hosted-fixture', '--private-key-file', temp / 'private-key',
             '--public-key-file', temp / 'fixture-keys.json'], 'generate-fixture-key', as_user=True)
        staged = temp / 'source'
        for name in ('update-installer', 'update-signing'):
            shutil.copytree(source / 'tools' / name, staged / 'tools' / name,
                            ignore=shutil.ignore_patterns('target'))
        (staged / 'packaging').mkdir()
        shutil.copyfile(temp / 'fixture-keys.json', staged / 'packaging/update-keys.json')
        (staged / 'Cargo.toml').write_text(f'[package]\nname="recorde"\nversion={json.dumps(floor)}\n')
        for path in [staged, *staged.rglob('*')]:
            os.chown(path, uid, account.pw_gid)
        for original in (source / 'tools/update-installer/src').glob('*.rs'):
            if digest(original) != digest(staged / 'tools/update-installer/src' / original.name):
                raise RuntimeError('Fixture helper runtime differs from production source.')
        run(['cargo', 'build', '--locked', '--manifest-path', staged / 'tools/update-installer/Cargo.toml'],
            'compile-fixture-helper', timeout=600, as_user=True,
            extra_env={'CARGO_TARGET_DIR': str(temp / 'target')})
        artifact = own_file(Path(shutil.copyfile(args.candidate, temp / 'candidate.deb')))
        envelope = signed(artifact, 'candidate')
        owned_installation = True
        run(['/usr/bin/apt-get', 'install', '-y', args.previous], 'install-baseline', timeout=300)
        HELPER.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
        run(['/usr/bin/install', '-o', 'root', '-g', 'root', '-m', '0755', args.helper, HELPER], 'install-production-helper')
        helper(envelope, artifact, 'production-rejects-fixture-key')
        if STATE.exists():
            raise RuntimeError('Untrusted signature created privileged state.')
        fixture_helper = temp / 'target/debug/recorde-update-installer'
        run(['/usr/bin/install', '-o', 'root', '-g', 'root', '-m', '0755', fixture_helper, HELPER], 'install-fixture-helper')
        m = HELPER.lstat()
        if not stat.S_ISREG(m.st_mode) or m.st_uid != 0 or stat.S_IMODE(m.st_mode) != 0o755:
            raise RuntimeError('Fixture helper ownership/mode is unsafe.')
        invalid = temp / 'invalid.envelope.json'
        invalid.write_bytes(b'{}')
        helper(own_file(invalid), artifact, 'reject-invalid-envelope')
        helper(signed(artifact, 'wrong-arch', {'arch': 'aarch64' if arch == 'x86_64' else 'x86_64'}), artifact, 'reject-target-architecture')
        helper(signed(artifact, 'expired', {'issued_at': int(time.time()) - 3600, 'expires_at': int(time.time()) - 1}), artifact, 'reject-expired')
        wrong_version = signed(artifact, 'wrong-version', {'version': '9999.0.0', 'channel': 'stable'})
        helper(wrong_version, artifact, 'reject-control-version')
        unchanged()
        stale = STATE / 'install-owned-stale'
        stale.mkdir(mode=0o700)
        (stale / 'recorde.deb').write_bytes(b'interrupted private copy')
        (stale / 'recorde.deb').chmod(0o600)
        helper(wrong_version, artifact, 'clean-owned-stale-stage')
        if stale.exists():
            raise RuntimeError('Owned stale private stage was not removed.')
        outside = temp / 'outside-sentinel'
        outside.mkdir()
        (outside / 'recorde.deb').write_bytes(b'preserve outside bytes')
        unsafe_stage = STATE / 'install-unsafe-symlink'
        unsafe_stage.symlink_to(outside, target_is_directory=True)
        helper(envelope, artifact, 'preserve-unsafe-stale-stage')
        if not unsafe_stage.is_symlink() or (outside / 'recorde.deb').read_bytes() != b'preserve outside bytes':
            raise RuntimeError('Stale-stage cleanup crossed its ownership boundary.')
        unsafe_stage.unlink()
        # Root-only checkpoint is never silently reset, and concurrent callers
        # cannot enter package installation while another instance holds flock.
        checkpoint = STATE / 'checkpoint.json'
        checkpoint.write_bytes(b'corrupt checkpoint')
        checkpoint.chmod(0o600)
        helper(envelope, artifact, 'reject-corrupt-checkpoint')
        if checkpoint.read_bytes() != b'corrupt checkpoint':
            raise RuntimeError('Corrupt checkpoint was modified.')
        checkpoint.unlink()  # This disposable test owns the deliberately corrupt file.
        with (STATE / 'lock').open('r+b') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            helper(envelope, artifact, 'reject-concurrent-installer')
        mutated = own_file(Path(shutil.copyfile(artifact, temp / 'tampered.deb')))
        with mutated.open('r+b') as stream:
            stream.seek(16)
            stream.write(b'TAMPERED')
        helper(envelope, mutated, 'reject-tampered-input')
        # Metadata checks operate on genuinely signed alternate packages, not
        # merely a damaged hash. Neither package is installed.
        for name, key, value in [('identity', 'Package', 'not-recorde'),
                                 ('control-arch', 'Architecture', 'arm64' if arch == 'x86_64' else 'amd64')]:
            unpacked = temp / f'unpacked-{name}'
            run(['/usr/bin/dpkg-deb', '--raw-extract', artifact, unpacked], f'extract-{name}', as_user=True)
            control = unpacked / 'DEBIAN/control'
            text = control.read_text()
            control.write_text('\n'.join(f'{key}: {value}' if line.startswith(f'{key}:') else line
                                         for line in text.splitlines()) + '\n')
            alternate = temp / f'{name}.deb'
            run(['/usr/bin/dpkg-deb', '--build', '--root-owner-group', unpacked, alternate], f'build-{name}', as_user=True)
            own_file(alternate)
            helper(signed(alternate, name), alternate, f'reject-package-{name}')
        unchanged()
        # Replace the pathname repeatedly during one attempt. Either refusal or
        # installation of exactly the signed candidate is valid. The unit test
        # separately guarantees deterministic replacement after retaining an FD.
        race = temp / 'race.deb'
        stop = threading.Event()
        attempts = [0]
        race_errors = []
        def swap():
            try:
                while not stop.is_set():
                    for source_file in (artifact, mutated):
                        replacement = temp / 'race-next'
                        own_file(Path(shutil.copyfile(source_file, replacement)))
                        os.replace(replacement, race)
                        attempts[0] += 1
                        if stop.wait(0.005):
                            return
            except Exception as error:
                race_errors.append(str(error))
        own_file(Path(shutil.copyfile(artifact, race)))
        worker = threading.Thread(target=swap, name='owned-input-replacement', daemon=False)
        worker.start()
        try:
            try:
                helper(envelope, race, 'racing-path-replacement', success=True)
                results['racing-path-replacement'] = 'signed candidate installed'
            except RuntimeError as error:
                if 'Unexpected exit status for racing-path-replacement' not in str(error):
                    raise
                unchanged()
                results['racing-path-replacement'] = 'rejected; installed baseline unchanged'
        finally:
            stop.set()
            worker.join(timeout=30)
            if worker.is_alive():
                raise RuntimeError('Owned input replacement worker did not stop.')
        if race_errors or attempts[0] == 0:
            raise RuntimeError('Path replacement fixture did not execute successfully.')
        if installed_version() == old_version:
            helper(envelope, artifact, 'signed-upgrade', success=True)
        if installed_version() != new_version or digest(Path('/usr/bin/recorde')) != digest(args.binary):
            raise RuntimeError('Signed upgrade did not install the exact candidate.')
        if digest(HELPER) != digest(args.helper):
            raise RuntimeError('Candidate did not replace the fixture helper with production trust.')
        if json.loads(checkpoint.read_text())['version'] != version:
            raise RuntimeError('Successful upgrade did not retain its root rollback checkpoint.')
        results.update({'signed-upgrade': 'passed', 'path-replacements': attempts[0],
                        'baseline': old_version, 'candidate': new_version,
                        'polkit-desktop-prompt': 'not exercised by headless acceptance'})
        (args.artifacts / 'result.json').write_text(json.dumps(results, indent=2) + '\n')
    finally:
        try:
            if owned_installation:
                run(['/usr/bin/apt-get', 'remove', '-y', 'recorde'], 'remove-owned-package', timeout=300)
                # An older baseline did not own the fixture helper package path.
                if HELPER.exists():
                    if HELPER.is_symlink() or HELPER.stat().st_uid != 0 or digest(HELPER) not in {digest(args.helper), digest(temp / 'target/debug/recorde-update-installer')}:
                        raise RuntimeError('Unknown helper was preserved during cleanup.')
                    HELPER.unlink()
            if STATE.exists():
                if STATE.is_symlink() or STATE.stat().st_uid != 0 or marker.read_text() != token:
                    raise RuntimeError('Unknown updater state was preserved during cleanup.')
                shutil.rmtree(STATE)
            if any(not path.is_file() or digest(path) != expected for path, expected in sentinels.items()):
                raise RuntimeError('Existing installer user-data sentinels changed.')
        finally:
            if temp.parent == runner_temp and temp.name.startswith('recorde-update-acceptance-'):
                shutil.rmtree(temp)
            for path in [args.artifacts, *args.artifacts.rglob('*')]:
                if not path.is_symlink():
                    os.chown(path, uid, account.pw_gid)
    print('PASS: production trust rejection, signed helper upgrade, guards and owned cleanup.')


if __name__ == '__main__':
    main()
