//! Native private-storage DACL contract. No shell, environment identity or package-name guesses.
//!
//! Normal tokens require explicit full-control ACEs for token user and SYSTEM.
//! AppContainer tokens additionally require their exact TokenAppContainerSid (not
//! ALL APPLICATION PACKAGES or capabilities). Existing noncanonical ACLs fail
//! closed; only newly-created objects are initialized. Mandatory labels are not
//! changed: AppContainer low-IL directories retain their inherited low label.
pub use native::{private_acl, private_storage_root};

// All Windows ABI calls and allocation ownership stay behind this native boundary.
mod native {
    use hermod_core::identity::windows_acl_policy::{Ace, Principals, denied};
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, path::Path};
    use windows::{
        Win32::{
            Foundation::{HANDLE, HLOCAL, LocalFree},
            Security::{Authorization::*, *},
        },
        core::{PCWSTR, PWSTR},
    };

    type Result<T> = std::io::Result<T>;
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    // Owns a LocalAlloc result from Windows security APIs, never a borrowed pointer.
    struct Local(*mut c_void);
    impl Drop for Local {
        fn drop(&mut self) {
            // SAFETY: Each instance owns exactly one LocalAlloc allocation; no borrowed
            // data survives this guard, and LocalFree accepts a null pointer.
            unsafe {
                LocalFree(Some(HLOCAL(self.0)));
            }
        }
    }

    fn token_info(class: TOKEN_INFORMATION_CLASS) -> Result<Vec<usize>> {
        // The effective-token pseudo handle is borrowed, must not be closed, and
        // queries the thread impersonation token when present (otherwise process).
        let token = HANDLE(-6_isize as *mut c_void);
        let mut length = 0;
        // SAFETY: Sizing call supplies no buffer and a valid output length pointer.
        let _ = unsafe { GetTokenInformation(token, class, None, 0, &mut length) };
        if length == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut buffer = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
        // SAFETY: usize storage is aligned for TOKEN_* structs, with at least length
        // initialized writable bytes; token is a valid borrowed pseudo handle.
        unsafe {
            GetTokenInformation(
                token,
                class,
                Some(buffer.as_mut_ptr().cast()),
                length,
                &mut length,
            )?;
        }
        Ok(buffer)
    }
    fn sid_text(sid: PSID) -> Result<String> {
        if sid.0.is_null() {
            return Err(denied());
        }
        let mut text = PWSTR::null();
        // SAFETY: Caller passes a SID within a live Windows-produced token/descriptor.
        // Conversion validates it and allocates a NUL-terminated LocalAlloc string.
        unsafe {
            ConvertSidToStringSidW(sid, &mut text)?;
        }
        let _allocation = Local(text.0.cast());
        // SAFETY: Successful conversion supplies a live NUL-terminated UTF-16 string.
        unsafe { text.to_string().map_err(|_| denied()) }
    }

    fn current_principals() -> Result<Principals> {
        let user = token_info(TokenUser)?;
        // SAFETY: Windows initialized the aligned TOKEN_USER buffer; it stays alive
        // through SID conversion, including its trailing SID bytes.
        let user = sid_text(unsafe { (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid })?;
        let app = token_info(TokenIsAppContainer)?;
        // SAFETY: This information class returns a DWORD in aligned live storage.
        let is_app = unsafe { *app.as_ptr().cast::<u32>() } != 0;
        let package = if is_app {
            let info = token_info(TokenAppContainerSid)?;
            // SAFETY: This class initializes TOKEN_APPCONTAINER_INFORMATION and
            // its SID in the same live, aligned buffer.
            let sid = unsafe {
                (*info.as_ptr().cast::<TOKEN_APPCONTAINER_INFORMATION>()).TokenAppContainer
            };
            let text = sid_text(sid)?;
            Some(text)
        } else {
            None
        };
        Principals::new(user, package)
    }

    /// Return the system-authorized LocalFolder boundary only for a real packaged
    /// AppContainer whose package identity derives the SID present in its token.
    /// Normal CLI tokens return None and retain full ancestor traversal. No caller
    /// path, environment variable, or hard-coded package family establishes trust.
    pub fn private_storage_root() -> Result<Option<std::path::PathBuf>> {
        let principals = current_principals()?;
        let Some(token_package) = principals.package() else {
            return Ok(None);
        };
        // WinRT initialization is thread-local and balanced only on success. An
        // already-initialized STA returns RPC_E_CHANGED_MODE; it is already usable.
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
        struct Apartment(bool);
        impl Drop for Apartment {
            fn drop(&mut self) {
                if self.0 {
                    // SAFETY: Balances this thread's successful RoInitialize call.
                    unsafe {
                        RoUninitialize();
                    }
                }
            }
        }
        // SAFETY: No pointers; initializes the current thread's WinRT apartment.
        let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let apartment = match initialized {
            Ok(()) => Apartment(true),
            Err(error) if error.code().0 == 0x80010106_u32 as i32 => Apartment(false),
            Err(error) => return Err(error.into()),
        };
        let family = windows::ApplicationModel::Package::Current()?
            .Id()?
            .FamilyName()?;
        let family = wide(&family.to_string());
        // SAFETY: NUL-terminated package family name from the system. Returned SID
        // is owned and must be released with FreeSid, not LocalFree.
        let sid = unsafe {
            Isolation::DeriveAppContainerSidFromAppContainerName(PCWSTR(family.as_ptr()))?
        };
        let derived = sid_text(sid);
        // SAFETY: Frees exactly the owned SID returned by DeriveAppContainerSid.
        unsafe {
            FreeSid(sid);
        }
        if derived? != token_package {
            return Err(denied());
        }
        let root = windows::Storage::ApplicationData::Current()?
            .LocalFolder()?
            .Path()?;
        let root = std::path::PathBuf::from(root.to_string());
        if !root.is_absolute() {
            return Err(denied());
        }
        drop(apartment);
        Ok(Some(root))
    }

    /// Initialize a newly-created object's DACL, or verify without repairing it.
    /// Paths must be caller-controlled and checked for reparse points by the caller.
    /// Errors (including unavailable token information) always deny access.
    pub fn private_acl(path: &Path, directory: bool, initialize: bool) -> Result<()> {
        let principals = current_principals()?;
        apply_acl(path, directory, initialize, &principals)
    }

    fn apply_acl(
        path: &Path,
        directory: bool,
        initialize: bool,
        principals: &Principals,
    ) -> Result<()> {
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if path[..path.len() - 1].contains(&0) {
            return Err(denied());
        }
        if initialize {
            let sddl = wide(&principals.private_sddl(directory));
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            // SAFETY: NUL-terminated SDDL input, valid output; successful result is
            // LocalAlloc-owned and retained until SetNamedSecurityInfoW returns.
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    PCWSTR(sddl.as_ptr()),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    None,
                )?;
            }
            let _allocation = Local(descriptor.0);
            let mut present = windows::core::BOOL(0);
            let mut defaulted = windows::core::BOOL(0);
            let mut acl = std::ptr::null_mut();
            // SAFETY: Descriptor is Windows-produced and live, output pointers valid.
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
                .ok()
                .map_err(|error| {
                    std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!("set private DACL: {error}"),
                    )
                })?;
            }
        }
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let mut owner = PSID::default();
        let mut acl = std::ptr::null_mut();
        // SAFETY: Path is NUL terminated; outputs are valid. Descriptor owns owner
        // and ACL storage and remains live for all subsequent inspection.
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
        verify_descriptor(descriptor, owner, acl, directory, principals)
    }

    fn verify_descriptor(
        descriptor: PSECURITY_DESCRIPTOR,
        owner: PSID,
        acl: *mut ACL,
        directory: bool,
        principals: &Principals,
    ) -> Result<()> {
        let owner = sid_text(owner)?;
        if acl.is_null() {
            return Err(denied());
        }
        let mut control = 0;
        let mut revision = 0;
        // SAFETY: Live Windows-produced descriptor and valid scalar outputs.
        unsafe {
            GetSecurityDescriptorControl(descriptor, &mut control, &mut revision)?;
        }
        // SAFETY: GetNamedSecurityInfoW supplies a valid live ACL.
        let count = unsafe { (*acl).AceCount };
        let mut entries = Vec::with_capacity(usize::from(count));
        for index in 0..count {
            let mut ace = std::ptr::null_mut();
            // SAFETY: Index is within the live ACL's AceCount; output pointer valid.
            unsafe {
                GetAce(acl, u32::from(index), &mut ace)?;
            }
            // SAFETY: Every ACE starts with ACE_HEADER; Windows validated the ACL.
            let header = unsafe { &*ace.cast::<ACE_HEADER>() };
            // Only ACCESS_ALLOWED_ACE has the SID layout decoded here. Preserve
            // other kinds for the pure policy to reject, without interpreting their
            // potentially different native payload layout.
            let (mask, sid) = if header.AceType == 0 {
                if usize::from(header.AceSize) < size_of::<ACCESS_ALLOWED_ACE>() {
                    return Err(denied());
                }
                // SAFETY: Type and size checked above. SidStart is the in-ACE SID,
                // whose variable tail is within the Windows-produced ACE allocation.
                let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
                let sid = sid_text(PSID(std::ptr::addr_of!(allowed.SidStart).cast_mut().cast()))?;
                (allowed.Mask, sid)
            } else {
                (0, String::new())
            };
            entries.push(Ace {
                kind: header.AceType,
                flags: header.AceFlags,
                mask,
                sid,
            });
        }
        // Policy requires the exact token user or SYSTEM owner (never the package
        // SID), a protected DACL, and the complete canonical principal whitelist.
        principals.verify(
            &owner,
            control & SE_DACL_PROTECTED.0 != 0,
            directory,
            &entries,
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn check_policy_descriptor(stored: &Principals, expected: &Principals) -> Result<()> {
            let sddl = format!("O:{}{}", stored.user(), stored.private_sddl(true));
            let text = wide(&sddl);
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            // SAFETY: NUL-terminated SDDL, initialized outputs, retained allocation.
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    PCWSTR(text.as_ptr()),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    None,
                )?;
            }
            let _allocation = Local(descriptor.0);
            let mut owner = PSID::default();
            let mut acl = std::ptr::null_mut();
            let mut present = windows::core::BOOL(0);
            let mut defaulted = windows::core::BOOL(0);
            // SAFETY: Live Windows-produced descriptor and valid output pointers.
            unsafe {
                GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted)?;
                GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted)?;
            }
            verify_descriptor(descriptor, owner, acl, true, expected)
        }

        #[test]
        fn exact_package_sid_is_required_but_broad_application_group_is_rejected() {
            let actual = current_principals().unwrap();
            let name = format!("hermod-acl-policy-{}", std::process::id());
            let path = std::env::temp_dir().join(name);
            std::fs::create_dir(&path).unwrap();
            apply_acl(&path, true, true, &actual).expect("initialize normal fixture policy");
            // This deliberately injects a synthetic policy, NOT a synthetic token.
            // It tests descriptor validation only; real AppContainer execution is a
            // separate package acceptance gate and is not claimed by this test.
            let package = Principals::new(
                actual.user().to_owned(),
                Some("S-1-15-2-123456-234567-345678-456789-567890-678901-789012".into()),
            )
            .unwrap();
            check_policy_descriptor(&package, &package).unwrap();
            assert!(check_policy_descriptor(&package, &actual).is_err());
            assert!(Principals::new(actual.user().to_owned(), Some("S-1-15-2-1".into())).is_err());
            assert!(apply_acl(&path, false, false, &actual).is_err());
            // Restore the real policy solely for disposable test fixture cleanup.
            apply_acl(&path, true, true, &actual).unwrap();
            std::fs::remove_dir(&path).unwrap();
        }
    }
}
