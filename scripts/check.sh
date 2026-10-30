#!/usr/bin/env bash
# 이 프로젝트의 유일한 검증 명령. 어디서 돌든 이것 하나만 실행한다.
#   공통: Python 프로토타입(diskread) 단위 테스트
#   WSL(Windows): windows/check.sh — Windows 앱 빌드·Rust 테스트·실제 마운트 smoke
#   macOS: Mac 앱 검증 — mac/ 를 만들면서 아래 Darwin 칸에 채운다 (docs/mac-plan.md의 "검증")
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

[ -x .venv/bin/python ] || { echo "먼저: python3 -m venv .venv && .venv/bin/pip install -r requirements.txt"; exit 1; }
echo "== unit tests (Python)"
.venv/bin/python -m pytest -q tests 2>&1 | tail -1

case "$(uname -s)" in
  Linux)
    if grep -qi microsoft /proc/version 2>/dev/null; then
      windows/check.sh
    else
      echo "== Windows 앱 검증은 WSL에서만 돈다 (powershell.exe·WinFsp 필요): 건너뜀"
    fi
    ;;
  Darwin)
    if [ -d mac ]; then
      echo "mac/ 이 생겼으면 여기에 Mac 앱 빌드·테스트·smoke를 넣을 것 (docs/mac-plan.md)"; exit 1
    fi
    echo "== Mac 앱(mac/)이 아직 없음: 공통 테스트만 확인"
    ;;
esac
echo "== ALL CHECKS PASSED"
