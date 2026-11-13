//! MAC address normalization helpers.

/// Normalize MAC to `AA:BB:CC:DD:EE:FF` uppercase.
pub fn normalize_mac(input: &str) -> Result<String, String> {
    let hex: String = input
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if hex.len() != 12 {
        return Err(format!("invalid MAC address: {input}"));
    }
    let parts: Vec<&str> = hex
        .as_bytes()
        .chunks(2)
        .map(|chunk| std::str::from_utf8(chunk).unwrap())
        .collect();
    Ok(parts.join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_variants() {
        assert_eq!(
            normalize_mac("aa-bb-cc-dd-ee-ff").unwrap(),
            "AA:BB:CC:DD:EE:FF"
        );
        assert_eq!(normalize_mac("AABBCCDDEEFF").unwrap(), "AA:BB:CC:DD:EE:FF");
    }
}
