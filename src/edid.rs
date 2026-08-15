//! Minimal EDID base-block parser.
//!
//! The udev backend reads the connector's `EDID` blob property and hands it
//! to [`parse_edid`] to learn the monitor's make, model and serial. Only the
//! 128-byte base block is parsed (vendor/product identification and the four
//! 18-byte descriptors); extension blocks are ignored. This is deliberately
//! hand-written instead of enabling smithay-drm-extras' `display-info`
//! feature, which would pull libdisplay-info into the build.

use smithay::output::Output;

/// Monitor identity extracted from an EDID base block.
///
/// Stored in the [`Output`]'s user data by the udev backend so protocol
/// handlers can report a serial and build a wlroots-style description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdidInfo {
    /// Three-letter PNP manufacturer ID (e.g. `DEL`).
    pub make: String,
    /// Monitor name descriptor, or `0x{product:04X}` when absent.
    pub model: String,
    /// Serial string descriptor, else the decimal serial number, else `Unknown`.
    pub serial: String,
}

impl EdidInfo {
    /// wlroots-format description: `"{make} {model} {serial}"`. This is what
    /// kanshi/shikane match against.
    pub fn description(&self) -> String {
        format!("{} {} {}", self.make, self.model, self.serial)
    }
}

/// Description to advertise for `output`: the EDID identity when the udev
/// backend found one, otherwise the connector name (never the literal
/// `"Unknown Unknown Unknown"`).
///
/// Smithay's own `Output::description()` is fixed at construction time in
/// this revision (`"{make} - {model} - {name}"`), so the wlr-output-management
/// handler should advertise this instead.
#[allow(dead_code)] // wired up by the output-management handler
pub fn output_description(output: &Output) -> String {
    output
        .user_data()
        .get::<EdidInfo>()
        .map(EdidInfo::description)
        .unwrap_or_else(|| output.name())
}

const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
const DESCRIPTOR_OFFSETS: [usize; 4] = [54, 72, 90, 108];
const TAG_SERIAL_STRING: u8 = 0xFF;
const TAG_MONITOR_NAME: u8 = 0xFC;

/// Parse the 128-byte EDID base block. Returns `None` when the blob is
/// shorter than 128 bytes or does not start with the fixed EDID header.
pub fn parse_edid(blob: &[u8]) -> Option<EdidInfo> {
    if blob.len() < 128 || blob[..8] != HEADER {
        return None;
    }

    let make = pnp_id(u16::from_be_bytes([blob[8], blob[9]]));
    let product = u16::from_le_bytes([blob[10], blob[11]]);
    let serial_number = u32::from_le_bytes([blob[12], blob[13], blob[14], blob[15]]);

    let mut name = None;
    let mut serial_string = None;
    for off in DESCRIPTOR_OFFSETS {
        let d = &blob[off..off + 18];
        // Display descriptors (as opposed to detailed timings) start with a
        // zero pixel clock and a zero reserved byte.
        if d[0..3] != [0, 0, 0] {
            continue;
        }
        let text = descriptor_text(&d[5..18]);
        match d[3] {
            TAG_MONITOR_NAME if name.is_none() && !text.is_empty() => name = Some(text),
            TAG_SERIAL_STRING if serial_string.is_none() && !text.is_empty() => {
                serial_string = Some(text)
            }
            _ => {}
        }
    }

    let model = name.unwrap_or_else(|| format!("0x{product:04X}"));
    let serial = serial_string.unwrap_or_else(|| {
        if serial_number != 0 {
            serial_number.to_string()
        } else {
            "Unknown".to_string()
        }
    });

    Some(EdidInfo {
        make,
        model,
        serial,
    })
}

/// Decode a big-endian PNP ID: three 5-bit letters, `A` = 1.
fn pnp_id(raw: u16) -> String {
    let letter = |shift: u16| {
        let v = ((raw >> shift) & 0x1F) as u8;
        if (1..=26).contains(&v) {
            (b'A' + v - 1) as char
        } else {
            '?'
        }
    };
    [letter(10), letter(5), letter(0)].iter().collect()
}

