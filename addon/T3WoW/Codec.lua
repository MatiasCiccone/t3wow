-- Lua 5.1, no bit library: everything is plain arithmetic so the same file runs in tests.
local _, ns = ...
ns = ns or {}
local Codec = {}
ns.Codec = Codec

Codec.MAGIC1, Codec.MAGIC2, Codec.VERSION = 0x54, 0x33, 1
Codec.CELL_PX, Codec.CELLS_PER_ROW, Codec.MAX_ROWS = 4, 200, 40
-- 3 bits per cell; header 7 bytes + checksum 2 bytes.
Codec.MAX_PAYLOAD = math.floor(Codec.CELLS_PER_ROW * Codec.MAX_ROWS * 3 / 8) - 9

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
function Codec.Frame(seq, payload)
    local length = #payload
    if length > Codec.MAX_PAYLOAD then return nil end
    seq = seq % 65536
    local bytes = { Codec.MAGIC1, Codec.MAGIC2, Codec.VERSION,
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

return Codec
