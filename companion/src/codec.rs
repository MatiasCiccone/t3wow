//! Mirror of addon/T3WoW/Codec.lua. The addon draws a frame as cells holding 3 bits each
//! (one pure color per cell); the capture reads them back as values 0..=7, row-major from
//! the strip's top-left cell.
//!
//! Frame bytes: 'T' '3' version seqHi seqLo lenHi lenLo payload... sum1 sum2, where the
//! sums are Fletcher-16 over everything from the version byte to the end of the payload.
//! Version 1's payload is JSON. Version 2 carries a part of a long command: the same JSON
//! header, a zero byte, then the part's raw bytes. Versions 3 and 4 are 1 and 2 signed:
//! their payload starts with TAG_BYTES of an HMAC-SHA256, with the key the companion wrote
//! into the addon (Key.lua), over the version byte and the rest of the payload.

use ring::hmac;

pub const MAGIC: [u8; 2] = [0x54, 0x33];
pub const VERSION: u8 = 1;
pub const VERSION_PART: u8 = 2;
pub const VERSION_SIGNED: u8 = 3;
pub const VERSION_PART_SIGNED: u8 = 4;
pub const TAG_BYTES: usize = 8;
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
pub fn magic_cells() -> [Vec<u8>; 4] {
    [VERSION, VERSION_PART, VERSION_SIGNED, VERSION_PART_SIGNED].map(|version| bytes_to_cells(&[MAGIC[0], MAGIC[1], version])[..8].to_vec())
}

/// The signature a frame of `version` carries over `payload` (what follows the signature).
pub fn tag(key: &[u8], version: u8, payload: &[u8]) -> [u8; TAG_BYTES] {
    let mut context = hmac::Context::with_key(&hmac::Key::new(hmac::HMAC_SHA256, key));
    context.update(&[version]);
    context.update(payload);
    let mut tag = [0; TAG_BYTES];
    tag.copy_from_slice(&context.sign().as_ref()[..TAG_BYTES]);
    tag
}

/// The addon's encoder (Codec.lua), signing with `key` when given, for tests.
#[cfg(test)]
fn encode(seq: u16, version: u8, payload: Vec<u8>, key: Option<&[u8]>) -> Vec<u8> {
    let (version, body) = match key {
        Some(key) => {
            let version = if version == VERSION { VERSION_SIGNED } else { VERSION_PART_SIGNED };
            (version, [tag(key, version, &payload).as_slice(), &payload].concat())
        }
        None => (version, payload),
    };
    let mut bytes = vec![MAGIC[0], MAGIC[1], version, (seq >> 8) as u8, seq as u8, (body.len() >> 8) as u8, body.len() as u8];
    bytes.extend_from_slice(&body);
    let (s1, s2) = fletcher16(&bytes[2..]);
    bytes.push(s1);
    bytes.push(s2);
    bytes_to_cells(&bytes)
}

/// A frame as an addon draws it without a key, for tests.
#[cfg(test)]
pub fn encode_frame(seq: u16, payload: &str) -> Vec<u8> {
    encode(seq, VERSION, payload.as_bytes().to_vec(), None)
}

/// A part of a long command as an addon draws it without a key, for tests.
#[cfg(test)]
pub fn encode_part_frame(seq: u16, header: &str, part: &[u8]) -> Vec<u8> {
    encode(seq, VERSION_PART, [header.as_bytes(), &[0], part].concat(), None)
}

/// A frame signed with `key`, for tests.
#[cfg(test)]
pub fn encode_signed_frame(seq: u16, payload: &str, key: &[u8]) -> Vec<u8> {
    encode(seq, VERSION, payload.as_bytes().to_vec(), Some(key))
}

/// A part of a long command signed with `key`, for tests.
#[cfg(test)]
pub fn encode_signed_part_frame(seq: u16, header: &str, part: &[u8], key: &[u8]) -> Vec<u8> {
    encode(seq, VERSION_PART, [header.as_bytes(), &[0], part].concat(), Some(key))
}

