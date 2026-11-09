-- Lua 5.1, no bit library: everything is plain arithmetic so the same file runs in tests.
local _, ns = ...
ns = ns or {}
local Codec = {}
ns.Codec = Codec

Codec.MAGIC1, Codec.MAGIC2, Codec.VERSION = 0x54, 0x33, 1
Codec.CELL_PX, Codec.CELLS_PER_ROW, Codec.MAX_ROWS = 4, 200, 40
-- 3 bits per cell; header 7 bytes + checksum 2 bytes.
Codec.MAX_PAYLOAD = math.floor(Codec.CELLS_PER_ROW * Codec.MAX_ROWS * 3 / 8) - 9
-- A frame of version 2 carries one part of a command too long for a frame: its payload is
-- the usual JSON header, a zero byte (JSON never holds one), then the part's raw bytes.
-- Keep the limits in step with companion/src/bridge.rs.
Codec.VERSION_PART = 2
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
    if length > Codec.MAX_PAYLOAD then return nil end
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

function Codec.Encode(seq, payload)
    local bytes = Codec.Frame(seq, payload)
    return bytes and Codec.Cells(bytes) or nil
end

function Codec.EncodePart(seq, header, part)
    local bytes = Codec.Frame(seq, header .. "\0" .. part, Codec.VERSION_PART)
    return bytes and Codec.Cells(bytes) or nil
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
