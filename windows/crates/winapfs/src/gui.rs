//! 창(GUI): 인자 없이 실행(더블클릭)하면 뜨는 화면.
//!
//! 앱이 관리자 권한으로 실행되어 마운트를 직접 맡는다. 서비스 등록·Program Files
//! 복사·PATH 변경이 없으므로 설치 단계가 없고, 앱을 닫으면 마운트도 끝난다.
//! 드라이브 문자는 `\\.\M:` 형식(WinFsp 전역 드라이브)으로 만들어 권한 없이
//! 실행 중인 탐색기에서도 보인다.
//!
//! 화면은 Win32 기본 컨트롤만 쓴다. 디스크 조회와 마운트는 작업 스레드에서
//! 돌고, 결과는 PostMessageW로 창 스레드에 넘긴다. 마운트 하나당 스레드 하나가
//! `serve_mount`를 돌리며, [마운트 해제]는 `mountfs::request_stop`으로 그 스레드를 끝낸다.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;

use crate::winfsp::{wide, Api};
use crate::{automount, disks, mountfs, open_source, serve_mount, MountMode};

const TITLE: &str = "APFS 드라이브";
const WINFSP_MSI: &str = "https://github.com/winfsp/winfsp/releases/download/v2.1/winfsp-2.1.25156.msi";

const WM_APP_SCAN: u32 = 0x8001;
const WM_APP_READY: u32 = 0x8002;
const WM_APP_ENDED: u32 = 0x8003;
const ID_LIST: usize = 100;
const ID_MOUNT_RO: usize = 101;
const ID_MOUNT_RW: usize = 102;
const ID_UNMOUNT: usize = 103;
const ID_OPEN: usize = 104;
const ID_WINFSP: usize = 105;
const ID_REFRESH: usize = 106;
const TIMER_POLL: usize = 1;

/// 목록의 한 줄: 디스크 하나의 볼륨 하나, 또는 읽지 못한 디스크.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    disk: u32,
    /// 디스크 설명 (제품명, 용량)
    label: String,
    /// 볼륨 번호·이름·암호화 여부. 디스크를 읽지 못했으면 `Err(이유)`
    volume: Result<(usize, String, bool), String>,
}

