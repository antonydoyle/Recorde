"""DEB installation/upgrade/removal acceptance, ONLY on disposable hosted CI."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import stat
import xml.etree.ElementTree as ET


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--previous', type=Path, required=True)
    parser.add_argument('--checksums', type=Path, required=True)
    parser.add_argument('--candidate', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--artifacts', type=Path, required=True)
    args = parser.parse_args()
    if os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted':
        raise SystemExit('Installer acceptance only runs on disposable GitHub-hosted runners.')
    args.artifacts.mkdir(parents=True, exist_ok=True)
    def run(command, name):
        result = subprocess.run(command, capture_output=True, timeout=180)
        (args.artifacts / f'{name}.log').write_bytes(result.stdout + result.stderr)
        result.check_returncode()
        return result.stdout.decode().strip()
    def field(package, key):
        return run(['dpkg-deb', '-f', str(package.resolve()), key], f'{package.stem}-{key}')
    def digest(path):
        with path.open('rb') as stream:
            return hashlib.file_digest(stream, 'sha256').hexdigest()
    expected = [line.split()[0] for line in args.checksums.read_text().splitlines()
                if len(line.split()) == 2 and line.split()[1] == args.previous.name]
    if expected != [digest(args.previous)]:
        raise RuntimeError('Baseline DEB checksum mismatch')
    if field(args.previous, 'Package') != 'recorde' or field(args.candidate, 'Package') != 'recorde':
        raise RuntimeError('Unexpected DEB package identity')
    old_version, new_version = field(args.previous, 'Version'), field(args.candidate, 'Version')
    run(['dpkg', '--compare-versions', new_version, 'gt', old_version], 'compare-version')
    installed = Path('/usr/bin/recorde')
    if installed.exists() or subprocess.run(['dpkg-query', '-W', '-f=${Status}', 'recorde'], capture_output=True).returncode == 0:
        raise RuntimeError('Refusing to alter an existing Recorde installation')
    userdata = Path(os.environ.get('XDG_CONFIG_HOME', Path.home() / '.config')) / 'recorde'
    if userdata.exists():
        raise RuntimeError('Unexpected pre-existing user data')
    (userdata / 'drafts').mkdir(parents=True)
    sentinels = {'config.toml': b'dark_theme = false\nfavorites = []\n',
                 'drafts/installer-test.txt': b'Preserve unfinished commit message.',
                 'credentials.json': b'{"fixture":"metadata only; no credentials"}'}
    for name, data in sentinels.items():
        (userdata / name).write_bytes(data)
    def assert_userdata():
        for name, data in sentinels.items():
            if (userdata / name).read_bytes() != data:
                raise RuntimeError(f'User data changed: {name}')
    installed_package = False
    try:
        # apt resolves runtime dependencies on the ephemeral runner.
        installed_package = True
        run(['sudo', '-n', 'apt-get', 'install', '-y', str(args.previous.resolve())], 'install-baseline')
        if not installed.is_file():
            raise RuntimeError('Baseline binary missing')
        assert_userdata()
        run(['sudo', '-n', 'apt-get', 'install', '-y', str(args.candidate.resolve())], 'upgrade')
        if digest(installed) != digest(args.binary):
            raise RuntimeError('Installed executable differs from candidate')
        if json.loads(Path('/usr/bin/recorde-install.json').read_text())['kind'] != 'deb':
            raise RuntimeError('Installed update marker missing')
        helper = Path('/usr/lib/recorde/recorde-update-installer')
        policy = Path('/usr/share/polkit-1/actions/org.recorde.update.policy')
        for target, mode in [(helper, 0o755), (policy, 0o644)]:
            metadata = target.lstat()
            if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != 0 or metadata.st_gid != 0 or stat.S_IMODE(metadata.st_mode) != mode:
                raise RuntimeError(f'Unsafe installed helper ownership or permissions: {target}')
        actions = ET.parse(policy).getroot().findall('action')
        if len(actions) != 1 or actions[0].findtext('defaults/allow_active') != 'auth_admin':
            raise RuntimeError('Installer policy must require fresh administrator authentication')
        if actions[0].findtext("annotate[@key='org.freedesktop.policykit.exec.path']") != str(helper):
            raise RuntimeError('Installer policy path does not bind the fixed helper')
        # Debian may append its own revision (e.g. 0.4.0-1).
        expected_version = run([str(args.binary.resolve()), '--version'], 'candidate-version')
        if not expected_version.startswith('Recorde ') or run([str(installed), '--version'], 'version') != expected_version:
            raise RuntimeError('Installed executable version mismatch')
        assert_userdata()
        run(['sudo', '-n', 'apt-get', 'remove', '-y', 'recorde'], 'uninstall')
        installed_package = False
        if any(target.exists() for target in [installed, helper, policy]):
            raise RuntimeError('Uninstall left executables or privileged policy behind')
        assert_userdata()
        (args.artifacts / 'result.json').write_text(json.dumps({
            'baseline': old_version, 'candidate': new_version, 'upgrade': 'passed',
            'uninstall': 'passed', 'userdata': 'preserved'}, indent=2))
        print('PASS: DEB install, upgrade, uninstall and user-data retention.')
    finally:
        if installed_package:
            run(['sudo', '-n', 'apt-get', 'remove', '-y', 'recorde'], 'cleanup')


if __name__ == '__main__':
    main()
