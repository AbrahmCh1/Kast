# Genera assets/kast.ico (PNG multi-tamaño), assets/kast-256.png y assets/kast-1024.png (macOS) con System.Drawing.
# El diseño es assets/kast.svg: baldosa grafito y una pantalla con la mitad derecha rellena (brillo/contraste).
# Cada tamaño se dibuja a 4x y se reduce; de 16 a 24 px los trazos son más gruesos.
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

# $margin: margen de la baldosa en unidades de la rejilla de 64 (macOS usa el de sus iconos: ~6.25).
function Draw([int]$size, [float]$margin = -1) {
    $S = $size * 4
    $small = $size -le 24
    if ($margin -lt 0) { $margin = if ($small) { 1 } else { 3 } }
    # La baldosa ocupa 64 - 2*margin; el dibujo se escala dentro de ella (diseño en una rejilla de 64 con margen 3).
    $u = $S / 64.0 * (64 - 2 * $margin) / 58.0
    $o = $S / 64.0 * $margin - 3 * $u
    $bmp = New-Object Drawing.Bitmap $S, $S
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.PixelOffsetMode = 'HighQuality'
    $g.TranslateTransform($o, $o)

    # baldosa
    $tile = RoundRect (3 * $u) (3 * $u) (58 * $u) (58 * $u) (14 * $u)
    $rect = New-Object Drawing.RectangleF 0, (3 * $u), $S, (58 * $u)
    $grad = New-Object Drawing.Drawing2D.LinearGradientBrush $rect, ([Drawing.Color]::FromArgb(255, 58, 58, 64)), ([Drawing.Color]::FromArgb(255, 22, 22, 24)), 90.0
    $g.FillPath($grad, $tile)
    # filo claro muy sutil: que no se pierda sobre fondos oscuros (barra de tareas, Dock)
    if (-not $small) {
        $edge = RoundRect (3.25 * $u) (3.25 * $u) (57.5 * $u) (57.5 * $u) (13.75 * $u)
        $g.DrawPath((New-Object Drawing.Pen ([Drawing.Color]::FromArgb(28, 255, 255, 255)), (0.5 * $u)), $edge)
    }

    # pantalla: contorno, mitad derecha rellena y pie
    $white = [Drawing.Color]::White
    $fill = New-Object Drawing.SolidBrush $white
    if ($small) { $x = 11; $y = 14; $w = 42; $h = 29; $r = 6; $sw = 5.5; $base = 52 }
    else        { $x = 14; $y = 16; $w = 36; $h = 25; $r = 6; $sw = 4;   $base = 49 }
    $pen = New-Object Drawing.Pen $white, ($sw * $u)
    $pen.StartCap = 'Round'; $pen.EndCap = 'Round'; $pen.LineJoin = 'Round'
    $screen = RoundRect ($x * $u) ($y * $u) ($w * $u) ($h * $u) ($r * $u)
    $g.SetClip((New-Object Drawing.RectangleF (32 * $u), 0, $S, $S))
    $g.FillPath($fill, $screen)
    $g.ResetClip()
    $g.DrawPath($pen, $screen)
    $g.DrawLine($pen, (24 * $u), ($base * $u), (40 * $u), ($base * $u))
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
# Icono de macOS (build.sh genera el .icns a partir de éste), con el margen de la rejilla de iconos de Apple (100/1024).
[IO.File]::WriteAllBytes((Join-Path $dir 'kast-1024.png'), (Draw 1024 6.25))
