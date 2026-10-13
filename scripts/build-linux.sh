#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/fetch_runtime.py --platform linux
QUOTA_BUNDLE_RUNTIME=1 cargo build --release --locked
mkdir -p dist
cp target/release/QuotaMonitori dist/QuotaMonitori-linux-x64
sha256sum dist/QuotaMonitori-linux-x64

