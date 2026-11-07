// Mirror of addon/T3WoW/Codec.lua. The capture script hands over cell values (0..7)
// as a digit string, row-major from the strip's top-left cell.

export const MAGIC = [0x54, 0x33];
export const VERSION = 1;
export const CELLS_PER_ROW = 200;
export const MAX_ROWS = 40;
const HEADER = 7;

export function fletcher16(bytes, start, end) {
  let s1 = 0;
  let s2 = 0;
  for (let i = start; i < end; i++) {
    s1 = (s1 + bytes[i]) % 255;
    s2 = (s2 + s1) % 255;
  }
  return [s1, s2];
}

export function cellsToBytes(cells, byteCount) {
  const out = new Uint8Array(byteCount);
  let acc = 0;
  let bits = 0;
  let o = 0;
  for (let i = 0; i < cells.length && o < byteCount; i++) {
    acc = (acc << 3) | (cells[i] & 7);
    bits += 3;
    if (bits >= 8) {
      bits -= 8;
      out[o++] = (acc >> bits) & 0xff;
      acc &= (1 << bits) - 1;
    }
  }
  return o === byteCount ? out : null;
}

/** Cell values the frame header starts with, for locating the strip. */
export function magicCells() {
  return bytesToCells(Uint8Array.from([...MAGIC, VERSION])).slice(0, 8);
}

export function bytesToCells(bytes) {
  const cells = [];
  let acc = 0;
  let bits = 0;
  for (const b of bytes) {
    acc = (acc << 8) | b;
    bits += 8;
    while (bits >= 3) {
      bits -= 3;
      cells.push((acc >> bits) & 7);
      acc &= (1 << bits) - 1;
    }
  }
  if (bits > 0) cells.push((acc << (3 - bits)) & 7);
  return cells;
}

export function encodeFrame(seq, payload) {
  const body = Buffer.from(payload, "utf8");
  const bytes = [MAGIC[0], MAGIC[1], VERSION, (seq >> 8) & 0xff, seq & 0xff, body.length >> 8, body.length & 0xff, ...body];
  const [s1, s2] = fletcher16(bytes, 2, bytes.length);
  bytes.push(s1, s2);
  return bytesToCells(bytes);
}

/** Returns { seq, payload } or null when the cells are not one complete, valid frame. */
export function decodeCells(digits) {
  const cells = typeof digits === "string" ? Array.from(digits, (c) => c.charCodeAt(0) - 48) : digits;
  const header = cellsToBytes(cells, HEADER);
  if (!header || header[0] !== MAGIC[0] || header[1] !== MAGIC[1] || header[2] !== VERSION) return null;
  const length = (header[5] << 8) | header[6];
  const total = HEADER + length + 2;
  const bytes = cellsToBytes(cells, total);
  if (!bytes) return null;
  const [s1, s2] = fletcher16(bytes, 2, HEADER + length);
  if (bytes[total - 2] !== s1 || bytes[total - 1] !== s2) return null;
  return {
    seq: (header[3] << 8) | header[4],
    payload: Buffer.from(bytes.subarray(HEADER, HEADER + length)).toString("utf8"),
  };
}
