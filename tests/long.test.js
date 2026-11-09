// A command too long for one strip frame goes out in parts, drawn in turn until the
// companion acks it once.
import { test } from "node:test";
import assert from "node:assert/strict";
import { runGame } from "./game.js";

// A stand-in for the client's zlib: the output has a zlib header and is shorter than the
// input, and only comes back through DecompressString. Real zlib bytes are in contract.test.js.
const COMPRESSOR = `
Enum = { CompressionMethod = { Deflate = 0, Zlib = 1, Gzip = 2 } }
local packed = {}
compressCalls = 0
C_EncodingUtil = {
  CompressString = function(text, method)
    assert(method == 1, "not asked for zlib")
    compressCalls = compressCalls + 1
    local out = "\\120\\156" .. text:sub(1, math.floor(#text / 3))
    packed[out] = text
    return out
  end,
  DecompressString = function(out, method)
    assert(method == 1, "not asked for zlib")
    return packed[out]
  end,
}`;

const SETUP = `
notices = {}
ns.UI.Notice = function(text) notices[#notices + 1] = text end
frames_drawn = {}
local encodePart = ns.Codec.EncodePart
ns.Codec.EncodePart = function(seq, header, chunk)
  frames_drawn[#frames_drawn + 1] = { header = header, chunk = chunk }
  return encodePart(seq, header, chunk)
end
local json = ns.Codec.Json
plain_frames = {}
ns.Codec.Json = function(value)
  local out = json(value)
  if type(value) == "table" and value.k then plain_frames[#plain_frames + 1] = out end
  return out
end

-- Hands the game an update from a companion that takes long commands (or doesn't).
function update(extra)
  local name = string.format("T3WoW_S%03d", Link.Status().nextSlot)
  local inbox = { k = "c1", u = Link.Session(), ack = 0 }
  for key, value in pairs(extra) do inbox[key] = value end
  inboxes[name] = inbox
  Link.state.pollAt = 0
  tick(0.25)
end

-- The parts drawn so far, by number, and what they join into.
function joined()
  local byIndex, count = {}, 0
  for _, f in ipairs(frames_drawn) do
    local i = tonumber(f.header:match('"i":(%d+)'))
    count = tonumber(f.header:match('"n":(%d+),"s"')) or count
    byIndex[i] = f.chunk
  end
  local out = {}
  for i = 1, count do out[i] = assert(byIndex[i], "part " .. i .. " was never drawn") end
  return table.concat(out), count
end

-- Text that doesn't repeat, with multi-byte letters all through it.
function longText(bytes)
  local words, seed = {}, 7
  local size = 0
  while size < bytes do
    seed = (seed * 1103 + 12345) % 65536
    local word = string.format("%xñ€%x", seed, seed % 97)
    words[#words + 1] = word
    size = size + #word + 1
  end
  return table.concat(words, " ")
end
`;

const run = (script, options = {}) => runGame(`${SETUP}\nLink.Init()\n${script}`, options);

test("a long message is cut into parts, drawn in turn, and stops at the ack", () => {
  const out = run(`
update({ lm = 1 })
local text = longText(9000)
local command = { t = "send", e = "", id = "t1", m = text }
assert(Link.Fits(command), "a long message doesn't fit")
-- The update above also acks nothing: hello is still first, so ack it to reach the message.
local s = Link.Send(command)
update({ lm = 1, ack = s - 1 })
assert(#frames_drawn > 0, "no part was drawn")
frames_drawn = { frames_drawn[#frames_drawn] }
tick(6)
local body, count = joined()
assert(count >= 2, "it took " .. count .. " part(s)")
assert(body == ns.Codec.Json({ e = "", id = "t1", m = text, s = s, t = "send" }), "the parts don't join into the command")
-- Every part fits in a frame, with its header.
for _, f in ipairs(frames_drawn) do
  assert(#f.header + 1 + #f.chunk <= ns.Codec.MAX_PAYLOAD, "too big for a frame")
end
local before = #frames_drawn
update({ lm = 1, ack = s })
assert(Link.Pending() == 0, "still queued after the ack")
tick(3)
assert(#frames_drawn == before, "parts still drawn after the ack")
return "ok"`);
  assert.equal(out, "ok");
});

test("multi-byte text survives a cut through the middle of a letter", () => {
  const out = run(`
update({ lm = 1 })
local text = string.rep("€", 2000)
local command = { t = "send", id = "t1", m = text }
local s = Link.Send(command)
update({ lm = 1, ack = s - 1 })
frames_drawn = { frames_drawn[#frames_drawn] }
tick(6)
local body = joined()
assert(body == ns.Codec.Json({ id = "t1", m = text, s = s, t = "send" }), "not the same bytes")
return "ok"`);
  assert.equal(out, "ok");
});

test("a long message is compressed when that's smaller, and only once", () => {
  const out = run(`
update({ lm = 1 })
local s = Link.Send({ t = "send", id = "t1", m = longText(9000) })
update({ lm = 1, ack = s - 1 })
tick(6)
local last = frames_drawn[#frames_drawn].header
assert(last:find('"z":1', 1, true), last)
assert(compressCalls == 1, "compressed " .. compressCalls .. " times")
return "ok"`, { before: COMPRESSOR });
  assert.equal(out, "ok");
});

