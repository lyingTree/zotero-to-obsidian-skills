#Requires -Version 5.1
param(
  [Parameter(Mandatory)][ValidatePattern('^v\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$')][string]$Version,
  [switch]$OcrWorker
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$skillRoot = Split-Path -Parent $PSScriptRoot
$runtime = Join-Path $skillRoot 'runtime'
$installed = Join-Path $runtime 'z2o.exe'
$target = switch ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()) {
  X64 { 'x86_64-pc-windows-msvc' }
  Arm64 { 'aarch64-pc-windows-msvc' }
  default { throw 'Unsupported Windows architecture for z2o' }
}
$proxy = @($env:Z2O_PROXY, $env:HTTPS_PROXY, $env:ALL_PROXY) | Where-Object { $_ } | Select-Object -First 1
function Receive-Asset([string]$Name, [string]$Destination, [int]$MaxSeconds) {
  if (-not $script:proxy) {
    try {
      $systemProxy = [Net.WebRequest]::GetSystemWebProxy().GetProxy([uri]$url)
      if ($systemProxy -and $systemProxy.Authority -ne ([uri]$url).Authority) { $script:proxy = $systemProxy.AbsoluteUri }
    } catch { Write-Verbose 'Windows proxy discovery unavailable' }
  }
  $routes = @('direct')
  if ($proxy) {
    $routes = if ($downloadHost -eq 'github' -and $env:Z2O_PROXY) { @('proxy','direct') } else { @('direct','proxy') }
  }
  foreach ($route in $routes) {
    $proxyArgs = if ($route -eq 'proxy') { @('--proxy', $proxy, '--noproxy', 'unused.invalid') } else { @('--noproxy', '*') }
    & curl.exe --disable @proxyArgs -fsSL --connect-timeout 10 --max-time $MaxSeconds --speed-limit 1024 --speed-time 25 --retry 1 --retry-max-time 15 --continue-at - --output $Destination "$url/$Name"
    if ($LASTEXITCODE -eq 0) { return }
    if ($LASTEXITCODE -eq 33) {
      Remove-Item -LiteralPath $Destination -Force -ErrorAction SilentlyContinue
      & curl.exe --disable @proxyArgs -fsSL --connect-timeout 10 --max-time $MaxSeconds --speed-limit 1024 --speed-time 25 --output $Destination "$url/$Name"
      if ($LASTEXITCODE -eq 0) { return }
    }
  }
  throw "Download failed for $Name from $url. Check connectivity and the configured proxy."
}
function Read-Checksum([string]$Name) {
  foreach ($line in Get-Content -LiteralPath $sums) {
    if ($line.Trim() -match "^([0-9a-fA-F]{64})\s+\*?$([regex]::Escape($Name))$") { return $Matches[1] }
  }
}
function Receive-VerifiedAsset([string]$Name, [string]$Expected, [string]$Part, [int]$MaxSeconds) {
  if (-not $Expected) { throw "Missing checksum for $Name" }
  if ((Test-Path -LiteralPath $Part) -and (Get-FileHash -LiteralPath $Part).Hash -eq $Expected) { return }
  Receive-Asset $Name $Part $MaxSeconds
  if ((Get-FileHash -LiteralPath $Part).Hash -ne $Expected) {
    Remove-Item -LiteralPath $Part -Force
    throw "SHA-256 mismatch for $Name"
  }
}
function Test-Z2oRuntime([string]$Executable) {
  if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) { return $false }
  try {
    $info = & $Executable --version 2>$null | Out-String | ConvertFrom-Json
    return $LASTEXITCODE -eq 0 -and $info.engine -ceq 'rust' -and $info.version -ceq $Version
  } catch { return $false }
}
$ocr = Join-Path $runtime 'ocr'
if (Test-Path -LiteralPath (Join-Path $ocr 'package.json') -PathType Leaf) {
  if ($OcrWorker) { return }
  if (Test-Z2oRuntime $installed) { $installed; return }
}
$region = $env:Z2O_NETWORK_REGION
if (-not $region) {
  $trace = & curl.exe --disable --noproxy '*' -fsS --connect-timeout 2 --max-time 3 'https://www.cloudflare.com/cdn-cgi/trace' 2>$null
  $location = @($trace) | Where-Object { $_ -cmatch '^loc=[A-Z]{2}$' } | Select-Object -First 1
  $region = if ($location) { $location.Substring(4) } else { 'unknown' }
}
if ($region -eq 'CN') {
  $downloadHost = 'cnb'
  $url = "https://cnb.cool/lyingTree/zotero-to-obsidian-skills/-/releases/download/$Version"
} else {
  $downloadHost = 'github'
  $url = "https://github.com/lyingTree/zotero-to-obsidian-skills/releases/download/$Version"
}
$state = Join-Path $skillRoot "user/downloads/$Version/$target/$downloadHost"
[IO.Directory]::CreateDirectory($state) | Out-Null
[IO.Directory]::CreateDirectory($runtime) | Out-Null
$mode = if ($OcrWorker) { 'ocr' } else { 'runtime' }
$lockPath = Join-Path (Split-Path -Parent $state) "$mode.lock"
$deadline = [DateTime]::UtcNow.AddMinutes(2)
while ($true) {
  try { $lock = [IO.File]::Open($lockPath, 'OpenOrCreate', 'ReadWrite', 'None'); break }
  catch [IO.IOException] {
    if ($OcrWorker) { return }
    if ([DateTime]::UtcNow -ge $deadline) { throw }
    Start-Sleep -Seconds 1
  }
}
$stage = $null
try {
  $ready = Test-Path -LiteralPath (Join-Path $ocr 'package.json') -PathType Leaf
  if ($OcrWorker -and $ready) { return }
  $sums = Join-Path $state 'SHA256SUMS.txt'
  $hasRuntime = Test-Z2oRuntime $installed
  if ($OcrWorker -or -not $hasRuntime) {
    if (-not (Test-Path -LiteralPath $sums)) {
      $tempSums = Join-Path $state ('checksums-' + [guid]::NewGuid().ToString('N'))
      try { Receive-Asset 'SHA256SUMS.txt' $tempSums 30; Move-Item -LiteralPath $tempSums -Destination $sums -Force }
      finally { Remove-Item -LiteralPath $tempSums -Force -ErrorAction SilentlyContinue }
    }
  }
  if (-not $OcrWorker) {
    if (-not $hasRuntime) {
      $name = "z2o-$target.exe"
      $part = Join-Path $state 'z2o.part.exe'
      Receive-VerifiedAsset $name (Read-Checksum $name) $part 120
      if (-not (Test-Z2oRuntime $part)) { throw 'Downloaded z2o cannot start' }
      if (Test-Path -LiteralPath $installed) { [IO.File]::Replace($part, $installed, $null) }
      else { [IO.File]::Move($part, $installed) }
    }
    if (-not $ready) {
      $previousRegion = $env:Z2O_NETWORK_REGION
      try {
        $env:Z2O_NETWORK_REGION = $region
        Start-Process -FilePath (Get-Process -Id $PID).Path -WindowStyle Hidden `
          -ArgumentList '-NoProfile', '-File', ('"' + $PSCommandPath + '"'), '-Version', $Version, '-OcrWorker' `
          -RedirectStandardOutput (Join-Path $state ('ocr-' + $PID + '.out.log')) `
          -RedirectStandardError (Join-Path $state ('ocr-' + $PID + '.err.log'))
      } catch { Write-Warning 'OCR background worker could not start. Re-run runtime setup to retry.' }
      finally { $env:Z2O_NETWORK_REGION = $previousRegion }
    }
    $installed
    return
  }
  $name = "z2o-ocr-$target.zip"
  $member = 'ocr'
  $expected = Read-Checksum $name
  if (-not $expected) {
    $name = "z2o-skills-$target.zip"
    $member = 'skills/literature-zotero2obsidian/runtime/ocr'
    $expected = Read-Checksum $name
  }
  $stage = Join-Path $state ('extract-' + [guid]::NewGuid().ToString('N'))
  $part = Join-Path $state "$name.part"
  Receive-VerifiedAsset $name $expected $part 900
  $archive = Join-Path $state $name
  Move-Item -LiteralPath $part -Destination $archive -Force
  try { Expand-Archive -LiteralPath $archive -DestinationPath $stage }
  finally { Remove-Item -LiteralPath $archive -Force }
  $stagedOcr = Join-Path $stage $member
  $manifest = Get-Content -LiteralPath (Join-Path $stagedOcr 'package.sha256') -Raw
  if ((Get-FileHash -LiteralPath (Join-Path $stagedOcr 'package.json')).Hash -ne $manifest.Trim()) {
    throw 'OCR manifest checksum mismatch'
  }
  if (Test-Path -LiteralPath $ocr) { throw "OCR destination exists without a manifest: $ocr" }
  [IO.Directory]::Move($stagedOcr, $ocr)
  'OCR package ready'
} finally {
  try {
    if ($stage -and (Test-Path -LiteralPath $stage)) {
      $resolved = (Resolve-Path -LiteralPath $stage).ProviderPath
      if ($resolved -eq $stage -and (Split-Path -Parent $resolved) -eq $state) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
      }
    }
  } finally { $lock.Dispose() }
}
