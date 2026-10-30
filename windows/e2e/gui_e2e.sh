#!/usr/bin/env bash
# GUI 종단 테스트. 관리자 권한(UAC)을 한 번 묻는다: 사람이 [예]를 눌러야 한다.
# 앱은 관리자 권한으로 돌고, 이 스크립트는 일반 권한에서 드라이브가 보이는지·쓸 수 있는지 확인한다.
# 사용: windows/e2e/gui_e2e.sh <APFSDrive.exe 경로>
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
exe="$(wslpath -w "$1")"
work="$root/windows/e2e/.work-gui"; rm -rf "$work"; mkdir -p "$work"
W="$(wslpath -w "$work")"
gzip -dc "$here/data/large-apfs.img.gz" > "$work/apfs.img"
python3 "$here/make_vhd.py" "$work/apfs.img" "$work/test.vhd" >/dev/null && rm "$work/apfs.img"
cp "$here/gui_test.ps1" "$work/"
powershell.exe -NoProfile -Command "\$ErrorActionPreference='Stop'; Start-Process powershell.exe -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','$W\\gui_test.ps1','-Exe','$exe','-Vhd','$W\\test.vhd','-Work','$W'" >/dev/null || { echo '권한 요청이 취소됐습니다'; exit 1; }
echo "UAC 확인 창에서 [예]를 누르세요…"
wait_for() { for _ in $(seq 1 300); do [ -f "$work/$1" ] && return 0; grep -q FAIL "$work/gui_test.log" 2>/dev/null && return 1; sleep 2; done; return 1; }
ps() { powershell.exe -NoProfile -ExecutionPolicy Bypass "$@"; }
wait_for ro.ready
L=$(tr -d '\r\n\357\273\277' < "$work/letter.txt")
ps -Command "if (-not (Test-Path ${L}:\\yomomma.bin)) { throw 'not visible' }; try { [IO.File]::WriteAllText('${L}:\\x.txt','x'); throw 'read-only drive accepted a write' } catch [UnauthorizedAccessException] { 'ro ok: visible to a normal user, writes refused' }"
touch "$work/ro.go"
wait_for rw.ready
L=$(tr -d '\r\n\357\273\277' < "$work/letter.txt")
ps -File "$(wslpath -w "$here/ops.ps1")" -Drive "$L" -Work "$W\\ops"
touch "$work/rw.go"
for _ in $(seq 1 300); do grep -q "VHD detached" "$work/gui_test.log" 2>/dev/null && break; sleep 2; done
cat "$work/gui_test.log"
grep -q PASS "$work/gui_test.log"
"$root/.venv/bin/python" "$here/verify_apfs.py" "$work/test.vhd" "$work/ops/expected.txt" $((2048 * 512))
echo "GUI e2e PASS (스크린샷: $work/*.png)"