#[derive(Debug, PartialEq)]
pub struct Frame {
    pub seq: u16,
    pub payload: String,
    /// The raw bytes after the header in a version 2 frame.
    pub part: Option<Vec<u8>>,
    /// A signed frame's signature, its version and what it signs.
    pub signature: Option<Signature>,
}

#[derive(Debug, PartialEq)]
pub struct Signature {
    tag: [u8; TAG_BYTES],
    version: u8,
    signed: Vec<u8>,
}

impl Frame {
    /// Whether the frame is signed with `key`.
    pub fn signed_with(&self, key: &[u8]) -> bool {
        self.signature.as_ref().is_some_and(|s| tag(key, s.version, &s.signed) == s.tag)
    }
}

/// Why cells read from the strip aren't a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unreadable {
    /// Its first cells aren't a frame's start: the magic and a known version.
    NotAFrame,
    /// Its header asks for more cells than a strip has.
    TooLong,
    /// Fewer cells than its header asks for were read.
    CutOff,
    /// The checksum doesn't add up: cells were read wrong.
    Checksum,
    /// The checksum adds up but the payload isn't one: no room for a signature, no zero
    /// after a part's header, or not UTF-8.
    Malformed,
}

impl Unreadable {
    /// In a few words, for the log and the game's window.
    pub fn text(self) -> &'static str {
        match self {
            Unreadable::NotAFrame => "no frame start",
            Unreadable::TooLong => "too long",
            Unreadable::CutOff => "cut off",
            Unreadable::Checksum => "bad checksum",
            Unreadable::Malformed => "malformed payload",
        }
    }
}

