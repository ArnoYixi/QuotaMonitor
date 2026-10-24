$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$env:QUOTA_BUNDLE_RUNTIME = '0'
cargo build --release --locked --target x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { throw 'Build failed' }
New-Item -ItemType Directory -Force dist | Out-Null
Copy-Item target/x86_64-pc-windows-msvc/release/QuotaMonitor.exe dist/QuotaMonitor.exe
python scripts/collect_licenses.py
if ($LASTEXITCODE -ne 0) { throw 'License collection failed' }
Copy-Item README.md, CONTRIBUTING.md, LICENSE, THIRD-PARTY-NOTICES.md dist/
Copy-Item -Recurse -Force licenses, docs dist/
Copy-Item assets/FONT-LICENSE.txt dist/licenses/
Copy-Item scripts/Demo.bat dist/
Get-FileHash dist/QuotaMonitor.exe -Algorithm SHA256