/// Descriptor text: 13 bytes, terminated by `\n`, padded with spaces.
fn descriptor_text(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end])
        .trim_end_matches(' ')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Dell U2412M-style base block: header, PNP ID `DEL` (0x10AC),
    /// product 0xA0F0, serial 0x12345678, then a serial-string descriptor
    /// and a monitor-name descriptor. Structure only; the checksum byte is
    /// not validated by the parser.
    fn dell_fixture() -> Vec<u8> {
        let mut e = vec![0u8; 128];
        e[..8].copy_from_slice(&HEADER);
        e[8] = 0x10;
        e[9] = 0xAC; // DEL
        e[10] = 0xF0;
        e[11] = 0xA0; // product 0xA0F0 (LE)
        e[12..16].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        e[16] = 0x0C; // week
        e[17] = 0x15; // year 2011
        e[18] = 0x01;
        e[19] = 0x03; // EDID 1.3
        // Descriptor 0 (54): detailed timing (non-zero pixel clock) - skipped.
        e[54] = 0x02;
        e[55] = 0x3A;
        // Descriptor 1 (72): serial string "YMYH12345678\n"
        e[75] = TAG_SERIAL_STRING;
        e[77..90].copy_from_slice(b"YMYH12345678\n");
        // Descriptor 2 (90): range limits (0xFD) - ignored.
        e[93] = 0xFD;
        // Descriptor 3 (108): monitor name "DELL U2412M\n " (padded).
        e[111] = TAG_MONITOR_NAME;
        e[113..126].copy_from_slice(b"DELL U2412M\n ");
        let sum: u8 = e[..127].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        e[127] = sum.wrapping_neg();
        e
    }

    #[test]
    fn parses_make_model_serial_from_descriptors() {
        let info = parse_edid(&dell_fixture()).expect("valid fixture");
        assert_eq!(
            info,
            EdidInfo {
                make: "DEL".into(),
                model: "DELL U2412M".into(),
                serial: "YMYH12345678".into(),
            }
        );
        assert_eq!(info.description(), "DEL DELL U2412M YMYH12345678");
    }

    #[test]
    fn falls_back_to_product_code_and_numeric_serial() {
        let mut e = dell_fixture();
        // Blank out both text descriptors.
        e[72..90].fill(0);
        e[108..126].fill(0);
        let info = parse_edid(&e).unwrap();
        assert_eq!(info.model, "0xA0F0");
        assert_eq!(info.serial, "305419896"); // 0x12345678
        // Zero serial number -> "Unknown".
        e[12..16].fill(0);
        assert_eq!(parse_edid(&e).unwrap().serial, "Unknown");
    }

    #[test]
    fn rejects_truncated_and_bad_header() {
        let e = dell_fixture();
        assert!(parse_edid(&e[..64]).is_none());
        assert!(parse_edid(&[]).is_none());
        let mut bad = e.clone();
        bad[1] = 0x00;
        assert!(parse_edid(&bad).is_none());
    }

    #[test]
    fn extension_blocks_are_ignored() {
        let mut e = dell_fixture();
        e.extend_from_slice(&[0xAA; 128]);
        assert_eq!(parse_edid(&e).unwrap().make, "DEL");
    }

    #[test]
    fn output_description_prefers_edid_then_connector_name() {
        use smithay::output::{PhysicalProperties, Subpixel};
        let props = PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Unknown".into(),
            model: "Unknown".into(),
            serial_number: "Unknown".into(),
        };
        let bare = Output::new("DP-1".into(), props.clone());
        assert_eq!(output_description(&bare), "DP-1");
        let with_edid = Output::new("DP-2".into(), props);
        with_edid
            .user_data()
            .insert_if_missing(|| parse_edid(&dell_fixture()).unwrap());
        assert_eq!(
            output_description(&with_edid),
            "DEL DELL U2412M YMYH12345678"
        );
    }

    #[test]
    fn pnp_id_decodes_letters() {
        assert_eq!(pnp_id(0x10AC), "DEL");
        assert_eq!(pnp_id(0x0000), "???");
    }
}
