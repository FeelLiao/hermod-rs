//! Native normal-token storage adapter. Unsafe is confined to this module.
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
use hermod_core::identity::{
    StoragePermissions,
    windows_acl_policy::{Ace, Principals, denied},
};
use std::{
    ffi::c_void,
    io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};
use windows::{
    Win32::{
        Foundation::{HANDLE, HLOCAL, LocalFree},
        Security::{Authorization::*, *},
    },
    core::{PCWSTR, PWSTR},
};

#[derive(Debug)]
pub(crate) struct NativePermissions;
impl StoragePermissions for NativePermissions {
    fn private_storage_root(&self) -> io::Result<Option<PathBuf>> {
        // This executable only supports normal-token private storage. Reject an
        // AppContainer rather than guessing its OS-authorized LocalFolder.
        principals()?;
        Ok(None)
    }
    fn initialize_private(&self, path: &Path, directory: bool) -> io::Result<()> {
        acl(path, directory, true)
    }
    fn verify_private(&self, path: &Path, directory: bool) -> io::Result<()> {
        acl(path, directory, false)
    }
}
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: Owns exactly one LocalAlloc allocation, no borrowed data outlives it.
        unsafe {
            LocalFree(Some(HLOCAL(self.0)));
        }
    }
}
fn info(class: TOKEN_INFORMATION_CLASS) -> io::Result<Vec<usize>> {
    let token = HANDLE(-6_isize as *mut c_void);
    let mut size = 0;
    // SAFETY: Borrowed effective-token pseudo handle, valid sizing output.
    let _ = unsafe { GetTokenInformation(token, class, None, 0, &mut size) };
    if size == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
    // SAFETY: Aligned initialized storage has at least size writable bytes.
    unsafe {
        GetTokenInformation(
            token,
            class,
            Some(buffer.as_mut_ptr().cast()),
            size,
            &mut size,
        )?;
    }
    Ok(buffer)
}
fn sid_text(sid: PSID) -> io::Result<String> {
    if sid.0.is_null() {
        return Err(denied());
    }
    let mut text = PWSTR::null();
    // SAFETY: All callers pass a SID from a live Windows-produced descriptor/token.
    unsafe {
        ConvertSidToStringSidW(sid, &mut text)?;
    }
    let _allocation = Local(text.0.cast());
    // SAFETY: Successful conversion returns a live NUL-terminated UTF16 string.
    unsafe { text.to_string().map_err(|_| denied()) }
}
fn principals() -> io::Result<Principals> {
    let app = info(TokenIsAppContainer)?;
    if app.len() * size_of::<usize>() < size_of::<u32>() {
        return Err(denied());
    }
    // SAFETY: Correct information class and checked aligned buffer size.
    if unsafe { *app.as_ptr().cast::<u32>() } != 0 {
        return Err(denied());
    }
    let user = info(TokenUser)?;
    if user.len() * size_of::<usize>() < size_of::<TOKEN_USER>() {
        return Err(denied());
    }
    // SAFETY: Windows initialized TOKEN_USER and its SID in the live aligned buffer.
    let sid = unsafe { (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    Principals::new(sid_text(sid)?, None)
}
fn acl(path: &Path, directory: bool, initialize: bool) -> io::Result<()> {
    let policy = principals()?;
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if path[..path.len() - 1].contains(&0) {
        return Err(denied());
    }
    if initialize {
        let text: Vec<u16> = policy
            .private_sddl(directory)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: NUL-terminated SDDL and valid output. Allocation retained below.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(text.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )?;
        }
        let _allocation = Local(descriptor.0);
        let mut present = windows::core::BOOL(0);
        let mut defaulted = windows::core::BOOL(0);
        let mut acl = std::ptr::null_mut();
        // SAFETY: Live Windows-produced descriptor and valid output pointers.
        unsafe {
            GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted)?;
            if !present.as_bool() || acl.is_null() {
                return Err(denied());
            }
            SetNamedSecurityInfoW(
                PCWSTR(path.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(acl),
                None,
            )
            .ok()?;
        }
    }
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let mut owner = PSID::default();
    let mut acl = std::ptr::null_mut();
    // SAFETY: Valid NUL path and outputs. Descriptor owns owner/ACL for inspection.
    unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(path.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut acl),
            None,
            &mut descriptor,
        )
        .ok()?;
    }
    let _allocation = Local(descriptor.0);
    if acl.is_null() {
        return Err(denied());
    }
    let owner = sid_text(owner)?;
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: Live descriptor, valid scalar outputs and live ACL pointer.
    unsafe {
        GetSecurityDescriptorControl(descriptor, &mut control, &mut revision)?;
    }
    // SAFETY: Windows-produced valid ACL is owned by the retained descriptor.
    let count = unsafe { (*acl).AceCount };
    let mut entries = Vec::with_capacity(count as usize);
    for index in 0..count {
        let mut ace = std::ptr::null_mut();
        // SAFETY: Index is below live ACL AceCount, valid output pointer.
        unsafe {
            GetAce(acl, u32::from(index), &mut ace)?;
        }
        // SAFETY: Windows validates the ACL; every ACE begins with ACE_HEADER.
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        if header.AceType != 0 || usize::from(header.AceSize) < size_of::<ACCESS_ALLOWED_ACE>() {
            return Err(denied());
        }
        // SAFETY: ACE kind and minimum size checked; SID tail is in live ACE.
        let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        entries.push(Ace {
            kind: header.AceType,
            flags: header.AceFlags,
            mask: allowed.Mask,
            sid: sid_text(PSID(std::ptr::addr_of!(allowed.SidStart).cast_mut().cast()))?,
        });
    }
    policy.verify(
        &owner,
        control & SE_DACL_PROTECTED.0 != 0,
        directory,
        &entries,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_acl_tampering_fails_closed_without_repair() {
        let path = std::env::temp_dir().join(format!("hermod-cli-acl-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        NativePermissions.initialize_private(&path, true).unwrap();
        NativePermissions.verify_private(&path, true).unwrap();
        let file = path.join("identity.json");
        std::fs::write(&file, b"private fixture, never rotate").unwrap();
        NativePermissions.initialize_private(&file, false).unwrap();
        NativePermissions.verify_private(&file, false).unwrap();
        let original = std::fs::read(&file).unwrap();
        let status = std::process::Command::new("icacls.exe")
            .arg(&file)
            .args(["/grant", "*S-1-1-0:R"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert!(NativePermissions.verify_private(&file, false).is_err());
        assert!(NativePermissions.verify_private(&file, false).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), original);
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(&path).unwrap();
    }
}