/// One complete, valid frame from cell values, or why they aren't one.
pub fn decode_cells(cells: &[u8]) -> Result<Frame, Unreadable> {
    let header = cells_to_bytes(cells, HEADER).ok_or(Unreadable::CutOff)?;
    let version = header[2];
    if header[0] != MAGIC[0] || header[1] != MAGIC[1] || !(VERSION..=VERSION_PART_SIGNED).contains(&version) {
        return Err(Unreadable::NotAFrame);
    }
    let length = ((header[5] as usize) << 8) | header[6] as usize;
    let total = HEADER + length + 2;
    if (total * 8).div_ceil(3) > CELLS_PER_ROW * MAX_ROWS {
        return Err(Unreadable::TooLong);
    }
    let bytes = cells_to_bytes(cells, total).ok_or(Unreadable::CutOff)?;
    let (s1, s2) = fletcher16(&bytes[2..HEADER + length]);
    if bytes[total - 2] != s1 || bytes[total - 1] != s2 {
        return Err(Unreadable::Checksum);
    }
    let mut body = &bytes[HEADER..HEADER + length];
    let mut signature = None;
    if version == VERSION_SIGNED || version == VERSION_PART_SIGNED {
        let (tag, rest) = body.split_first_chunk::<TAG_BYTES>().ok_or(Unreadable::Malformed)?;
        signature = Some(Signature { tag: *tag, version, signed: rest.to_vec() });
        body = rest;
    }
    let (json, part) = if version == VERSION_PART || version == VERSION_PART_SIGNED {
        let zero = body.iter().position(|&b| b == 0).ok_or(Unreadable::Malformed)?;
        (&body[..zero], Some(body[zero + 1..].to_vec()))
    } else {
        (body, None)
    };
    let payload = String::from_utf8(json.to_vec()).map_err(|_| Unreadable::Malformed)?;
    Ok(Frame { seq: ((header[3] as u16) << 8) | header[4] as u16, payload, part, signature })
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
            assert_eq!(decode_cells(&cells), Ok(Frame { seq, payload: payload.to_string(), part: None, signature: None }));
            assert_eq!(encode_frame(seq, payload), cells, "encoder matches the addon's");
            assert_eq!(cells_needed(&cells[..19]), Some(cells.len()));
        }
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
    }

    /// The addon signs with its own HMAC-SHA256 (Codec.lua, in plain arithmetic): the frames
    /// it drew for the fixture check out here, and not with another key.
    #[test]
    fn frames_the_addon_signed_check_out_with_its_key_only() {
        let fixture: Value = serde_json::from_str(include_str!("../tests/fixtures/frames.json")).unwrap();
        let signed = &fixture["signed"];
        let key = hex(signed["key"].as_str().unwrap());
        let other = hex(&"ab".repeat(32));
        for frame in signed["frames"].as_array().unwrap() {
            let cells = digits(frame["cells"].as_str().unwrap());
            let seq = frame["seq"].as_u64().unwrap() as u16;
            let payload = frame["payload"].as_str().unwrap();
            let decoded = decode_cells(&cells).expect("decodes");
            assert_eq!((decoded.seq, decoded.payload.as_str()), (seq, payload));
            assert!(decoded.signed_with(&key));
            assert!(!decoded.signed_with(&other));
            match frame["part"].as_str() {
                Some(part) => {
                    assert_eq!(decoded.part.as_deref(), Some(part.as_bytes()));
                    assert_eq!(encode_signed_part_frame(seq, payload, part.as_bytes(), &key), cells, "encoder matches the addon's");
                    assert_eq!(cells[..8], magic_cells()[3]);
                }
                None => {
                    assert_eq!(decoded.part, None);
                    assert_eq!(encode_signed_frame(seq, payload, &key), cells, "encoder matches the addon's");
                    assert_eq!(cells[..8], magic_cells()[2]);
                }
            }
            assert_eq!(cells_needed(&cells[..19]), Some(cells.len()));
        }
    }

    #[test]
    fn an_unsigned_or_altered_frame_doesnt_count_as_signed() {
        let key = [7u8; 32];
        assert!(!decode_cells(&encode_frame(1, r#"{"k":"a"}"#)).unwrap().signed_with(&key));
        // The same signature over another payload: the checksum still adds up, the signature doesn't.
        let mut frame = decode_cells(&encode_signed_frame(1, r#"{"k":"a"}"#, &key)).unwrap();
        assert!(frame.signed_with(&key));
        frame.signature.as_mut().unwrap().signed = br#"{"k":"b"}"#.to_vec();
        assert!(!frame.signed_with(&key));
        // A signed frame too short to hold a signature isn't a frame.
        let mut bytes = vec![MAGIC[0], MAGIC[1], VERSION_SIGNED, 0, 1, 0, 3, b'{', b'}', b' '];
        let (s1, s2) = fletcher16(&bytes[2..]);
        bytes.extend([s1, s2]);
        assert_eq!(decode_cells(&bytes_to_cells(&bytes)), Err(Unreadable::Malformed));
    }

    #[test]
    fn a_part_frame_splits_into_header_and_raw_bytes() {
        let part = [0xe2, 0x82, 0, 0xff];
        let cells = encode_part_frame(7, r#"{"k":"a"}"#, &part);
        assert_eq!(decode_cells(&cells), Ok(Frame { seq: 7, payload: r#"{"k":"a"}"#.to_string(), part: Some(part.to_vec()), signature: None }));
        assert_eq!(cells[..8], magic_cells()[1]);
    }

    #[test]
    fn damaged_frames_are_rejected() {
        let mut cells = encode_frame(1, r#"{"k":"a"}"#);
        let short = cells[..cells.len() - 3].to_vec();
        cells[30] = (cells[30] + 1) % 8;
        assert_eq!(decode_cells(&cells), Err(Unreadable::Checksum));
        assert_eq!(decode_cells(&short), Err(Unreadable::CutOff));
        assert_eq!(decode_cells(&cells[..8]), Err(Unreadable::CutOff));
        let mut other = encode_frame(1, r#"{"k":"a"}"#);
        other[7] = 0; // the version's last bits: version 0
        assert_eq!(decode_cells(&other), Err(Unreadable::NotAFrame));
        // A header asking for 60000 bytes, more than a strip's 3000.
        let long = bytes_to_cells(&[MAGIC[0], MAGIC[1], VERSION, 0, 1, 0xea, 0x60]);
        assert_eq!(decode_cells(&long), Err(Unreadable::TooLong));
    }
}
