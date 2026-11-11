$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
trap { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }
[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
# Inputs (environment only): KOR_DESK_ACTION plus KOR_HWND, KOR_STATE, KOR_X/Y/W/H, KOR_APP, KOR_ARGS, KOR_CWD, KOR_TEXT, KOR_LIMIT
$win=@'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class KorWin {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hh, bool repaint);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
  public static void Dpi() { try { SetProcessDpiAwarenessContext((IntPtr)(-4)); } catch {} }
  public static List<IntPtr> Top() {
    var list = new List<IntPtr>();
    EnumWindows((h, l) => {
      if (IsWindowVisible(h) && GetWindowTextLength(h) > 0 && GetWindow(h, 4) == IntPtr.Zero) list.Add(h);
      return true;
    }, IntPtr.Zero);
    return list;
  }
  public static string Title(IntPtr h) {
    var sb = new StringBuilder(GetWindowTextLength(h) + 1);
    GetWindowText(h, sb, sb.Capacity);
    return sb.ToString();
  }
  // Windows only lets the foreground app hand over focus; a synthetic Alt press lifts the lock.
  public static bool Focus(IntPtr h) {
    if (IsIconic(h)) ShowWindow(h, 9);
    keybd_event(0x12, 0, 0, UIntPtr.Zero);
    keybd_event(0x12, 0, 2, UIntPtr.Zero);
    return SetForegroundWindow(h);
  }
}
'@
function Load-Win { Add-Type -TypeDefinition $win; [KorWin]::Dpi() }
function Hwnd {
  $h=[IntPtr][int64]$env:KOR_HWND
  if(-not [KorWin]::IsWindow($h)){ throw "No window $($env:KOR_HWND); call list_windows again." }
  return $h
}

switch($env:KOR_DESK_ACTION){
  'list_windows' {
    Load-Win
    $fg=[KorWin]::GetForegroundWindow()
    $out=foreach($h in [KorWin]::Top()){
      $procId=[uint32]0; [void][KorWin]::GetWindowThreadProcessId($h,[ref]$procId)
      $name=''; try { $name=(Get-Process -Id $procId -ErrorAction Stop).ProcessName } catch {}
      $r=New-Object KorWin+RECT; [void][KorWin]::GetWindowRect($h,[ref]$r)
      $state= if([KorWin]::IsIconic($h)){'minimized'} elseif([KorWin]::IsZoomed($h)){'maximized'} else {'normal'}
      [pscustomobject]@{ hwnd=[int64]$h; title=[KorWin]::Title($h); process=$name; pid=$procId; x=$r.L; y=$r.T; w=$r.R-$r.L; h=$r.B-$r.T; state=$state; focused=($h -eq $fg) }
    }
    ConvertTo-Json -Compress -Depth 3 -InputObject @($out)
  }
  'focus' {
    Load-Win
    $h=Hwnd
    if([KorWin]::Focus($h)){ "focused: $([KorWin]::Title($h))" } else { throw 'Windows refused to bring the window to the front.' }
  }
  'state' {
    Load-Win
    $h=Hwnd
    $t=[KorWin]::Title($h)
    switch($env:KOR_STATE){
      'minimize' { [void][KorWin]::ShowWindow($h,6) }
      'maximize' { [void][KorWin]::ShowWindow($h,3) }
      'restore'  { [void][KorWin]::ShowWindow($h,9) }
      'close'    { [void][KorWin]::PostMessage($h,0x0010,[IntPtr]::Zero,[IntPtr]::Zero) }
      default    { throw 'state must be minimize, maximize, restore or close' }
    }
    "$($env:KOR_STATE): $t"
  }
  'move' {
    Load-Win
    $h=Hwnd
    if([KorWin]::IsIconic($h) -or [KorWin]::IsZoomed($h)){ [void][KorWin]::ShowWindow($h,9) }
    if(-not [KorWin]::MoveWindow($h,[int]$env:KOR_X,[int]$env:KOR_Y,[int]$env:KOR_W,[int]$env:KOR_H,$true)){ throw 'MoveWindow failed.' }
    "moved: $([KorWin]::Title($h))"
  }
  'launch' {
    $sp=@{ FilePath=$env:KOR_APP; PassThru=$true }
    if($env:KOR_ARGS){ $sp.ArgumentList=$env:KOR_ARGS }
    if($env:KOR_CWD){ $sp.WorkingDirectory=$env:KOR_CWD }
    # App execution aliases (Windows 11 notepad, Store apps) have no process handle to return.
    $p=$null
    try { $p=Start-Process @sp } catch { $sp.Remove('PassThru'); Start-Process @sp }
    if($null -ne $p){ "started pid $($p.Id)" } else { 'started' }
  }
  'clipboard_read' {
    $t=Get-Clipboard -Raw
    if($null -eq $t){ '' } else { [string]$t }
  }
  'clipboard_write' {
    Set-Clipboard -Value $env:KOR_TEXT
    'ok'
  }
  'processes' {
    $n=[int]$env:KOR_LIMIT
    $out=Get-Process | Sort-Object WorkingSet64 -Descending | Select-Object -First $n | ForEach-Object {
      $cpu=0; try { $cpu=[math]::Round($_.CPU,1) } catch {}
      [pscustomobject]@{ pid=$_.Id; name=$_.ProcessName; cpu_s=$cpu; mem_mb=[math]::Round($_.WorkingSet64/1MB,1); title=[string]$_.MainWindowTitle }
    }
    ConvertTo-Json -Compress -Depth 3 -InputObject @($out)
  }
  'system' {
    $os=Get-CimInstance Win32_OperatingSystem
    $cpu=Get-CimInstance Win32_Processor | Select-Object -First 1
    $up=(Get-Date)-$os.LastBootUpTime
    "OS: $($os.Caption) $($os.Version) ($($os.OSArchitecture))"
    "Computer: $env:COMPUTERNAME  User: $env:USERNAME"
    "CPU: $($cpu.Name.Trim()) ($($cpu.NumberOfLogicalProcessors) threads)"
    "Memory: $([math]::Round(($os.TotalVisibleMemorySize-$os.FreePhysicalMemory)/1MB,1)) / $([math]::Round($os.TotalVisibleMemorySize/1MB,1)) GB used"
    "Uptime: $([int]$up.TotalDays)d $($up.Hours)h $($up.Minutes)m"
    "Locale: $((Get-Culture).Name)  Time zone: $((Get-TimeZone).Id)"
  }
  default { throw "unknown action $($env:KOR_DESK_ACTION)" }
}
