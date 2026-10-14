$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class DesktopProbe {
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint m, UIntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool GetMenuItemRect(IntPtr h, IntPtr menu, uint item, out Rect r);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetMenuStringW(IntPtr menu, uint item, StringBuilder text, int count, uint flags);
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc proc, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtrW(IntPtr h, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, UIntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr z, int x, int y, int w, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool SystemParametersInfoW(uint a, uint b, out Rect r, uint d);
    [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    public static IntPtr Find(uint process, string wanted) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((h,l) => { uint pid; GetWindowThreadProcessId(h, out pid); var c=new StringBuilder(256); GetClassName(h,c,256);
            if (pid==process && c.ToString()==wanted) { result=h; return false; } return true; }, IntPtr.Zero);
        return result;
    }
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int L,T,R,B; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
'@
function Key([byte]$key) {
    [DesktopProbe]::keybd_event($key,0,0,[UIntPtr]::Zero)
    [DesktopProbe]::keybd_event($key,0,2,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 80
}
function TrayMenu([int]$index) {
    [DesktopProbe]::SetCursorPos(500,450) | Out-Null
    [DesktopProbe]::PostMessageW($script:tray,6002,[UIntPtr]::Zero,[IntPtr]0x204) | Out-Null
    Start-Sleep -Milliseconds 400
    if ([DesktopProbe]::Find($p.Id,'#32768') -eq [IntPtr]::Zero) { throw 'Native tray menu did not open' }
    $popup=[DesktopProbe]::Find($p.Id,'#32768')
    $menu=[DesktopProbe]::SendMessageW($popup,0x1e1,[UIntPtr]::Zero,[IntPtr]::Zero)
    $nativeIndex=if ($index -eq 4) { 5 } else { $index }
    $item=New-Object DesktopProbe+Rect
    if (-not [DesktopProbe]::GetMenuItemRect([IntPtr]::Zero,$menu,$nativeIndex,[ref]$item)) { throw 'Cannot locate native menu item' }
    $label=New-Object System.Text.StringBuilder 256
    [DesktopProbe]::GetMenuStringW($menu,$nativeIndex,$label,256,0x400) | Out-Null
    Write-Host "Click tray item: $label"
    [DesktopProbe]::SetCursorPos([int](($item.L+$item.R)/2),[int](($item.T+$item.B)/2)) | Out-Null
    Start-Sleep -Milliseconds 150
    [DesktopProbe]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 120
    [DesktopProbe]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 1400
}
function Rect { $r=New-Object DesktopProbe+Rect; [DesktopProbe]::GetWindowRect($script:hwnd,[ref]$r) | Out-Null; return $r }
$data = Join-Path $env:RUNNER_TEMP ('quota-ui-' + [guid]::NewGuid())
$exe = if ($env:QUOTA_GUI_EXE) { $env:QUOTA_GUI_EXE } else { 'dist/QuotaMonitori.exe' }
$p = Start-Process -FilePath $exe -ArgumentList @('--demo', '--data-dir', $data, '--view', 'bar') -PassThru
try {
    for ($i=0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 300
        $p.Refresh()
        if ($p.HasExited) { throw 'Window process exited' }
        $script:hwnd=[DesktopProbe]::Find($p.Id,'Window Class')
        if ($hwnd -ne [IntPtr]::Zero) { break }
    }
    if ($hwnd -eq [IntPtr]::Zero) { throw 'No native floating window' }
    Start-Sleep -Seconds 2
    $class = New-Object System.Text.StringBuilder 256
    [DesktopProbe]::GetClassName($hwnd, $class, 256) | Out-Null
    if ($class.ToString() -eq '#32770') { throw 'Application showed an error dialog; this is not the floating window' }
    [DesktopProbe]::SetForegroundWindow($hwnd) | Out-Null
    $before = New-Object DesktopProbe+Rect
    [DesktopProbe]::GetWindowRect($hwnd, [ref]$before) | Out-Null
    Write-Host "Main window: $($p.MainWindowTitle); class: $class; size: $($before.R-$before.L) x $($before.B-$before.T)"
    $x=$before.L+30; $y=$before.T+30
    [DesktopProbe]::SetCursorPos($x,$y) | Out-Null
    Start-Sleep -Milliseconds 300
    [DesktopProbe]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 250
    for ($i=1; $i -le 10; $i++) {
        [DesktopProbe]::SetCursorPos($x+8*$i,$y+5*$i) | Out-Null
        Start-Sleep -Milliseconds 30
    }
    [DesktopProbe]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 500
    $after = New-Object DesktopProbe+Rect
    [DesktopProbe]::GetWindowRect($hwnd, [ref]$after) | Out-Null
    if ([Math]::Abs($after.L-$before.L) -lt 30) { throw 'Dragging the quota bar did not move the window' }
    [DesktopProbe]::SetCursorPos($after.L+30,$after.T+30) | Out-Null
    Start-Sleep -Milliseconds 300
    [DesktopProbe]::mouse_event(8,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 120
    [DesktopProbe]::mouse_event(16,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Seconds 1
    $menu = New-Object DesktopProbe+Rect
    [DesktopProbe]::GetWindowRect($hwnd, [ref]$menu) | Out-Null
    Write-Host "Window moved: $($before.L),$($before.T) -> $($after.L),$($after.T); settings size: $($menu.R-$menu.L) x $($menu.B-$menu.T)"
    if (($menu.B-$menu.T) -lt 400) { throw 'Right click did not open settings' }
    $style=[DesktopProbe]::GetWindowLongPtrW($hwnd,-20).ToInt64()
    if (($style -band 0x40000) -ne 0 -or ($style -band 0x80) -eq 0) { throw "Taskbar style is incorrect: $style" }
    Key 0x1b
    Start-Sleep -Seconds 1
    $work=New-Object DesktopProbe+Rect
    [DesktopProbe]::SystemParametersInfoW(48,0,[ref]$work,0) | Out-Null
    [DesktopProbe]::SetWindowPos($hwnd,[IntPtr]::Zero,200,$work.T+4,0,0,0x15) | Out-Null
    Start-Sleep -Seconds 2
    $top=Rect
    if (($top.R-$top.L) -lt 450 -or ($top.B-$top.T) -gt 150) { throw 'Top edge did not switch to a horizontal row' }
    [DesktopProbe]::SetWindowPos($hwnd,[IntPtr]::Zero,200,$work.B-($top.B-$top.T)-4,0,0,0x15) | Out-Null
    Start-Sleep -Seconds 2
    $bottom=Rect
    if (($bottom.R-$bottom.L) -lt 450 -or $bottom.B -gt $work.B) { throw 'Bottom edge layout is invalid' }
    [DesktopProbe]::SetWindowPos($hwnd,[IntPtr]::Zero,$work.L+4,200,0,0,0x15) | Out-Null
    Start-Sleep -Seconds 2
    $side=Rect
    if (($side.R-$side.L) -gt 300 -or ($side.B-$side.T) -lt 150) { throw 'Side edge did not switch back to a column' }
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    $backdrop=New-Object System.Windows.Forms.Form
    $backdrop.FormBorderStyle='None'
    $backdrop.StartPosition='Manual'
    $backdrop.Location=New-Object System.Drawing.Point($side.L,$side.T)
    $backdrop.Size=New-Object System.Drawing.Size(($side.R-$side.L),($side.B-$side.T))
    $backdrop.BackColor=[System.Drawing.Color]::Black
    $backdrop.Show()
    $backdrop.Refresh()
    [System.Windows.Forms.Application]::DoEvents()
    [DesktopProbe]::SetForegroundWindow($hwnd) | Out-Null
    Start-Sleep -Seconds 3
    $dark=New-Object System.Drawing.Bitmap(($side.R-$side.L),($side.B-$side.T))
    $g=[System.Drawing.Graphics]::FromImage($dark)
    $g.CopyFromScreen($side.L,$side.T,0,0,$dark.Size)
    $g.Dispose()
    $backdrop.BackColor=[System.Drawing.Color]::White
    $backdrop.Refresh()
    [System.Windows.Forms.Application]::DoEvents()
    Start-Sleep -Seconds 3
    $light=New-Object System.Drawing.Bitmap($dark.Width,$dark.Height)
    $g=[System.Drawing.Graphics]::FromImage($light)
    $g.CopyFromScreen($side.L,$side.T,0,0,$light.Size)
    $g.Dispose()
    $ink=0; $inverted=0
    for ($y=5; $y -lt 19; $y++) { for ($x=6; $x -lt 125; $x++) {
        $d=$dark.GetPixel($x,$y); $l=$light.GetPixel($x,$y)
        if ($d.R -gt 240 -and $d.G -gt 240 -and $d.B -gt 240) {
            $ink++
            if ($l.R -lt 30 -and $l.G -lt 30 -and $l.B -lt 30) { $inverted++ }
        }
    } }
    Write-Host "Adaptive text: $inverted / $ink glyph pixels inverted on light background"
    $dark.Save((Join-Path $env:RUNNER_TEMP 'quota-dark.png'))
    $light.Save((Join-Path $env:RUNNER_TEMP 'quota-light.png'))
    $dark.Dispose(); $light.Dispose(); $backdrop.Close(); $backdrop.Dispose()
    if ($ink -lt 20 -or $inverted -lt $ink*0.8) { throw 'Text did not adapt from white to black on the changed desktop background' }
    $script:tray=[DesktopProbe]::Find($p.Id,'tray_icon_app')
    if ($tray -eq [IntPtr]::Zero) { throw 'No native tray icon owner' }
    TrayMenu 1
    if ([DesktopProbe]::IsWindowVisible($hwnd)) { throw 'Tray Hide had no effect' }
    TrayMenu 0
    if (-not [DesktopProbe]::IsWindowVisible($hwnd)) { throw 'Tray Show failed to restore hidden window' }
    TrayMenu 3
    $settings=Rect
    if (($settings.B-$settings.T) -lt 400) { throw 'Tray Settings had no effect' }
    TrayMenu 1
    TrayMenu 3
    if (-not [DesktopProbe]::IsWindowVisible($hwnd)) { throw 'Tray Settings failed while hidden' }
    TrayMenu 0
    $monitor=Rect
    if (($monitor.B-$monitor.T) -gt 300 -or ($monitor.R-$monitor.L) -gt 300 -or [Math]::Abs($monitor.T-$side.T) -gt 5) { throw 'Tray Show did not return to monitor' }
    TrayMenu 4
    $p.Refresh()
    if (-not $p.HasExited) { throw 'Tray Quit had no effect' }
    @{ drag=$true; right_click_settings=$true; top_row=$true; bottom_row=$true; side_column=$true; no_taskbar=$true; adaptive_text=$true; tray_hide_show_settings_quit=$true } | ConvertTo-Json | Set-Content dist/window-test.json

} finally {
    [DesktopProbe]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
}