test("short commands stay in plain frames and are never compressed", () => {
  const out = run(`
update({ lm = 1 })
local s = Link.Send({ t = "send", id = "t1", m = "hello" })
update({ lm = 1, ack = s - 1 })
assert(#frames_drawn == 0, "a short command went out in parts")
assert(compressCalls == 0, "a short command was compressed")
assert(plain_frames[#plain_frames]:find('"m":"hello"', 1, true), plain_frames[#plain_frames])
return "ok"`, { before: COMPRESSOR });
  assert.equal(out, "ok");
});

for (const [name, before] of [
  ["missing", ""],
  ["without the zlib method", `Enum = {}; C_EncodingUtil = { CompressString = function() error("no") end, DecompressString = function() end }`],
  ["failing", `Enum = { CompressionMethod = { Zlib = 1 } }; C_EncodingUtil = { CompressString = function() error("no") end, DecompressString = function() end }`],
  ["not giving zlib", `Enum = { CompressionMethod = { Zlib = 1 } }; C_EncodingUtil = { CompressString = function(t) return t:sub(1, 5) end, DecompressString = function(t) return t end }`],
  ["not giving it back", `Enum = { CompressionMethod = { Zlib = 1 } }; C_EncodingUtil = { CompressString = function(t) return "\\120\\156" .. t:sub(1, 9) end, DecompressString = function() return "other" end }`],
  ["growing the text", `Enum = { CompressionMethod = { Zlib = 1 } }; C_EncodingUtil = { CompressString = function(t) return "\\120\\156" .. t end, DecompressString = function(t) return t:sub(3) end }`],
]) {
  test(`a client with C_EncodingUtil ${name} sends uncompressed`, () => {
    const out = run(`
update({ lm = 1 })
local text = longText(9000)
local s = Link.Send({ t = "send", id = "t1", m = text })
update({ lm = 1, ack = s - 1 })
frames_drawn = { frames_drawn[#frames_drawn] }
tick(6)
local body = joined()
assert(not frames_drawn[1].header:find('"z"', 1, true), frames_drawn[1].header)
assert(body == ns.Codec.Json({ id = "t1", m = text, s = s, t = "send" }), "not the same bytes")
return "ok"`, { before });
    assert.equal(out, "ok");
  });
}

test("a message past the limit doesn't fit", () => {
  const out = run(`
update({ lm = 1 })
local fits, why = Link.Fits({ t = "send", id = "t1", m = longText(70000) })
assert(fits == false and why == nil, tostring(fits) .. " " .. tostring(why))
return "ok"`);
  assert.equal(out, "ok");
});

test("an older companion gets no parts, and the window says what to update", () => {
  const out = run(`
update({ })
local fits, why = Link.Fits({ t = "send", id = "t1", m = longText(9000) })
assert(fits == false, "fits on a companion that can't take it")
assert(why:find("companion", 1, true), why)
-- A message that fits one frame still goes.
assert(Link.Fits({ t = "send", id = "t1", m = "hello" }))
-- One already queued (saved before the companion was rolled back) goes with the same words.
local s = Link.Send({ t = "send", id = "t1", m = longText(9000) })
update({ ack = s - 1 })
assert(#frames_drawn == 0, "drew parts for a companion that can't read them")
assert(Link.Pending() == 0, "stayed in the queue")
assert(notices[#notices]:find("companion", 1, true), notices[#notices] or "no notice")
return "ok"`);
  assert.equal(out, "ok");
});

test("a saved long message waits to hear from the companion, then goes", () => {
  const json = "HELD";
  const out = runGame(`${SETUP}
local text = longText(9000)
T3WoWDB.queue = { { t = "send", id = "t1", m = text, s = 5 } }
T3WoWDB.seq = 5
Link.Init()
assert(#frames_drawn == 0, "drew parts before knowing the companion takes them")
assert(Link.Pending() == 2, "the saved message was dropped")
assert(#notices == 0, notices[1])
-- The header-only frame tells the companion this UI session is here.
assert(not plain_frames[#plain_frames]:find('"q"', 1, true), plain_frames[#plain_frames])
-- And which command waits, so the companion's first answer can't ack it away unsent.
assert(plain_frames[#plain_frames]:find('"pt":{"s":5}', 1, true), plain_frames[#plain_frames])
update({ lm = 1 })
assert(#frames_drawn > 0, "nothing drawn once the companion said so")
tick(6)
local body = joined()
assert(body == ns.Codec.Json({ id = "t1", m = text, s = 5, t = "send" }), "not the same bytes")
return "${json}"`);
  assert.equal(out, json);
});

test("a long message in flight survives a Reload UI", () => {
  const out = run(`
update({ lm = 1 })
local text = longText(9000)
local s = Link.Send({ t = "send", id = "t1", m = text })
update({ lm = 1, ack = s - 1 })
fire("PLAYER_LOGOUT")
assert(#T3WoWDB.queue == 1 and T3WoWDB.queue[1].m == text, "the message wasn't saved")
return "ok"`);
  assert.equal(out, "ok");
});

test("the wait for an answer starts once every part has been shown", () => {
  const out = run(`
update({ lm = 1 })
local s = Link.Send({ t = "send", id = "t1", m = longText(30000) })
update({ lm = 1, ack = s - 1 })
local _, count = (function() tick(0.25 * 40); return joined() end)()
assert(count >= 8, "only " .. count .. " parts")
tick(0.5)
assert(not Link.Stalled(), "counted as stalled while parts were still going round")
return "ok"`);
  assert.equal(out, "ok");
});
