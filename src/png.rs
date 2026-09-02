// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimal dependency-free PNG writer for `wm-screenshot`.
//!
//! Emits a valid RGBA8 PNG using zlib *stored* (uncompressed) deflate blocks,
//! so no compressor dependency is needed. Files are larger than zopfli/libpng
//! output but decode everywhere; screenshots are throwaway verification
//! artifacts, not archival images.

use std::io::Write;

/// Byte-indexed lookup table for the reflected CRC-32 polynomial
/// (0xEDB88320), built at compile time.
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// CRC-32 (ISO 3309), one table lookup per byte; the IDAT chunk of a 4K
/// screenshot is ~32 MB, so the bitwise loop was a visible chunk of the
/// encode time.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc = CRC32_TABLE[((crc ^ byte as u32) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    // The CRC covers type + data; hash them in place instead of copying the
    // payload just to concatenate.
    let start = out.len() - payload.len() - 4;
    out.extend_from_slice(&crc32(&out[start..]).to_be_bytes());
}

/// zlib stream with stored deflate blocks (max 65535 bytes each).
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 65535 * 5 + 16);
    out.extend_from_slice(&[0x78, 0x01]);
    let mut chunks = data.chunks(65535).peekable();
    // An empty input still needs one final stored block.
    if chunks.peek().is_none() {
        out.extend_from_slice(&[0x01, 0, 0, 0xff, 0xff]);
    }
    while let Some(block) = chunks.next() {
        out.push(if chunks.peek().is_none() { 0x01 } else { 0x00 });
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Writes `rgba` (row-major, 4 bytes/pixel, `width * height * 4` long) as a
/// PNG to `path`.
pub fn write_rgba(
    path: &std::path::Path,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> std::io::Result<()> {
    let expected = width as usize * height as usize * 4;
    if rgba.len() < expected || width == 0 || height == 0 {
        return Err(std::io::Error::other("png: pixel buffer size mismatch"));
    }

    // Raw scanlines, each prefixed with filter type 0 (None).
    let stride = width as usize * 4;
    let mut raw = Vec::with_capacity(expected + height as usize);
    for row in 0..height as usize {
        raw.push(0);
        raw.extend_from_slice(&rgba[row * stride..row * stride + stride]);
    }

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    // 8-bit, color type 6 (RGBA), deflate, adaptive filtering, no interlace.
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);

    let mut out = Vec::with_capacity(raw.len() + 1024);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);

    let mut file = std::fs::File::create(path)?;
    file.write_all(&out)?;
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_known_vector() {
        // CRC-32 of "123456789" is the classic check value.
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
    }

    #[test]
    fn adler32_matches_known_vector() {
        // adler32("Wikipedia") = 0x11E60398.
        assert_eq!(adler32(b"Wikipedia"), 0x11e60398);
    }

    #[test]
    fn writes_decodable_structure() {
        let dir = std::env::temp_dir().join("minde-png-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.png");
        let px = [
            255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 0, 255,
        ];
        write_rgba(&path, 2, 2, &px).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            &bytes[..8],
            &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']
        );
        assert!(bytes.windows(4).any(|w| w == b"IEND"));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn rejects_short_buffer() {
        assert!(write_rgba(std::path::Path::new("/nonexistent/x.png"), 2, 2, &[0; 4]).is_err());
    }
}
