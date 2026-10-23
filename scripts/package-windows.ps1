$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$versionMatch = Select-String -LiteralPath Cargo.toml -Pattern '^version = "([0-9]+\.[0-9]+\.[0-9]+)"$'
if ($versionMatch.Count -ne 1) { throw 'Expected one package version in Cargo.toml' }
$version = $versionMatch.Matches[0].Groups[1].Value
$packageName = "QuotaMonitor-v$version-windows-x64"
$package = Join-Path $pwd "dist/$packageName"
$release = Join-Path $pwd "dist/releases/v$version"
if (Test-Path -LiteralPath $package) { throw "Package directory already exists: $package" }
if (-not (Test-Path -LiteralPath dist/QuotaMonitor.exe)) { throw 'Run build-windows.ps1 first' }
New-Item -ItemType Directory -Path $package -Force | Out-Null
New-Item -ItemType Directory -Path $release -Force | Out-Null
Copy-Item -LiteralPath dist/QuotaMonitor.exe -Destination $package
Copy-Item -LiteralPath README.md, CONTRIBUTING.md, LICENSE, THIRD-PARTY-NOTICES.md -Destination $package
Copy-Item -LiteralPath licenses, docs -Recurse -Destination $package
Copy-Item -LiteralPath assets/FONT-LICENSE.txt -Destination (Join-Path $package 'licenses')
Copy-Item -LiteralPath scripts/Demo.bat -Destination $package

# Exercise the embedded runtime with an isolated, logged-out data directory.
$report = Join-Path $release 'self-test.json'
$process = Start-Process -FilePath (Join-Path $package 'QuotaMonitor.exe') -ArgumentList @('--self-test', ('"' + $report + '"')) -WindowStyle Hidden -PassThru
if (-not $process.WaitForExit(60000)) { $process.Kill(); throw 'Bundled runtime self-test timed out' }
if ($process.ExitCode -ne 0) {
    Get-Content -LiteralPath $report -ErrorAction SilentlyContinue
    throw 'Bundled runtime self-test failed'
}
Get-Content -LiteralPath $report

$zip = Join-Path $release "$packageName.zip"
if (Test-Path -LiteralPath $zip) { throw "Release archive already exists: $zip" }
Compress-Archive -Path (Join-Path $package '*') -DestinationPath $zip -CompressionLevel Optimal
$hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  $packageName.zip" | Set-Content -LiteralPath (Join-Path $release 'SHA256SUMS.txt') -Encoding ascii
Write-Output "Release package: $zip"
