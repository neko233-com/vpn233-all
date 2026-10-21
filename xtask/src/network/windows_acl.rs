//! Local SID-only ACL operations. No shell, module discovery or account lookup.
use anyhow::{anyhow, ensure, Result};
use std::{
    ffi::c_void,
    fs, mem,
    os::windows::{fs::MetadataExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::{Component, Path, Prefix},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, LocalFree, ERROR_INSUFFICIENT_BUFFER, HANDLE},
    Security::{
        Authorization::{GetSecurityInfo, SetSecurityInfo, SE_FILE_OBJECT},
        *,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

const FILE_ALL_ACCESS: u32 = 0x001f_01ff;
const REPARSE_POINT: u32 = 0x400;
const MAX_DESCRIPTOR: u32 = 1024 * 1024;

fn checked(ok: i32, operation: &'static str) -> Result<()> {
    if ok == 0 {
        return Err(anyhow!(
            "native ACL {operation} failed (os error {})",
            unsafe { GetLastError() }
        ));
    }
    Ok(())
}

// Native descriptors/SIDs contain pointers or DWORDs; byte Vecs do not promise
// the required alignment. Keep every native buffer usize-aligned and bounded.
struct Buffer(Vec<usize>);
impl Buffer {
    fn new(bytes: u32) -> Result<Self> {
        ensure!(
            bytes > 0 && bytes <= MAX_DESCRIPTOR,
            "native ACL buffer size invalid"
        );
        Ok(Self(vec![
            0;
            (bytes as usize).div_ceil(mem::size_of::<usize>())
        ]))
    }
    fn pointer(&self) -> *const c_void {
        self.0.as_ptr().cast()
    }
    fn pointer_mut(&mut self) -> *mut c_void {
        self.0.as_mut_ptr().cast()
    }
}
struct Token(HANDLE);
impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Sid(Buffer);
impl Sid {
    fn current() -> Result<Self> {
        let mut handle = ptr::null_mut();
        checked(
            unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) },
            "token-open",
        )?;
        let token = Token(handle);
        let mut needed = 0;
        let queried =
            unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut needed) };
        ensure!(
            queried == 0 && unsafe { GetLastError() } == ERROR_INSUFFICIENT_BUFFER,
            "native ACL token size unavailable"
        );
        let mut buffer = Buffer::new(needed)?;
        checked(
            unsafe {
                GetTokenInformation(
                    token.0,
                    TokenUser,
                    buffer.pointer_mut(),
                    needed,
                    &mut needed,
                )
            },
            "token-read",
        )?;
        let user = unsafe { &*buffer.pointer().cast::<TOKEN_USER>() };
        checked(unsafe { IsValidSid(user.User.Sid) }, "token-sid")?;
        let size = unsafe { GetLengthSid(user.User.Sid) };
        let mut sid = Self(Buffer::new(size)?);
        checked(
            unsafe { CopySid(size, sid.0.pointer_mut(), user.User.Sid) },
            "sid-copy",
        )?;
        Ok(sid)
    }
    fn well_known(kind: WELL_KNOWN_SID_TYPE) -> Result<Self> {
        let mut sid = Self(Buffer::new(SECURITY_MAX_SID_SIZE)?);
        let mut size = SECURITY_MAX_SID_SIZE;
        checked(
            unsafe { CreateWellKnownSid(kind, ptr::null_mut(), sid.0.pointer_mut(), &mut size) },
            "well-known-sid",
        )?;
        Ok(sid)
    }
    fn pointer(&self) -> PSID {
        // Windows takes PSID even for read-only SID arguments. This pointer is
        // never passed as an output; all native writes use pointer_mut instead.
        self.0.pointer().cast_mut()
    }
    fn matches(&self, other: PSID) -> bool {
        unsafe { EqualSid(self.pointer(), other) != 0 }
    }
}
struct Principals {
    user: Sid,
    system: Sid,
    administrators: Sid,
}
impl Principals {
    fn current() -> Result<Self> {
        Ok(Self {
            user: Sid::current()?,
            system: Sid::well_known(WinLocalSystemSid)?,
            administrators: Sid::well_known(WinBuiltinAdministratorsSid)?,
        })
    }
    fn trusted(&self, sid: PSID) -> bool {
        self.user.matches(sid) || self.system.matches(sid) || self.administrators.matches(sid)
    }
}
struct Object {
    file: fs::File,
    // Keep each verified ancestor open without FILE_SHARE_DELETE, preventing
    // a checked path component from being renamed/replaced before ACL access.
    _ancestors: Vec<fs::File>,
}
impl Object {
    fn handle(&self) -> HANDLE {
        self.file.as_raw_handle()
    }
}
fn open_component(path: &Path, access: u32) -> Result<fs::File> {
    let file = fs::OpenOptions::new()
        .read(true)
        .access_mode(access)
        .share_mode(3) // FILE_SHARE_READ | FILE_SHARE_WRITE; deliberately no DELETE.
        .custom_flags(0x0220_0000) // OPEN_REPARSE_POINT | BACKUP_SEMANTICS.
        .open(path)
        .map_err(|_| anyhow!("native ACL object open failed"))?;
    let metadata = file
        .metadata()
        .map_err(|_| anyhow!("native ACL object metadata unavailable"))?;
    ensure!(
        metadata.file_attributes() & REPARSE_POINT == 0
            && (metadata.is_dir() || metadata.is_file()),
        "native ACL path must be a regular local object"
    );
    Ok(file)
}
fn open_object(path: &Path, writable: bool) -> Result<Object> {
    // Reject UNC/device paths before filesystem access, avoiding network lookup.
    ensure!(
        path.is_absolute()
            && matches!(path.components().next(), Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
            && !path
                .components()
                .any(|part| matches!(part, Component::ParentDir)),
        "native ACL requires an absolute local path"
    );
    let mut ancestors = Vec::new();
    for ancestor in path
        .ancestors()
        .skip(1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let file = open_component(ancestor, 0x80)?; // FILE_READ_ATTRIBUTES.
        ensure!(
            file.metadata()
                .map_err(|_| anyhow!("native ACL ancestor metadata unavailable"))?
                .is_dir(),
            "native ACL ancestor must be a directory"
        );
        ancestors.push(file);
    }
    let access = 0x0002_0080 // READ_CONTROL | FILE_READ_ATTRIBUTES.
        | if writable { 0x000c_0000 } else { 0 }; // WRITE_DAC | WRITE_OWNER.
    Ok(Object {
        file: open_component(path, access)?,
        _ancestors: ancestors,
    })
}
struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn descriptor(object: &Object) -> Result<Descriptor> {
    let mut pointer = ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            object.handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut pointer,
        )
    };
    let descriptor = Descriptor(pointer);
    ensure!(
        status == 0,
        "native ACL descriptor-read failed (os error {status})"
    );
    ensure!(!pointer.is_null(), "native ACL descriptor is null");
    Ok(descriptor)
}
fn validate_descriptor(
    descriptor: PSECURITY_DESCRIPTOR,
    principals: &Principals,
    protected: bool,
) -> Result<()> {
    let mut owner = ptr::null_mut();
    let mut defaulted = 0;
    checked(
        unsafe { GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted) },
        "owner-read",
    )?;
    ensure!(
        !owner.is_null() && unsafe { IsValidSid(owner) } != 0 && principals.trusted(owner),
        "native ACL owner is not trusted"
    );
    let mut control = 0;
    let mut revision = 0;
    checked(
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) },
        "control-read",
    )?;
    ensure!(
        !protected || control & SE_DACL_PROTECTED != 0,
        "native ACL unexpectedly inherits parent access"
    );
    let mut present = 0;
    let mut acl = ptr::null_mut();
    checked(
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) },
        "dacl-read",
    )?;
    ensure!(
        present != 0 && !acl.is_null() && unsafe { IsValidAcl(acl) } != 0,
        "native ACL must have a valid non-null DACL"
    );
    let mut writable_user = false;
    for index in 0..unsafe { (*acl).AceCount } {
        let mut pointer = ptr::null_mut();
        checked(
            unsafe { GetAce(acl, u32::from(index), &mut pointer) },
            "ace-read",
        )?;
        let header = unsafe { &*pointer.cast::<ACE_HEADER>() };
        ensure!(
            header.AceType == 0 && usize::from(header.AceSize) >= 16,
            "native ACL contains a deny or unsupported ACE"
        );
        let ace = unsafe { &*pointer.cast::<ACCESS_ALLOWED_ACE>() };
        let sid: PSID = ptr::addr_of!(ace.SidStart).cast_mut().cast();
        // Check the complete SID fits this ACE before asking Windows to read it.
        let sub_authorities = unsafe { *sid.cast::<u8>().add(1) };
        ensure!(
            8 + 4 * usize::from(sub_authorities) <= usize::from(header.AceSize) - 8
                && unsafe { IsValidSid(sid) } != 0,
            "native ACL ACE SID is invalid"
        );
        ensure!(
            principals.trusted(sid),
            "native ACL grants access to an unexpected principal"
        );
        if principals.user.matches(sid)
            && u32::from(header.AceFlags) & INHERIT_ONLY_ACE == 0
            && ace.Mask & 3 == 3
        {
            writable_user = true;
        }
    }
    ensure!(
        writable_user,
        "native ACL does not give the current user read/write access"
    );
    Ok(())
}
fn policy_acl(principals: &Principals) -> Result<Buffer> {
    let mut buffer = Buffer::new(1024)?;
    let acl = buffer.pointer_mut().cast::<ACL>();
    checked(
        unsafe { InitializeAcl(acl, 1024, ACL_REVISION) },
        "acl-initialize",
    )?;
    for sid in [
        &principals.user,
        &principals.system,
        &principals.administrators,
    ] {
        checked(
            unsafe {
                AddAccessAllowedAceEx(
                    acl,
                    ACL_REVISION,
                    CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE,
                    FILE_ALL_ACCESS,
                    sid.pointer(),
                )
            },
            "ace-add",
        )?;
    }
    Ok(buffer)
}
fn apply(object: &Object, owner: PSID, acl: *const ACL) -> Result<()> {
    let status = unsafe {
        SetSecurityInfo(
            object.handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            owner,
            ptr::null_mut(),
            acl,
            ptr::null(),
        )
    };
    ensure!(
        status == 0,
        "native ACL policy-set failed (os error {status})"
    );
    Ok(())
}
pub(super) fn protect_directory(path: &Path) -> Result<()> {
    let object = open_object(path, true)?;
    ensure!(
        object
            .file
            .metadata()
            .map_err(|_| anyhow!("native ACL object metadata unavailable"))?
            .is_dir(),
        "native ACL protection requires an existing directory"
    );
    let principals = Principals::current()?;
    let acl = policy_acl(&principals)?;
    apply(&object, principals.user.pointer(), acl.pointer().cast())?;
    validate_descriptor(descriptor(&object)?.0, &principals, true)
}
pub(super) fn verify(path: &Path, protected: bool) -> Result<()> {
    let object = open_object(path, false)?;
    validate_descriptor(descriptor(&object)?.0, &Principals::current()?, protected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{process::Command, time::Duration};

    fn acl_snapshot(path: &Path) -> Vec<u8> {
        let object = open_object(path, false).unwrap();
        let descriptor = descriptor(&object).unwrap();
        let mut present = 0;
        let mut defaulted = 0;
        let mut acl = ptr::null_mut();
        checked(
            unsafe {
                GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted)
            },
            "synthetic-snapshot",
        )
        .unwrap();
        assert!(present != 0 && !acl.is_null());
        unsafe {
            std::slice::from_raw_parts(acl.cast::<u8>(), usize::from((*acl).AclSize)).to_vec()
        }
    }

    #[test]
    fn final_and_parent_junctions_are_refused_without_changing_target_acl() {
        let parent = tempfile::tempdir().unwrap();
        protect_directory(parent.path()).unwrap();
        let target = parent.path().join("synthetic-target");
        let child = target.join("synthetic-child");
        fs::create_dir_all(&child).unwrap();
        protect_directory(&target).unwrap();
        protect_directory(&child).unwrap();
        let before_target = acl_snapshot(&target);
        let before_child = acl_snapshot(&child);
        let junction = parent.path().join("synthetic-junction");
        let output = crate::support::process_output(
            Command::new("cmd.exe")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(&junction)
                .arg(&target),
            None,
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "synthetic junction creation failed"
        );
        assert!(protect_directory(&junction).is_err());
        assert!(verify(&junction, false).is_err());
        assert!(protect_directory(&junction.join("synthetic-child")).is_err());
        assert!(verify(&junction.join("synthetic-child"), false).is_err());
        assert_eq!(before_target, acl_snapshot(&target));
        assert_eq!(before_child, acl_snapshot(&child));
        fs::remove_dir(&junction).unwrap();
    }

    #[test]
    fn live_acl_rejects_extra_allow_deny_and_null_then_restores() {
        let parent = tempfile::tempdir().unwrap();
        protect_directory(parent.path()).unwrap();
        let child = parent.path().join("synthetic-private-fixture");
        fs::create_dir(&child).unwrap();
        protect_directory(&child).unwrap();
        let principals = Principals::current().unwrap();
        let object = open_object(&child, true).unwrap();
        for deny in [false, true] {
            let mut acl = if deny {
                Buffer::new(1024).unwrap()
            } else {
                policy_acl(&principals).unwrap()
            };
            let extra = Sid::well_known(WinWorldSid).unwrap();
            if deny {
                checked(
                    unsafe { InitializeAcl(acl.pointer_mut().cast(), 1024, ACL_REVISION) },
                    "synthetic-acl",
                )
                .unwrap();
            }
            let result = unsafe {
                if deny {
                    AddAccessDeniedAceEx(
                        acl.pointer_mut().cast(),
                        ACL_REVISION,
                        0,
                        1, // Deny reading data, keeping owner policy restoration possible.
                        principals.user.pointer(),
                    )
                } else {
                    AddAccessAllowedAceEx(
                        acl.pointer_mut().cast(),
                        ACL_REVISION,
                        0,
                        FILE_ALL_ACCESS,
                        extra.pointer(),
                    )
                }
            };
            checked(result, "synthetic-ace").unwrap();
            if deny {
                for sid in [
                    &principals.user,
                    &principals.system,
                    &principals.administrators,
                ] {
                    checked(
                        unsafe {
                            AddAccessAllowedAceEx(
                                acl.pointer_mut().cast(),
                                ACL_REVISION,
                                0,
                                FILE_ALL_ACCESS,
                                sid.pointer(),
                            )
                        },
                        "synthetic-allow",
                    )
                    .unwrap();
                }
            }
            apply(&object, principals.user.pointer(), acl.pointer().cast()).unwrap();
            assert!(verify(&child, true).is_err());
            protect_directory(&child).unwrap();
        }
        apply(&object, principals.user.pointer(), ptr::null()).unwrap();
        assert!(verify(&child, true).is_err());
        protect_directory(&child).unwrap();
        verify(&child, true).unwrap();
        fs::write(child.join("restored.txt"), b"synthetic writable fixture").unwrap();
    }
    #[test]
    fn native_descriptor_rejects_foreign_owner_and_missing_user_access() {
        let principals = Principals::current().unwrap();
        let foreign = Sid::well_known(WinWorldSid).unwrap();
        let mut acl = policy_acl(&principals).unwrap();
        let mut descriptor = SECURITY_DESCRIPTOR::default();
        let pointer = ptr::addr_of_mut!(descriptor).cast();
        checked(
            unsafe { InitializeSecurityDescriptor(pointer, 1) },
            "synthetic-descriptor",
        )
        .unwrap();
        checked(
            unsafe { SetSecurityDescriptorOwner(pointer, foreign.pointer(), 0) },
            "synthetic-owner",
        )
        .unwrap();
        checked(
            unsafe { SetSecurityDescriptorDacl(pointer, 1, acl.pointer().cast_mut().cast(), 0) },
            "synthetic-dacl",
        )
        .unwrap();
        assert!(validate_descriptor(pointer, &principals, false).is_err());
        checked(
            unsafe { SetSecurityDescriptorOwner(pointer, principals.user.pointer(), 0) },
            "synthetic-owner",
        )
        .unwrap();
        checked(
            unsafe { InitializeAcl(acl.pointer_mut().cast(), 1024, ACL_REVISION) },
            "synthetic-empty-acl",
        )
        .unwrap();
        assert!(validate_descriptor(pointer, &principals, false).is_err());
    }
}
