$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
# Compile against PowerShell's BCL reference assemblies, plus the desktop UI
# assemblies. Runtime type-forwarder assemblies alone are insufficient.
$references=@((Get-ChildItem (Join-Path $PSHOME 'ref') -Filter '*.dll').FullName)
$desktop=Split-Path ([System.Windows.Forms.Form].Assembly.Location)
$references+=@(Get-ChildItem $desktop -File | Where-Object { $_.Name -match '^(System\.Windows\.Forms|System\.Drawing\.Common|System\.Private\.Windows|Accessibility).*\.dll$' } | ForEach-Object { $_.FullName })
Add-Type -ReferencedAssemblies $references -TypeDefinition @'
using System;
using System.Drawing;
using System.Text;
using System.Threading;
using System.Windows.Forms;
using System.Runtime.InteropServices;
public sealed class QuotaBackdrop : Form {
    public static volatile int Mode;
    public static bool ClickTarget;
    public static int LeftClicks, RightClicks, WheelEvents;
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
            form.TopMost=!ClickTarget;
            form.ShowInTaskbar=false;
            form.DoubleBuffered=true;
            if(ClickTarget) {
                var button=new Button();
                button.Dock=DockStyle.Fill;
                button.Text="Underlying click target";
                button.Click+=(s,e)=>Interlocked.Increment(ref LeftClicks);
                button.MouseDown+=(s,e)=>{if(e.Button==MouseButtons.Right) Interlocked.Increment(ref RightClicks);};
                button.MouseWheel+=(s,e)=>Interlocked.Increment(ref WheelEvents);
                form.Controls.Add(button);
            }
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
        if(Mode==3) { e.Graphics.Clear(Color.FromArgb(128,128,128)); return; }
        // Paint both tile colors into a buffered surface. Clearing an occluded
        // window then painting only white tiles can erase texture under tint.
        using(var dark=new SolidBrush(Color.FromArgb(8,16,30)))
            for(int y=0;y<Height;y+=16) for(int x=0;x<Width;x+=16)
                e.Graphics.FillRectangle(((x/16+y/16)&1)==0 ? Brushes.White : dark,x,y,16,16);
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
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h,uint m,UIntPtr w,IntPtr l);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h,uint m,UIntPtr w,IntPtr l);
    [DllImport("user32.dll")] public static extern int GetMenuItemCount(IntPtr menu);
    [DllImport("user32.dll")] public static extern uint GetMenuState(IntPtr menu,uint item,uint flags);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetMenuStringW(IntPtr menu,uint item,StringBuilder text,int count,uint flags);
    [DllImport("user32.dll")] public static extern bool GetMenuItemRect(IntPtr h,IntPtr menu,uint item,out Rect r);
    [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtrW(IntPtr h,int n);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint dx,uint dy,uint data,UIntPtr extra);
    [DllImport("user32.dll")] public static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr extra);
    public static IntPtr FindClass(uint pid,string wanted) {
        IntPtr found=IntPtr.Zero;
        EnumWindows((h,l)=>{uint p;GetWindowThreadProcessId(h,out p);var c=new StringBuilder(128);
            GetClassName(h,c,128);if(p==pid && c.ToString()==wanted){found=h;return false;}return true;},IntPtr.Zero);
        return found;
    }
    public static IntPtr Find(uint pid) {
        IntPtr found=IntPtr.Zero;
        EnumWindows((h,l)=> { uint p; GetWindowThreadProcessId(h,out p); var c=new StringBuilder(128);
            GetClassName(h,c,128); if(p==pid && c.ToString()=="Window Class") {found=h;return false;} return true; },IntPtr.Zero);
        return found;
    }
}
'@
[BackdropNative]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
