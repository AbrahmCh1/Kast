# Genera assets/kast.ico (PNG multi-tamaño) y assets/kast-256.png con System.Drawing.
# Cada tamaño se dibuja a 4x y se reduce, con una versión simplificada para 16-24 px.
Add-Type -AssemblyName System.Drawing

function RoundRect([float]$x, [float]$y, [float]$w, [float]$h, [float]$r) {
    $p = New-Object Drawing.Drawing2D.GraphicsPath
    $d = $r * 2
    $p.AddArc($x, $y, $d, $d, 180, 90)
    $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    $p
}

function Draw([int]$size) {
    $S = $size * 4
    $u = $S / 64.0                       # diseño en una rejilla de 64
    $small = $size -le 24
    $bmp = New-Object Drawing.Bitmap $S, $S
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.PixelOffsetMode = 'HighQuality'

    # baldosa con degradado
    $m = if ($small) { 1 * $u } else { 3 * $u }
    $tile = RoundRect $m $m ($S - 2 * $m) ($S - 2 * $m) (14 * $u)
    $rect = New-Object Drawing.RectangleF 0, 0, $S, $S
    $grad = New-Object Drawing.Drawing2D.LinearGradientBrush $rect, ([Drawing.Color]::FromArgb(255, 47, 128, 255)), ([Drawing.Color]::FromArgb(255, 142, 84, 247)), 45.0
    $g.FillPath($grad, $tile)
    # brillo superior
    $g.SetClip($tile)
    $hi = New-Object Drawing.Drawing2D.LinearGradientBrush $rect, ([Drawing.Color]::FromArgb(70, 255, 255, 255)), ([Drawing.Color]::FromArgb(0, 255, 255, 255)), 90.0
    $g.FillRectangle($hi, 0, 0, $S, $S * 0.55)
    $g.ResetClip()
    if (-not $small) {
        $edge = New-Object Drawing.Pen ([Drawing.Color]::FromArgb(60, 255, 255, 255)), (1.2 * $u)
        $g.DrawPath($edge, $tile)
    }

    $white = [Drawing.Color]::White
    $sw = if ($small) { 5.5 * $u } else { 4 * $u }
    $pen = New-Object Drawing.Pen $white, $sw
    $pen.LineJoin = 'Round'; $pen.StartCap = 'Round'; $pen.EndCap = 'Round'
    $fill = New-Object Drawing.SolidBrush $white

    $cx = 32 * $u
    if ($small) {
        # silueta rellena con el símbolo recortado en el color de la baldosa: legible a 16-24 px
        $mx = 9; $my = 13; $mw = 46; $mh = 30
        $screen = RoundRect ($mx * $u) ($my * $u) ($mw * $u) ($mh * $u) (5 * $u)
        $g.FillPath($fill, $screen)
        $by = ($my + $mh) * $u
        $g.FillRectangle($fill, $cx - 3.5 * $u, $by, 7 * $u, 6 * $u)
        $g.FillPath($fill, (RoundRect ($cx - 12 * $u) ($by + 5 * $u) (24 * $u) (6 * $u) (3 * $u)))
        $r = 9.5 * $u; $cy = ($my + $mh / 2) * $u
        $ink = New-Object Drawing.SolidBrush ([Drawing.Color]::FromArgb(255, 92, 104, 250))
        $g.FillPie($ink, $cx - $r, $cy - $r, $r * 2, $r * 2, 90, 180)
        $g.DrawEllipse((New-Object Drawing.Pen ([Drawing.Color]::FromArgb(255, 92, 104, 250)), (4 * $u)), $cx - $r, $cy - $r, $r * 2, $r * 2)
    } else {
        $mx = 13; $my = 15; $mw = 38; $mh = 26
        $screen = RoundRect ($mx * $u) ($my * $u) ($mw * $u) ($mh * $u) (5 * $u)
        $g.DrawPath($pen, $screen)
        $by = ($my + $mh) * $u
        $g.DrawLine($pen, $cx, $by, $cx, $by + 6 * $u)
        $g.DrawLine($pen, $cx - 9 * $u, $by + 7 * $u, $cx + 9 * $u, $by + 7 * $u)
        # símbolo de brillo/contraste
        $r = 7.5 * $u; $cy = ($my + $mh / 2) * $u
        $g.DrawEllipse((New-Object Drawing.Pen $white, (3 * $u)), $cx - $r, $cy - $r, $r * 2, $r * 2)
        $g.FillPie($fill, $cx - $r, $cy - $r, $r * 2, $r * 2, 90, 180)
    }
    $g.Dispose()

    $out = New-Object Drawing.Bitmap $size, $size
    $g2 = [Drawing.Graphics]::FromImage($out)
    $g2.InterpolationMode = 'HighQualityBicubic'
    $g2.PixelOffsetMode = 'HighQuality'
    $g2.DrawImage($bmp, 0, 0, $size, $size)
    $g2.Dispose(); $bmp.Dispose()
    $ms = New-Object IO.MemoryStream
    $out.Save($ms, [Drawing.Imaging.ImageFormat]::Png)
    $out.Dispose()
    , $ms.ToArray()
}

$sizes = 16, 20, 24, 32, 40, 48, 64, 256
$pngs = foreach ($s in $sizes) { , (Draw $s) }
$out = New-Object IO.MemoryStream
$bw = New-Object IO.BinaryWriter $out
$bw.Write([UInt16]0); $bw.Write([UInt16]1); $bw.Write([UInt16]$sizes.Count)
$off = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i] % 256
    $bw.Write([byte]$s); $bw.Write([byte]$s); $bw.Write([byte]0); $bw.Write([byte]0)
    $bw.Write([UInt16]1); $bw.Write([UInt16]32)
    $bw.Write([UInt32]$pngs[$i].Length); $bw.Write([UInt32]$off)
    $off += $pngs[$i].Length
}
foreach ($b in $pngs) { $bw.Write($b) }
$dir = Join-Path $PSScriptRoot '..\assets'
New-Item -ItemType Directory -Force $dir | Out-Null
[IO.File]::WriteAllBytes((Join-Path $dir 'kast.ico'), $out.ToArray())
[IO.File]::WriteAllBytes((Join-Path $dir 'kast-256.png'), $pngs[-1])
