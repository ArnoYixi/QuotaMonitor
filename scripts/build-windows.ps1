$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
python scripts/fetch_runtime.py --platform windows
if ($LASTEXITCODE -ne 0) { throw 'Runtime download failed' }
$env:QUOTA_BUNDLE_RUNTIME = '1'
cargo build --release --locked --target x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { throw 'Build failed' }
New-Item -ItemType Directory -Force dist | Out-Null
Copy-Item target/x86_64-pc-windows-msvc/release/QuotaMonitori.exe dist/QuotaMonitori.exe
python scripts/collect_licenses.py
if ($LASTEXITCODE -ne 0) { throw 'License collection failed' }
Copy-Item README.md, LICENSE, THIRD-PARTY-NOTICES.md dist/
Copy-Item -Recurse -Force licenses, docs dist/
Copy-Item assets/FONT-LICENSE.txt dist/licenses/
Get-FileHash dist/QuotaMonitori.exe -Algorithm SHA256

