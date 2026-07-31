//! Declared sizes, in bytes.
//!
//! **Only the declared side is ever parsed.** Every backend that reports a current size is asked
//! for raw bytes — `zfs list -p`, `lvs --units b --nosuffix`, `btrfs qgroup show --raw` — so the
//! comparison is `u64` against `u64` and no tool's display formatting is in the loop. That is the
//! whole design: `btrfs qgroup show` prints `10.00GiB`, `lvs` prints `10.00g`, and `zfs list`
//! prints `10G` for the same number, and a comparator that had to understand all three would
//! report a change on every sync for ever the first time one of them rounded.

/// A declared size (`10G`, `100M`, `1.5GiB`, `10737418240`) in bytes, or `None` if it is not one.
///
/// Units are powers of 1024, because that is what all three tools mean by `G`: `lvcreate -L 10G`,
/// `zfs set quota=10G` and `btrfs qgroup limit 10G` each reserve 10 GiB. Reading `G` as a
/// thousand million here would make every declared size disagree with the volume it created.
pub fn parse_size(declared: &str) -> Option<u64> {
    let s = declared.trim();
    if s.is_empty() {
        return None;
    }
    let digits_end = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    let (number, suffix) = s.split_at(digits_end);
    let value: f64 = number.parse().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let suffix = suffix.trim();
    // `10G`, `10GB` and `10GiB` are one size written three ways. The letter is the magnitude and
    // the rest is punctuation — LVM accepts a bare `G`, ZFS accepts `GB`, and `btrfs qgroup show`
    // prints `GiB`.
    let letter = suffix.chars().next().map(|c| c.to_ascii_uppercase());
    let rest = suffix.get(letter.map_or(0, |_| 1)..).unwrap_or("");
    if !matches!(rest.to_ascii_uppercase().as_str(), "" | "B" | "IB") {
        return None;
    }
    let scale: u64 = match letter {
        None => 1,
        Some('B') if rest.is_empty() => 1,
        Some('K') => 1 << 10,
        Some('M') => 1 << 20,
        Some('G') => 1 << 30,
        Some('T') => 1 << 40,
        Some('P') => 1 << 50,
        Some('E') => 1 << 60,
        _ => return None,
    };
    let bytes = value * scale as f64;
    if bytes > u64::MAX as f64 {
        return None;
    }
    Some(bytes as u64)
}

/// Bytes as the shortest exact-enough form a person reads, for an error that names two sizes.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [(u64, &str); 5] = [
        (1 << 50, "P"),
        (1 << 40, "T"),
        (1 << 30, "G"),
        (1 << 20, "M"),
        (1 << 10, "K"),
    ];
    for (scale, unit) in UNITS {
        if bytes >= scale {
            let whole = bytes as f64 / scale as f64;
            return if (whole - whole.round()).abs() < f64::EPSILON {
                format!("{}{}", whole.round() as u64, unit)
            } else {
                format!("{:.2}{}", whole, unit)
            };
        }
    }
    format!("{}B", bytes)
}

/// Whether a declared size and a reported byte count describe the same volume.
///
/// A declaration that cannot be parsed is **not** drift. The alternative is a sync that reports a
/// change it can never carry out, every run, for a line the tool will refuse anyway — and the
/// refusal is the backend's to make, where it can name the value.
pub fn same_size(declared: &str, actual_bytes: u64) -> bool {
    parse_size(declared).is_none_or(|want| want == actual_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three spellings each tool prints, and the bare byte count every one of them is read
    /// as. `G` is 1024-based on all three; a decimal reading here would put every declared volume
    /// 7% away from the one that got created.
    #[test]
    fn one_size_written_every_way_parses_to_one_number() {
        for spelling in ["10G", "10g", "10GB", "10gb", "10GiB", "10gib", "10737418240"] {
            assert_eq!(
                parse_size(spelling),
                Some(10 * (1 << 30)),
                "{} is 10 GiB",
                spelling
            );
        }
        assert_eq!(parse_size("100M"), Some(100 * (1 << 20)));
        assert_eq!(parse_size("512K"), Some(512 * 1024));
        assert_eq!(parse_size("2T"), Some(2 * (1 << 40)));
        assert_eq!(parse_size("1P"), Some(1 << 50));
        assert_eq!(parse_size("4096"), Some(4096));
        assert_eq!(parse_size("4096B"), Some(4096));
        assert_eq!(parse_size(" 8G "), Some(8 * (1 << 30)));
    }

    /// A fraction is legal in every one of these tools (`lvcreate -L 1.5G`), so it is legal here.
    #[test]
    fn a_fractional_size_is_a_size() {
        assert_eq!(parse_size("1.5G"), Some(1536 * (1 << 20)));
        assert_eq!(parse_size("0.5M"), Some(512 * 1024));
    }

    /// Junk parses to nothing rather than to a number that would silently resize a volume.
    #[test]
    fn what_is_not_a_size_is_not_guessed_at() {
        for junk in ["", "  ", "big", "10X", "G10", "-5G", "10 G B", "10Gigs", "10.5.5G"] {
            assert_eq!(parse_size(junk), None, "{} parsed as a size", junk);
        }
    }

    /// `none` is how ZFS spells "no quota" and it must not read as a size — a declaration
    /// compared against a parsed `none` would look satisfied by a word.
    #[test]
    fn the_words_a_tool_uses_for_no_limit_are_not_sizes() {
        for word in ["none", "-", "unlimited", "0B?"] {
            assert_eq!(parse_size(word), None, "{}", word);
        }
        // `0` is a number and ZFS's `-p` prints it for "no quota" — so it parses, and reading it
        // is the caller's job, not the parser's.
        assert_eq!(parse_size("0"), Some(0));
    }

    #[test]
    fn a_size_formats_back_into_something_a_person_reads() {
        assert_eq!(format_size(10 * (1 << 30)), "10G");
        assert_eq!(format_size(100 * (1 << 20)), "100M");
        assert_eq!(format_size(1536 * (1 << 20)), "1.50G");
        assert_eq!(format_size(512), "512B");
    }

    /// The comparison the planner runs. An unparseable declaration is not drift: the backend
    /// refuses it by name, and a planner that called it a change would schedule that refusal on
    /// every sync for ever.
    #[test]
    fn comparison_is_by_value_and_an_unreadable_declaration_is_not_drift() {
        assert!(same_size("10G", 10 * (1 << 30)));
        assert!(same_size("10240M", 10 * (1 << 30)));
        assert!(!same_size("10G", 20 * (1 << 30)));
        assert!(same_size("enormous", 42));
    }
}
