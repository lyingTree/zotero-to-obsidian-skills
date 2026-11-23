#Requires -Version 5.1
<#
  Install external dependency skills: obsidian-cli, zotero-skills.
  Fetches them from their upstream repos and installs into the Skills root
  (same level as literature-*).

  Usage:
    ./install.ps1
    ./install.ps1 -SkillsRoot <dir>
    ./install.ps1 -ObsidianSkillsRef v1 -ZoteroSkillsRef v1.0.0   # pin versions (default branch if empty)
#>
param(
  [string]$SkillsRoot = $PSScriptRoot,
  [string]$ObsidianSkillsRef = "",
  [string]$ZoteroSkillsRef = ""
)
$ErrorActionPreference = "Stop"

$OBSIDIAN_REPO = "https://github.com/obsidianmd/obsidian-skills.git"
$ZOTERO_REPO   = "https://github.com/WenyuChiou/zotero-skills.git"

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("z2o-deps-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tmp -Force | Out-Null

function Clone-SkillRepo([string]$url, [string]$dest, [string]$ref) {
  $gitArgs = @("clone", "--depth", "1")
  if ($ref) { $gitArgs += @("--branch", $ref) }
  $gitArgs += @($url, $dest)
  & git @gitArgs
  if ($LASTEXITCODE -ne 0) { throw "git clone failed: $url" }
}

try {
  Write-Host "==> Fetching obsidianmd/obsidian-skills ..."
  Clone-SkillRepo $OBSIDIAN_REPO "$tmp\obsidian-skills" $ObsidianSkillsRef
  Write-Host "==> Fetching WenyuChiou/zotero-skills ..."
  Clone-SkillRepo $ZOTERO_REPO "$tmp\zotero-skills" $ZoteroSkillsRef

  Write-Host "==> Installing obsidian-cli -> $SkillsRoot\obsidian-cli"
  $dstCli = Join-Path $SkillsRoot "obsidian-cli"
  if (Test-Path $dstCli) { Remove-Item $dstCli -Recurse -Force }
  Copy-Item "$tmp\obsidian-skills\plugins\obsidian\skills\obsidian-cli" $dstCli -Recurse -Force

  Write-Host "==> Installing zotero-skills -> $SkillsRoot\zotero-skills"
  $dstZ = Join-Path $SkillsRoot "zotero-skills"
  if (Test-Path $dstZ) { Remove-Item $dstZ -Recurse -Force }
  Copy-Item "$tmp\zotero-skills\skills\zotero-skills" $dstZ -Recurse -Force
  Copy-Item "$tmp\zotero-skills\scripts" (Join-Path $dstZ "scripts") -Recurse -Force
  Copy-Item "$tmp\zotero-skills\requirements.txt" $dstZ -Force
  Copy-Item "$tmp\zotero-skills\LICENSE" $dstZ -Force

  Write-Host "==> Installing Python deps (requirements.txt) ..."
  $py = $null
  foreach ($c in @("python", "python3", "py")) {
    $found = Get-Command $c -ErrorAction SilentlyContinue
    if ($found) { $py = $found.Source; break }
  }
  if (-not $py) {
    Write-Warning "Python not found on PATH. Install Python 3.10+ then run: python -m pip install -r `"$dstZ\requirements.txt`""
  } else {
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    & $py -m pip install -r (Join-Path $dstZ "requirements.txt")
    $pipCode = $LASTEXITCODE
    $ErrorActionPreference = $prevEap
    if ($pipCode -ne 0) {
      Write-Warning "pip install failed. Run manually: `"$py`" -m pip install -r `"$dstZ\requirements.txt`""
    }
  }

  Write-Host "Done. obsidian-cli requires the Obsidian desktop app installed and running."
}
finally {
  Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