/// 디스크들을 읽어 목록 줄을 만든다. 작업 스레드에서 돈다.
fn scan(disk_numbers: &[u32]) -> Vec<Entry> {
    let mut out = Vec::new();
    for &n in disk_numbers {
        let label = disks::describe(n);
        match open_source(&format!("disk{n}"), None).and_then(|(c, _)| Ok(c.volume_superblocks()?)) {
            Ok(vols) => {
                for (i, v) in vols.iter().enumerate() {
                    // Preboot·Recovery·VM 같은 macOS 내부용 볼륨은 사용자가 쓸 일이 없다
                    let internal = ["preboot", "recovery", "vm", "update", "xart", "hardware", "baseband", "installer", "prelogin"];
                    if !internal.contains(&v.role_name()) {
                        out.push(Entry { disk: n, label: label.clone(), volume: Ok((i, v.name.clone(), v.is_encrypted())) });
                    }
                }
            }
            Err(e) => out.push(Entry { disk: n, label, volume: Err(e.msg) }),
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Starting,
    Ready,
    Stopping,
}

/// 이 앱이 붙인 드라이브 하나.
struct Mount {
    disk: u32,
    vol: usize,
    letter: char,
    mode: MountMode,
    phase: Phase,
}

fn mount_point(letter: char) -> String {
    format!(r"\\.\{letter}:")
}

fn entry_line(e: &Entry, mount: Option<&Mount>) -> String {
    let disk = if e.label.is_empty() { format!("디스크 {}", e.disk) } else { format!("디스크 {} ({})", e.disk, e.label) };
    let (name, encrypted) = match &e.volume {
        Ok((_, name, enc)) => (name.as_str(), *enc),
        Err(why) => return format!("{disk}  —  읽지 못함: {why}"),
    };
    let state = match mount {
        None if encrypted => "암호화됨 · 지원 안 함".to_string(),
        None => "마운트 안 됨".to_string(),
        Some(m) => match (m.phase, m.mode) {
            (Phase::Starting, MountMode::ReadOnly) => format!("{}: 읽기 전용으로 붙이는 중…", m.letter),
            (Phase::Starting, _) => format!("{}: 드라이브 검사 중… (읽기/쓰기 준비)", m.letter),
            (Phase::Stopping, _) => format!("{}: 해제 중… (저장·검사)", m.letter),
            (Phase::Ready, MountMode::ReadOnly) => format!("{}: 읽기 전용", m.letter),
            (Phase::Ready, _) => format!("{}: 읽기/쓰기", m.letter),
        },
    };
    format!("{disk}  ·  \"{name}\"  —  {state}")
}

// ---- 창 ----

struct App {
    hwnd: Hwnd,
    status: Hwnd,
    list: Hwnd,
    message: Hwnd,
    buttons: Vec<(usize, Hwnd)>,
    winfsp: bool,
    /// 마지막으로 읽은 디스크 번호들 (바뀌면 다시 읽는다)
    disk_numbers: Vec<u32>,
    entries: Vec<Entry>,
    scanning: bool,
    mounts: Vec<Mount>,
    /// 선택한 줄 (디스크, 볼륨). 목록이 다시 그려져도 유지
    selected: Option<(u32, usize)>,
    /// 창을 닫는 중: 마운트가 모두 끝나면 창을 없앤다
    closing: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// 해제가 끝나면 다시 붙일 마운트 (드라이브 문자 → 디스크, 볼륨, 모드)
    static PENDING: RefCell<HashMap<char, (u32, usize, MountMode)>> = RefCell::new(HashMap::new());
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok()?.as_mut().map(f))
}

fn key(e: &Entry) -> Option<(u32, usize)> {
    e.volume.as_ref().ok().map(|v| (e.disk, v.0))
}

impl App {
    fn mount_of(&self, k: (u32, usize)) -> Option<&Mount> {
        self.mounts.iter().find(|m| (m.disk, m.vol) == k)
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.entries.iter().find(|e| key(e).is_some() && key(e) == self.selected)
    }

    fn render(&mut self) {
        let status = match (self.winfsp, self.entries.is_empty()) {
            (false, _) => "WinFsp가 없습니다 → [WinFsp 설치]를 눌러 받은 파일을 실행한 뒤 [새로고침]",
            (true, true) => "Mac(APFS) 디스크를 꽂으면 여기에 나타납니다.",
            (true, false) => "볼륨을 고르고 마운트하세요. 앱을 닫으면 마운트도 안전하게 해제됩니다.",
        };
        set_text(self.status, status);
        if self.selected_entry().is_none() {
            self.selected = self.entries.iter().find_map(key);
        }
        unsafe { SendMessageW(self.list, LB_RESETCONTENT, 0, 0) };
        for (i, e) in self.entries.iter().enumerate() {
            let line = wide(&entry_line(e, key(e).and_then(|k| self.mount_of(k))));
            unsafe {
                SendMessageW(self.list, LB_ADDSTRING, 0, line.as_ptr() as isize);
                if key(e).is_some() && key(e) == self.selected {
                    SendMessageW(self.list, LB_SETCURSEL, i, 0);
                }
            }
        }
        self.update_buttons();
    }

    fn update_buttons(&self) {
        let entry = self.selected_entry();
        let mount = self.selected.and_then(|k| self.mount_of(k));
        let mountable = entry.is_some_and(|e| matches!(e.volume, Ok((_, _, false))));
        // 붙이는 중에는 다른 마운트를 시작하지 않는다 (드라이브 문자가 겹치지 않게)
        let starting = self.mounts.iter().any(|m| m.phase == Phase::Starting);
        let idle = mount.is_none_or(|m| m.phase == Phase::Ready);
        for &(id, h) in &self.buttons {
            let on = !self.closing
                && match id {
                    ID_WINFSP => !self.winfsp,
                    ID_MOUNT_RO | ID_MOUNT_RW => self.winfsp && mountable && idle && !starting,
                    ID_UNMOUNT | ID_OPEN => mount.is_some_and(|m| m.phase == Phase::Ready),
                    _ => true,
                };
            unsafe { EnableWindow(h, on as i32) };
        }
    }

    /// 디스크 번호가 바뀌었거나 `force`면 다시 읽는다.
    fn poll(&mut self, force: bool) {
        self.winfsp = Api::load().is_ok();
        let numbers = disks::apfs_disks();
        // 뽑힌 디스크의 마운트는 끝낸다 (읽기 오류가 쌓이기 전에)
        for m in &mut self.mounts {
            if !numbers.contains(&m.disk) && m.phase == Phase::Ready {
                mountfs::request_stop(&mount_point(m.letter));
                m.phase = Phase::Stopping;
            }
        }
        if (force || numbers != self.disk_numbers) && !self.scanning {
            self.scanning = true;
            self.disk_numbers = numbers.clone();
            let hwnd = self.hwnd as isize;
            std::thread::spawn(move || post(hwnd, WM_APP_SCAN, scan(&numbers)));
        }
        self.update_buttons();
    }

    fn start_mount(&mut self, disk: u32, vol: usize, mode: MountMode) {
        let taken: Vec<char> = self.mounts.iter().map(|m| m.letter).collect();
        let Some(letter) = automount::free_letter().and_then(|l| l.chars().next()).filter(|l| !taken.contains(l)) else {
            set_text(self.message, "남은 드라이브 문자가 없습니다 (M:~Z:).");
            return;
        };
        self.mounts.push(Mount { disk, vol, letter, mode, phase: Phase::Starting });
        set_text(self.message, match mode {
            MountMode::ReadOnly => "읽기 전용으로 붙이는 중…",
            _ => "읽기/쓰기로 붙이기 전에 드라이브 전체를 검사합니다 (큰 드라이브는 30초 이상)…",
        });
        let hwnd = self.hwnd as isize;
        std::thread::spawn(move || {
            let mp = mount_point(letter);
            let result = serve_mount(Some(&format!("disk{disk}")), &mp, vol, None, mode, &mut |actual, _, fallback| {
                post(hwnd, WM_APP_READY, (letter, actual, fallback.map(str::to_string)));
            });
            post(hwnd, WM_APP_ENDED, (letter, result.map_err(|e| e.msg)));
        });
        self.render();
    }

    fn stop_mount(&mut self, letter: char) {
        if let Some(m) = self.mounts.iter_mut().find(|m| m.letter == letter && m.phase == Phase::Ready) {
            m.phase = Phase::Stopping;
            mountfs::request_stop(&mount_point(letter));
            set_text(self.message, &format!("{letter}: 해제 중… 남은 변경을 저장하고 드라이브를 검사합니다."));
        }
        self.render();
    }
}

/// 작업 스레드 → 창 스레드. 받는 쪽이 `Box::from_raw`로 되찾는다.
fn post<T: Send>(hwnd: isize, msg: u32, value: T) {
    let raw = Box::into_raw(Box::new(value));
    unsafe {
        if PostMessageW(hwnd as Hwnd, msg, 0, raw as isize) == 0 {
            drop(Box::from_raw(raw)); // 창이 이미 없다
        }
    }
}

fn set_text(h: Hwnd, text: &str) {
    let w = wide(&text.replace('\n', "\r\n"));
    unsafe { SetWindowTextW(h, w.as_ptr()) };
}

fn message_box(hwnd: Hwnd, text: &str, flags: u32) -> i32 {
    let (t, c) = (wide(text), wide(TITLE));
    unsafe { MessageBoxW(hwnd, t.as_ptr(), c.as_ptr(), flags) }
}

fn shell_open(hwnd: Hwnd, target: &str) {
    let (op, t) = (wide("open"), wide(target));
    unsafe { ShellExecuteW(hwnd, op.as_ptr(), t.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
}

/// 버튼이 눌렸을 때. 대화상자는 APP을 빌리지 않은 상태에서 띄운다
/// (대화상자가 떠 있는 동안에도 창 메시지가 들어오기 때문).
fn on_command(hwnd: Hwnd, id: usize, notify: usize) {
    if id == ID_LIST {
        if notify == LBN_SELCHANGE {
            with_app(|a| {
                let i = unsafe { SendMessageW(a.list, LB_GETCURSEL, 0, 0) };
                a.selected = usize::try_from(i).ok().and_then(|i| a.entries.get(i)).and_then(key);
                a.update_buttons();
            });
        }
        return;
    }
    let Some((selected, letter)) = with_app(|a| (a.selected, a.selected.and_then(|k| a.mount_of(k)).map(|m| m.letter))) else {
        return;
    };
    match id {
        ID_REFRESH => {
            with_app(|a| a.poll(true));
        }
        ID_WINFSP => {
            shell_open(hwnd, WINFSP_MSI);
            with_app(|a| {
                set_text(a.message, "브라우저가 WinFsp 설치 파일을 받습니다. 받은 파일을 실행해 설치한 뒤 [새로고침]을 누르세요.")
            });
        }
        ID_OPEN => {
            if let Some(l) = letter {
                shell_open(hwnd, &format!("{l}:\\"));
            }
        }
        ID_UNMOUNT => {
            if let Some(l) = letter {
                with_app(|a| a.stop_mount(l));
            }
        }
        ID_MOUNT_RO | ID_MOUNT_RW => {
            let Some((disk, vol)) = selected else { return };
            let mode = if id == ID_MOUNT_RW { MountMode::Write } else { MountMode::ReadOnly };
            if mode == MountMode::Write {
                let ask = "읽기/쓰기 마운트는 베타 기능입니다.\n\n\
                    • 잃으면 안 되는 파일은 먼저 백업하세요.\n\
                    • 뽑기 전에 반드시 [마운트 해제]를 누르거나 앱을 닫으세요.\n\
                    • 스냅샷·Fusion처럼 아직 쓸 수 없는 볼륨은 읽기 전용으로 붙습니다.\n\n계속할까요?";
                if message_box(hwnd, ask, MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) != IDYES {
                    return;
                }
            }
            with_app(|a| match a.mount_of((disk, vol)).map(|m| (m.letter, m.mode)) {
                Some((_, m)) if m == mode => {}
                // 다른 모드로 붙어 있으면: 먼저 해제하고, 끝나면(WM_APP_ENDED) 새 모드로 붙인다
                Some((l, _)) => {
                    a.stop_mount(l);
                    PENDING.with(|p| p.borrow_mut().insert(l, (disk, vol, mode)));
                }
                None => a.start_mount(disk, vol, mode),
            });
        }
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize {
    match msg {
        WM_COMMAND => on_command(hwnd, wparam & 0xffff, wparam >> 16),
        WM_TIMER => {
            with_app(|a| a.poll(false));
        }
        WM_APP_SCAN => {
            let entries = *Box::from_raw(lparam as *mut Vec<Entry>);
            with_app(|a| {
                a.scanning = false;
                a.entries = entries;
                a.render();
            });
        }
        WM_APP_READY => {
            let (letter, actual, fallback) = *Box::from_raw(lparam as *mut (char, MountMode, Option<String>));
            with_app(|a| {
                if let Some(m) = a.mounts.iter_mut().find(|m| m.letter == letter) {
                    m.phase = Phase::Ready;
                    m.mode = actual;
                    if a.closing {
                        // 검사 중에 창을 닫았다: 붙자마자 해제한다
                        m.phase = Phase::Stopping;
                        mountfs::request_stop(&mount_point(letter));
                    }
                }
                set_text(a.message, &match actual {
                    MountMode::ReadOnly => format!("{letter}: 에 읽기 전용으로 붙였습니다. [탐색기에서 열기]로 보세요."),
                    _ => format!("{letter}: 에 읽기/쓰기로 붙였습니다. 뽑기 전에 [마운트 해제]를 누르세요."),
                });
                a.render();
            });
            if let Some(why) = fallback {
                message_box(hwnd, &format!("쓰기 가능으로 붙이지 못해 읽기 전용으로 붙였습니다.\n\n이유: {why}"), MB_OK | MB_ICONWARNING);
            }
        }
        WM_APP_ENDED => {
            let (letter, result) = *Box::from_raw(lparam as *mut (char, Result<(), String>));
            let again = PENDING.with(|p| p.borrow_mut().remove(&letter));
            let close_now = with_app(|a| {
                a.mounts.retain(|m| m.letter != letter);
                set_text(a.message, &match &result {
                    Ok(()) => format!("{letter}: 를 해제했습니다. 이제 뽑아도 됩니다."),
                    Err(_) => format!("{letter}: 를 붙이지 못했거나 오류로 끝났습니다."),
                });
                if let (Some((disk, vol, mode)), Ok(())) = (again, &result) {
                    a.start_mount(disk, vol, mode);
                }
                a.render();
                a.closing && a.mounts.is_empty()
            });
            if let Err(e) = result {
                message_box(hwnd, &format!("{letter}: 오류\n\n{e}"), MB_OK | MB_ICONERROR);
            }
            if close_now == Some(true) {
                DestroyWindow(hwnd);
            }
        }
        WM_CLOSE => {
            let mounted: Vec<char> = with_app(|a| a.mounts.iter().map(|m| m.letter).collect()).unwrap_or_default();
            if mounted.is_empty() {
                DestroyWindow(hwnd);
            } else if with_app(|a| a.closing) != Some(true) {
                let ask = format!(
                    "마운트된 드라이브({})를 안전하게 해제하고 끝낼까요?\n쓰기 중이던 내용은 저장한 뒤 닫힙니다.",
                    mounted.iter().map(|l| format!("{l}:")).collect::<Vec<_>>().join(", ")
                );
                if message_box(hwnd, &ask, MB_YESNO | MB_ICONQUESTION) == IDYES {
                    with_app(|a| {
                        a.closing = true;
                        PENDING.with(|p| p.borrow_mut().clear());
                        for m in &mut a.mounts {
                            if m.phase == Phase::Ready {
                                m.phase = Phase::Stopping;
                                mountfs::request_stop(&mount_point(m.letter));
                            }
                        }
                        set_text(a.message, "드라이브를 해제하는 중… 끝나면 창이 닫힙니다.");
                        a.render();
                    });
                }
            }
        }
        WM_DESTROY => PostQuitMessage(0),
        _ => return DefWindowProcW(hwnd, msg, wparam, lparam),
    }
    0
}

/// 관리자 권한이 아니면 권한을 얻어 다시 실행하고 끝낸다.
pub fn run_elevated_or_here() {
    if crate::service::is_elevated() {
        return run();
    }
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    let (op, file, params) = (wide("runas"), wide(&exe), wide("gui"));
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), params.as_ptr(), std::ptr::null(), SW_SHOWNORMAL) };
    if r <= 32 {
        message_box(
            std::ptr::null_mut(),
            "Mac 디스크를 직접 읽고 쓰려면 관리자 권한이 필요합니다.\n다시 실행해 Windows가 물으면 [예]를 누르세요.",
            MB_OK | MB_ICONWARNING,
        );
    }
}

pub fn run() {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide("winapfs.gui");
        let wc = WndClassExW {
            size: std::mem::size_of::<WndClassExW>() as u32,
            style: 0,
            wndproc: Some(wndproc),
            cls_extra: 0,
            wnd_extra: 0,
            instance,
            icon: LoadIconW(instance, 1 as *const u16), // 리소스의 앱 아이콘 (build.sh)
            cursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            background: (COLOR_BTNFACE + 1) as Hwnd,
            menu_name: std::ptr::null(),
            class_name: class.as_ptr(),
            icon_sm: std::ptr::null_mut(),
        };
        RegisterClassExW(&wc);

        let dpi = GetDpiForSystem().max(96) as i32;
        let px = |v: i32| v * dpi / 96;
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut r = Rect { left: 0, top: 0, right: px(600), bottom: px(372) };
        AdjustWindowRectEx(&mut r, style, 0, 0);
        let title = wide(TITLE);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            r.right - r.left,
            r.bottom - r.top,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null_mut(),
        );
        let font_name = wide("Malgun Gothic");
        let font = CreateFontW(-(9 * dpi / 72), 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, font_name.as_ptr());
        let make = |class: &str, text: &str, style: u32, id: usize, x: i32, y: i32, w: i32, h: i32| -> Hwnd {
            let (c, t) = (wide(class), wide(text));
            let h = CreateWindowExW(
                if class == "LISTBOX" { WS_EX_CLIENTEDGE } else { 0 },
                c.as_ptr(),
                t.as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                px(x),
                px(y),
                px(w),
                px(h),
                hwnd,
                id as Hwnd,
                instance,
                std::ptr::null_mut(),
            );
            SendMessageW(h, WM_SETFONT, font as usize, 1);
            h
        };

        let status = make("STATIC", "디스크를 찾는 중…", 0, 0, 16, 12, 568, 20);
        let list = make("LISTBOX", "", LBS_NOTIFY | WS_VSCROLL | WS_TABSTOP, ID_LIST, 16, 38, 568, 132);
        let button_specs = [
            (ID_MOUNT_RO, "읽기 전용 마운트", 16, 180),
            (ID_MOUNT_RW, "읽기/쓰기 마운트", 160, 180),
            (ID_UNMOUNT, "마운트 해제", 304, 180),
            (ID_OPEN, "탐색기에서 열기", 448, 180),
            (ID_WINFSP, "WinFsp 설치", 16, 218),
            (ID_REFRESH, "새로고침", 448, 218),
        ];
        let buttons = button_specs
            .iter()
            .map(|&(id, text, x, y)| (id, make("BUTTON", text, BS_PUSHBUTTON | WS_TABSTOP, id, x, y, 136, 30)))
            .collect();
        let message = make("STATIC", "", 0, 0, 16, 262, 568, 60);
        make("STATIC", crate::WINFSP_NOTICE, 0, 0, 16, 328, 568, 36);

        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                hwnd,
                status,
                list,
                message,
                buttons,
                winfsp: false,
                disk_numbers: Vec::new(),
                entries: Vec::new(),
                scanning: false,
                mounts: Vec::new(),
                selected: None,
                closing: false,
            })
        });
        with_app(|a| a.poll(true));
        SetTimer(hwnd, TIMER_POLL, 2000, std::ptr::null_mut());
        ShowWindow(hwnd, SW_SHOWNORMAL);

        let mut msg: Msg = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(hwnd, &mut msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

// ---- Win32 ----

type Hwnd = *mut c_void;

#[repr(C)]
struct WndClassExW {
    size: u32,
    style: u32,
    wndproc: Option<unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize>,
    cls_extra: i32,
    wnd_extra: i32,
    instance: Hwnd,
    icon: Hwnd,
    cursor: Hwnd,
    background: Hwnd,
    menu_name: *const u16,
    class_name: *const u16,
    icon_sm: Hwnd,
}

#[repr(C)]
struct Msg {
    hwnd: Hwnd,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: [i32; 2],
    private: u32,
}

#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

const WS_OVERLAPPED: u32 = 0;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_VSCROLL: u32 = 0x0020_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const WS_EX_CLIENTEDGE: u32 = 0x200;
const BS_PUSHBUTTON: u32 = 0;
const LBS_NOTIFY: u32 = 0x1;
const CW_USEDEFAULT: i32 = 0x8000_0000_u32 as i32;
const SW_SHOWNORMAL: i32 = 1;
const COLOR_BTNFACE: usize = 15;
const IDC_ARROW: *const u16 = 32512 as *const u16;

const WM_DESTROY: u32 = 0x0002;
const WM_CLOSE: u32 = 0x0010;
const WM_SETFONT: u32 = 0x0030;
const WM_COMMAND: u32 = 0x0111;
const WM_TIMER: u32 = 0x0113;
const LB_ADDSTRING: u32 = 0x0180;
const LB_RESETCONTENT: u32 = 0x0184;
const LB_SETCURSEL: u32 = 0x0186;
const LB_GETCURSEL: u32 = 0x0188;
const LBN_SELCHANGE: usize = 1;

const MB_OK: u32 = 0x0;
const MB_YESNO: u32 = 0x4;
const MB_ICONERROR: u32 = 0x10;
const MB_ICONQUESTION: u32 = 0x20;
const MB_ICONWARNING: u32 = 0x30;
const MB_DEFBUTTON2: u32 = 0x100;
const IDYES: i32 = 6;

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(wc: *const WndClassExW) -> u16;
    #[allow(clippy::too_many_arguments)]
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: Hwnd,
        menu: Hwnd,
        instance: Hwnd,
        param: *mut c_void,
    ) -> Hwnd;
    fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn DestroyWindow(hwnd: Hwnd) -> i32;
    fn GetMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> i32;
    fn IsDialogMessageW(hwnd: Hwnd, msg: *mut Msg) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> isize;
    fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
    fn PostQuitMessage(code: i32);
    fn SendMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn SetWindowTextW(hwnd: Hwnd, text: *const u16) -> i32;
    fn EnableWindow(hwnd: Hwnd, enable: i32) -> i32;
    fn MessageBoxW(hwnd: Hwnd, text: *const u16, caption: *const u16, flags: u32) -> i32;
    fn SetTimer(hwnd: Hwnd, id: usize, ms: u32, func: *mut c_void) -> usize;
    fn LoadCursorW(instance: Hwnd, name: *const u16) -> Hwnd;
    fn LoadIconW(instance: Hwnd, name: *const u16) -> Hwnd;
    fn ShowWindow(hwnd: Hwnd, cmd: i32) -> i32;
    fn AdjustWindowRectEx(rect: *mut Rect, style: u32, menu: i32, ex_style: u32) -> i32;
    fn GetDpiForSystem() -> u32;
}

