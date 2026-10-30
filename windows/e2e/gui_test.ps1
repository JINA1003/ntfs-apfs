# 관리자 권한으로 실행되는 GUI 종단 테스트 (gui_e2e.sh가 띄운다).
# VHD 연결 → 앱 실행 → [읽기 전용 마운트] → (일반 권한 쪽 확인) → [읽기/쓰기 마운트] → 확인 대화상자 [예]
# → (일반 권한 쪽 파일 작업) → [마운트 해제] → 창 닫기 → VHD 분리. 단계마다 창을 PrintWindow로 캡처한다.
param([Parameter(Mandatory)][string]$Exe, [Parameter(Mandatory)][string]$Vhd, [Parameter(Mandatory)][string]$Work)
$ErrorActionPreference = 'Stop'
$log = "$Work\gui_test.log"; "" | Set-Content $log -Encoding UTF8
function Log($m) { "$(Get-Date -Format HH:mm:ss) $m" | Add-Content $log -Encoding UTF8 }
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices;
public class U {
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
 [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
 [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, int m, IntPtr w, IntPtr l);
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, int m, IntPtr w, StringBuilder l);
 [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, int m, IntPtr w, IntPtr l);
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
 [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, int flags);
 [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
 public struct R { public int L, T, Rt, B; }
 public static string Line(IntPtr lb, int i) { int n = (int)SendMessage(lb, 0x018A, (IntPtr)i, IntPtr.Zero); if (n < 0) return ""; var sb = new StringBuilder(n + 1); SendMessage(lb, 0x0189, (IntPtr)i, sb); return sb.ToString(); }
}
"@
[U]::SetProcessDPIAware() | Out-Null
function Shot($h, $name) {
  $r = New-Object U+R; [U]::GetWindowRect($h, [ref]$r) | Out-Null
  $b = New-Object System.Drawing.Bitmap ($r.Rt - $r.L), ($r.B - $r.T)
  $g = [System.Drawing.Graphics]::FromImage($b); $hdc = $g.GetHdc()
  [U]::PrintWindow($h, $hdc, 2) | Out-Null   # PW_RENDERFULLCONTENT: 가려져 있어도 창 내용 그대로
  $g.ReleaseHdc($hdc); $b.Save("$Work\$name.png"); Log "shot $name"
}
function WaitFor($what, $sec, [scriptblock]$cond) {
  $t = [Diagnostics.Stopwatch]::StartNew()
  while (-not (& $cond)) { if ($t.Elapsed.TotalSeconds -gt $sec) { throw "timeout: $what" }; Start-Sleep -Milliseconds 300 }
  Log "ok: $what ($([int]$t.Elapsed.TotalSeconds)s)"
}
function Click($h, $id) { [U]::PostMessage([U]::GetDlgItem($h, $id), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null }
function Lines($h) { $lb = [U]::GetDlgItem($h, 100); $n = [int][U]::SendMessage($lb, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero); (0..($n-1) | ForEach-Object { [U]::Line($lb, $_) }) -join "`n" }
function Handshake($name, $sec) { "x" | Set-Content "$Work\$name.ready"; Log "waiting for $name.go"; WaitFor "$name.go" $sec { Test-Path "$Work\$name.go" } }
$h = [IntPtr]::Zero; $p = $null
try {
  $disk = (Mount-DiskImage -ImagePath $Vhd -PassThru | Get-Disk).Number; Log "VHD attached as disk $disk"
  $p = Start-Process -FilePath $Exe -PassThru
  WaitFor "window" 20 { $script:h = [U]::FindWindow("winapfs.gui", [NullString]::Value); $script:h -ne [IntPtr]::Zero }
  WaitFor "volume listed" 30 { (Lines $h) -match "디스크 $disk .*Large" }
  Log "list: $(Lines $h)"; Shot $h "s1_listed"
  Click $h 101
  WaitFor "read-only mounted" 60 { (Lines $h) -match ": 읽기 전용$" }
  $letter = [regex]::Match((Lines $h), '([M-Z]): 읽기 전용').Groups[1].Value
  Log "list: $(Lines $h)"; Shot $h "s2_readonly"; $letter | Set-Content "$Work\letter.txt"
  Handshake "ro" 180
  Click $h 102
  $dlg = [IntPtr]::Zero
  WaitFor "confirm dialog" 10 { $script:dlg = [U]::FindWindow("#32770", "APFS 드라이브"); $script:dlg -ne [IntPtr]::Zero }
  Shot $dlg "s3_confirm"
  [U]::PostMessage($dlg, 0x0111, [IntPtr]6, [IntPtr]::Zero) | Out-Null   # IDYES
  WaitFor "read-write mounted" 300 { (Lines $h) -match ": 읽기/쓰기$" }
  $letter = [regex]::Match((Lines $h), '([M-Z]): 읽기/쓰기').Groups[1].Value
  Log "list: $(Lines $h)"; Shot $h "s4_readwrite"; $letter | Set-Content "$Work\letter.txt"
  Handshake "rw" 300
  Click $h 103
  WaitFor "unmounted" 300 { (Lines $h) -match "마운트 안 됨$" }
  Log "list: $(Lines $h)"; Shot $h "s5_unmounted"
  [U]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null   # WM_CLOSE
  WaitFor "app exited" 30 { $p.HasExited }
  Log "PASS"
} catch { Log "FAIL: $_"; if ($h -ne [IntPtr]::Zero) { try { Shot $h "fail" } catch {} } }
finally {
  if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
  Start-Sleep 2; Dismount-DiskImage -ImagePath $Vhd -ErrorAction SilentlyContinue | Out-Null; Log "VHD detached"
}
