#!/usr/bin/env bash
# Windows 앱 검증 (WSL에서 실행). scripts/check.sh가 WSL일 때 부른다.
#   1) 빌드: dist/APFSDrive.exe
#   2) 단위 테스트: Rust (Windows exe로 실행)
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

echo "== unit tests (Rust)"
windows/build.sh test --release 2>&1 | grep -E "^test result|FAILED|panicked"

echo "== smoke (Windows)"
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

echo "== snapshot smoke (Windows)"
# macOS가 만든 스냅샷 볼륨 이미지 (dissect.apfs 테스트 데이터, AGPL이라 저장소에 넣지 않고 받아 둔다).
cache="$root/windows/e2e/.cache"; snap_gz="$cache/snapshot.bin.gz"
if [ ! -f "$snap_gz" ]; then
  mkdir -p "$cache"
  .venv/bin/pip download -q --disable-pip-version-check --no-deps --no-binary :all: dissect.apfs==1.1 -d "$cache"
  tar -xzf "$cache/dissect_apfs-1.1.tar.gz" -C "$cache" dissect_apfs-1.1/tests/_data/snapshot.bin.gz
  mv "$cache/dissect_apfs-1.1/tests/_data/snapshot.bin.gz" "$snap_gz"; rm -rf "$cache/dissect_apfs-1.1" "$cache"/*.tar.gz
fi
gzip -dc "$snap_gz" > "$work/snap.img"
lin="$(mktemp -d)"; trap 'rm -rf "$work" "$lin"' EXIT
cp "$work/snap.img" "$lin/before.img"
S="$W\\snap.img"
head -c 3000000 /dev/urandom > "$work/big.bin"; printf 'replaced\n' > "$work/small.txt"
"$exe" dev-put "$S" "$W\\big.bin" /big.bin >/dev/null
"$exe" dev-mkdir "$S" /dir >/dev/null
"$exe" dev-rm "$S" /.fseventsd/000000000006eba6 >/dev/null     # 스냅샷과 공유하는 파일
"$exe" dev-put "$S" "$W\\small.txt" /.fseventsd/000000000006eba7 >/dev/null
"$exe" dev-mv "$S" /.fseventsd/000000000006ebd3 /dir/moved >/dev/null
"$exe" dev-space-check "$S" | grep -q "result     consistent" || { "$exe" dev-space-check "$S"; exit 1; }
echo "post-write space check: consistent"
cp "$work/snap.img" "$lin/after.img"
.venv/bin/python windows/e2e/snapshot_hashes.py "$lin/before.img" 16 > "$lin/before.txt"
.venv/bin/python windows/e2e/snapshot_hashes.py "$lin/after.img" 16 > "$lin/after.txt"
cmp -s "$lin/before.txt" "$lin/after.txt" || { diff "$lin/before.txt" "$lin/after.txt" | head; exit 1; }
echo "snapshots unchanged: $(wc -l < "$lin/after.txt") files (dissect.apfs)"
printf '%s /big.bin\nabsent /.fseventsd/000000000006eba6\nabsent /.fseventsd/000000000006ebd3\n' "$(sha256sum "$work/big.bin" | cut -d' ' -f1)" > "$lin/expected.txt"
.venv/bin/python windows/e2e/verify_apfs.py "$lin/after.img" "$lin/expected.txt"
if docker image inspect apfsprogs >/dev/null 2>&1; then
  docker run --rm -v "$lin:/work" apfsprogs apfsck -cw /work/after.img && echo "apfsck: clean"
else
  echo "apfsck: SKIPPED (docker build -t apfsprogs windows/tools/apfsprogs)"
fi
