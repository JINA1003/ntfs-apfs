# Mac 앱 계획: NTFS 읽기·쓰기 (.dmg)

Mac에서 이 저장소를 연 사람(또는 Claude Code 세션)이 이 문서 하나로 바로 이어갈 수 있게 쓴 인계 문서.
Windows 쪽(`windows/`, `dist/APFSDrive.exe`)은 완료됐다. 남은 것은 Mac 앱이다.

표기: [확인됨] 출처 문서·실행으로 확인 / [추정] 판단·추론 / [확인 필요] Mac에서 직접 확인할 것.

## 무엇을 만드나

- **사용 흐름**: Windows(NTFS) SSD를 Mac에 꽂는다 → 앱을 켠다 → [마운트] → Finder에서 파일을 읽고 쓴다.
- **결과물**: 내 Mac에 설치하는 `.dmg` 하나. 혼자 쓰는 용도 (판매·배포 아님).
- **환경**: Apple Silicon, 최신 macOS (26 이상).
- **지킬 것**
  - 복구 모드로 보안을 낮추지 않는다 (커널 확장·kext 금지).
  - 실제 SSD보다 **이미지·RAM 디스크로 먼저** 쓰기를 검증한다. 데이터가 날아가면 실패다.
- Windows 앱과 같은 원칙: **파일 시스템 엔진은 검증된 오픈소스를 쓰고, 앱과 마운트 흐름은 직접 만든다.**

## 결정과 이유

