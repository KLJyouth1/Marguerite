$ErrorActionPreference = 'Stop'
$zip = 'D:\Downloads\Marguerite\deliverables\Marguerite_工作台升级总览.pptx'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$z = [System.IO.Compression.ZipFile]::OpenRead($zip)
$slides = @($z.Entries | Where-Object { $_.FullName -match '^ppt/slides/slide[0-9]+\.xml$' })
$size = (Get-Item $zip).Length
Write-Output ("Slide XML count: {0}" -f $slides.Count)
Write-Output ("File size bytes: {0}" -f $size)
$z.Dispose()