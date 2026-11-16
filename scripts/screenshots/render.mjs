// Draws assets/screenshots from the addon's real Theme.lua and UI.lua: game.lua is a pretend
// game with real layout, scene.lua feeds it made-up threads, and headless Chrome paints what
// the addon built. Nothing here ships: the exe embeds addon/T3WoW only, and the addon zip is
// that folder.
// usage: npm run screenshots [-- scene...]   (Chrome, or set CHROME to its path; needs network
// the first time, for the two fonts that stand in for the game's own)
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "../..");
const outDir = join(repo, "assets/screenshots");
const only = process.argv.slice(2);
const require = createRequire(join(repo, "package.json"));
const { lua, lauxlib, lualib, to_luastring } = require("fengari");
const CHROME = process.env.CHROME || "C:/Program Files/Google/Chrome/Application/chrome.exe";
const media = join(repo, "addon/T3WoW/Media");
const b64 = (f) => readFileSync(join(media, f)).toString("base64");
const work = join(tmpdir(), "t3wow-screenshots");
mkdirSync(work, { recursive: true });
mkdirSync(outDir, { recursive: true });

const FACES = `
<link href="https://fonts.googleapis.com/css2?family=Marcellus&family=Roboto+Condensed:wght@400;600&display=block" rel="stylesheet">
<style>
@font-face{font-family:Inter;font-weight:400;src:url(data:font/ttf;base64,${b64("Inter-Regular.ttf")})}
@font-face{font-family:Inter;font-weight:600;src:url(data:font/ttf;base64,${b64("Inter-SemiBold.ttf")})}
@font-face{font-family:'JetBrains Mono';src:url(data:font/ttf;base64,${b64("JetBrainsMono-Regular.ttf")})}
html,body{margin:0;padding:0;background:#000}
</style>`;
const FONT = {
  inter: "400 SIZEpx Inter", semi: "600 SIZEpx Inter", mono: "400 SIZEpx 'JetBrains Mono'",
  narrow: "400 SIZEpx 'Roboto Condensed'", friz: "400 SIZEpx Marcellus",
};

function chrome(args) {
  const r = spawnSync(CHROME, ["--headless=new", "--disable-gpu", "--hide-scrollbars", `--user-data-dir=${join(work, "profile")}`,
    "--force-device-scale-factor=1", "--virtual-time-budget=20000", ...args], { encoding: "utf8", maxBuffer: 1 << 28, timeout: 120000 });
  if (r.status !== 0) throw new Error(`chrome: ${r.status} ${r.stderr}`);
  return r.stdout;
}

function widths() {
  const cache = join(work, "widths.json");
  if (existsSync(cache)) return JSON.parse(readFileSync(cache, "utf8"));
  const chars = [];
  for (let c = 32; c < 127; c++) chars.push(String.fromCharCode(c));
  chars.push(..."…·×→←↑↓—–•✓’‘“”");
  const page = `<!doctype html><meta charset=utf-8>${FACES}<pre id=out></pre><script>
  const FONT=${JSON.stringify(FONT)}, chars=${JSON.stringify(chars)};
  (async()=>{ const out={}; const ctx=document.createElement('canvas').getContext('2d');
  for(const k in FONT){ const f=FONT[k].replace('SIZE','100'); await document.fonts.load(f,'Abc'); ctx.font=f; ctx.fontKerning='none';
    out[k]={ok:document.fonts.check(f)}; for(const ch of chars) out[k][ch]=ctx.measureText(ch).width*10; }
  document.getElementById('out').textContent=JSON.stringify(out); })();</script>`;
  const file = join(work, "measure.html");
  writeFileSync(file, page);
  const dom = chrome(["--dump-dom", pathToFileURL(file).href]);
  const m = dom.match(/<pre id="out">(.*?)<\/pre>/s);
  if (!m || !m[1]) throw new Error("measure failed: " + dom.slice(-400));
  // &amp; last, so an escaped "&lt;" in the text stays "&lt;".
  const json = m[1].replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");
  const data = JSON.parse(json);
  for (const k in data) { if (!data[k].ok) throw new Error(`font ${k} failed to load`); delete data[k].ok; }
  writeFileSync(cache, JSON.stringify(data));
  return data;
}

const luaStr = (s) => JSON.stringify(s).replace(/\\u([0-9a-f]{4})/g, (_, h) => `\\u{${h}}`);
const luaWidths = (w) => "WIDTHS = {" + Object.entries(w).map(([k, v]) =>
  `${k}={` + Object.entries(v).map(([c, x]) => `[${luaStr(c)}]=${x}`).join(",") + "}").join(",") + "}";

