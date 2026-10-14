$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class DesktopProbe {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int L,T,R,B; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
'@
$data = Join-Path $env:RUNNER_TEMP ('quota-ui-' + [guid]::NewGuid())
$p = Start-Process -FilePath 'dist/QuotaMonitori.exe' -ArgumentList @('--demo', '--data-dir', $data, '--view', 'bar') -PassThru
try {
    for ($i=0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 300
        $p.Refresh()
        if ($p.HasExited) { throw 'Window process exited' }
        if ($p.MainWindowHandle -ne 0) { break }
    }
    if ($p.MainWindowHandle -eq 0) { throw 'No native window' }
    Start-Sleep -Seconds 2
    [DesktopProbe]::SetForegroundWindow($p.MainWindowHandle) | Out-Null
    $before = New-Object DesktopProbe+Rect
    [DesktopProbe]::GetWindowRect($p.MainWindowHandle, [ref]$before) | Out-Null
    $x=$before.L+30; $y=$before.T+30
    [DesktopProbe]::SetCursorPos($x,$y) | Out-Null
    [DesktopProbe]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 250
    for ($i=1; $i -le 10; $i++) {
        [DesktopProbe]::SetCursorPos($x+8*$i,$y+5*$i) | Out-Null
        Start-Sleep -Milliseconds 30
    }
    [DesktopProbe]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 500
    $after = New-Object DesktopProbe+Rect
    [DesktopProbe]::GetWindowRect($p.MainWindowHandle, [ref]$after) | Out-Null
    if ([Math]::Abs($after.L-$before.L) -lt 30) { throw 'Dragging the quota bar did not move the window' }
    [DesktopProbe]::SetCursorPos($after.L+30,$after.T+30) | Out-Null
    [DesktopProbe]::mouse_event(8,0,0,0,[UIntPtr]::Zero)
    [DesktopProbe]::mouse_event(16,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Seconds 1
    $menu = New-Object DesktopProbe+Rect
    [DesktopProbe]::GetWindowRect($p.MainWindowHandle, [ref]$menu) | Out-Null
    if (($menu.B-$menu.T) -lt 400) { throw 'Right click did not open settings' }
    @{ drag=$true; right_click_settings=$true; before=@($before.L,$before.T); after=@($after.L,$after.T) } | ConvertTo-Json | Set-Content dist/window-test.json
} finally {
    [DesktopProbe]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
}
