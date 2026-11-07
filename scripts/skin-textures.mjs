// Draws the addon's skin textures (addon/T3WoW/Media/*.tga): white shapes the themes tint.
// Run it after changing a shape: node scripts/skin-textures.mjs
//
// The box shapes are 64×64 with their corner in the outer 16 pixels, so the addon can cut
// them into nine pieces (Theme.lua) and draw the corner at any size. The glyphs are 32×32.
import { writeFileSync } from "node:fs";

const media = new URL("../addon/T3WoW/Media/", import.meta.url);
const clamp = (v) => Math.max(0, Math.min(1, v));

// Signed distance from (x, y) to a rounded square filling size×size, corner radius r.
function roundedBox(x, y, size, r) {
  const h = size / 2;
  const qx = Math.abs(x - h) - (h - r);
  const qy = Math.abs(y - h) - (h - r);
  return Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) + Math.min(Math.max(qx, qy), 0) - r;
}

function segment(px, py, [ax, ay], [bx, by]) {
  const dx = bx - ax, dy = by - ay;
  const t = clamp(((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy));
  return Math.hypot(px - ax - t * dx, py - ay - t * dy);
}

// A 32-bit TGA; pixel(x, y) gives [r, g, b, a] in 0..1, y counting down from the top.
function tga(name, size, pixel) {
  const out = Buffer.alloc(18 + size * size * 4);
  out[2] = 2; // uncompressed true color
  out.writeUInt16LE(size, 12);
  out.writeUInt16LE(size, 14);
  out[16] = 32;
  out[17] = 0x08; // 8 alpha bits, rows bottom first (the usual order, which every reader takes)
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const [r, g, b, a] = pixel(x + 0.5, y + 0.5);
      const at = 18 + ((size - 1 - y) * size + x) * 4;
      out[at] = Math.round(b * 255);
      out[at + 1] = Math.round(g * 255);
      out[at + 2] = Math.round(r * 255);
      out[at + 3] = Math.round(clamp(a) * 255);
    }
  }
  writeFileSync(new URL(name, media), out);
}

// Fill and 1px-when-drawn outline, rounded (the T3 theme, and every theme's pills and dots).
tga("round.tga", 64, (x, y) => [1, 1, 1, clamp(0.5 - roundedBox(x, y, 64, 16))]);
tga("ring.tga", 64, (x, y) => {
  const d = roundedBox(x, y, 64, 16);
  return [1, 1, 1, clamp(0.5 - d) - clamp(0.5 - (d + 2))];
});

// The Classic theme: a tighter corner, and a bevelled grey edge like the game's tooltips
// (light outside, dark, then mid grey). Tinted white it keeps its greys.
tga("classic.tga", 64, (x, y) => [1, 1, 1, clamp(0.5 - roundedBox(x, y, 64, 10))]);
tga("classic-ring.tga", 64, (x, y) => {
  const d = roundedBox(x, y, 64, 10);
  const depth = -d;
  const grey = depth < 2 ? 0.55 : depth < 4 ? 0.12 : 0.3;
  return [grey, grey, grey, clamp(0.5 - d) - clamp(0.5 - (d + 6))];
});

// Glyphs, stroked: close, a chevron pointing down, a check mark and the resize grip.
function strokes(name, width, lines) {
  tga(name, 32, (x, y) => {
    let d = Infinity;
    for (const line of lines) {
      for (let i = 0; i + 1 < line.length; i++) d = Math.min(d, segment(x, y, line[i], line[i + 1]));
    }
    return [1, 1, 1, clamp(width / 2 + 0.5 - d)];
  });
}
strokes("close.tga", 2.6, [[[9, 9], [23, 23]], [[23, 9], [9, 23]]]);
strokes("chevron.tga", 2.6, [[[9, 12.5], [16, 19.5], [23, 12.5]]]);
strokes("check.tga", 2.8, [[[8.5, 16.5], [13.5, 21.5], [23.5, 10.5]]]);
strokes("grip.tga", 2.2, [[[27, 11], [11, 27]], [[27, 19], [19, 27]]]);
console.log("skin textures written to addon/T3WoW/Media");
