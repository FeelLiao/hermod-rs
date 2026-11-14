//! Pure, safe Windows private-DACL policy shared by native host adapters.
use std::io;

pub fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "non-private Windows security descriptor",
    )
}

/// Token identities, obtained by a native adapter from the effective token.
#[derive(Debug, Clone)]
pub struct Principals {
    user: String,
    package: Option<String>,
}
impl Principals {
    pub fn new(user: String, package: Option<String>) -> io::Result<Self> {
        if !valid_sid(&user)
            || package.as_deref().is_some_and(|sid| {
                !valid_sid(sid) || !sid.starts_with("S-1-15-2-") || sid == "S-1-15-2-1"
            })
        {
            return Err(denied());
        }
        Ok(Self { user, package })
    }
    pub fn user(&self) -> &str {
        &self.user
    }
    pub fn package(&self) -> Option<&str> {
        self.package.as_deref()
    }
    pub fn sids(&self) -> Vec<&str> {
        let mut sids = vec![self.user.as_str()];
        if self.user != "S-1-5-18" {
            sids.push("S-1-5-18");
        }
        if let Some(package) = &self.package {
            sids.push(package);
        }
        sids
    }
    pub fn private_sddl(&self, directory: bool) -> String {
        let flags = if directory { "OICI" } else { "" };
        let mut sddl = String::from("D:P");
        for sid in self.sids() {
            sddl.push_str(&format!("(A;{flags};FA;;;{sid})"));
        }
        sddl
    }
    /// Exact protected allow-only full-control ACL, no extra or duplicate ACEs.
    pub fn verify(
        &self,
        owner: &str,
        protected: bool,
        directory: bool,
        entries: &[Ace],
    ) -> io::Result<()> {
        if !protected || (owner != self.user && owner != "S-1-5-18") {
            return Err(denied());
        }
        let sids = self.sids();
        let mut seen = vec![false; sids.len()];
        for ace in entries {
            if ace.kind != 0 || ace.flags != if directory { 3 } else { 0 } || ace.mask != 0x001f01ff
            {
                return Err(denied());
            }
            let Some(index) = sids.iter().position(|sid| *sid == ace.sid) else {
                return Err(denied());
            };
            if seen[index] {
                return Err(denied());
            }
            seen[index] = true;
        }
        if seen.iter().any(|seen| !seen) {
            return Err(denied());
        }
        Ok(())
    }
}
fn valid_sid(sid: &str) -> bool {
    sid.starts_with("S-1-")
        && sid[4..]
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}
/// Native adapters must bounds/type-check ACE memory before constructing this value.
#[derive(Debug, Clone)]
pub struct Ace {
    pub kind: u8,
    pub flags: u8,
    pub mask: u32,
    pub sid: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_acl_rejects_broad_extra_duplicate_wrong_flags_and_owner() {
        let policy = Principals::new("S-1-5-21-42".into(), None).unwrap();
        let entries: Vec<_> = policy
            .sids()
            .into_iter()
            .map(|sid| Ace {
                kind: 0,
                flags: 3,
                mask: 0x001f01ff,
                sid: sid.into(),
            })
            .collect();
        assert!(policy.verify(policy.user(), true, true, &entries).is_ok());
        assert!(policy.verify(policy.user(), false, true, &entries).is_err());
        assert!(policy.verify("S-1-5-21-99", true, true, &entries).is_err());
        assert!(policy.verify(policy.user(), true, false, &entries).is_err());
        assert!(
            policy
                .verify(policy.user(), true, true, &entries[..1])
                .is_err()
        );
        let mut extra = entries.clone();
        extra.push(entries[0].clone());
        assert!(policy.verify(policy.user(), true, true, &extra).is_err());
        extra[2].sid = "S-1-1-0".into();
        assert!(policy.verify(policy.user(), true, true, &extra).is_err());
        assert!(Principals::new(policy.user().into(), Some("S-1-15-2-1".into())).is_err());
        assert!(Principals::new("S-1-5-21)(A;;FA;;;WD".into(), None).is_err());
    }
}
