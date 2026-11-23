//! Alt süreçler (çeviri ve ses sunucuları) programa bağlanır: Windows iş nesnesi (job object) "kapanınca öldür"
//! ayarıyla. Program nasıl kapanırsa kapansın (çökme, görev yöneticisinden sonlandırma) sunucular da kapanır;
//! ekran kartı belleğinde 5–6 GB'lık sahipsiz süreç kalmaz.

use std::process::Child;
use std::sync::OnceLock;

#[cfg(windows)]
pub fn bagla(c: &Child) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::*;

    struct Is(isize);
    // tutamaç programın ömrü boyunca açık kalır (kapanınca Windows kapatır → alt süreçler sonlanır)
    unsafe impl Send for Is {}
    unsafe impl Sync for Is {}
    static IS: OnceLock<Option<Is>> = OnceLock::new();
    let is = IS.get_or_init(|| unsafe {
        let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if h.is_null() {
            return None;
        }
        let mut bilgi: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        bilgi.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            h,
            JobObjectExtendedLimitInformation,
            &bilgi as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 { None } else { Some(Is(h as isize)) }
    });
    if let Some(Is(h)) = is {
        unsafe {
            AssignProcessToJobObject(*h as _, c.as_raw_handle() as _);
        }
    }
}

#[cfg(not(windows))]
pub fn bagla(_c: &Child) {}
