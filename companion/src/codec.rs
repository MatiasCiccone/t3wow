//! Mirror of addon/T3WoW/Codec.lua. The addon draws a frame as cells holding 3 bits each
//! (one pure color per cell); the capture reads them back as values 0..=7, row-major from
//! the strip's top-left cell.
//!
//! Frame bytes: 'T' '3' version seqHi seqLo lenHi lenLo payload... sum1 sum2, where the
//! sums are Fletcher-16 over everything from the version byte to the end of the payload.
//! Version 1's payload is JSON. Version 2 carries a part of a long command: the same JSON
//! header, a zero byte, then the part's raw bytes.

pub const MAGIC: [u8; 2] = [0x54, 0x33];
pub const VERSION: u8 = 1;
pub const VERSION_PART: u8 = 2;
pub const CELL_PX: usize = 4;
pub const CELLS_PER_ROW: usize = 200;
pub const MAX_ROWS: usize = 40;
const HEADER: usize = 7;

pub fn fletcher16(bytes: &[u8]) -> (u8, u8) {
    let (mut s1, mut s2) = (0u32, 0u32);
    for &b in bytes {
        s1 = (s1 + b as u32) % 255;
        s2 = (s2 + s1) % 255;
    }
    (s1 as u8, s2 as u8)
}

/// Unpacks 3-bit cells (MSB first) into `count` bytes, or None when there are too few cells.
pub fn cells_to_bytes(cells: &[u8], count: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(count);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &cell in cells {
        if out.len() == count {
            break;
        }
        acc = (acc << 3) | (cell as u32 & 7);
        bits += 3;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    (out.len() == count).then_some(out)
}

pub fn bytes_to_cells(bytes: &[u8]) -> Vec<u8> {
    let mut cells = Vec::with_capacity(bytes.len() * 8 / 3 + 1);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &b in bytes {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 3 {
            bits -= 3;
            cells.push(((acc >> bits) & 7) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if bits > 0 {
        cells.push(((acc << (3 - bits)) & 7) as u8);
    }
    cells
}

/// The cell values a frame starts with, one for each version, to find the strip on screen.
pub fn magic_cells() -> [Vec<u8>; 2] {
    [VERSION, VERSION_PART].map(|version| bytes_to_cells(&[MAGIC[0], MAGIC[1], version])[..8].to_vec())
}

/// The addon's encoder (Codec.lua), for tests.
#[cfg(test)]
pub fn encode_frame(seq: u16, payload: &str) -> Vec<u8> {
    let body = payload.as_bytes();
    let mut bytes = vec![MAGIC[0], MAGIC[1], VERSION, (seq >> 8) as u8, seq as u8, (body.len() >> 8) as u8, body.len() as u8];
    bytes.extend_from_slice(body);
    let (s1, s2) = fletcher16(&bytes[2..]);
    bytes.push(s1);
    bytes.push(s2);
    bytes_to_cells(&bytes)
}

/// The addon's encoder for a part of a long command, for tests.
#[cfg(test)]
pub fn encode_part_frame(seq: u16, header: &str, part: &[u8]) -> Vec<u8> {
    let mut body = header.as_bytes().to_vec();
    body.push(0);
    body.extend_from_slice(part);
    let mut bytes = vec![MAGIC[0], MAGIC[1], VERSION_PART, (seq >> 8) as u8, seq as u8, (body.len() >> 8) as u8, body.len() as u8];
    bytes.extend_from_slice(&body);
    let (s1, s2) = fletcher16(&bytes[2..]);
    bytes.push(s1);
    bytes.push(s2);
    bytes_to_cells(&bytes)
}

#[derive(Debug, PartialEq)]
pub struct Frame {
    pub seq: u16,
    pub payload: String,
    /// The raw bytes after the header in a version 2 frame.
    pub part: Option<Vec<u8>>,
}

/// One complete, valid frame from cell values, or None.
pub fn decode_cells(cells: &[u8]) -> Option<Frame> {
    let header = cells_to_bytes(cells, HEADER)?;
    if header[0] != MAGIC[0] || header[1] != MAGIC[1] || (header[2] != VERSION && header[2] != VERSION_PART) {
        return None;
    }
    let length = ((header[5] as usize) << 8) | header[6] as usize;
    let total = HEADER + length + 2;
    let bytes = cells_to_bytes(cells, total)?;
    let (s1, s2) = fletcher16(&bytes[2..HEADER + length]);
    if bytes[total - 2] != s1 || bytes[total - 1] != s2 {
        return None;
    }
    let body = &bytes[HEADER..HEADER + length];
    let (json, part) = if header[2] == VERSION_PART {
        let zero = body.iter().position(|&b| b == 0)?;
        (&body[..zero], Some(body[zero + 1..].to_vec()))
    } else {
        (body, None)
    };
    Some(Frame { seq: ((header[3] as u16) << 8) | header[4] as u16, payload: String::from_utf8(json.to_vec()).ok()?, part })
}

/// How many cells a frame needs, from the cells holding its header (None if unreadable).
pub fn cells_needed(header_cells: &[u8]) -> Option<usize> {
    let header = cells_to_bytes(header_cells, HEADER)?;
    let length = ((header[5] as usize) << 8) | header[6] as usize;
    Some(((HEADER + length + 2) * 8).div_ceil(3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn digits(text: &str) -> Vec<u8> {
        text.bytes().map(|b| b - b'0').collect()
    }

    #[test]
    fn frames_drawn_by_the_addon_decode() {
        let fixture: Value = serde_json::from_str(include_str!("../tests/fixtures/frames.json")).unwrap();
        assert_eq!(magic_cells()[0], digits(fixture["magic"].as_str().unwrap()));
        for frame in fixture["frames"].as_array().unwrap() {
            let cells = digits(frame["cells"].as_str().unwrap());
            let payload = frame["payload"].as_str().unwrap();
            let seq = frame["seq"].as_u64().unwrap() as u16;
            assert_eq!(decode_cells(&cells), Some(Frame { seq, payload: payload.to_string(), part: None }));
            assert_eq!(encode_frame(seq, payload), cells, "encoder matches the addon's");
            assert_eq!(cells_needed(&cells[..19]), Some(cells.len()));
        }
    }

    #[test]
    fn a_part_frame_splits_into_header_and_raw_bytes() {
        let part = [0xe2, 0x82, 0, 0xff];
        let cells = encode_part_frame(7, r#"{"k":"a"}"#, &part);
        assert_eq!(decode_cells(&cells), Some(Frame { seq: 7, payload: r#"{"k":"a"}"#.to_string(), part: Some(part.to_vec()) }));
        assert_eq!(cells[..8], magic_cells()[1]);
    }

    #[test]
    fn damaged_frames_are_rejected() {
        let mut cells = encode_frame(1, r#"{"k":"a"}"#);
        let short = cells[..cells.len() - 3].to_vec();
        cells[30] = (cells[30] + 1) % 8;
        assert_eq!(decode_cells(&cells), None);
        assert_eq!(decode_cells(&short), None);
    }
}
