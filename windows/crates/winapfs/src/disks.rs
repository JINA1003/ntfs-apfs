//! Finding Mac (APFS) disks without administrator rights: any user may ask a
//! disk for its partition table (IOCTL_DISK_GET_DRIVE_LAYOUT_EX is a
//! FILE_ANY_ACCESS request), even though reading the disk needs admin.

use std::ffi::c_void;

use apfs::gpt::APFS_PARTITION_TYPE;

use crate::winfsp::wide;

const IOCTL_DISK_GET_DRIVE_LAYOUT_EX: u32 = 0x0007_0050;
const PARTITION_STYLE_GPT: u32 = 1;
/// `DRIVE_LAYOUT_INFORMATION_EX` header: style, count, then a 40-byte union.
const LAYOUT_HEADER: usize = 48;
/// `sizeof(PARTITION_INFORMATION_EX)` on 64-bit Windows.
const PARTITION_ENTRY: usize = 144;
/// Offset of `PartitionInformationGpt.PartitionType` within an entry.
const GPT_TYPE_OFFSET: usize = 32;
const MAX_DISKS: u32 = 32;

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *mut c_void,
        disposition: u32,
        flags: u32,
        template: *mut c_void,
    ) -> *mut c_void;
    fn DeviceIoControl(
        device: *mut c_void,
        code: u32,
        input: *mut c_void,
        input_len: u32,
        output: *mut c_void,
        output_len: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
}

/// Numbers of the physical disks that have an APFS partition, ascending.
pub fn apfs_disks() -> Vec<u32> {
    (0..MAX_DISKS).filter(|&n| has_apfs_partition(n) == Some(true)).collect()
}

/// `None` when the disk does not exist or will not answer.
fn has_apfs_partition(n: u32) -> Option<bool> {
    query(n, IOCTL_DISK_GET_DRIVE_LAYOUT_EX, &[], LAYOUT_HEADER + 128 * PARTITION_ENTRY).map(|b| layout_has_apfs(&b))
}

/// 디스크를 사람이 알아볼 수 있게: 제품명과 용량 (예: `Samsung PSSD T7, 2.0 TB`).
/// 레이아웃 조회처럼 접근 권한 없이 열 수 있는 질의만 쓴다.
pub fn describe(n: u32) -> String {
    const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
    const IOCTL_DISK_GET_DRIVE_GEOMETRY_EX: u32 = 0x0007_00A0;
    // STORAGE_PROPERTY_QUERY { StorageDeviceProperty, PropertyStandardQuery }
    let name = query(n, IOCTL_STORAGE_QUERY_PROPERTY, &[0u8; 12], 1024).map(|d| device_name(&d)).unwrap_or_default();
    // DISK_GEOMETRY_EX: 24바이트 DISK_GEOMETRY 다음이 DiskSize
    let size = query(n, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], 256)
        .and_then(|g| g.get(24..32).map(|b| u64::from_le_bytes(b.try_into().unwrap())));
    match (name.is_empty(), size) {
        (false, Some(s)) => format!("{name}, {}", human_size(s)),
        (false, None) => name,
        (true, Some(s)) => human_size(s),
        (true, None) => String::new(),
    }
}

/// STORAGE_DEVICE_DESCRIPTOR의 VendorId·ProductId 문자열.
fn device_name(d: &[u8]) -> String {
    let text_at = |field: usize| -> String {
        let off = d.get(field..field + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()) as usize);
        if off == 0 || off >= d.len() {
            return String::new();
        }
        let end = d[off..].iter().position(|&c| c == 0).map_or(d.len(), |p| off + p);
        String::from_utf8_lossy(&d[off..end]).trim().to_string()
    };
    [text_at(12), text_at(16)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let (mut v, mut u) = (bytes as f64, 0);
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    format!("{v:.1} {}", UNITS[u])
}

/// 디스크를 권한 없이(desired access 0) 열고 IOCTL 하나를 보낸다.
fn query(n: u32, code: u32, input: &[u8], out_len: usize) -> Option<Vec<u8>> {
    const FILE_SHARE_READ_WRITE: u32 = 0x3;
    const OPEN_EXISTING: u32 = 3;
    const INVALID_HANDLE: isize = -1;
    let path = wide(&format!(r"\\.\PhysicalDrive{n}"));
    let h = unsafe {
        CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ_WRITE, std::ptr::null_mut(), OPEN_EXISTING, 0, std::ptr::null_mut())
    };
    if h as isize == INVALID_HANDLE {
        return None;
    }
    let mut input = input.to_vec();
    // u64 단위로 잡아 출력 구조체 정렬을 맞춘다
    let mut buf = vec![0u64; out_len.div_ceil(8)];
    let mut got = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            h,
            code,
            if input.is_empty() { std::ptr::null_mut() } else { input.as_mut_ptr().cast() },
            input.len() as u32,
            buf.as_mut_ptr().cast(),
            (buf.len() * 8) as u32,
            &mut got,
            std::ptr::null_mut(),
        )
    };
    unsafe { CloseHandle(h) };
    if ok == 0 {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), got as usize) }.to_vec())
}

fn layout_has_apfs(layout: &[u8]) -> bool {
    let u32_at = |o: usize| layout.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    if u32_at(0) != Some(PARTITION_STYLE_GPT) {
        return false;
    }
    let count = u32_at(4).unwrap_or(0) as usize;
    (0..count).any(|i| {
        let t = LAYOUT_HEADER + i * PARTITION_ENTRY + GPT_TYPE_OFFSET;
        layout.get(t..t + 16) == Some(&APFS_PARTITION_TYPE[..])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_gpt_layout() {
        let mut layout = vec![0u8; LAYOUT_HEADER + 2 * PARTITION_ENTRY];
        layout[0] = PARTITION_STYLE_GPT as u8;
        layout[4] = 2;
        assert!(!layout_has_apfs(&layout));
        let t = LAYOUT_HEADER + PARTITION_ENTRY + GPT_TYPE_OFFSET;
        layout[t..t + 16].copy_from_slice(&APFS_PARTITION_TYPE);
        assert!(layout_has_apfs(&layout));
        layout[0] = 0; // MBR disks never count
        assert!(!layout_has_apfs(&layout));
    }

    #[test]
    fn names_a_device_and_sizes() {
        let mut d = vec![0u8; 64];
        d[12] = 40; // VendorIdOffset
        d[16] = 48; // ProductIdOffset
        d[40..47].copy_from_slice(b"Samsung");
        d[48..56].copy_from_slice(b"PSSD T7 ");
        assert_eq!(device_name(&d), "Samsung PSSD T7");
        d[12] = 0; // 제조사 없음
        assert_eq!(device_name(&d), "PSSD T7");
        assert_eq!(human_size(2_000_398_934_016), "2.0 TB");
        assert_eq!(human_size(512), "512.0 B");
    }
}
