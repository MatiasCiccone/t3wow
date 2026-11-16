-- Lua 5.1, no bit library: everything is plain arithmetic so the same file runs in tests.
local _, ns = ...
ns = ns or {}
local Codec = {}
ns.Codec = Codec

Codec.MAGIC1, Codec.MAGIC2, Codec.VERSION = 0x54, 0x33, 1
Codec.CELL_PX, Codec.CELLS_PER_ROW, Codec.MAX_ROWS = 4, 200, 40
-- A signed frame's payload starts with this many bytes of its signature (see Codec.Sign).
Codec.TAG_BYTES = 8
-- 3 bits per cell; header 7 bytes + checksum 2 bytes. MAX_BODY is what a frame holds after
-- its header; MAX_PAYLOAD leaves room in it for a signature.
Codec.MAX_BODY = math.floor(Codec.CELLS_PER_ROW * Codec.MAX_ROWS * 3 / 8) - 9
Codec.MAX_PAYLOAD = Codec.MAX_BODY - Codec.TAG_BYTES
-- A frame of version 2 carries one part of a command too long for a frame: its payload is
-- the usual JSON header, a zero byte (JSON never holds one), then the part's raw bytes.
-- Versions 3 and 4 are versions 1 and 2 signed: the payload starts with the signature.
-- Keep the limits in step with companion/src/bridge.rs.
Codec.VERSION_PART = 2
Codec.VERSION_SIGNED, Codec.VERSION_PART_SIGNED = 3, 4
Codec.PART_BYTES = Codec.MAX_PAYLOAD - 400 -- leaves room for the header
Codec.MAX_PARTS = 16
Codec.MAX_LONG_JSON = 64000

local floor, byte, char, format = math.floor, string.byte, string.char, string.format

local function escapeString(value)
    return '"' .. value:gsub('[%c"\\]', function(c)
        if c == '"' then return '\\"' end
        if c == "\\" then return "\\\\" end
        if c == "\n" then return "\\n" end
        if c == "\r" then return "\\r" end
        if c == "\t" then return "\\t" end
        return format("\\u%04x", byte(c))
    end) .. '"'
end

local function isArray(value)
    local count = 0
    for _ in pairs(value) do count = count + 1 end
    return count == #value
end