#[link(name = "gdi32")]
extern "system" {
    #[allow(clippy::too_many_arguments)]
    fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike: u32,
        charset: u32,
        out_precision: u32,
        clip_precision: u32,
        quality: u32,
        pitch_family: u32,
        face: *const u16,
    ) -> Hwnd;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Hwnd;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(hwnd: Hwnd, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> isize;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(vol: Result<(usize, String, bool), String>) -> Entry {
        Entry { disk: 3, label: "Samsung PSSD T7, 2.0 TB".into(), volume: vol }
    }

    #[test]
    fn describes_volumes_for_the_list() {
        let e = entry(Ok((0, "Untitled".into(), false)));
        assert_eq!(entry_line(&e, None), "디스크 3 (Samsung PSSD T7, 2.0 TB)  ·  \"Untitled\"  —  마운트 안 됨");
        let m = Mount { disk: 3, vol: 0, letter: 'M', mode: MountMode::Write, phase: Phase::Ready };
        assert!(entry_line(&e, Some(&m)).ends_with("—  M: 읽기/쓰기"));
        let m = Mount { phase: Phase::Starting, ..m };
        assert!(entry_line(&e, Some(&m)).contains("검사 중"));
        let e = entry(Ok((1, "Secret".into(), true)));
        assert!(entry_line(&e, None).ends_with("암호화됨 · 지원 안 함"));
        let e = Entry { label: String::new(), ..entry(Err("access denied".into())) };
        assert_eq!(entry_line(&e, None), "디스크 3  —  읽지 못함: access denied");
    }

    #[test]
    fn only_readable_volumes_have_a_key() {
        assert_eq!(key(&entry(Ok((2, "x".into(), false)))), Some((3, 2)));
        assert_eq!(key(&entry(Err("x".into()))), None);
        assert_eq!(mount_point('M'), r"\\.\M:");
    }
}
