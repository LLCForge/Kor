$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
trap { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }
[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class KorUia {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, int d, UIntPtr e);
  public static void Dpi() { try { SetProcessDpiAwarenessContext((IntPtr)(-4)); } catch {} }
}
'@
[KorUia]::Dpi()
# Inputs (environment only): KOR_UIA_ACTION, KOR_UIA_HWND, KOR_UIA_MAX, KOR_UIA_TEXT, KOR_UIA_RID, KOR_UIA_VERB, KOR_UIA_VALUE
$A=[System.Windows.Automation.AutomationElement]
$h=[IntPtr]::Zero
if($env:KOR_UIA_HWND){ $h=[IntPtr][int64]$env:KOR_UIA_HWND } else { $h=[KorUia]::GetForegroundWindow() }
if($h -eq [IntPtr]::Zero -or -not [KorUia]::IsWindow($h)){ throw 'No such window; call list_windows for a current hwnd.' }
$root=$A::FromHandle($h)

if($env:KOR_UIA_ACTION -eq 'snapshot'){
  $ct=[System.Windows.Automation.ControlType]
  $types=@($ct::Button,$ct::Hyperlink,$ct::Edit,$ct::CheckBox,$ct::RadioButton,$ct::ComboBox,$ct::MenuItem,$ct::TabItem,$ct::ListItem,$ct::TreeItem,$ct::SplitButton,$ct::Slider,$ct::Spinner,$ct::DataItem,$ct::Document)
  if($env:KOR_UIA_TEXT -eq '1'){ $types+=@($ct::Text) }
  $conds=foreach($t in $types){ New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty,$t) }
  $cond=New-Object System.Windows.Automation.OrCondition(,[System.Windows.Automation.Condition[]]$conds)
  $max=[int]$env:KOR_UIA_MAX
  $out=New-Object System.Collections.Generic.List[object]
  foreach($e in $root.FindAll([System.Windows.Automation.TreeScope]::Descendants,$cond)){
    if($out.Count -ge $max){ break }
    try{
      $c=$e.Current
      if($c.IsOffscreen){ continue }
      $r=$c.BoundingRectangle
      if([double]::IsInfinity($r.Width) -or $r.Width -le 0 -or $r.Height -le 0){ continue }
      $value=''
      $p=$null
      if(-not $c.IsPassword -and $e.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern,[ref]$p)){
        $value=[string]([System.Windows.Automation.ValuePattern]$p).Current.Value
        if($value.Length -gt 120){ $value=$value.Substring(0,120)+'…' }
      }
      $toggle=''
      if($e.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern,[ref]$p)){
        $toggle=[string]([System.Windows.Automation.TogglePattern]$p).Current.ToggleState
      }
      $out.Add([pscustomobject]@{
        rid=(($e.GetRuntimeId()) -join '.')
        type=[string]$c.ControlType.ProgrammaticName.Replace('ControlType.','')
        name=[string]$c.Name
        value=$value
        toggle=$toggle
        password=[bool]$c.IsPassword
        enabled=[bool]$c.IsEnabled
        x=[int]$r.X; y=[int]$r.Y; w=[int]$r.Width; h=[int]$r.Height
      })
    }catch{ continue }
  }
  $title=[string]$root.Current.Name
  ConvertTo-Json -Compress -Depth 4 -InputObject ([pscustomobject]@{ hwnd=[int64]$h; title=$title; items=@($out.ToArray()) })
  exit 0
}

# invoke: find the element by its runtime id, so a changed tree cannot redirect the action.
$rid=[int[]]($env:KOR_UIA_RID -split '\.' | ForEach-Object { [int]$_ })
$cond=New-Object System.Windows.Automation.PropertyCondition($A::RuntimeIdProperty,$rid)
$el=$root.FindFirst([System.Windows.Automation.TreeScope]::Descendants,$cond)
if($null -eq $el){ throw 'The element is gone; call uia_snapshot again.' }
$c=$el.Current
if(-not $c.IsEnabled){ throw "The element is disabled: $($c.Name)" }
$verb=$env:KOR_UIA_VERB
$p=$null
function Try-Pattern($pattern){ $script:p=$null; return $el.TryGetCurrentPattern($pattern,[ref]$script:p) }
switch($verb){
  'set_value' {
    if($c.IsPassword){ throw 'Refusing to fill a password field; the user types passwords.' }
    if(-not (Try-Pattern ([System.Windows.Automation.ValuePattern]::Pattern))){ throw 'This element has no editable value.' }
    ([System.Windows.Automation.ValuePattern]$p).SetValue([string]$env:KOR_UIA_VALUE); 'set'; break
  }
  'focus' { $el.SetFocus(); 'focused'; break }
  'toggle' {
    if(-not (Try-Pattern ([System.Windows.Automation.TogglePattern]::Pattern))){ throw 'This element cannot be toggled.' }
    ([System.Windows.Automation.TogglePattern]$p).Toggle(); 'toggled'; break
  }
  'select' {
    if(-not (Try-Pattern ([System.Windows.Automation.SelectionItemPattern]::Pattern))){ throw 'This element cannot be selected.' }
    ([System.Windows.Automation.SelectionItemPattern]$p).Select(); 'selected'; break
  }
  'expand' {
    if(-not (Try-Pattern ([System.Windows.Automation.ExpandCollapsePattern]::Pattern))){ throw 'This element cannot be expanded.' }
    ([System.Windows.Automation.ExpandCollapsePattern]$p).Expand(); 'expanded'; break
  }
  'collapse' {
    if(-not (Try-Pattern ([System.Windows.Automation.ExpandCollapsePattern]::Pattern))){ throw 'This element cannot be collapsed.' }
    ([System.Windows.Automation.ExpandCollapsePattern]$p).Collapse(); 'collapsed'; break
  }
  default {
    if(Try-Pattern ([System.Windows.Automation.InvokePattern]::Pattern)){ ([System.Windows.Automation.InvokePattern]$p).Invoke(); 'invoked'; break }
    if(Try-Pattern ([System.Windows.Automation.TogglePattern]::Pattern)){ ([System.Windows.Automation.TogglePattern]$p).Toggle(); 'toggled'; break }
    if(Try-Pattern ([System.Windows.Automation.SelectionItemPattern]::Pattern)){ ([System.Windows.Automation.SelectionItemPattern]$p).Select(); 'selected'; break }
    if(Try-Pattern ([System.Windows.Automation.ExpandCollapsePattern]::Pattern)){ ([System.Windows.Automation.ExpandCollapsePattern]$p).Expand(); 'expanded'; break }
    # No pattern: a real click at the element's centre (physical pixels, DPI aware).
    $r=$c.BoundingRectangle
    [void][KorUia]::SetCursorPos([int]($r.X+$r.Width/2),[int]($r.Y+$r.Height/2))
    [KorUia]::mouse_event(0x0002,0,0,0,[UIntPtr]::Zero)
    [KorUia]::mouse_event(0x0004,0,0,0,[UIntPtr]::Zero)
    'clicked'
  }
}
