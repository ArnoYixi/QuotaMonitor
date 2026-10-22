$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'desktop-fixture.ps1')
$exe=if($env:QUOTA_GUI_EXE) { $env:QUOTA_GUI_EXE } else { 'dist/QuotaMonitori.exe' }
$data=Join-Path $env:RUNNER_TEMP ('quota-lock-'+[guid]::NewGuid())
$script:p=$null
$fixture=$false
function Rect {
    $r=New-Object BackdropNative+Rect
    [BackdropNative]::GetWindowRect($script:hwnd,[ref]$r) | Out-Null
    return $r
}
function Start-Monitor {
    $script:p=Start-Process $exe -ArgumentList @('--demo','--data-dir',$data,'--view','bar') -PassThru
    $script:hwnd=[IntPtr]::Zero
    for($i=0;$i -lt 40;$i++) {
        Start-Sleep -Milliseconds 250
        $script:hwnd=[BackdropNative]::Find($script:p.Id)
        if($script:hwnd -ne [IntPtr]::Zero) {break}
    }
    if($script:hwnd -eq [IntPtr]::Zero) {throw 'No monitor window'}
    Start-Sleep -Seconds 2
    $script:tray=[BackdropNative]::FindClass($script:p.Id,'tray_icon_app')
    if($script:tray -eq [IntPtr]::Zero) {throw 'No tray recovery icon'}
    $script:scale=[BackdropNative]::GetDpiForWindow($script:hwnd)/96.0
    [BackdropNative]::SetWindowPos($script:hwnd,[IntPtr](-1),250,200,0,0,0x11) | Out-Null
    Start-Sleep -Seconds 1
}
function Click([int]$x,[int]$y,[bool]$right=$false) {
    [BackdropNative]::SetCursorPos($x,$y) | Out-Null
    Start-Sleep -Milliseconds 150
    [BackdropNative]::mouse_event($(if($right){8}else{2}),0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 100
    [BackdropNative]::mouse_event($(if($right){16}else{4}),0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 500
}
function Popup {
    $window=[IntPtr]::Zero
    for($i=0;$i -lt 30;$i++) {
        Start-Sleep -Milliseconds 100
        $window=[BackdropNative]::FindClass($script:p.Id,'#32768')
        if($window -ne [IntPtr]::Zero) {break}
    }
    if($window -eq [IntPtr]::Zero) {throw 'Native popup did not open'}
    return [BackdropNative]::SendMessageW($window,0x1e1,[UIntPtr]::Zero,[IntPtr]::Zero)
}
function Labels($menu) {
    return @(for($i=0;$i -lt [BackdropNative]::GetMenuItemCount($menu);$i++) {
        $text=New-Object System.Text.StringBuilder 256
        [BackdropNative]::GetMenuStringW($menu,$i,$text,256,0x400) | Out-Null
        $text.ToString()
    })
}
function Choose($menu,[string]$wanted) {
    $labels=Labels $menu
    $index=[Array]::IndexOf($labels,$wanted)
    if($index -lt 0) {throw "Menu item missing: $wanted"}
    if(([BackdropNative]::GetMenuState($menu,$index,0x400) -band 3) -ne 0) {throw "Menu item is disabled: $wanted"}
    $item=New-Object BackdropNative+Rect
    if(-not [BackdropNative]::GetMenuItemRect([IntPtr]::Zero,$menu,$index,[ref]$item)) {throw 'No menu item rectangle'}
    Click ([int](($item.L+$item.R)/2)) ([int](($item.T+$item.B)/2))
    Start-Sleep -Milliseconds 700
}
function Tray([string]$wanted) {
    [BackdropNative]::SetCursorPos(650,450) | Out-Null
    [BackdropNative]::PostMessageW($script:tray,6002,[UIntPtr]::Zero,[IntPtr]0x204) | Out-Null
    $menu=Popup
    Choose $menu $wanted
}
function Assert-Locked([bool]$expected) {
    $style=[BackdropNative]::GetWindowLongPtrW($script:hwnd,-20).ToInt64()
    $pass=($style -band 0x20) -ne 0
    if($pass -ne $expected -or ($expected -and ($style -band 0x80000) -eq 0)) {throw "Incorrect click-through window style: $style; expected lock: $expected"}
}
function Capture([string]$name) {
    $r=Rect
    $b=New-Object System.Drawing.Bitmap(($r.R-$r.L),($r.B-$r.T))
    $g=[System.Drawing.Graphics]::FromImage($b)
    $dst=$g.GetHdc(); $src=[BackdropNative]::GetDC([IntPtr]::Zero)
    try {
        if(-not [BackdropNative]::BitBlt($dst,0,0,$b.Width,$b.Height,$src,$r.L,$r.T,0x40cc0020)) {throw 'Screen capture failed'}
    } finally {
        [BackdropNative]::ReleaseDC([IntPtr]::Zero,$src) | Out-Null
        $g.ReleaseHdc($dst); $g.Dispose()
    }
    $qa=Join-Path $pwd 'dist/qa'
    New-Item -ItemType Directory -Force $qa | Out-Null
    $b.Save((Join-Path $qa "$name.png"))
    return $b
}
function Assert-Green($image,[int]$y) {
    $c=$image.GetPixel([int](30*$script:scale),$y)
    if($c.G -lt 100 -or $c.G -lt $c.R+35) {throw "Overlay is invisible or quota did not reset: $c"}
}
try {
    Start-Monitor
    $r=Rect
    $card=($r.B-$r.T-20*$scale)/3
    $pitch=$card+6*$scale
    $barY=[int](4*$scale+$card-10*$scale)
    [QuotaBackdrop]::ClickTarget=$true
    [QuotaBackdrop]::Start(($r.L-20),($r.T-20),($r.R-$r.L+40),($r.B-$r.T+40))
    $fixture=$true
    [BackdropNative]::SetWindowPos([QuotaBackdrop]::Window,$hwnd,0,0,0,0,0x13) | Out-Null
    [BackdropNative]::SetForegroundWindow($hwnd) | Out-Null
    Start-Sleep -Seconds 2
    Assert-Locked $false
    $before=Capture 'lock-before'
    try {Assert-Green $before $barY} finally {$before.Dispose()}
    $clicks=[QuotaBackdrop]::LeftClicks
    Click ($r.L+[int](30*$scale)) ($r.T+$barY)
    if([QuotaBackdrop]::LeftClicks -ne $clicks) {throw 'Unlocked overlay unexpectedly passed a click'}
    Click ($r.L+[int](30*$scale)) ($r.T+[int](16*$scale)) $true
    $menu=Popup
    $labels=Labels $menu
    if(($labels -join '|') -ne '使用重置卡|锁定悬浮|打开菜单|退出') {throw "Incorrect floating menu: $labels"}
    if(([BackdropNative]::GetMenuState($menu,0,0x400) -band 3) -eq 0) {throw 'Reset was enabled for a nondepleted account'}
    Choose $menu '锁定悬浮'
    Assert-Locked $true
    $locked=Capture 'lock-active'
    try {Assert-Green $locked $barY} finally {$locked.Dispose()}
    # Exercise real mouse delivery into a different process at text, progress,
    # and card padding positions. Window styles alone do not prove hit testing.
    $points=@(@(30,16),@(30,($barY/$scale)),@(7,($card/2+4)))
    foreach($point in $points) {
        $previous=[QuotaBackdrop]::LeftClicks
        Click ($r.L+[int]($point[0]*$scale)) ($r.T+[int]($point[1]*$scale))
        if([QuotaBackdrop]::LeftClicks -ne $previous+1) {throw "Locked overlay blocked underlying button at $point"}
    }
    $right=[QuotaBackdrop]::RightClicks
    Click ($r.L+[int](30*$scale)) ($r.T+[int](16*$scale)) $true
    if([QuotaBackdrop]::RightClicks -ne $right+1) {throw 'Locked overlay blocked a right click'}
    if([BackdropNative]::FindClass($p.Id,'#32768') -ne [IntPtr]::Zero) {throw 'Locked overlay opened its own menu'}
    $still=Rect
    if($still.L -ne $r.L -or $still.T -ne $r.T) {throw 'Locked overlay moved'}
    [BackdropNative]::PostMessageW($tray,6002,[UIntPtr]::Zero,[IntPtr]0x202) | Out-Null
    Start-Sleep -Seconds 1
    Assert-Locked $true
    Tray '解锁悬浮'
    Assert-Locked $false
    $previous=[QuotaBackdrop]::LeftClicks
    Click ($r.L+[int](30*$scale)) ($r.T+$barY)
    if([QuotaBackdrop]::LeftClicks -ne $previous) {throw 'Tray unlock did not restore mouse interception'}
    # The depleted third account is the only eligible demo reset target.
    $thirdY=[int]($barY+2*$pitch)
    Click ($r.L+[int](30*$scale)) ($r.T+$thirdY) $true
    $menu=Popup
    Choose $menu '使用重置卡'
    Start-Sleep -Seconds 2
    $reset=Capture 'menu-reset'
    try {Assert-Green $reset $thirdY} finally {$reset.Dispose()}
    Tray '锁定悬浮'
    Assert-Locked $true
    Start-Sleep -Seconds 3
    # Restart with the same settings and confirm startup is already click-through.
    Stop-Process -Id $p.Id -Force
    Start-Sleep -Milliseconds 600
    Start-Monitor
    Assert-Locked $true
    $r=Rect
    $previous=[QuotaBackdrop]::LeftClicks
    Click ($r.L+[int](30*$scale)) ($r.T+[int](16*$scale))
    if([QuotaBackdrop]::LeftClicks -ne $previous+1) {throw 'Restart lost click-through'}
    Tray '解锁悬浮'
    Assert-Locked $false
    Tray '锁定悬浮'
    Assert-Locked $true
    Tray '退出'
    if(-not $p.WaitForExit(5000)) {throw 'Tray Quit failed while locked'}
    @{exact_context_menu=$true; reset_eligibility=$true; menu_reset=$true; real_left_click_through=$true; real_right_click_through=$true; overlay_remains_visible=$true; tray_left_click_keeps_lock=$true; tray_only_unlock=$true; persisted_click_through=$true; quit_while_locked=$true} | ConvertTo-Json | Set-Content dist/lock-test.json
} finally {
    [BackdropNative]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    [BackdropNative]::mouse_event(16,0,0,0,[UIntPtr]::Zero)
    if($fixture) {[QuotaBackdrop]::Stop()}
    if($null -ne $p -and -not $p.HasExited) {Stop-Process -Id $p.Id -Force}
}
