//! Host filesystem observations used by the pure Application health registry.

use application::health::HealthRegistry;
use std::path::Path;

pub fn refresh_storage(
    registry: &HealthRegistry,
    data_root: &Path,
    minimum_free_bytes: u64,
    backup_required: bool,
    maximum_backup_age_secs: u64,
) {
    let free = filesystem_free_bytes(data_root).ok();
    let age = data_root
        .join("state/backup-marker.json")
        .metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .map(|elapsed| elapsed.as_secs());
    registry.update_storage(
        free,
        minimum_free_bytes,
        backup_required,
        age,
        maximum_backup_age_secs,
    );
}

#[cfg(unix)]
fn filesystem_free_bytes(path: &Path) -> Result<u64, ()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| ())?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(());
    }
    let stats = unsafe { stats.assume_init() };
    Ok(stats.f_bavail.saturating_mul(stats.f_frsize))
}

#[cfg(not(unix))]
fn filesystem_free_bytes(_: &Path) -> Result<u64, ()> {
    Err(())
}
