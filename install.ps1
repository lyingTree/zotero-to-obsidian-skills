#Requires -Version 5.1
<#
  Install the 12 native literature-* skills (with precompiled runtime/z2o.exe)
  from GitHub Releases, plus optional external dependency skills (obsidian-cli, zotero-skills).
  No Python or local Rust compilation required.

  Usage:
    ./install.ps1 [-SkillsRoot <dir>] [-Version <tag>] [-Repo <owner/repo>] [-SkipExternalDependencies]
#>
param(
  [string]$SkillsRoot = "",
  [string]$Version = "",
  [string]$Repo = "lyingTree/zotero-to-obsidian-skills",
  [string]$ObsidianSkillsRef = "",
  [string]$ZoteroSkillsRef = "",
  [switch]$SkipExternalDependencies
)
$ErrorActionPreference = "Stop"

if (-not $SkillsRoot) {
  if ($env:Z2O_SKILLS_ROOT) {
    $SkillsRoot = $env:Z2O_SKILLS_ROOT
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

  Write-Host "==> Installing 12 native literature-* skills into $resolvedSkillsRoot ..."
  $installOutput = & $z2oExe install $manifestPath $manifestSha $resolvedSkillsRoot
  if ($LASTEXITCODE -ne 0) {
    throw "z2o install failed with exit code $LASTEXITCODE"
  }
  Write-Host $installOutput

  $installedZ2o = Join-Path $resolvedSkillsRoot "literature-zotero2obsidian\runtime\z2o.exe"
  & $installedZ2o verify-install $manifestPath $manifestSha $resolvedSkillsRoot | Out-Null
  if ($LASTEXITCODE -ne 0) {
    throw "z2o verify-install failed with exit code $LASTEXITCODE"
  }

  if (-not $SkipExternalDependencies) {
    $lockFile = Join-Path $PSScriptRoot "dependencies.lock.json"
    $obsidianRev = if ($ObsidianSkillsRef) { $ObsidianSkillsRef } else { "b110b2f8e9eb1f287892f937050dfd40af862cbb" }
    $zoteroRev   = if ($ZoteroSkillsRef)   { $ZoteroSkillsRef }   else { "e5c11e744fab34d9dd322319195249daf735c2b1" }
    if (Test-Path $lockFile) {
      $lockData = Get-Content $lockFile -Raw | ConvertFrom-Json
      if (-not $ObsidianSkillsRef -and $lockData.dependencies.'obsidian-cli'.revision) {
        $obsidianRev = $lockData.dependencies.'obsidian-cli'.revision
      }
      if (-not $ZoteroSkillsRef -and $lockData.dependencies.'zotero-skills'.revision) {
        $zoteroRev = $lockData.dependencies.'zotero-skills'.revision
      }
    }

    if (Get-Command git -ErrorAction SilentlyContinue) {
      Write-Host "==> Fetching pinned external skills (obsidian-cli@$obsidianRev, zotero-skills@$zoteroRev) ..."
      $obsTmp = Join-Path $tmpRoot "obsidian-skills"
      & git init -q $obsTmp
      & git -C $obsTmp remote add origin "https://github.com/obsidianmd/obsidian-skills.git"
      & git -C $obsTmp fetch -q --depth 1 origin $obsidianRev
      & git -C $obsTmp checkout -q FETCH_HEAD
      $dstObs = Join-Path $resolvedSkillsRoot "obsidian-cli"
      if (Test-Path $dstObs) { Remove-Item $dstObs -Recurse -Force }
      Copy-Item (Join-Path $obsTmp "plugins\obsidian\skills\obsidian-cli") $dstObs -Recurse -Force
      Copy-Item (Join-Path $obsTmp "LICENSE") (Join-Path $dstObs "LICENSE") -Force

      $zotTmp = Join-Path $tmpRoot "zotero-skills"
      & git init -q $zotTmp
      & git -C $zotTmp remote add origin "https://github.com/WenyuChiou/zotero-skills.git"
      & git -C $zotTmp fetch -q --depth 1 origin $zoteroRev
      & git -C $zotTmp checkout -q FETCH_HEAD
      $dstZot = Join-Path $resolvedSkillsRoot "zotero-skills"
      if (Test-Path $dstZot) { Remove-Item $dstZot -Recurse -Force }
      Copy-Item (Join-Path $zotTmp "skills\zotero-skills") $dstZot -Recurse -Force
      Copy-Item (Join-Path $zotTmp "scripts") (Join-Path $dstZot "scripts") -Recurse -Force
      Copy-Item (Join-Path $zotTmp "requirements.txt") (Join-Path $dstZot "requirements.txt") -Force
      Copy-Item (Join-Path $zotTmp "LICENSE") (Join-Path $dstZot "LICENSE") -Force
    } else {
      Write-Warning "git not found on PATH; skipped external skills (obsidian-cli, zotero-skills)."
    }
  }

  Write-Host "==> Native installation complete ($Version, $target)."
}
finally {
  Remove-Item $tmpRoot -Recurse -Force -ErrorAction SilentlyContinue
}
