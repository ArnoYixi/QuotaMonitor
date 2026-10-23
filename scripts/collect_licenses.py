#!/usr/bin/env python3
"""Collect license notices from exactly the dependency versions in Cargo.lock."""
import json
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent
metadata = json.loads(subprocess.check_output(
    ['cargo', 'metadata', '--locked', '--format-version', '1'], cwd=ROOT))
rows = []
for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
    if package['name'] == 'quotamonitor':
        continue
    root = pathlib.Path(package['manifest_path']).parent
    section = ['\n' + '=' * 72, f"{package['name']} {package['version']}",
               f"SPDX: {package.get('license')}",
               f"Source: {package.get('repository') or package.get('source')}"]
    files = sorted({f for pattern in ['LICENSE*', 'LICENCE*', 'COPYING*', 'NOTICE*', 'license*']
                    for f in root.glob(pattern) if f.is_file()})
    if package.get('license_file'):
        files.append(root / package['license_file'])
    for f in dict.fromkeys(files):
        if f.is_file():
            section.extend(['\n--- ' + f.name + ' ---\n', f.read_text(encoding='utf-8', errors='replace')])
    rows.append('\n'.join(section))
out = ROOT / 'licenses' / 'RUST-DEPENDENCY-LICENSES.txt'
out.parent.mkdir(exist_ok=True)
out.write_text('License notices for locked Rust dependencies. Includes build and platform dependencies.\n'
               + '\n'.join(rows), encoding='utf-8')
print(f'{len(rows)} dependency notices: {out}')
