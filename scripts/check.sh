#!/usr/bin/env bash
# 이 프로젝트의 유일한 검증 명령.
#   1) 빌드: Windows 앱(dist/APFSDrive.exe)
#   2) 단위 테스트: Rust(Windows exe로 실행) + Python 프로토타입(diskread)
#   3) smoke: 실제 exe로 macOS가 만든 1GiB APFS 이미지를 읽기/쓰기 마운트하고 Windows 파일 API로
#      만들기·복사·덮어쓰기·이름 바꾸기·이동·삭제 → 해제(빈 공간 맵 검사) →
#      winapfs와 별개 구현(dissect.apfs)으로 이미지를 직접 읽어 결과를 대조
# 관리자 권한이 필요한 GUI 종단 테스트는 windows/e2e/gui_e2e.sh (UAC 클릭 필요).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

echo "== build"
windows/build.sh build --release 2>&1 | tail -1
exe_built="$HOME/build/apfsapp/x86_64-pc-windows-gnu/release/winapfs.exe"
mkdir -p dist && cp "$exe_built" dist/APFSDrive.exe
exe="$root/dist/APFSDrive.exe"

echo "== unit tests"
windows/build.sh test --release 2>&1 | grep -E "^test result|FAILED|panicked" | tee /dev/stderr | grep -qv FAILED
.venv/bin/python -m pytest -q tests 2>&1 | tail -1

echo "== smoke"
work="$root/windows/e2e/.work-smoke"; rm -rf "$work"; mkdir -p "$work"
trap 'rm -rf "$work"' EXIT
gzip -dc windows/e2e/data/large-apfs.img.gz > "$work/rw.img"
W="$(wslpath -w "$work")"
"$exe" info "$W\\rw.img" | grep -q '"Large"'
letter=$(powershell.exe -NoProfile -Command "[char[]](87..90) | Where-Object { -not (Test-Path \"\${_}:\\\") } | Select-Object -First 1" | tr -d '\r\n')
[ -n "$letter" ] || { echo "free drive letter not found"; exit 1; }
("$exe" mount "$W\\rw.img" "$letter:" --write > "$work/mount.log" 2>&1; echo "exit=$?" >> "$work/mount.log") &
for _ in $(seq 1 60); do grep -q "^mounted" "$work/mount.log" 2>/dev/null && break; grep -q "^exit=" "$work/mount.log" 2>/dev/null && break; sleep 1; done
grep -q "READ-WRITE" "$work/mount.log" || { cat "$work/mount.log"; exit 1; }
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$(wslpath -w windows/e2e/ops.ps1)" -Drive "$letter" -Work "$W\\ops" || { "$exe" unmount "$letter:"; exit 1; }
"$exe" unmount "$letter:" >/dev/null
for _ in $(seq 1 120); do grep -q "^exit=" "$work/mount.log" && break; sleep 1; done
grep -q "^exit=0" "$work/mount.log" && grep -q "result     consistent" "$work/mount.log" || { cat "$work/mount.log"; exit 1; }
echo "post-write space check: consistent"
.venv/bin/python windows/e2e/verify_apfs.py "$work/rw.img" "$work/ops/expected.txt"
echo "== ALL CHECKS PASSED"
