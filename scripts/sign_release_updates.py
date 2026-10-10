"""Sign each updater target after builds/tests, then independently verify its bytes.

The signer must be built before RECORDE_UPDATE_SIGNING_KEY enters the environment.
Only signed envelopes go into release assets; intermediate manifests stay outside dist.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--signer', type=Path, required=True)
    parser.add_argument('--keys', type=Path, required=True)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--key-id', required=True)
    args = parser.parse_args()
    if not os.environ.get('RECORDE_UPDATE_SIGNING_KEY'):
        raise SystemExit('The release signing secret is not configured; no unsigned update is published.')
    keys = json.loads(args.keys.read_text())
    if any(args.artifacts.glob('recorde-update-*.json')):
        raise SystemExit('Update envelopes must be generated only by this signing step; remove pre-existing envelopes.')
    if not any(key['key_id'] == args.key_id for key in keys['keys']):
        raise SystemExit('Signing key ID must already be pinned in the application.')
    signer = str(args.signer.resolve(strict=True))
    targets = [
        ('windows', 'x86_64', 'portable', f'recorde-{args.version}-windows-x86_64.exe'),
        ('windows', 'x86_64', 'msi', f'recorde-{args.version}-windows-x86_64.msi'),
        ('linux', 'x86_64', 'portable', f'recorde-{args.version}-linux-x86_64'),
        ('linux', 'x86_64', 'deb', f'recorde_{args.version}_amd64.deb'),
        ('linux', 'x86_64', 'appimage', f'Recorde-{args.version}-x86_64.AppImage'),
        ('macos', 'universal', 'dmg', f'Recorde-{args.version}-macos-universal.dmg'),
    ]
    for _, _, _, name in targets:
        path = args.artifacts / name
        if not path.is_file() or path.is_symlink():
            raise SystemExit(f'Missing or linked release artifact: {name}')
    issued = int(time.time())
    public_env = {key: value for key, value in os.environ.items() if key != 'RECORDE_UPDATE_SIGNING_KEY'}
    def run(arguments, signing=False):
        subprocess.run([signer, *arguments], env=None if signing else public_env, check=True, timeout=120)
    with tempfile.TemporaryDirectory(prefix='recorde-release-signing-') as directory:
        for platform, arch, kind, name in targets:
            raw = Path(directory) / f'{platform}-{kind}.json'
            envelope = args.artifacts / f'recorde-update-{platform}-{arch}-{kind}.json'
            run(['manifest', '--artifact', str(args.artifacts / name), '--asset-name', name,
                 '--version', args.version, '--platform', platform, '--arch', arch, '--kind', kind,
                 '--channel', 'preview' if '-' in args.version.split('+')[0] else 'stable',
                 '--key-id', args.key_id, '--issued-at', str(issued), '--expires-at', str(issued + 180 * 86400),
                 '--output', str(raw)])
            run(['sign', '--manifest', str(raw), '--output', str(envelope)], signing=True)
            run(['verify', '--keys', str(args.keys), '--envelope', str(envelope), '--artifact', str(args.artifacts / name)])
    print(f'Verified signed update envelopes for {len(targets)} package targets.')


if __name__ == '__main__':
    main()
