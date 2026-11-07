# Reads the T3WoW pixel strip from the top-left of the WoW client area and prints
# one JSON line per change. Decoding and checksums happen in Node (codec.js).
# It never sends input to the game and never reads its memory.
param(
  [string]$ProcessName = "Wow",
  [string]$ClientFolder = "",
  [string]$MagicCells = "",
  [int]$IntervalMs = 250,
  [string]$ImageFile = ""
)

$ErrorActionPreference = "Stop"
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;

public static class T3Cap {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);

  public const int CellPx = 4, PerRow = 200, MaxRows = 40, Search = 12;

  public static void Init() { SetProcessDPIAware(); }

  // null: window not capturable; "": no strip; otherwise cell digits.
  public static string SampleWindow(IntPtr hwnd, string magic) {
    if (hwnd == IntPtr.Zero || IsIconic(hwnd)) return null;
    RECT r; if (!GetClientRect(hwnd, out r)) return null;
    POINT p = new POINT(); if (!ClientToScreen(hwnd, ref p)) return null;
    int w = Math.Min(r.Right - r.Left, PerRow * CellPx + Search + 4);
    int h = Math.Min(r.Bottom - r.Top, MaxRows * CellPx + Search + 4);
    if (w < 64 || h < 16) return null;
    using (Bitmap bmp = new Bitmap(w, h, PixelFormat.Format32bppArgb)) {
      using (Graphics g = Graphics.FromImage(bmp)) {
        g.CopyFromScreen(p.X, p.Y, 0, 0, new Size(w, h), CopyPixelOperation.SourceCopy);
      }
      return Sample(bmp, magic);
    }
  }

  public static string SampleFile(string path, string magic) {
    using (Bitmap src = new Bitmap(path))
    using (Bitmap bmp = src.Clone(new Rectangle(0, 0, src.Width, src.Height), PixelFormat.Format32bppArgb)) {
      return Sample(bmp, magic);
    }
  }

  static string Sample(Bitmap bmp, string magic) {
    int w = bmp.Width, h = bmp.Height;
    BitmapData data = bmp.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
    byte[] px = new byte[data.Stride * h];
    Marshal.Copy(data.Scan0, px, 0, px.Length);
    int stride = data.Stride;
    bmp.UnlockBits(data);
    for (int dy = 0; dy <= Search; dy++) {
      for (int dx = 0; dx <= Search; dx++) {
        bool match = true;
        for (int i = 0; i < magic.Length && match; i++) {
          match = Cell(px, stride, w, h, dx + i * CellPx + CellPx / 2, dy + CellPx / 2) == magic[i] - '0';
        }
        if (!match) continue;
        // Bytes 5..6 hold the payload length; read just the cells the frame needs so
        // the game scene behind the strip doesn't change the output.
        int need = 19;
        StringBuilder sb = new StringBuilder();
        for (int n = 0; n < need; n++) {
          int row = n / PerRow, col = n % PerRow;
          int v = Cell(px, stride, w, h, dx + col * CellPx + CellPx / 2, dy + row * CellPx + CellPx / 2);
          if (v < 0) return "";
          sb.Append((char)('0' + v));
          if (n == 18) {
            long acc = 0;
            for (int k = 0; k < 19; k++) acc = acc * 8 + (sb[k] - '0');
            acc = acc / 2; // 57 bits read, 56 are the 7 header bytes
            int length = (int)(acc % 65536);
            need = ((7 + length + 2) * 8 + 2) / 3;
            if (need > PerRow * MaxRows) return "";
          }
        }
        return sb.ToString();
      }
    }
    return "";
  }

  static int Cell(byte[] px, int stride, int w, int h, int x, int y) {
    if (x >= w || y >= h) return -1;
    int i = y * stride + x * 4;
    int b = px[i] > 127 ? 1 : 0, g = px[i + 1] > 127 ? 1 : 0, rr = px[i + 2] > 127 ? 1 : 0;
    return rr * 4 + g * 2 + b;
  }
}
"@

function Emit($object) {
  [Console]::Out.WriteLine(($object | ConvertTo-Json -Compress -Depth 4))
  [Console]::Out.Flush()
}

[T3Cap]::Init()

if ($ImageFile) {
  Emit @{ type = "strip"; cells = [T3Cap]::SampleFile($ImageFile, $MagicCells) }
  exit 0
}

$lastWow = "unknown"
$lastCells = ""
while ($true) {
  try {
    $proc = Get-Process $ProcessName -ErrorAction SilentlyContinue | Where-Object {
      $_.MainWindowHandle -ne [IntPtr]::Zero -and (-not $ClientFolder -or ($_.Path -and $_.Path.StartsWith($ClientFolder, [StringComparison]::OrdinalIgnoreCase)))
    } | Select-Object -First 1
    $wowKey = if ($proc) { "$($proc.Id)@$($proc.StartTime.ToFileTimeUtc())" } else { "" }
    if ($wowKey -ne $lastWow) {
      $lastWow = $wowKey
      Emit @{ type = "wow"; key = $wowKey }
    }
    if ($proc) {
      $cells = [T3Cap]::SampleWindow($proc.MainWindowHandle, $MagicCells)
      if ($null -ne $cells -and $cells -ne $lastCells) {
        $lastCells = $cells
        if ($cells -ne "") { Emit @{ type = "strip"; cells = $cells } }
      }
    }
  } catch {
    Emit @{ type = "error"; message = "$($_.Exception.Message)" }
  }
  Start-Sleep -Milliseconds $IntervalMs
}
