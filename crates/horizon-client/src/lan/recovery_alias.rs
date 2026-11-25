/// Reads the operator's one-to-one authenticated-to-portable recovery mappings.
fn recovery_alias(aliases: &str, uuid: u128) -> Result<Option<u128>, usize> {
    let is_portable = |id: u128| (id >> 76) & 15 == 8 && (id >> 62) & 3 == 2;
    let mut sources = std::collections::HashSet::new();
    let mut targets = std::collections::HashSet::new();
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
        if fields.next().is_some()
            || source == 0
            || is_portable(source)
            || !is_portable(target)
            || !sources.insert(source)
            || !targets.insert(target)
        {
            return Err(index + 1);
        }
        if source == uuid {
            recovered = Some(target);
        }
    }
    Ok(recovered)
}

#[cfg(test)]
mod recovery_tests {
    use super::recovery_alias;

    #[test]
    fn only_the_exact_authenticated_id_maps_and_invalid_files_are_rejected() {
        let source = 0xc59ef013af5e40e888af5ddcfc81d0b0;
        let other = 0x392c4cfa8df54ef695b4336b28d3be52;
        let recovered = 0x2cd40043521f87adbe005612699b9f39;
        let only = format!("# Album\n{source:032x} {recovered:032x}\n");
        assert_eq!(recovery_alias(&only, source), Ok(Some(recovered)));
        assert_eq!(recovery_alias(&only, other), Ok(None));
        assert_eq!(recovery_alias(&only, recovered), Ok(None));
        assert_eq!(
            recovery_alias(&format!("{only}{other:032x} {recovered:032x}"), source),
            Err(3)
        );
        assert_eq!(
            recovery_alias(&format!("{recovered:032x} {source:032x}"), source),
            Err(1)
        );
    }
}