function Codec.Json(value)
    local kind = type(value)
    if kind == "string" then return escapeString(value) end
    if kind == "number" then
        if value ~= value or value == math.huge or value == -math.huge then return "null" end
        if value == floor(value) then return format("%d", value) end
        return format("%.6f", value)
    end
    if kind == "boolean" then return value and "true" or "false" end
    if kind ~= "table" then return "null" end
    local parts = {}
    if next(value) ~= nil and isArray(value) then
        for index = 1, #value do parts[index] = Codec.Json(value[index]) end
        return "[" .. table.concat(parts, ",") .. "]"
    end
    local keys = {}
    for key in pairs(value) do keys[#keys + 1] = tostring(key) end
    table.sort(keys)
    for index, key in ipairs(keys) do
        parts[index] = escapeString(key) .. ":" .. Codec.Json(value[key])
    end
    return "{" .. table.concat(parts, ",") .. "}"
end

function Codec.Fletcher16(bytes, first, last)
    local s1, s2 = 0, 0
    for index = first, last do
        s1 = (s1 + bytes[index]) % 255
        s2 = (s2 + s1) % 255
    end
    return s1, s2
end

--- Frame bytes: 'T' '3' version seqHi seqLo lenHi lenLo payload... sum1 sum2
function Codec.Frame(seq, payload, version)
    local length = #payload
    if length > Codec.MAX_BODY then return nil end
    seq = seq % 65536
    local bytes = { Codec.MAGIC1, Codec.MAGIC2, version or Codec.VERSION,
        floor(seq / 256), seq % 256, floor(length / 256), length % 256 }
    for index = 1, length do bytes[#bytes + 1] = byte(payload, index) end
    local s1, s2 = Codec.Fletcher16(bytes, 3, #bytes)
    bytes[#bytes + 1] = s1
    bytes[#bytes + 1] = s2
    return bytes
end

--- Packs bytes MSB-first into 3-bit cell values (0..7); the last cell is zero-padded.
function Codec.Cells(bytes)
    local cells, accumulator, bits = {}, 0, 0
    for index = 1, #bytes do
        accumulator, bits = accumulator * 256 + bytes[index], bits + 8
        while bits >= 3 do
            bits = bits - 3
            local divisor = 2 ^ bits
            cells[#cells + 1] = floor(accumulator / divisor)
            accumulator = accumulator % divisor
        end
    end
    if bits > 0 then cells[#cells + 1] = accumulator * 2 ^ (3 - bits) end
    return cells
end

--- Cell value -> r, g, b in {0, 1}. Pure primaries survive gamma and contrast settings.
function Codec.Color(value)
    return floor(value / 4) % 2, floor(value / 2) % 2, value % 2
end

-- ---------------------------------------------------------------- signatures
-- Any addon can draw a strip, so with the key the companion wrote for this install (see
-- Key.xml) every frame carries the first TAG_BYTES of an HMAC-SHA256 over its version byte
-- and payload, and the companion follows only signed frames for anything that changes T3.
-- 32-bit words are plain numbers (0 to 2^32 - 1); AND and XOR go a nibble at a time
-- through these tables. The game's numbers are all doubles, but fengari (where the tests
-- run) has 32-bit integers, so words stay floats here: the constants are written in
-- decimal (fengari reads a hex literal past 2^31 as a negative integer) and products
-- start from a float.
local TWO32 = 4294967296
local XOR4, AND4 = {}, {}
for a = 0, 15 do
    for b = 0, 15 do
        local x, n, bit, aa, bb = 0, 0, 1, a, b
        for _ = 1, 4 do
            local abit, bbit = aa % 2, bb % 2
            if abit ~= bbit then x = x + bit elseif abit == 1 then n = n + bit end
            aa, bb, bit = (aa - abit) / 2, (bb - bbit) / 2, bit * 2
        end
        XOR4[a * 16 + b], AND4[a * 16 + b] = x, n
    end
end

local function bxor(a, b)
    local out, scale = 0, 1.0
    for _ = 1, 8 do
        local na, nb = a % 16, b % 16
        out = out + XOR4[na * 16 + nb] * scale
        a, b, scale = (a - na) / 16, (b - nb) / 16, scale * 16
    end
    return out
end

local function bxor3(a, b, c)
    local out, scale = 0, 1.0
    for _ = 1, 8 do
        local na, nb, nc = a % 16, b % 16, c % 16
        out = out + XOR4[XOR4[na * 16 + nb] * 16 + nc] * scale
        a, b, c, scale = (a - na) / 16, (b - nb) / 16, (c - nc) / 16, scale * 16
    end
    return out
end

local function band(a, b)
    local out, scale = 0, 1.0
    for _ = 1, 8 do
        local na, nb = a % 16, b % 16
        out = out + AND4[na * 16 + nb] * scale
        a, b, scale = (a - na) / 16, (b - nb) / 16, scale * 16
    end
    return out
end

local function rotr(x, n)
    local low = x % 2 ^ n
    return (x - low) / 2 ^ n + low * 2 ^ (32 - n)
end

local function shr(x, n)
    return (x - x % 2 ^ n) / 2 ^ n
end

-- SHA-256's round constants and starting hash, in decimal (see above).
local K = {
    1116352408, 1899447441, 3049323471, 3921009573, 961987163, 1508970993, 2453635748, 2870763221,
    3624381080, 310598401, 607225278, 1426881987, 1925078388, 2162078206, 2614888103, 3248222580,
    3835390401, 4022224774, 264347078, 604807628, 770255983, 1249150122, 1555081692, 1996064986,
    2554220882, 2821834349, 2952996808, 3210313671, 3336571891, 3584528711, 113926993, 338241895,
    666307205, 773529912, 1294757372, 1396182291, 1695183700, 1986661051, 2177026350, 2456956037,
    2730485921, 2820302411, 3259730800, 3345764771, 3516065817, 3600352804, 4094571909, 275423344,
    430227734, 506948616, 659060556, 883997877, 958139571, 1322822218, 1537002063, 1747873779,
    1955562222, 2024104815, 2227730452, 2361852424, 2428436474, 2756734187, 3204031479, 3329325298,
}
local H0 = { 1779033703, 3144134277, 1013904242, 2773480762, 1359893119, 2600822924, 528734635, 1541459225 }

local function compress(h, bytes, at)
    local w = {}
    for t = 1, 16 do
        local i = at + (t - 1) * 4
        w[t] = bytes[i] * 16777216.0 + bytes[i + 1] * 65536 + bytes[i + 2] * 256 + bytes[i + 3]
    end
    for t = 17, 64 do
        local x, y = w[t - 15], w[t - 2]
        w[t] = (w[t - 16] + bxor3(rotr(x, 7), rotr(x, 18), shr(x, 3)) + w[t - 7] + bxor3(rotr(y, 17), rotr(y, 19), shr(y, 10))) % TWO32
    end
    local a, b, c, d, e, f, g, hh = h[1], h[2], h[3], h[4], h[5], h[6], h[7], h[8]
    for t = 1, 64 do
        local t1 = (hh + bxor3(rotr(e, 6), rotr(e, 11), rotr(e, 25)) + bxor(g, band(e, bxor(f, g))) + K[t] + w[t]) % TWO32
        local t2 = bxor3(rotr(a, 2), rotr(a, 13), rotr(a, 22)) + bxor(band(a, b), band(c, bxor(a, b)))
        a, b, c, d, e, f, g, hh = (t1 + t2) % TWO32, a, b, c, (d + t1) % TWO32, e, f, g
    end
    h[1], h[2], h[3], h[4] = (h[1] + a) % TWO32, (h[2] + b) % TWO32, (h[3] + c) % TWO32, (h[4] + d) % TWO32
    h[5], h[6], h[7], h[8] = (h[5] + e) % TWO32, (h[6] + f) % TWO32, (h[7] + g) % TWO32, (h[8] + hh) % TWO32
end

--- SHA-256 of a string, as a 32-byte string.
function Codec.Sha256(text)
    local bytes, length = {}, #text
    for i = 1, length do bytes[i] = byte(text, i) end
    bytes[length + 1] = 0x80
    while (#bytes + 8) % 64 ~= 0 do bytes[#bytes + 1] = 0 end
    local bits = length * 8
    for i = 7, 0, -1 do bytes[#bytes + 1] = floor(bits / 2 ^ (i * 8)) % 256 end
    local h = { H0[1], H0[2], H0[3], H0[4], H0[5], H0[6], H0[7], H0[8] }
    for at = 1, #bytes, 64 do compress(h, bytes, at) end
    local out = {}
    for i = 1, 8 do
        local word = h[i]
        out[i] = char(floor(word / 16777216), floor(word / 65536) % 256, floor(word / 256) % 256, word % 256)
    end
    return table.concat(out)
end

--- HMAC-SHA256 of `text` with `key` (a string of at most 64 bytes), as a 32-byte string.
function Codec.HmacSha256(key, text)
    local inner, outer = {}, {}
    for i = 1, 64 do
        local k = byte(key, i) or 0
        local high, low = floor(k / 16), k % 16
        inner[i] = char(XOR4[high * 16 + 3] * 16 + XOR4[low * 16 + 6]) -- k xor 0x36
        outer[i] = char(XOR4[high * 16 + 5] * 16 + XOR4[low * 16 + 12]) -- k xor 0x5c
    end
    return Codec.Sha256(table.concat(outer) .. Codec.Sha256(table.concat(inner) .. text))
end

--- The key Key.lua holds as 64 hex digits, as 32 bytes; nil if it isn't one.
function Codec.KeyFromHex(hex)
    if type(hex) ~= "string" or #hex ~= 64 or hex:find("[^%x]") then return nil end
    return (hex:gsub("%x%x", function(pair) return char(tonumber(pair, 16)) end))
end

-- Signatures of the last few frames drawn: the strip is drawn again often with the same
-- payload (and the parts of a long command take turns), and each signature takes a while.
local tags, tagCount = {}, 0

--- The signature a frame of `version` with `payload` carries.
function Codec.Sign(key, version, payload)
    local message = char(version) .. payload
    local tag = tags[message]
    if not tag then
        if tagCount >= 32 then tags, tagCount = {}, 0 end
        tag = Codec.HmacSha256(key, message):sub(1, Codec.TAG_BYTES)
        tags[message], tagCount = tag, tagCount + 1
    end
    return tag
end

-- With `key`, a signed frame (version 3 or 4); without, the frame an addon from before
-- signing drew.
local function encode(seq, payload, version, signedVersion, key)
    if key then payload, version = Codec.Sign(key, signedVersion, payload) .. payload, signedVersion end
    local bytes = Codec.Frame(seq, payload, version)
    return bytes and Codec.Cells(bytes) or nil
end

function Codec.Encode(seq, payload, key)
    return encode(seq, payload, Codec.VERSION, Codec.VERSION_SIGNED, key)
end

function Codec.EncodePart(seq, header, part, key)
    return encode(seq, header .. "\0" .. part, Codec.VERSION_PART, Codec.VERSION_PART_SIGNED, key)
end

--- `text` zlib-compressed by the client, or nil when that isn't smaller or can't be trusted:
--- the API is missing on older clients, and the companion can only read zlib.
function Codec.Zlib(text)
    local api, method = C_EncodingUtil, Enum and Enum.CompressionMethod and Enum.CompressionMethod.Zlib
    if not (api and api.CompressString and api.DecompressString and method) then return nil end
    local ok, out = pcall(api.CompressString, text, method)
    if not ok or type(out) ~= "string" or #out < 2 or #out >= #text then return nil end
    local first, second = byte(out, 1, 2)
    if first % 16 ~= 8 or (first * 256 + second) % 31 ~= 0 then return nil end
    local same, back = pcall(api.DecompressString, out, method)
    if not same or back ~= text then return nil end
    return out
end

--- A command's JSON as parts of at most PART_BYTES (cut anywhere: the companion joins
--- the bytes before reading them), and whether they are compressed; nil if it's too long.
function Codec.Split(json)
    if #json > Codec.MAX_LONG_JSON then return nil end
    local compressed = Codec.Zlib(json)
    local body = compressed or json
    local count = math.ceil(#body / Codec.PART_BYTES)
    if count > Codec.MAX_PARTS then return nil end
    local parts = {}
    for index = 1, count do
        parts[index] = body:sub((index - 1) * Codec.PART_BYTES + 1, index * Codec.PART_BYTES)
    end
    return parts, compressed ~= nil
end

return Codec
