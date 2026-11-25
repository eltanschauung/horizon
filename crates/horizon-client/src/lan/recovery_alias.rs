#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RecoveryAlias {
    target: u128,
    first_ipv4: Option<std::net::Ipv4Addr>,
}

/// An operator binds an exact verified account UUID to an existing portable save.
/// The optional third column verifies the address once, before the link becomes portable.
fn recovery_alias(aliases: &str, uuid: u128) -> Result<Option<RecoveryAlias>, usize> {
    let is_portable = |id: u128| (id >> 76) & 15 == 8 && (id >> 62) & 3 == 2;
    let mut sources = std::collections::HashSet::new();
    let mut recovered = None;
    for (index, line) in aliases.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let parse = |field: Option<&str>| {
            field
                .filter(|text| text.len() == 32)
                .and_then(|text| u128::from_str_radix(text, 16).ok())
        };
        let (Some(source), Some(target)) = (parse(fields.next()), parse(fields.next())) else {
            return Err(index + 1);
        };
        let first_ipv4 = fields
            .next()
            .map(str::parse)
            .transpose()
            .map_err(|_| index + 1)?;
        if fields.next().is_some()
            || source == 0
            || is_portable(source)
            || !is_portable(target)
            || !sources.insert(source)
        {
            return Err(index + 1);
        }
        if source == uuid {
            recovered = Some(RecoveryAlias { target, first_ipv4 });
        }
    }
    Ok(recovered)
}

/// The first authenticated join checks the peer address. Subsequent joins keep their
/// identity even when their address changes. A marker records the UUID pair only.
fn verify_recovery_address(
    confirmations: &std::path::Path,
    source: u128,
    alias: RecoveryAlias,
    peer: std::net::IpAddr,
) -> std::io::Result<bool> {
    let Some(expected) = alias.first_ipv4 else {
        return Ok(true);
    };
    let path = confirmations.join(format!("{source:032x}-{:032x}", alias.target));
    let confirmed = || match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => Err(std::io::Error::other(
            "Invalid account recovery confirmation",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    };
    if confirmed()? {
        return Ok(true);
    }
    if peer.to_canonical() != std::net::IpAddr::V4(expected) {
        return Ok(false);
    }

    let mut directory = std::fs::DirBuilder::new();
    directory.recursive(true);
    let mut marker = std::fs::OpenOptions::new();
    marker.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        directory.mode(0o700);
        marker.mode(0o600);
    }
    directory.create(confirmations)?;
    match marker.open(path.as_path()) {
        Ok(file) => {
            file.sync_all()?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => confirmed(),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::{RecoveryAlias, recovery_alias, verify_recovery_address};
    use std::net::Ipv4Addr;

    const FIRST: u128 = 0xc59ef013af5e40e888af5ddcfc81d0b0;
    const SECOND: u128 = 0x392c4cfa8df54ef695b4336b28d3be52;
    const TARGET: u128 = 0x2cd40043521f87adbe005612699b9f39;

    #[test]
    fn matching_ipv4_never_links_an_unknown_login_and_approved_providers_can_share_a_save() {
        let expected = Ipv4Addr::new(192, 0, 2, 4);
        let rule = format!("{FIRST:032x} {TARGET:032x} {expected}\n");
        let alias = RecoveryAlias {
            target: TARGET,
            first_ipv4: Some(expected),
        };
        assert_eq!(recovery_alias(&rule, FIRST), Ok(Some(alias)));
        assert_eq!(recovery_alias(&rule, SECOND), Ok(None));
        assert_eq!(recovery_alias(&rule, TARGET), Ok(None));
        let both = format!("{rule}{SECOND:032x} {TARGET:032x} {expected}");
        assert_eq!(recovery_alias(&both, SECOND), Ok(Some(alias)));
        assert_eq!(recovery_alias(&format!("{rule}{rule}"), FIRST), Err(2));
        assert_eq!(
            recovery_alias(&format!("{TARGET:032x} {FIRST:032x}"), FIRST),
            Err(1)
        );
        assert_eq!(
            recovery_alias(&format!("{FIRST:032x} {TARGET:032x} invalid"), FIRST),
            Err(1)
        );
    }

    #[test]
    fn first_join_checks_ipv4_and_later_joins_use_only_the_verified_pair() {
        let expected = Ipv4Addr::new(192, 0, 2, 4);
        let other = Ipv4Addr::new(192, 0, 2, 9);
        let dir =
            std::env::temp_dir().join(format!("horizon-recovery-test-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let alias = RecoveryAlias {
            target: TARGET,
            first_ipv4: Some(expected),
        };
        assert!(!verify_recovery_address(&dir, FIRST, alias, other.into()).unwrap());
        assert!(
            verify_recovery_address(&dir, FIRST, alias, expected.to_ipv6_mapped().into()).unwrap()
        );
        assert!(verify_recovery_address(&dir, FIRST, alias, other.into()).unwrap());
        assert!(!verify_recovery_address(&dir, SECOND, alias, other.into()).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
