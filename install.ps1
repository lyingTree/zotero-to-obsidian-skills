#Requires -Version 5.1
<#
  Install the 12 literature-* skills (with precompiled Rust runtime/z2o.exe)
  from GitHub Releases, plus optional external dependency skills (obsidian-cli, zotero-skills).
  No local compilation required.

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

function Invoke-Git([string[]]$GitArgs) {
  & git @GitArgs
  if ($LASTEXITCODE -ne 0) {
    throw "git $($GitArgs -join ' ') failed with exit code $LASTEXITCODE"
  }
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

  Write-Host "==> Installing 12 literature-* skills into $resolvedSkillsRoot ..."
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
      Invoke-Git @("init", "-q", $obsTmp)
      Invoke-Git @("-C", $obsTmp, "remote", "add", "origin", "https://github.com/obsidianmd/obsidian-skills.git")
      Invoke-Git @("-C", $obsTmp, "fetch", "-q", "--depth", "1", "origin", $obsidianRev)
      Invoke-Git @("-C", $obsTmp, "checkout", "-q", "FETCH_HEAD")
      $obsSkillSrc = Join-Path $obsTmp "plugins\obsidian\skills\obsidian-cli"
      $obsLicenseSrc = Join-Path $obsTmp "LICENSE"
      if (-not (Test-Path $obsSkillSrc -PathType Container) -or -not (Test-Path $obsLicenseSrc -PathType Leaf)) {
        throw "Cloned obsidian-skills is missing expected skill files."
      }
      $dstObs = Join-Path $resolvedSkillsRoot "obsidian-cli"
      if (Test-Path $dstObs) { Remove-Item $dstObs -Recurse -Force }
      Copy-Item $obsSkillSrc $dstObs -Recurse -Force
      Copy-Item $obsLicenseSrc (Join-Path $dstObs "LICENSE") -Force

      $zotTmp = Join-Path $tmpRoot "zotero-skills"
      Invoke-Git @("init", "-q", $zotTmp)
      Invoke-Git @("-C", $zotTmp, "remote", "add", "origin", "https://github.com/WenyuChiou/zotero-skills.git")
      Invoke-Git @("-C", $zotTmp, "fetch", "-q", "--depth", "1", "origin", $zoteroRev)
      Invoke-Git @("-C", $zotTmp, "checkout", "-q", "FETCH_HEAD")
      $zotSkillSrc = Join-Path $zotTmp "skills\zotero-skills"
      $zotScriptsSrc = Join-Path $zotTmp "scripts"
      $zotLicenseSrc = Join-Path $zotTmp "LICENSE"
      $zotReqSrc = Join-Path $zotTmp "requirements.txt"
      if (-not (Test-Path $zotSkillSrc -PathType Container) -or -not (Test-Path $zotScriptsSrc -PathType Container) -or -not (Test-Path $zotLicenseSrc -PathType Leaf)) {
        throw "Cloned zotero-skills is missing expected skill files."
      }
      $dstZot = Join-Path $resolvedSkillsRoot "zotero-skills"
      if (Test-Path $dstZot) { Remove-Item $dstZot -Recurse -Force }
      Copy-Item $zotSkillSrc $dstZot -Recurse -Force
      Copy-Item $zotScriptsSrc (Join-Path $dstZot "scripts") -Recurse -Force
      if (Test-Path $zotReqSrc -PathType Leaf) {
        Copy-Item $zotReqSrc (Join-Path $dstZot "requirements.txt") -Force
      }
      Copy-Item $zotLicenseSrc (Join-Path $dstZot "LICENSE") -Force
    } else {
      Write-Warning "git not found on PATH; skipped external skills (obsidian-cli, zotero-skills)."
    }
  }

  Write-Host "==> Rust skill installation complete ($Version, $target)."
}
finally {
  Remove-Item $tmpRoot -Recurse -Force -ErrorAction SilentlyContinue
}