function runScene(name, W, H) {
  const L = lauxlib.luaL_newstate();
  lualib.luaL_openlibs(L);
  const read = (f) => readFileSync(join(repo, "addon/T3WoW", f), "utf8");
  const load = (f) => `assert(load(${luaStr(read(f))}, "${f}"))("T3WoW", ns)`;
  const chunk = `SCREEN_W, SCREEN_H = ${W}, ${H}
${luaWidths(widths())}
${readFileSync(join(here, "game.lua"), "utf8")}
local ns = {}
T3WoWDB = { client = "c1", seq = 0, queue = {}, settings = {} }
${load("Codec.lua")}
${load("Theme.lua")}
${load("UI.lua")}
${load("Link.lua")}
Link, UI, Skin = ns.Link, ns.UI, ns.Skin
SCENE = ${luaStr(name)}
${readFileSync(join(here, "scene.lua"), "utf8")}
return JSON(DRAW())`;
  if (lauxlib.luaL_loadstring(L, to_luastring(chunk)) !== lua.LUA_OK || lua.lua_pcall(L, 0, 1, 0) !== lua.LUA_OK) {
    throw new Error(lua.lua_tojsstring(L, -1));
  }
  return lua.lua_tojsstring(L, -1);
}

function tgas() {
  const out = {};
  for (const n of ["round", "ring", "classic", "classic-ring", "alarm", "brain", "check", "chevron", "clock", "close", "compose", "done", "eye", "gear",
    "globe", "grip", "hammer", "image-off", "lock", "logo", "message", "pencil", "question", "refresh", "search", "shield-off", "shield", "sparkle", "spinner", "terminal"]) {
    const b = readFileSync(join(media, n + ".tga"));
    const w = b.readUInt16LE(12), h = b.readUInt16LE(14), top = (b[17] & 0x20) !== 0;
    const px = Buffer.alloc(w * h * 4);
    for (let y = 0; y < h; y++) {
      const row = top ? y : h - 1 - y;
      for (let x = 0; x < w; x++) {
        const s = 18 + (row * w + x) * 4, d = (y * w + x) * 4;
        px[d] = b[s + 2]; px[d + 1] = b[s + 1]; px[d + 2] = b[s]; px[d + 3] = b[s + 3];
      }
    }
    out[n] = { w, h, d: px.toString("base64") };
  }
  return out;
}

const SCENES = {
  "theme-t3": { W: 1324, H: 873, cap: "The T3 theme." },
  "theme-flat": { W: 1324, H: 873, cap: "The Flat theme, with Roboto Condensed standing in for the game's Arial Narrow." },
  "theme-classic": { W: 1324, H: 873, cap: "The Classic theme, with Marcellus standing in for Friz Quadrata and Roboto Condensed for Arial Narrow." },
  "compact-list": { W: 988, H: 658, cap: "The T3 theme with Compact thread list." },
};

