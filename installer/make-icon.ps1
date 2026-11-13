# Draws the D200X Configurator icon (a deck of keys, some lit, on a dark
# tile) and writes installer\d200x.ico with 16-256 px images (PNG-compressed).

Add-Type -AssemblyName System.Drawing
$ErrorActionPreference = 'Stop'

function New-RoundedRect([float]$x, [float]$y, [float]$w, [float]$h, [float]$r) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = 2 * $r
    $p.AddArc($x, $y, $d, $d, 180, 90)
    $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    $p
}

function New-IconBitmap([int]$s) {
    $bmp = New-Object System.Drawing.Bitmap $s, $s, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.Clear([System.Drawing.Color]::Transparent)
    $k = $s / 256.0

    # dark tile
    $g.FillPath((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 28, 31, 40))), (New-RoundedRect 0 0 ($s - 1) ($s - 1) (52 * $k)))

    # deck body
    $g.FillPath((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 52, 57, 70))), (New-RoundedRect (26 * $k) (40 * $k) (204 * $k) (176 * $k) (22 * $k)))

    # 4 x 3 keys, a few lit
    $lit = @{
        '0,0' = [System.Drawing.Color]::FromArgb(255, 80, 170, 255)
        '2,0' = [System.Drawing.Color]::FromArgb(255, 255, 170, 60)
        '1,1' = [System.Drawing.Color]::FromArgb(255, 120, 220, 120)
        '3,1' = [System.Drawing.Color]::FromArgb(255, 240, 90, 120)
        '0,2' = [System.Drawing.Color]::FromArgb(255, 180, 120, 255)
        '3,2' = [System.Drawing.Color]::FromArgb(255, 80, 210, 210)
    }
    $off = [System.Drawing.Color]::FromArgb(255, 86, 92, 108)
    $kw = 40; $gap = 9; $x0 = 128 - (4 * $kw + 3 * $gap) / 2; $y0 = 128 - (3 * $kw + 2 * $gap) / 2
    for ($r = 0; $r -lt 3; $r++) {
        for ($c = 0; $c -lt 4; $c++) {
            $col = $lit["$c,$r"]; if (-not $col) { $col = $off }
            $x = ($x0 + $c * ($kw + $gap)) * $k
            $y = ($y0 + $r * ($kw + $gap)) * $k
            $g.FillPath((New-Object System.Drawing.SolidBrush $col), (New-RoundedRect $x $y ($kw * $k) ($kw * $k) (9 * $k)))
        }
    }
    $g.Dispose()
    return $bmp
}

$sizes = 256, 64, 48, 32, 16
$pngs = foreach ($s in $sizes) {
    $bmp = New-IconBitmap $s
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    , $ms.ToArray()
}

$out = Join-Path $PSScriptRoot 'd200x.ico'
$fs = [System.IO.File]::Create($out)
$w = New-Object System.IO.BinaryWriter $fs
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]
    $w.Write([byte]($(if ($s -ge 256) { 0 } else { $s })))
    $w.Write([byte]($(if ($s -ge 256) { 0 } else { $s })))
    $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32)
    $w.Write([uint32]$pngs[$i].Length); $w.Write([uint32]$offset)
    $offset += $pngs[$i].Length
}
foreach ($p in $pngs) { $w.Write($p) }
$w.Close()
Write-Host "wrote $out"
