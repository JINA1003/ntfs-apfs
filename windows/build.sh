#!/usr/bin/env bash
# WSL/Linux에서 Windows용 winapfs.exe를 만든다 (x86_64-pc-windows-gnu, mingw).
# 아이콘·매니페스트·버전 정보(crates/winapfs/res)를 windres로 먼저 컴파일해 링크에 넘긴다.
# 사용: ./build.sh [cargo 인자...]   예) ./build.sh build --release,  ./build.sh test --release
set -euo pipefail
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/build/apfsapp}"
obj="$CARGO_TARGET_DIR/app-res.o"
mkdir -p "$CARGO_TARGET_DIR"
(cd crates/winapfs/res && x86_64-w64-mingw32-windres --codepage 65001 -O coff -i app.rc -o "$obj")
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C link-arg=$obj"
cargo "${@:-build}"
