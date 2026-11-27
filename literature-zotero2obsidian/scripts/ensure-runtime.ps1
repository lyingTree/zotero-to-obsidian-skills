#Requires -Version 5.1
param([Parameter(Mandatory)][string]$Version)
$ErrorActionPreference = 'Stop'
if ($Version -cnotmatch '^v\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$') {
  throw "Invalid z2o release tag: $Version"
}

$skillRoot = Split-Path -Parent $PSScriptRoot
$runtime = Join-Path $skillRoot 'runtime'
$installed = Join-Path $runtime 'z2o.exe'

function Test-Z2oRuntime([string]$Executable) {
  if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) { return $false }
  try {
    $description = & $Executable describe 2>$null
    if ($LASTEXITCODE -ne 0) { return $false }
    $info = $description | Out-String | ConvertFrom-Json
    return $info.engine -ceq 'rust' -and $info.version -ceq $Version
  } catch {
    return $false
  }
}

if (Test-Z2oRuntime $installed) { $installed; return }

$lockPath = Join-Path $skillRoot 'user'
[IO.Directory]::CreateDirectory($lockPath) | Out-Null
$lockPath = Join-Path $lockPath 'runtime.lock'
$deadline = [DateTime]::UtcNow.AddMinutes(5)
while ($true) {
  try {
    $lock = [IO.File]::Open($lockPath, 'OpenOrCreate', 'ReadWrite', 'None')
    break
  } catch [IO.IOException] {
    if ([DateTime]::UtcNow -ge $deadline) { throw }
    Start-Sleep -Seconds 1
  }
}

$stage = Join-Path $skillRoot ('.z2o-runtime-' + [guid]::NewGuid().ToString('N'))
try {
  if (Test-Z2oRuntime $installed) { $installed; return }
  $target = switch ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()) {
    'X64' { 'x86_64-pc-windows-msvc' }
    'Arm64' { 'aarch64-pc-windows-msvc' }
    default { throw 'Unsupported Windows architecture for z2o' }
  }
  [IO.Directory]::CreateDirectory($stage) | Out-Null
  $name = "z2o-skills-$target.zip"
  $url = "https://github.com/lyingTree/zotero-to-obsidian-skills/releases/download/$Version"
  $archive = Join-Path $stage $name
  $sums = Join-Path $stage 'SHA256SUMS.txt'
  Invoke-WebRequest -UseBasicParsing -Uri "$url/SHA256SUMS.txt" -OutFile $sums
  $expected = $null
  foreach ($line in Get-Content -LiteralPath $sums) {
    if ($line.Trim() -match "^([0-9a-fA-F]{64})\s+\*?$([regex]::Escape($name))$") {
      $expected = $Matches[1]
      break
    }
  }
  if (-not $expected) { throw "Release checksum missing for $name" }
  Invoke-WebRequest -UseBasicParsing -Uri "$url/$name" -OutFile $archive
  if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $expected) {
    throw "SHA-256 mismatch for $name"
  }
  $bundle = Join-Path $stage 'bundle'
  Expand-Archive -LiteralPath $archive -DestinationPath $bundle
  $stagedRuntime = Join-Path $bundle 'skills/literature-zotero2obsidian/runtime'
  if (-not (Test-Z2oRuntime (Join-Path $stagedRuntime 'z2o.exe'))) {
    throw "Release runtime cannot start or does not match $Version"
  }

  $previous = Join-Path $stage 'previous'
  if (Test-Path -LiteralPath $runtime) { [IO.Directory]::Move($runtime, $previous) }
  try {
    [IO.Directory]::Move($stagedRuntime, $runtime)
    if (-not (Test-Z2oRuntime $installed)) { throw 'Installed z2o cannot start' }
  } catch {
    if (Test-Path -LiteralPath $runtime) {
      [IO.Directory]::Move($runtime, (Join-Path $stage 'failed'))
    }
    if (Test-Path -LiteralPath $previous) { [IO.Directory]::Move($previous, $runtime) }
    throw
  }
  if (Test-Path -LiteralPath $previous) {
    [IO.Directory]::Move($previous, (Join-Path $stage 'replaced'))
  }
  $installed
} finally {
  try {
    # Preserve the previous runtime if rollback itself could not finish.
    if ((Test-Path -LiteralPath $stage) -and
        -not (Test-Path -LiteralPath (Join-Path $stage 'previous'))) {
      $resolvedStage = (Resolve-Path -LiteralPath $stage).ProviderPath
      if ($resolvedStage -eq $stage -and (Split-Path -Parent $resolvedStage) -eq $skillRoot) {
        Remove-Item -LiteralPath $resolvedStage -Recurse -Force
      }
    }
  } finally {
    $lock.Dispose()
  }
}
