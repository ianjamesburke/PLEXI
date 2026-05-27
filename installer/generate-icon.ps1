#Requires -Version 5.1
<#
.SYNOPSIS
    Generate a multi-resolution Windows .ico file from a square PNG master.

.DESCRIPTION
    Reads assets\app-icon.png (any square size, preferably >= 256 with alpha)
    and writes a multi-image .ico file containing the standard Windows icon
    sizes (16, 32, 48, 64, 128, 256). Each sub-image is encoded as PNG inside
    the .ico (supported on Windows 7+), which is smaller than the legacy BMP
    encoding and preserves the alpha channel cleanly.

    Requires no external tools — uses only .NET System.Drawing.

.PARAMETER Source
    Path to the square source PNG. Defaults to ..\assets\app-icon.png.

.PARAMETER Destination
    Output .ico path. Defaults to ..\assets\app-icon.ico.

.PARAMETER Sizes
    Pixel sizes to include. Defaults to 16, 32, 48, 64, 128, 256.
#>

[CmdletBinding()]
param(
    [string]$Source      = (Join-Path (Split-Path -Parent $PSScriptRoot) 'assets\app-icon.png'),
    [string]$Destination = (Join-Path (Split-Path -Parent $PSScriptRoot) 'assets\app-icon.ico'),
    [int[]]$Sizes        = @(16, 32, 48, 64, 128, 256)
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path $Source)) {
    Write-Error "Source PNG not found: $Source"
}

Add-Type -AssemblyName System.Drawing

$srcBmp = [System.Drawing.Bitmap]::new($Source)
try {
    if ($srcBmp.Width -ne $srcBmp.Height) {
        Write-Warning "Source is not square ($($srcBmp.Width)x$($srcBmp.Height)). Output will be stretched."
    }

    # Encode each requested size as PNG bytes, in memory.
    $pngBlobs = @{}
    foreach ($size in $Sizes) {
        $resized = [System.Drawing.Bitmap]::new($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $g = [System.Drawing.Graphics]::FromImage($resized)
        try {
            $g.CompositingMode    = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
            $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
            $g.InterpolationMode  = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $g.PixelOffsetMode    = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $g.SmoothingMode      = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
            $g.DrawImage($srcBmp, 0, 0, $size, $size)
        } finally {
            $g.Dispose()
        }

        $ms = [System.IO.MemoryStream]::new()
        try {
            $resized.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
            $pngBlobs[$size] = $ms.ToArray()
        } finally {
            $ms.Dispose()
            $resized.Dispose()
        }
    }
} finally {
    $srcBmp.Dispose()
}

# Build the .ico binary:
#   ICONDIR (6 bytes) + ICONDIRENTRY (16 bytes) * N + concatenated image data.
$headerSize = 6
$entrySize  = 16
$imageStart = $headerSize + ($entrySize * $Sizes.Count)

$out = [System.IO.MemoryStream]::new()
$bw  = [System.IO.BinaryWriter]::new($out)
try {
    # ICONDIR
    $bw.Write([uint16]0)                  # Reserved
    $bw.Write([uint16]1)                  # Type: 1 = icon
    $bw.Write([uint16]$Sizes.Count)       # Count

    # ICONDIRENTRY for each image
    $offset = $imageStart
    foreach ($size in $Sizes) {
        $blob = $pngBlobs[$size]
        $dim  = if ($size -ge 256) { [byte]0 } else { [byte]$size }  # 0 == 256 per spec
        $bw.Write([byte]$dim)             # Width
        $bw.Write([byte]$dim)             # Height
        $bw.Write([byte]0)                # ColorCount (0 for >= 256 colors)
        $bw.Write([byte]0)                # Reserved
        $bw.Write([uint16]1)              # Planes
        $bw.Write([uint16]32)             # BitCount
        $bw.Write([uint32]$blob.Length)   # BytesInRes
        $bw.Write([uint32]$offset)        # ImageOffset
        $offset += $blob.Length
    }

    # Concatenated image data
    foreach ($size in $Sizes) {
        $bw.Write($pngBlobs[$size])
    }

    $bw.Flush()
    [System.IO.File]::WriteAllBytes($Destination, $out.ToArray())
} finally {
    $bw.Dispose()
    $out.Dispose()
}

$bytes = (Get-Item $Destination).Length
Write-Output "Wrote $Destination ($bytes bytes, sizes: $($Sizes -join ', '))"
