$ErrorActionPreference = 'Stop'
# Software OpenGL for GPU-less CI only. Never copy these DLLs into dist.
$root = Join-Path $env:RUNNER_TEMP 'quota-ui-renderer'
New-Item -ItemType Directory -Force $root | Out-Null
$archive = Join-Path $root 'mesa.7z'
Invoke-WebRequest 'https://github.com/pal1000/mesa-dist-win/releases/download/26.2.4/mesa3d-26.2.4-release-msvc.7z' -OutFile $archive
if ((Get-FileHash $archive -Algorithm SHA256).Hash.ToLower() -ne '351fc8c8b695878ffb3eaa044b3ead08672a48b1a045e3c3e3975811df0f6695') { throw 'Mesa digest mismatch' }
& 7z x $archive "-o$root/mesa" -y | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Mesa extraction failed' }
$gl = Get-ChildItem "$root/mesa" -Recurse -Filter opengl32.dll | Where-Object { $_.FullName -match '[\\/]x64[\\/]' } | Select-Object -First 1
if (-not $gl) { throw 'Missing 64-bit software renderer' }
$app = Join-Path $root 'app'
New-Item -ItemType Directory -Force $app | Out-Null
Copy-Item dist/QuotaMonitor.exe "$app/QuotaMonitor.exe"
Copy-Item "$($gl.DirectoryName)/*.dll" $app
"QUOTA_GUI_EXE=$app/QuotaMonitor.exe" | Out-File $env:GITHUB_ENV -Append -Encoding utf8
'GALLIUM_DRIVER=llvmpipe' | Out-File $env:GITHUB_ENV -Append -Encoding utf8
