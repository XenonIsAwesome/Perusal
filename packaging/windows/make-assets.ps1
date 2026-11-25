# Generates the Windows icon (.ico with 16/32/48/256 PNG frames) and the MSIX tile images from the 256 px app icon.
param([string]$Src = "$PSScriptRoot\..\..\crates\ui\assets\perusal-256.png", [string]$IcoOut = "$PSScriptRoot\..\..\crates\ui\assets\perusal.ico", [string]$TilesDir = "")
Add-Type -AssemblyName System.Drawing
function Resize-Png([string]$src, [int]$w, [int]$h) {
    $img = [System.Drawing.Image]::FromFile((Resolve-Path $src)); $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp); $g.InterpolationMode = 'HighQualityBicubic'; $g.SmoothingMode = 'HighQuality'; $g.PixelOffsetMode = 'HighQuality'
    $g.Clear([System.Drawing.Color]::Transparent); $g.DrawImage($img, 0, 0, $w, $h); $g.Dispose(); $img.Dispose()
    $ms = New-Object System.IO.MemoryStream; $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose(); return ,$ms.ToArray()
}
function Dib-Frame([string]$src, [int]$s) {
    # 32-bit BGRA DIB (bottom-up) + 1-bit AND mask: the classic icon frame every Windows API understands
    $img = [System.Drawing.Image]::FromFile((Resolve-Path $src)); $bmp = New-Object System.Drawing.Bitmap $s, $s, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp); $g.InterpolationMode = 'HighQualityBicubic'; $g.SmoothingMode = 'HighQuality'; $g.PixelOffsetMode = 'HighQuality'
    $g.Clear([System.Drawing.Color]::Transparent); $g.DrawImage($img, 0, 0, $s, $s); $g.Dispose(); $img.Dispose()
    $ms = New-Object System.IO.MemoryStream; $w = New-Object System.IO.BinaryWriter $ms
    $w.Write([uint32]40); $w.Write([int32]$s); $w.Write([int32]($s * 2)); $w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]0); $w.Write([uint32]($s * $s * 4)); $w.Write([int32]0); $w.Write([int32]0); $w.Write([uint32]0); $w.Write([uint32]0)
    for ($y = $s - 1; $y -ge 0; $y--) { for ($x = 0; $x -lt $s; $x++) { $c = $bmp.GetPixel($x, $y); $w.Write([byte]$c.B); $w.Write([byte]$c.G); $w.Write([byte]$c.R); $w.Write([byte]$c.A) } }
    $maskRow = [int]([math]::Ceiling($s / 32.0) * 4); $w.Write((New-Object byte[] ($maskRow * $s)))
    $bmp.Dispose(); return ,$ms.ToArray()
}
$sizes = 16, 32, 48, 256; $frames = @(); foreach ($s in $sizes) { if ($s -ge 256) { $frames += , (Resize-Png $Src $s $s) } else { $frames += , (Dib-Frame $Src $s) } }
$ms = New-Object System.IO.MemoryStream; $bw = New-Object System.IO.BinaryWriter $ms
$bw.Write([uint16]0); $bw.Write([uint16]1); $bw.Write([uint16]$sizes.Count)
$off = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]; $d = if ($s -ge 256) { 0 } else { $s }
    $bw.Write([byte]$d); $bw.Write([byte]$d); $bw.Write([byte]0); $bw.Write([byte]0); $bw.Write([uint16]1); $bw.Write([uint16]32)
    $bw.Write([uint32]$frames[$i].Length); $bw.Write([uint32]$off); $off += $frames[$i].Length
}
foreach ($f in $frames) { $bw.Write($f) }
[System.IO.File]::WriteAllBytes($IcoOut, $ms.ToArray()); Write-Host "wrote $IcoOut"
if ($TilesDir) {
    New-Item -ItemType Directory -Force $TilesDir | Out-Null
    foreach ($t in @(@('Square44x44Logo.png',44), @('Square150x150Logo.png',150), @('StoreLogo.png',50), @('Square44x44Logo.targetsize-44_altform-unplated.png',44))) {
        [System.IO.File]::WriteAllBytes((Join-Path $TilesDir $t[0]), (Resize-Png $Src $t[1] $t[1]))
    }
}

