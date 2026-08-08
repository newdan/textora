//! Hex color parsing for theme definitions.

/// Parse a hex color string (with or without `#` prefix).
pub fn parse_hex(s: &str) -> Result<[f32; 4], String> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| invalid_hex(s))?;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| invalid_hex(s))?;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| invalid_hex(s))?;
            Ok([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| invalid_hex(s))?;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| invalid_hex(s))?;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| invalid_hex(s))?;
            let a = u8::from_str_radix(&hex[6..8], 16).map_err(|_| invalid_hex(s))?;
            Ok([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0])
        }
        _ => Err(invalid_hex(s)),
    }
}

fn invalid_hex(s: &str) -> String {
    format!("invalid hex color: expected 6 or 8 hex chars, got \"{}\"", s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_direct_6() {
        let c = parse_hex("#FF0000").unwrap();
        assert_eq!(c, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn parse_hex_direct_8() {
        let c = parse_hex("#FF000080").unwrap();
        assert!((c[3] - 0.502).abs() < 0.01);
    }

    #[test]
    fn parse_hex_no_prefix() {
        let c = parse_hex("00FF00").unwrap();
        assert_eq!(c, [0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn parse_hex_lowercase() {
        let c = parse_hex("#ff0000").unwrap();
        assert_eq!(c, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn parse_hex_mixed_case() {
        let c = parse_hex("#aAbBcC").unwrap();
        assert!((c[0] - 0.667).abs() < 0.01);
        assert!((c[1] - 0.733).abs() < 0.01);
        assert!((c[2] - 0.800).abs() < 0.01);
    }

    #[test]
    fn parse_hex_black() {
        let c = parse_hex("#000000").unwrap();
        assert_eq!(c, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn parse_hex_white() {
        let c = parse_hex("#FFFFFF").unwrap();
        assert_eq!(c, [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn parse_hex_empty_string() {
        let err = parse_hex("").unwrap_err();
        assert!(err.contains("hex"));
    }

    #[test]
    fn parse_hex_single_char() {
        let err = parse_hex("#F").unwrap_err();
        assert!(err.contains("hex"));
    }
}
