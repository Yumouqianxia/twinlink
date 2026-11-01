"""Versioned signed GitHub releases; rolling metadata lives on the updates branch.

No credentials are accepted as arguments. gh reads its normal authenticated
session or GH_TOKEN. The signing key is only used by Tauri, never by this script.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
REPO = 'Yumouqianxia/twinlink'
PLATFORMS = {'darwin-aarch64', 'darwin-x86_64', 'windows-x86_64'}


def version():
    return json.loads((ROOT / 'package.json').read_text(encoding='utf-8'))['version']


def validate():
    value = version()
    assert re.fullmatch(r'\d+\.\d+\.\d+', value), 'Use a numeric x.y.z version; GitHub marks 0.x as prerelease'
    assert os.environ.get('RELEASE_TAG') == 'v' + value, 'Tag must match package version'
    assert json.loads((ROOT / 'src-tauri/tauri.conf.json').read_text(encoding='utf-8'))['version'] == value
    import tomllib
    for path in ['src-tauri/Cargo.toml', 'crates/twinlink-core/Cargo.toml']:
        assert tomllib.loads((ROOT / path).read_text(encoding='utf-8'))['package']['version'] == value
    print('Validated release version: v' + value)
    return value


def stage(platform, target):
    assert platform in PLATFORMS
    destination = ROOT / 'release-assets'
    destination.mkdir(exist_ok=True)
    pattern = 'release/bundle/nsis/*-setup.exe' if platform.startswith('windows') else 'release/bundle/macos/*.app.tar.gz'
    candidates = list(Path(target).glob(pattern))
    assert len(candidates) == 1, 'Expected exactly one built updater package'
    source = candidates[0]
    signature = Path(str(source) + '.sig').read_text(encoding='utf-8').strip()
    assert signature and len(signature) < 8192, 'Missing update signature'
    suffix = '-setup.exe' if platform.startswith('windows') else '.app.tar.gz'
    name = f'TwinLink_{version()}_{platform}{suffix}'
    shutil.copyfile(source, destination / name)
    (destination / (name + '.sig')).write_text(signature + '\n', encoding='utf-8')
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT).decode().strip()
    fragment = {'platform': platform, 'url': f'https://github.com/{REPO}/releases/download/v{version()}/{name}', 'signature': signature, 'sha256': digest, 'file': name, 'commit': commit}
    (destination / (platform + '.json')).write_text(json.dumps(fragment, indent=2) + '\n', encoding='utf-8')
    print('Staged signed package:', name)


def gh(*args, data=None, optional=False):
    result = subprocess.run(['gh', *args], input=json.dumps(data).encode() if data is not None else None, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode:
        if optional:
            return None
        # gh errors don't include credentials; never print key or env values.
        raise RuntimeError(result.stderr.decode(errors='replace'))
    output = result.stdout.decode()
    return json.loads(output) if output.strip().startswith(('{', '[')) else output.strip()


def publish():
    value = validate()
    assert os.environ.get('GH_REPO') == REPO, 'Updates are locked to the official repository'
    destination = ROOT / 'release-assets'
    fragments = [json.loads((destination / (platform + '.json')).read_text(encoding='utf-8')) for platform in sorted(PLATFORMS)]
    platforms = {}
    paths = []
    checksums = []
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT).decode().strip()
    for f in fragments:
        assert f['commit'] == commit, 'All packages must match the release tag source commit'
        platform = f['platform']
        assert platform in PLATFORMS and platform not in platforms
        assert Path(f['file']).name == f['file']
        expected = f'https://github.com/{REPO}/releases/download/v{value}/{f["file"]}'
        assert f['url'] == expected
        package = destination / f['file']
        assert hashlib.sha256(package.read_bytes()).hexdigest() == f['sha256']
        assert Path(str(package) + '.sig').read_text(encoding='utf-8').strip() == f['signature']
        platforms[platform] = {'url': f['url'], 'signature': f['signature']}
        paths.extend([str(package), str(package) + '.sig'])
        checksums.append(f'{f["sha256"]}  {f["file"]}')
    tag = 'v' + value
    # Refuse to move the public channel backwards. Only numeric releases are
    # accepted above, making numeric tuple comparison unambiguous.
    old = gh('api', f'repos/{REPO}/contents/latest.json?ref=updates', optional=True)
    if old:
        previous = json.loads(base64.b64decode(old['content']))
        assert tuple(map(int, value.split('.'))) >= tuple(map(int, previous['version'].split('.'))), 'Refusing channel downgrade'
    existing = gh('release', 'view', tag, '--repo', REPO, '--json', 'isDraft', optional=True)
    notes = (ROOT / 'docs/RELEASE-NOTES.md').read_text(encoding='utf-8')
    manifest = {'version': value, 'notes': notes, 'platforms': platforms}
    (destination / 'latest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    (destination / 'SHA256SUMS.txt').write_text('\n'.join(checksums) + '\n', encoding='utf-8')
    paths += [str(destination / 'latest.json'), str(destination / 'SHA256SUMS.txt')]
    if existing is not None and not existing['isDraft']:
        # A failed channel commit may be retried after publication, but never
        # replace already published bytes with a different rebuild.
        with tempfile.TemporaryDirectory() as directory:
            gh('release', 'download', tag, '--repo', REPO, '--pattern', 'latest.json', '--dir', directory)
            assert json.loads((Path(directory) / 'latest.json').read_text(encoding='utf-8')) == manifest, 'A published version is immutable; use a new version tag'
    else:
        if existing is None:
            gh('release', 'create', tag, '--repo', REPO, '--verify-tag', '--draft', '--title', f'同频 TwinLink {tag}' + (' · 预发布版' if value.startswith('0.') else ''), '--notes-file', str(ROOT / 'docs/RELEASE-NOTES.md'))
        gh('release', 'upload', tag, '--repo', REPO, '--clobber', *paths)
        gh('release', 'edit', tag, '--repo', REPO, '--draft=false', '--prerelease=' + str(value.startswith('0.')).lower(), '--latest=false')
    # Never point clients at a draft or a partially uploaded release.
    branch = gh('api', f'repos/{REPO}/git/ref/heads/updates', optional=True)
    if branch is None:
        ref = gh('api', f'repos/{REPO}/commits/{tag}')
        gh('api', f'repos/{REPO}/git/refs', '--method', 'POST', '--input', '-', data={'ref': 'refs/heads/updates', 'sha': ref['sha']})
    data = {'message': f'Update preview channel to {tag}', 'branch': 'updates', 'content': base64.b64encode((destination / 'latest.json').read_bytes()).decode()}
    if old:
        data['sha'] = old['sha']
    gh('api', f'repos/{REPO}/contents/latest.json', '--method', 'PUT', '--input', '-', data=data)
    print(f'Published https://github.com/{REPO}/releases/tag/{tag}; GitHub update channel advanced')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    sub.add_parser('validate')
    staging = sub.add_parser('stage')
    staging.add_argument('--platform', required=True, choices=sorted(PLATFORMS))
    staging.add_argument('--target-dir', default=os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target')))
    sub.add_parser('publish')
    args = parser.parse_args()
    if args.command == 'validate': validate()
    elif args.command == 'stage': stage(args.platform, args.target_dir)
    else: publish()


if __name__ == '__main__': main()
