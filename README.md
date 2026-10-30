# NTFS + APFS

Windows와 Mac에서 서로의 디스크를 읽고 쓰기 위한 개인용 도구 모음.

| 폴더 | 내용 | 상태 |
|---|---|---|
| `windows/` | **APFS 드라이브** (Windows 앱): Mac(APFS) SSD를 탐색기 드라이브로 붙여 읽고 쓴다 | 완료 (`dist/APFSDrive.exe`) |
| `diskread.py` | NTFS·APFS 이미지/디스크 읽기 전용 CLI (Python 프로토타입) | 완료 |
| Mac 앱 (NTFS 쓰기, `.dmg`) | Swift + FSKit + libntfs-3g로 계획 | 아직 |

## APFS 드라이브 (Windows)

### 쓰는 법

1. **WinFsp**가 없으면 먼저 설치한다 (앱의 [WinFsp 설치] 버튼이 설치 파일을 내려받는다. https://winfsp.dev).
2. Mac SSD를 꽂고 `APFSDrive.exe`를 더블클릭한다. Windows가 관리자 권한을 물으면 [예].
   (디스크를 직접 읽고 쓰려면 관리자 권한이 필요하다.)
3. 목록에서 볼륨을 고르고
   - **[읽기 전용 마운트]**: 안전하게 보기·복사
   - **[읽기/쓰기 마운트]**: 파일 복사·수정·삭제·이름 바꾸기 (베타, 붙이기 전에 드라이브 전체를 검사)
4. **[탐색기에서 열기]** 또는 탐색기에서 `N:` 같은 드라이브로 쓴다.
5. 뽑기 전에 **[마운트 해제]** 또는 앱 종료. 남은 변경을 저장하고 빈 공간 맵을 다시 검사한 뒤 해제된다.

설치 단계는 없다. 서비스 등록·시작 프로그램·PATH 변경 없이 앱이 떠 있는 동안만 마운트한다.

읽기/쓰기에서 아직 안 되는 것 (해당 볼륨은 읽기 전용으로 붙는다): 암호화(FileVault) 볼륨, 스냅샷이 있는 볼륨,
Fusion 드라이브, macOS 압축 파일·하드링크 파일 수정. **잃으면 안 되는 데이터는 백업한 뒤 쓴다.**

### 구조

- 엔진은 [winapfs](https://github.com/ZYRT3CH/winapfs) (MIT/Apache-2.0, Apple 공개 APFS 사양으로 작성,
  copy-on-write 쓰기)를 `windows/`에 가져와 고쳐 쓴다. 가져온 시점: `windows/UPSTREAM.txt`.
- 가상 드라이브는 WinFsp. 드라이브 문자는 `\\.\N:` (WinFsp 전역 드라이브) 형식이라 관리자 권한 앱이 만든
  드라이브가 일반 권한 탐색기에도 보인다.
- 원본 winapfs에서 바꾼 것
  - `gui.rs` (새 파일): Win32 창. 인자 없이 실행하면 뜨고, 관리자 권한이 아니면 권한을 얻어 다시 실행한다.
    마운트마다 작업 스레드 하나가 `serve_mount`를 돌린다.
  - `main.rs`: 창 앱(windows subsystem)으로 빌드. 인자가 있으면 원래 CLI 그대로 (터미널 콘솔에 붙어 출력).
    `cmd_mount`를 `serve_mount`(드라이브가 붙는 순간을 알리는 콜백)로 나눔. 쓰기 후 빈 공간 맵 검사가
    실패하면 오류로 돌려준다 (원래는 출력만).
  - `mountfs.rs`: 같은 프로세스가 같은 자리에 다시 마운트할 때 남아 있던 해제 신호를 지운다.
  - `disks.rs`: 디스크 제품명·용량 표시 (`describe`).
  - `service.rs`: `sc.exe`를 콘솔 창 없이 실행.
  - `res/`: 아이콘, 매니페스트(공용 컨트롤 v6, DPI 인식, asInvoker), 버전 정보.
- **왜 서비스 방식(winapfs 원래 방식)을 안 쓰나**: 서명 없는 exe가 서비스를 설치하는 동작을 Microsoft
  Defender가 `Behavior:Win32/Persistence.A!ml`로 잡아 격리했다 (개발 중 실제로 발생). 앱 안에서 마운트하는
  방식은 그 동작이 없다.

### 빌드 (WSL/Linux)

필요: Rust (`rustup target add x86_64-pc-windows-gnu`), mingw-w64 (`brew install mingw-w64` 등).

```bash
windows/build.sh build --release     # → ~/build/apfsapp/x86_64-pc-windows-gnu/release/winapfs.exe
```

### 검증

```bash
scripts/check.sh                     # 유일한 검증 명령: 빌드 → 단위 테스트 → smoke
windows/e2e/gui_e2e.sh dist/APFSDrive.exe   # GUI 종단 테스트 (UAC 클릭 필요)
```

- smoke: 실제 exe로 macOS가 만든 1GiB APFS 이미지를 읽기/쓰기 마운트 → Windows 파일 API로 만들기·복사·
  덮어쓰기·이름 바꾸기·이동·삭제 → 해제(빈 공간 맵 검사) → winapfs와 별개 구현인 `dissect.apfs`로 이미지를
  직접 읽어 해시 대조.
- GUI 종단: 그 이미지를 GPT 가상 디스크(VHD)로 Windows에 연결 → 앱 버튼으로 읽기 전용·읽기/쓰기 마운트·해제·
  종료. 그 사이 일반 권한 쪽에서 드라이브가 보이는지, 읽기 전용이 쓰기를 거부하는지, 파일 작업이 되는지 확인.

## diskread.py (Python 프로토타입)

```bash
python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
.venv/bin/python diskread.py parts <이미지 또는 /dev/sdX>
.venv/bin/python diskread.py ls    <source> [path] [-p N] [-v N]
.venv/bin/python diskread.py get   <source> <path> <dest> [-p N] [-v N]
```

읽기 전용. 파서는 Fox-IT `dissect.ntfs` / `dissect.apfs` (AGPL-3.0). 같은 라이브러리가 위 검증에서
독립 검사기로도 쓰인다.

## 라이선스

- `windows/`: MIT OR Apache-2.0 (winapfs). WinFsp - Windows File System Proxy, Copyright (C) Bill Zissimopoulos,
  https://github.com/winfsp/winfsp (GPLv3 FLOSS 예외; 앱 화면에 고지).
- `diskread.py`, 검증 스크립트: dissect(AGPL-3.0)를 쓴다. 개인용.
