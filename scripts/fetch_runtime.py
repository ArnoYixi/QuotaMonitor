#!/usr/bin/env python3
"""Fetch exact upstream runtime packages; verify SHA-256 before atomic rename."""
import argparse, concurrent.futures, hashlib, pathlib, urllib.request

VERSION = '0.162.1'
ASSETS = {
    'windows': ('x86_64-pc-windows-msvc', '402a36449947ffbdd137b8a6b372df5b3f220ab0fb5c52b1f032f801cd7caed2'),
    'linux': ('x86_64-unknown-linux-musl', '41153de39d4f4ea8f747cea6fbd36dc3aefb3b3a54b0dfffac9ac73c08237e5d'),
}

def fetch(platform):
    target, expected = ASSETS[platform]
    name = f'codex-app-server-package-{target}.tar.zst'
    folder = pathlib.Path(__file__).resolve().parent.parent/'vendor'
    folder.mkdir(exist_ok=True)
    path = folder/name
    def digest(p):
        with p.open('rb') as f:
            return hashlib.file_digest(f, 'sha256').hexdigest()
    if path.exists() and digest(path) == expected:
        print(f'Verified {name}')
        return
    temporary = path.with_suffix('.download')
    try:
        url = f'https://github.com/openai/codex/releases/download/rust-v{VERSION}/{name}'
        with urllib.request.urlopen(url, timeout=90) as response, temporary.open('wb') as output:
            while block := response.read(1024*1024):
                output.write(block)
        if digest(temporary) != expected:
            raise RuntimeError(f'SHA-256 mismatch for {name}')
        temporary.replace(path)
        print(f'Verified {name}')
    finally:
        temporary.unlink(missing_ok=True)

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--platform', choices=['windows', 'linux', 'all'], default='all')
    args = parser.parse_args()
    choices = list(ASSETS) if args.platform == 'all' else [args.platform]
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        list(pool.map(fetch, choices))