const CAPTION = "Drawn from the addon's own Theme.lua and UI.lua (not an in-game capture). ";
for (const [name, s] of Object.entries(SCENES)) {
  if (only.length && !only.includes(name)) continue;
  const items = runScene(name, s.W, s.H);
  const page = `<!doctype html><meta charset=utf-8>${FACES}<canvas id=c width=${s.W} height=${s.H} style="display:block"></canvas><script>
const FONT=${JSON.stringify(FONT)}, ITEMS=${items}, TEX=${JSON.stringify(tgas())}, W=${s.W}, H=${s.H}, CAP=${JSON.stringify(s.cap + " " + CAPTION)};
const rgba=(c,a)=>'rgba('+Math.round(c[0]*255)+','+Math.round(c[1]*255)+','+Math.round(c[2]*255)+','+((c[3]??1)*(a??1))+')';
const images={};
for(const n in TEX){const t=TEX[n],bin=atob(t.d),px=new Uint8ClampedArray(bin.length);for(let i=0;i<bin.length;i++)px[i]=bin.charCodeAt(i);
  const cv=document.createElement('canvas');cv.width=t.w;cv.height=t.h;cv.getContext('2d').putImageData(new ImageData(px,t.w,t.h),0,0);images[n]=cv;}
const tmp=document.createElement('canvas');tmp.width=W;tmp.height=H;
function drawTex(ctx,it){
  const img=images[it.img]; if(!img)return;
  const iw=img.width, ih=img.height;
  let ul,ll,ur;
  const uv=it.uv;
  if(uv&&uv.length===8){ul=[uv[0],uv[1]];ll=[uv[2],uv[3]];ur=[uv[4],uv[5]];}
  else if(uv&&uv.length===4){ul=[uv[0],uv[2]];ll=[uv[0],uv[3]];ur=[uv[1],uv[2]];}
  else{ul=[0,0];ll=[0,1];ur=[1,0];}
  if(it.w<=0||it.h<=0)return;
  // the tinted texture on a scratch canvas: multiply by the vertex color, keep the art's alpha
  const x0=Math.floor(it.x)-1,y0=Math.floor(it.y)-1,tw=Math.ceil(it.w)+3,th=Math.ceil(it.h)+3;
  const t=tmp.getContext('2d');
  t.setTransform(1,0,0,1,0,0);t.globalCompositeOperation='source-over';t.clearRect(0,0,tw,th);
  // dest = o + (u-ul)/(ur-ul)*w ... : image px -> dest px
  const ax=(ur[0]-ul[0])*iw, ay=(ur[1]-ul[1])*ih, bx=(ll[0]-ul[0])*iw, by=(ll[1]-ul[1])*ih;
  const det=ax*by-ay*bx; if(Math.abs(det)<1e-9)return;
  // inverse of [[ax,bx],[ay,by]] maps image-px offsets to (s,t) in 0..1 on the dest quad
  const a=by/det*it.w, c=-bx/det*it.w, b=-ay/det*it.h, d=ax/det*it.h;
  const ox=it.x-x0-(a*ul[0]*iw+c*ul[1]*ih), oy=it.y-y0-(b*ul[0]*iw+d*ul[1]*ih);
  t.save();t.beginPath();t.rect(it.x-x0,it.y-y0,it.w,it.h);t.clip();
  t.setTransform(a,b,c,d,ox,oy);t.imageSmoothingEnabled=true;t.drawImage(img,0,0);t.restore();
  t.setTransform(1,0,0,1,0,0);
  t.globalCompositeOperation='multiply';t.fillStyle=rgba([it.c[0],it.c[1],it.c[2],1]);t.fillRect(0,0,tw,th);
  t.globalCompositeOperation='destination-in';
  t.save();t.beginPath();t.rect(it.x-x0,it.y-y0,it.w,it.h);t.clip();t.setTransform(a,b,c,d,ox,oy);t.drawImage(img,0,0);t.restore();
  ctx.beginPath();ctx.rect(it.x,it.y,it.w,it.h);ctx.clip();ctx.globalAlpha=(it.c[3]??1)*it.a;ctx.drawImage(tmp,0,0,tw,th,x0,y0,tw,th);ctx.globalAlpha=1;
}
function drawText(ctx,it){
  ctx.font=FONT[it.font].replace('SIZE',it.size);ctx.fontKerning='none';ctx.textBaseline='middle';
  for(const row of it.rows){let x=row.x;const y=row.y+it.lh/2+0.5;
    for(const run of row.r){
      const hex=run.c,col=hex?'#'+hex:rgba(it.c,it.a);
      if(it.sc&&it.sc[3]>0&&(it.sx||it.sy)){ctx.fillStyle=rgba(it.sc,it.a);ctx.fillText(run.t,x+it.sx,y-it.sy);}
      ctx.globalAlpha=hex?it.a:1;ctx.fillStyle=col;ctx.fillText(run.t,x,y);ctx.globalAlpha=1;
      x+=ctx.measureText(run.t).width;
    }}
}
(async()=>{
  for(const k in FONT){await document.fonts.load(FONT[k].replace('SIZE','12'),'Abc');}
  const ctx=document.getElementById('c').getContext('2d',{alpha:false});
  const g=ctx.createLinearGradient(0,0,W,H);g.addColorStop(0,'#27404c');g.addColorStop(.35,'#0f1a22');g.addColorStop(.7,'#101820');g.addColorStop(1,'#3a3423');
  ctx.fillStyle=g;ctx.fillRect(0,0,W,H);
  for(const it of ITEMS){
    if(it.t!=='text'){const x1=Math.round(it.x+it.w),y1=Math.round(it.y+it.h);it.x=Math.round(it.x);it.y=Math.round(it.y);it.w=x1-it.x;it.h=y1-it.y;}
    ctx.save();
    if(it.clip){ctx.beginPath();ctx.rect(it.clip[0],it.clip[1],it.clip[2],it.clip[3]);ctx.clip();}
    if(it.t==='rect'){ctx.fillStyle=rgba(it.c,it.a);ctx.fillRect(it.x,it.y,it.w,it.h);}
    else if(it.t==='tex')drawTex(ctx,it);
    else if(it.t==='text')drawText(ctx,it);
    ctx.restore();
  }
  ctx.font="400 12px Inter";ctx.textBaseline='alphabetic';ctx.fillStyle='rgba(255,255,255,.62)';ctx.fillText(CAP,24,H-24);
  document.title='done';
})();
</script>`;
  const file = join(work, name + ".html");
  writeFileSync(file, page);
  const out = join(resolve(outDir), name + ".png");
  chrome([`--window-size=${s.W},${s.H}`, `--screenshot=${out}`, pathToFileURL(file).href]);
  console.log(name, JSON.parse(items).length, "items ->", out);
}
