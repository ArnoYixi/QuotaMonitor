$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
# Load the fixture's forwarded assemblies before collecting compiler references.
$fixtureTypes=@([System.Windows.Forms.Form],[System.Drawing.Graphics],[System.Drawing.Color],
    [System.ComponentModel.Component],[System.Threading.ManualResetEventSlim],[System.Threading.Thread])
$references=@([AppDomain]::CurrentDomain.GetAssemblies() | Where-Object { -not $_.IsDynamic -and $_.Location } | ForEach-Object { $_.Location })
Add-Type -ReferencedAssemblies $references -TypeDefinition @'
using System;
using System.Drawing;
using System.Text;
using System.Threading;
using System.Windows.Forms;
using System.Runtime.InteropServices;
public sealed class QuotaBackdrop : Form {
    public static volatile int Mode;
    static QuotaBackdrop form;
    public static IntPtr Window;
    public static void Start(int x,int y,int w,int h) {
        var ready=new ManualResetEventSlim();
        var thread=new Thread(() => {
            form=new QuotaBackdrop();
            form.AutoScaleMode=AutoScaleMode.None;
            form.FormBorderStyle=FormBorderStyle.None;
            form.StartPosition=FormStartPosition.Manual;
            form.Bounds=new Rectangle(x,y,w,h);
            form.TopMost=true;
            form.ShowInTaskbar=false;
            Window=form.Handle;
            var timer=new System.Windows.Forms.Timer();
            timer.Interval=100;
            timer.Tick+=(s,e)=>form.Invalidate();
            timer.Start();
            form.Shown+=(s,e)=>ready.Set();
            Application.Run(form);
            timer.Dispose();
        });
        thread.IsBackground=true;
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        if(!ready.Wait(10000)) throw new Exception("Backdrop UI did not start");
    }
    protected override void OnPaintBackground(PaintEventArgs e) {
        if(Mode==0) { e.Graphics.Clear(Color.Black); return; }
        if(Mode==1) { e.Graphics.Clear(Color.White); return; }
        e.Graphics.Clear(Color.FromArgb(8,16,30));
        for(int y=0;y<Height;y+=16) for(int x=0;x<Width;x+=16)
            if(((x/16+y/16)&1)==0) e.Graphics.FillRectangle(Brushes.White,x,y,16,16);
    }
    public static void Stop() { var current=form; current.BeginInvoke(new Action(()=>current.Close())); }
}
public static class BackdropNative {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int L,T,R,B; }
    public delegate bool EnumProc(IntPtr h,IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc proc,IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h,StringBuilder s,int n);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out Rect r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h,IntPtr z,int x,int y,int w,int height,uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int mode);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
    [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h,IntPtr dc);
    [DllImport("gdi32.dll")] public static extern bool BitBlt(IntPtr dst,int x,int y,int w,int h,IntPtr src,int sx,int sy,uint rop);
    public static IntPtr Find(uint pid) {
        IntPtr found=IntPtr.Zero;
        EnumWindows((h,l)=> { uint p; GetWindowThreadProcessId(h,out p); var c=new StringBuilder(128);
            GetClassName(h,c,128); if(p==pid && c.ToString()=="Window Class") {found=h;return false;} return true; },IntPtr.Zero);
        return found;
    }
}
'@
[BackdropNative]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
function Capture($r) {
    $b=New-Object System.Drawing.Bitmap(($r.R-$r.L),($r.B-$r.T))
    $g=[System.Drawing.Graphics]::FromImage($b)
    $dest=$g.GetHdc(); $src=[BackdropNative]::GetDC([IntPtr]::Zero)
    try {
        if (-not [BackdropNative]::BitBlt($dest,0,0,$b.Width,$b.Height,$src,$r.L,$r.T,0x40cc0020)) { throw 'Screen capture failed' }
    } finally {
        [BackdropNative]::ReleaseDC([IntPtr]::Zero,$src) | Out-Null
        $g.ReleaseHdc($dest); $g.Dispose()
    }
    return $b
}
function Contrast($b,[int]$y,[int]$start,[int]$end) {
    $values=@(for($x=$start;$x -lt $end;$x++) { $c=$b.GetPixel($x,$y); ($c.R+$c.G+$c.B)/3.0 })
    $mean=($values | Measure-Object -Average).Average
    $sum=0.0; foreach($v in $values) { $sum+=($v-$mean)*($v-$mean) }
    return [Math]::Sqrt($sum/$values.Count)
}
$exe=if($env:QUOTA_GUI_EXE) { $env:QUOTA_GUI_EXE } else { 'dist/QuotaMonitori.exe' }
$results=@()
foreach($view in @('bar','ring')) {
    $data=Join-Path $env:RUNNER_TEMP ('quota-backdrop-'+[guid]::NewGuid())
    $p=Start-Process $exe -ArgumentList @('--demo','--data-dir',$data,'--view',$view) -PassThru
    $started=$false
    $images=@()
    try {
        $hwnd=[IntPtr]::Zero
        for($i=0;$i -lt 40;$i++) {
            Start-Sleep -Milliseconds 250
            $hwnd=[BackdropNative]::Find($p.Id)
            if($hwnd -ne [IntPtr]::Zero) {break}
        }
        if($hwnd -eq [IntPtr]::Zero) {throw 'No monitor window'}
        [BackdropNative]::SetWindowPos($hwnd,[IntPtr](-1),200,200,0,0,0x11) | Out-Null
        Start-Sleep -Seconds 2
        $r=New-Object BackdropNative+Rect
        [BackdropNative]::GetWindowRect($hwnd,[ref]$r) | Out-Null
        $scale=[BackdropNative]::GetDpiForWindow($hwnd)/96.0
        [QuotaBackdrop]::Mode=0
        [QuotaBackdrop]::Start($r.L-32,$r.T-32,($r.R-$r.L+64),($r.B-$r.T+64))
        $started=$true
        [BackdropNative]::SetWindowPos([QuotaBackdrop]::Window,$hwnd,0,0,0,0,0x13) | Out-Null
        [BackdropNative]::SetForegroundWindow($hwnd) | Out-Null
        Start-Sleep -Seconds 3
        $dark=Capture $r; $images+=,$dark
        [QuotaBackdrop]::Mode=1
        Start-Sleep -Seconds 3
        $light=Capture $r; $images+=,$light
        $outside=$light.GetPixel(2,2)
        if($outside.R -lt 245 -or $outside.G -lt 245 -or $outside.B -lt 245) {
            throw "White fixture is obscured: $outside"
        }
        $ink=0; $inverted=0
        $textEnd=if($view -eq 'ring'){[int](80*$scale)}else{[int](125*$scale)}
        for($y=[int](5*$scale);$y -lt 19*$scale;$y++) { for($x=[int](6*$scale);$x -lt $textEnd;$x++) {
            $d=$dark.GetPixel($x,$y); $l=$light.GetPixel($x,$y)
            if($d.R -gt 240 -and $d.G -gt 240 -and $d.B -gt 240) {
                $ink++
                if($l.R -lt 30 -and $l.G -lt 30 -and $l.B -lt 30) {$inverted++}
            }
        } }
        if($ink -lt 20 -or $inverted -lt $ink*0.8) {throw "Text inversion failed: $inverted / $ink"}
        [QuotaBackdrop]::Mode=2
        [BackdropNative]::ShowWindow($hwnd,0) | Out-Null
        Start-Sleep -Seconds 1
        $raw=Capture $r; $images+=,$raw
        [BackdropNative]::ShowWindow($hwnd,4) | Out-Null
        [BackdropNative]::SetWindowPos([QuotaBackdrop]::Window,$hwnd,0,0,0,0,0x13) | Out-Null
        Start-Sleep -Seconds 3
        $busy=Capture $r; $images+=,$busy
        $probeY=if($view -eq 'ring'){[int](116*$scale)}else{[int](46*$scale)}
        $end=if($view -eq 'ring'){[int](80*$scale)}else{[int](220*$scale)}
        $rawDetail=Contrast $raw $probeY ([int](12*$scale)) $end
        $busyDetail=Contrast $busy $probeY ([int](12*$scale)) $end
        Write-Host "$view texture contrast: $rawDetail -> $busyDetail; text inverted $inverted / $ink"
        if($rawDetail -lt 60 -or $busyDetail -gt $rawDetail*0.8 -or $busyDetail -lt $rawDetail*0.25) {
            throw 'Adaptive tint is missing or too opaque'
        }
        Start-Sleep -Seconds 3
        $stable=Capture $r; $images+=,$stable
        $stableDetail=Contrast $stable $probeY ([int](12*$scale)) $end
        if([Math]::Abs($stableDetail-$busyDetail) -gt 8) {throw 'Tint oscillated while the background stayed unchanged'}
        [QuotaBackdrop]::Mode=1
        Start-Sleep -Seconds 3
        $recovered=Capture $r; $images+=,$recovered
        $plain=$recovered.GetPixel([int](24*$scale),$probeY)
        if($plain.R -lt 245 -or $plain.G -lt 245 -or $plain.B -lt 245) {throw 'Tint did not fade out on solid background'}
        $results+=@{view=$view; text_inversion=$true; texture_reduction=$true; stable_tint=$true; returns_to_transparent=$true}
        $busy.Save((Join-Path $env:RUNNER_TEMP "quota-$view-busy.png"))
    } finally {
        $qa=Join-Path $pwd 'dist/qa'
        New-Item -ItemType Directory -Force $qa | Out-Null
        for($i=0;$i -lt $images.Count;$i++) {$images[$i].Save((Join-Path $qa "$view-$i.png"))}
        foreach($b in $images) {$b.Dispose()}
        if($started) {[QuotaBackdrop]::Stop()}
        if(-not $p.HasExited) {Stop-Process -Id $p.Id -Force}
    }
}
$results | ConvertTo-Json | Set-Content dist/appearance-test.json
