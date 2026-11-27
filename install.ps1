#Requires -Version 5.1
<#
  All-in-one installer for the 12 literature-* skills, precompiled Rust runtime/z2o.exe,
  and bundled PP-OCRv6 OCR/layout/table runtime (when included in the release bundle)
  from GitHub Releases.
  No local compilation and no first-run OCR download prompts required.

  Usage:
    ./install.ps1 [-SkillsRoot <dir>] [-Version <tag>] [-Repo <owner/repo>]
#>
param(
  [string]$SkillsRoot = "",
  [string]$Version = "",
  [string]$Repo = "lyingTree/zotero-to-obsidian-skills",
  # Legacy options retained for existing installer callers.
  [string]$ObsidianSkillsRef = "",
  [string]$ZoteroSkillsRef = "",
  [switch]$SkipExternalDependencies
)
$ErrorActionPreference = "Stop"

if (-not $SkillsRoot) {
  if ($env:Z2O_SKILLS_ROOT) {
    $SkillsRoot = $env:Z2O_SKILLS_ROOT
  } elseif (Test-Path (Join-Path $PSScriptRoot "rust\Cargo.toml")) {
    throw "Refusing to install into the source repository root ($PSScriptRoot). Pass -SkillsRoot <dir> or set Z2O_SKILLS_ROOT."
  } else {
    $SkillsRoot = $PSScriptRoot
  }
}

$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToUpperInvariant()
switch ($arch) {
  "X64"   { $target = "x86_64-pc-windows-msvc" }
  "ARM64" { $target = "aarch64-pc-windows-msvc" }
  default { throw "Unsupported Windows architecture: $arch (supported: X64, ARM64)" }
}

if (-not $Version) {
  $versionFile = Join-Path $PSScriptRoot "VERSION"
  if (Test-Path $versionFile) {
    $Version = (Get-Content $versionFile -Raw).Trim()
  }
}
if ($Version -and -not $Version.StartsWith("v")) {
  $Version = "v$Version"
}
if (-not $Version) {
  $releaseInfo = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ "User-Agent" = "z2o-installer" }
  $Version = $releaseInfo.tag_name
}
if (-not $Version) {
  throw "Could not determine release version tag."
}

$archiveName = "z2o-skills-$target.zip"
$baseUrl = "https://github.com/$Repo/releases/download/$Version"
$tmpRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("z2o-install-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tmpRoot -Force | Out-Null

function Get-FileSha256Hex([string]$path) {
  return (Get-FileHash -Path $path -Algorithm SHA256).Hash.ToLowerInvariant()
}

try {
  $archivePath = Join-Path $tmpRoot $archiveName
  $sumsPath = Join-Path $tmpRoot "SHA256SUMS.txt"

  Write-Host "==> Downloading $archiveName and SHA256SUMS.txt from $Repo@$Version ..."
  Invoke-WebRequest -Uri "$baseUrl/$archiveName" -OutFile $archivePath -UseBasicParsing
  Invoke-WebRequest -Uri "$baseUrl/SHA256SUMS.txt" -OutFile $sumsPath -UseBasicParsing

  $expectedSha = $null
  foreach ($line in (Get-Content $sumsPath)) {
    $trimmed = $line.Trim()
    if ($trimmed -match "^([0-9a-fA-F]{64})\s+\*?$([regex]::Escape($archiveName))$") {
      $expectedSha = $Matches[1].ToLowerInvariant()
      break
    }
  }
  if (-not $expectedSha) {
    throw "Archive $archiveName not found in SHA256SUMS.txt"
  }
  $actualSha = Get-FileSha256Hex $archivePath
  if ($actualSha -ne $expectedSha) {
    throw "SHA-256 mismatch for $archiveName (expected $expectedSha, got $actualSha)"
  }

  $extractDir = Join-Path $tmpRoot "bundle"
  Expand-Archive -Path $archivePath -DestinationPath $extractDir -Force

  $manifestPath = Join-Path $extractDir "package.json"
  if (-not (Test-Path $manifestPath)) {
    throw "Bundle is missing package.json"
  }
  $manifestSha = Get-FileSha256Hex $manifestPath
  $z2oExe = Join-Path $extractDir "skills\literature-zotero2obsidian\runtime\z2o.exe"
  if (-not (Test-Path $z2oExe)) {
    throw "Bundle is missing precompiled runtime/z2o.exe"
  }

  if (-not (Test-Path $SkillsRoot)) {
    New-Item -ItemType Directory -Path $SkillsRoot -Force | Out-Null
  }
  $resolvedSkillsRoot = (Resolve-Path $SkillsRoot).Path

  Write-Host "==> Installing 12 literature-* skills, runtime/z2o.exe, and any bundled OCR runtime into $resolvedSkillsRoot ..."
  $installOutput = & $z2oExe install $manifestPath $manifestSha $resolvedSkillsRoot
  if ($LASTEXITCODE -ne 0) {
    throw "z2o install failed with exit code $LASTEXITCODE"
  }
  Write-Host $installOutput

  $installedZ2o = Join-Path $resolvedSkillsRoot "literature-zotero2obsidian\runtime\z2o.exe"
  $verifyOutput = & $installedZ2o verify-install $manifestPath $manifestSha $resolvedSkillsRoot
  if ($LASTEXITCODE -ne 0) {
    throw "z2o verify-install failed with exit code $LASTEXITCODE"
  }
  Write-Host $verifyOutput

  Write-Host "==> Rust skill installation complete ($Version, $target)."
}
finally {
  Remove-Item $tmpRoot -Recurse -Force -ErrorAction SilentlyContinue
}