| 결정 | 이유 |
|---|---|
| Swift + Xcode, Apple **FSKit** 파일 시스템 확장 | 커널 확장 없이 사용자 공간에서 동작. 보안 낮추기 불필요. [확인됨: macFUSE 문서 — FSKit은 macOS 15.4+] |
| 엔진: **libntfs-3g** (정적 링크) | 오래 검증된 NTFS 쓰기 엔진. FUSE 없이 라이브러리만 빌드 가능 (`--disable-ntfs-3g --disable-plugins --disable-crypto`). [확인됨: ntfs4mac README] |
| macFUSE + ntfs-3g **안 씀** | kext 방식은 Apple Silicon에서 복구 모드로 보안을 낮춰야 함. macFUSE의 FSKit 방식은 ntfs-3g와 함께 macOS 26에서 마운트 실패 버그 (macfuse/macfuse#1192). [확인됨] |
| Python 공유 UI **안 씀** | 처음엔 Windows와 Python으로 화면을 공유하려 했으나, FSKit 확장은 Swift로만 만들 수 있어 Mac은 Swift 네이티브로 결정. 사용자는 Swift·Xcode 경험이 있다. |

## 출발점: 참고 프로젝트

둘 다 FSKit + libntfs-3g. **MacNTFS를 바탕으로 가져와 고쳐 쓰는 것을 권한다** [추정]: 요구사항(Apple Silicon,
macOS 26, DMG, 메뉴 막대에서 읽기 전용/읽기·쓰기 마운트)과 거의 그대로 맞고, 실제 Apple Silicon에서 시험됐다.
Windows 쪽에서 winapfs를 `windows/`에 가져온 것과 같은 방식으로 `mac/`에 가져오고, 가져온 커밋을
`mac/UPSTREAM.txt`에 적는다.

| | [MacNTFS](https://github.com/alvarolorentedev/mac-ntfs) | [ntfs4mac](https://github.com/vahidzekic/ntfs4mac) |
|---|---|---|
| 라이선스 | GPL-3.0 (ntfs-3g GPL-2.0+ 정적 링크) | GPL-2.0-or-later |
| 시험 환경 | macOS 26.6.2, 27.0.1, Apple Silicon | RAM 디스크, Intel Mac 하나 |
| 구조 | `MacNTFS.xcodeproj`, `MacNTFS`(앱), `NTFSModule`(FSKit), `bridge`, `scripts`, `tests/engine`, `docs/DEVELOPMENT.md` | XcodeGen `project.yml`, `NTFSExtension.appex`, C 브리지, `scripts/build-libntfs3g.sh` (NTFS-3G 2022.10.3, sha256 고정) |
| 화면 | 메뉴 막대: Mount Read-Only / Mount Read/Write / Unmount / Eject | 상태 표시 + 설정 열기 |
| 제한 (요약) | 심볼릭 링크·하드링크·확장 속성 없음, Windows 빠른 시작으로 dirty한 볼륨은 읽기 전용 | 소유권 매핑 없음, xattr·ADS 없음, 볼륨당 직렬 처리 |

[확인됨: 두 저장소 README, 2026-10-30 기준] ntfs4mac README에 FSKit 개발 세부(서명·활성화·마운트 명령)가 자세하므로
MacNTFS를 쓰더라도 함께 참고한다.

**라이선스 결과**: `mac/`는 GPL이 된다. 이 저장소는 공개 저장소이므로 소스가 함께 있어 조건을 만족한다 [추정].
`windows/`(MIT/Apache)와 섞지 말고 폴더를 분리한 채 둔다.

## 먼저 확인할 것 (막히면 여기서 막힌다)

1. **[확인 필요] Apple Developer Program(유료, 연 99달러) 필요 여부.** ntfs4mac README: FSKit 확장은
   `com.apple.developer.fskit.fsmodule` 권한(entitlement)과 App Sandbox가 필요하고, **유료 팀이 필요하며
   ad-hoc 서명은 안 되고, 무료 Personal Team은 미확인**. → 가장 먼저 무료 Apple ID로 서명해 확장이 켜지는지 본다.
   안 되면 유료 가입은 사용자 결정 사항 (비용이 드므로 사용자에게 묻는다).
2. **확장 켜기** (사용자마다, 재설치마다): 시스템 설정 → 일반 → 로그인 항목 및 확장 → 확장 → **카테고리별** →
   **파일 시스템 확장** 옆 ⓘ → 켜기. macOS 26에서 "앱별" 스위치는 서드파티 모듈에 동작 안 함. 등록 확인:
   `pluginkit -mAvvv -p com.apple.fskit.fsmodule` [확인됨: ntfs4mac README]
3. **macOS 기본 NTFS 드라이버와 충돌**: macOS가 자체 읽기 전용 `ntfs.fs`로 먼저 붙일 수 있다. 짧은 이름을
   `ntfs`가 아닌 고유 이름으로 하고, 읽기 전용으로 붙었으면 해제 후 이 모듈로 다시 마운트. [확인됨: ntfs4mac README]
4. 물리 디스크에서 `mount -F`가 `EACCES`로 실패할 수 있음 → `diskutil mount` 사용 [확인됨: ntfs4mac README].

## 작업 순서

1. 확인 1번 (서명). 이게 안 되면 나머지가 의미 없다.
2. MacNTFS를 `mac/`로 가져와 그대로 빌드·설치 → 아래 "검증"의 테스트 이미지를 읽기 전용으로 마운트해 본다.
3. 읽기·쓰기 마운트 → 파일 작업 → 해제 → 독립 검사기로 대조 (아래). 이것을 `scripts/check.sh` Darwin 칸에 넣는다.
4. 앱 화면 손보기 (한국어, Windows 앱과 비슷한 흐름: 목록 → 읽기 전용/읽기·쓰기 마운트 → 해제 → Finder에서 열기).
5. `.dmg` 만들기 (`hdiutil create`). 사용자 Mac에 설치해 실제 SSD로 **읽기 전용부터** 확인.

## 검증 (`scripts/check.sh`의 Darwin 칸에 넣을 것)

`scripts/check.sh`가 이 프로젝트의 유일한 검증 명령이다. 지금 Darwin 칸은 공통 Python 테스트만 돈다. `mac/`을
만들면 다음을 채운다 — Windows smoke(`windows/check.sh`)와 같은 모양으로 [추정]:

1. 빌드: `xcodebuild` (앱 + 확장), 단위 테스트.
2. smoke: `tests/data/ntfs.bin.gz` (10MB NTFS 이미지, 이미 저장소에 있음)를 풀어 `hdiutil attach -nomount`로
   블록 장치로 붙임 → 이 앱의 모듈로 읽기·쓰기 마운트 → 만들기·복사·덮어쓰기·이름 바꾸기·이동·삭제 (한글 이름
   포함) → 해제·분리 → **libntfs-3g와 별개 구현인 `dissect.ntfs`**(`.venv`에 이미 있음)로 이미지를 직접 읽어
   해시 대조. 쓰기 대조 스크립트는 `windows/e2e/verify_apfs.py`를 본떠 `verify_ntfs.py`로 만든다.
3. 가능하면 Windows에서도 한 번 열어 본다 (Windows `chkdsk`가 가장 엄격한 NTFS 검사기다).

테스트 통과와 앱이 실제로 동작하는 것은 다른 명제다. smoke는 실제 빌드한 앱과 모듈로 마운트해야 한다.

## 이 저장소의 나머지 (참고)

- `windows/`: Windows APFS 앱. WSL에서 `windows/build.sh`로 빌드, `windows/check.sh`로 검증. Mac에서는 안 돈다.
- `diskread.py`, `tests/`: Python 읽기 전용 도구와 테스트 (Mac에서도 돈다:
  `python3 -m venv .venv && .venv/bin/pip install -r requirements.txt && scripts/check.sh`).
- 커밋 규칙: 작성자 `JINA <jinayang1003@gmail.com>`, Claude Co-Authored-By 줄 넣지 않음.
