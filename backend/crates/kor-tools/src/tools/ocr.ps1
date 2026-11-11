$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
trap { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }
[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null=[Windows.Storage.StorageFile,Windows.Storage,ContentType=WindowsRuntime]
$null=[Windows.Storage.FileAccessMode,Windows.Storage,ContentType=WindowsRuntime]
$null=[Windows.Storage.Streams.IRandomAccessStream,Windows.Storage.Streams,ContentType=WindowsRuntime]
$null=[Windows.Graphics.Imaging.BitmapDecoder,Windows.Graphics.Imaging,ContentType=WindowsRuntime]
$null=[Windows.Graphics.Imaging.SoftwareBitmap,Windows.Graphics.Imaging,ContentType=WindowsRuntime]
$null=[Windows.Graphics.Imaging.BitmapPixelFormat,Windows.Graphics.Imaging,ContentType=WindowsRuntime]
$null=[Windows.Graphics.Imaging.BitmapAlphaMode,Windows.Graphics.Imaging,ContentType=WindowsRuntime]
$null=[Windows.Media.Ocr.OcrEngine,Windows.Media.Ocr,ContentType=WindowsRuntime]
$null=[Windows.Media.Ocr.OcrResult,Windows.Media.Ocr,ContentType=WindowsRuntime]
$null=[Windows.Globalization.Language,Windows.Globalization,ContentType=WindowsRuntime]
$AsTaskGeneric=[System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.IsGenericMethodDefinition -and $_.GetParameters().Count -eq 1 -and $_.GetGenericArguments().Count -eq 1 -and $_.ReturnType.IsGenericType } | Select-Object -First 1
if($null -eq $AsTaskGeneric){ throw 'WinRT AsTask helper not found' }
function Await($op,$resultType){
  $t=$AsTaskGeneric.MakeGenericMethod($resultType).Invoke($null,@($op))
  $t.Wait(-1) | Out-Null
  $t.Result
}
# The only input is the path, passed through the environment, never spliced into this script.
$file=Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($env:KOR_OCR_PATH)) ([Windows.Storage.StorageFile])
$stream=Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream])
$decoder=Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
$bitmap=Await ($decoder.GetSoftwareBitmapAsync([Windows.Graphics.Imaging.BitmapPixelFormat]::Bgra8,[Windows.Graphics.Imaging.BitmapAlphaMode]::Premultiplied)) ([Windows.Graphics.Imaging.SoftwareBitmap])
$engine=$null
if($env:KOR_OCR_LANG){
  $lang=New-Object Windows.Globalization.Language $env:KOR_OCR_LANG
  if(-not [Windows.Media.Ocr.OcrEngine]::IsLanguageSupported($lang)){
    $have=([Windows.Media.Ocr.OcrEngine]::AvailableRecognizerLanguages | ForEach-Object { $_.LanguageTag }) -join ', '
    throw "OCR language '$($env:KOR_OCR_LANG)' is not installed. Installed: $have"
  }
  $engine=[Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage($lang)
} else {
  $engine=[Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
}
if($null -eq $engine){ throw 'No OCR language is installed for this Windows user (Settings > Time & language > Language).' }
$max=[Windows.Media.Ocr.OcrEngine]::MaxImageDimension
if($bitmap.PixelWidth -gt $max -or $bitmap.PixelHeight -gt $max){ throw "Image is larger than $max pixels on a side; crop or scale it first." }
$result=Await ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
$lines=New-Object System.Collections.Generic.List[object]
foreach($line in $result.Lines){
  $words=$line.Words
  if($words.Count -eq 0){ continue }
  $minX=[double]::MaxValue; $minY=[double]::MaxValue; $maxX=[double]::MinValue; $maxY=[double]::MinValue
  foreach($wd in $words){
    $r=$wd.BoundingRect
    if($r.X -lt $minX){ $minX=$r.X }
    if($r.Y -lt $minY){ $minY=$r.Y }
    if(($r.X+$r.Width) -gt $maxX){ $maxX=$r.X+$r.Width }
    if(($r.Y+$r.Height) -gt $maxY){ $maxY=$r.Y+$r.Height }
  }
  $lines.Add([pscustomobject]@{ text=[string]$line.Text; x=[int][math]::Round($minX); y=[int][math]::Round($minY); w=[int][math]::Round($maxX-$minX); h=[int][math]::Round($maxY-$minY) })
}
([pscustomobject]@{ lang=[string]$engine.RecognizerLanguage.LanguageTag; lines=$lines.ToArray() }) | ConvertTo-Json -Compress -Depth 6
