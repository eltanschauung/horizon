//! Ely.by profile formats, independent of credential storage and network verification.
/// Account registration uses UUIDv4; legacy UUIDv5 profiles remain supported.
/// This is a format check, never proof of account ownership.
pub fn valid_ely_uuid(uuid: u128) -> bool {
    matches!((uuid >> 76) & 15, 4 | 5) && (uuid >> 62) & 3 == 2
}
/// Ely permits letters, digits and these punctuation characters in profile names.
/// The login protocol bounds names in UTF-16 code units, like Minecraft.
pub fn valid_ely_name(name: &str) -> bool {
    (3..=16).contains(&name.encode_utf16().count())
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || "-_.!$%^&*()[]:;".contains(c))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_and_legacy_ids() {
        assert!(valid_ely_uuid(0x11111111222243338444555555555555));
        assert!(valid_ely_uuid(0x11111111222253338444555555555555));
        assert!(!valid_ely_uuid(0x11111111222283338444555555555555));
        assert!(!valid_ely_uuid(0));
        assert!(!valid_ely_uuid(0x11111111222243330444555555555555));
    }
    #[test]
    fn provider_names_are_not_offline_names() {
        for name in ["Ely_Player", "Lunar-Moon", "name.name", "Игрок"] {
            assert!(valid_ely_name(name));
        }
        for name in ["", "ab", "a b", "a\nb", "12345678901234567", "a§b"] {
            assert!(!valid_ely_name(name));
        }
    }
}
