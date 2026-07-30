//! serde module for `[f32; 4]` ↔ hex string (`#RRGGBB` or `#RRGGBBAA`).
//!
//! Use with `#[serde(with = "hex_color")]` on `[f32; 4]` fields.

use serde::{Deserialize, Deserializer, Serializer};

pub fn serialize<S>(color: &[f32; 4], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let clamp = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let r = clamp(color[0]);
    let g = clamp(color[1]);
    let b = clamp(color[2]);
    let a = clamp(color[3]);
    if a == 255 {
        serializer.serialize_str(&format!("#{:02X}{:02X}{:02X}", r, g, b))
    } else {
        serializer.serialize_str(&format!("#{:02X}{:02X}{:02X}{:02X}", r, g, b, a))
    }
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<[f32; 4], D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    parse_hex(&s).map_err(serde::de::Error::custom)
}

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

    #[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
    struct ColorWrap {
        #[serde(with = "super")]
        color: [f32; 4],
    }

    fn from_json(s: &str) -> ColorWrap {
        serde_json::from_str(s).unwrap()
    }

    fn to_json(c: &ColorWrap) -> String {
        serde_json::to_string(c).unwrap()
    }

    #[test]
    fn deserialize_6_char_rgb() {
        let w = from_json("{\"color\": \"#74ADE8\"}");
        assert!((w.color[0] - 0.4549).abs() < 0.01);
        assert!((w.color[1] - 0.6784).abs() < 0.01);
        assert!((w.color[2] - 0.9098).abs() < 0.01);
        assert_eq!(w.color[3], 1.0);
    }

    #[test]
    fn deserialize_8_char_rgba() {
        let w = from_json("{\"color\": \"#74ADE83D\"}");
        assert!((w.color[3] - 0.2392).abs() < 0.01);
    }

    #[test]
    fn deserialize_no_prefix() {
        let w = from_json("{\"color\": \"74ADE8\"}");
        assert!((w.color[0] - 0.4549).abs() < 0.01);
    }

    #[test]
    fn deserialize_invalid_length() {
        let err = serde_json::from_str::<ColorWrap>("{\"color\": \"#74ADE\"}").unwrap_err();
        assert!(err.to_string().contains("hex"));
    }

    #[test]
    fn deserialize_invalid_chars() {
        let err = serde_json::from_str::<ColorWrap>("{\"color\": \"#ZZZZZZ\"}").unwrap_err();
        assert!(err.to_string().contains("hex"));
    }

    #[test]
    fn round_trip_6_char() {
        let w = from_json("{\"color\": \"#74ADE8\"}");
        let s = to_json(&w);
        assert_eq!(s, "{\"color\":\"#74ADE8\"}");
    }

    #[test]
    fn round_trip_8_char() {
        let w = from_json("{\"color\": \"#74ADE83D\"}");
        let s = to_json(&w);
        assert_eq!(s, "{\"color\":\"#74ADE83D\"}");
    }

    #[test]
    fn serialize_opaque_as_6_char() {
        let w = ColorWrap { color: [1.0, 0.0, 0.0, 1.0] };
        assert_eq!(to_json(&w), "{\"color\":\"#FF0000\"}");
    }

    #[test]
    fn serialize_transparent_as_8_char() {
        let w = ColorWrap { color: [1.0, 0.0, 0.0, 0.5] };
        assert_eq!(to_json(&w), "{\"color\":\"#FF000080\"}");
    }

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

    #[test]
    fn serialize_clamps_above_one() {
        let w = ColorWrap { color: [1.5, 2.0, 0.5, 1.0] };
        // After clamp: [1.0, 1.0, 0.5, 1.0] = #FFFF80
        assert_eq!(to_json(&w), "{\"color\":\"#FFFF80\"}");
    }

    #[test]
    fn serialize_clamps_below_zero() {
        let w = ColorWrap { color: [-0.5, 0.0, 0.5, 1.0] };
        assert_eq!(to_json(&w), "{\"color\":\"#000080\"}");
    }

    #[test]
    fn parse_hex_lowercase_round_trip() {
        let c = parse_hex("#ff8800").unwrap();
        // serialize always outputs uppercase
        let w = ColorWrap { color: c };
        assert_eq!(to_json(&w), "{\"color\":\"#FF8800\"}");
    }
}
