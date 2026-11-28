$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$hwnd = [intptr][long]$args[0]
$liveIds = @{ 0 = 'Off'; 1 = 'Polite'; 2 = 'Assertive' }
$el = $null; $all = $null
for ($try = 1; $try -le 20; $try++) {
  $el = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
  $all = $el.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
  if ($all.Count -gt 0) { break }
  Start-Sleep -Milliseconds 500
}
Write-Output ("WINDOW name=[" + $el.Current.Name + "] type=" + ($el.Current.ControlType.ProgrammaticName -replace 'ControlType\.',''))
Write-Output ("DESCENDANTS=" + $all.Count)
Write-Output "------"
$i=0
foreach ($x in $all) {
  $i++
  $c = $x.Current
  $line = "{0,3}. {1,-12} name=[{2}]" -f $i, ($c.ControlType.ProgrammaticName -replace 'ControlType\.',''), $c.Name
  if ($c.HelpText) { $line += (" help=[" + $c.HelpText + "]") }
  try { $vp = $x.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern); if ($vp) { $line += (" value=[" + $vp.Current.Value + "]") } } catch {}
  try { $tp = $x.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern); if ($tp) { $line += (" toggle=" + $tp.Current.ToggleState) } } catch {}
  try { $rv = $x.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern); if ($rv) { $line += (" value=" + $rv.Current.Value + " min=" + $rv.Current.Minimum + " max=" + $rv.Current.Maximum) } } catch {}
  try { $ip = $x.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern); if ($ip) { $line += " invoke" } } catch {}
  try {
    $liveObj = $x.GetCurrentPropertyValue([System.Windows.Automation.AutomationElement]::LiveSettingProperty)
    if ($null -ne $liveObj) { $n = [int]$liveObj; if ($n -ne 0) { $line += (" LIVE=" + $liveIds[$n]) } }
  } catch {}
  Write-Output $line
}
Write-Output "------"
Write-Output ("END tries=" + $try)