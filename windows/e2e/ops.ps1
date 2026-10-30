# 마운트된 APFS 드라이브에 Windows 파일 작업을 하고, 기대 해시를 expected.txt에 남긴다.
# 사용: ops.ps1 -Drive W -Work <작업 폴더>
param([Parameter(Mandatory)][string]$Drive, [Parameter(Mandatory)][string]$Work)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$R = "${Drive}:\"
New-Item -ItemType Directory -Force $Work | Out-Null
$rng = [Security.Cryptography.RandomNumberGenerator]::Create()
function MakeFile($path, $size) { $b = New-Object byte[] $size; $rng.GetBytes($b); [IO.File]::WriteAllBytes($path, $b) }
MakeFile "$Work\small.bin" 1234
MakeFile "$Work\medium.bin" (3MB + 17)
MakeFile "$Work\big.bin" (64MB + 5)
MakeFile "$Work\tmp.bin" 5000
New-Item -ItemType Directory "${R}사진 폴더\하위" -Force | Out-Null
Copy-Item "$Work\small.bin" "${R}사진 폴더\작은 파일.bin"
Copy-Item "$Work\medium.bin" "${R}사진 폴더\하위\medium.bin"
Copy-Item "$Work\big.bin" "${R}big.bin"
[IO.File]::WriteAllText("${R}메모.txt", "안녕하세요 APFS")
Rename-Item "${R}메모.txt" "메모-이름바꿈.txt"
Copy-Item "$Work\tmp.bin" "${R}지울파일.bin"; Remove-Item "${R}지울파일.bin"
Copy-Item "$Work\small.bin" "${R}사진 폴더\하위\medium.bin" -Force   # 덮어쓰기
Move-Item "${R}big.bin" "${R}사진 폴더\big-moved.bin"                  # 이동
$h = { param($p) (Get-FileHash $p -Algorithm SHA256).Hash.ToLower() }
$expected = @(
  "$(& $h "$Work\small.bin") 사진 폴더/작은 파일.bin",
  "$(& $h "$Work\small.bin") 사진 폴더/하위/medium.bin",
  "$(& $h "$Work\big.bin") 사진 폴더/big-moved.bin",
  "absent 지울파일.bin",
  "absent big.bin"
)
# 마운트를 통해 읽은 해시도 같아야 한다
foreach ($line in $expected) {
  $hash, $rel = $line -split ' ', 2
  $p = $R + $rel.Replace('/', '\')
  if ($hash -eq 'absent') { if (Test-Path $p) { throw "should be gone: $rel" } }
  elseif ((& $h $p) -ne $hash) { throw "hash mismatch through the mount: $rel" }
}
if ([IO.File]::ReadAllText("${R}메모-이름바꿈.txt") -ne "안녕하세요 APFS") { throw "memo text mismatch" }
[IO.File]::WriteAllLines("$Work\expected.txt", [string[]]($expected + "text 메모-이름바꿈.txt 안녕하세요 APFS"))
"ops ok"
