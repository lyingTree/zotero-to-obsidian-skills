#Requires -Version 5.1
<#
Compatibility wrapper. For script-based use, edit and run install.py.
Usage: ./install.ps1 [-SkillsRoot <dir>] [-ObsidianSkillsRef <ref>] [-ZoteroSkillsRef <ref>]
#>
param(
  [string]$SkillsRoot = "",
  [string]$ObsidianSkillsRef = "",
  [string]$ZoteroSkillsRef = ""
)
$ErrorActionPreference = "Stop"

$environmentNames = @("Z2O_SKILLS_ROOT", "Z2O_OBSIDIAN_SKILLS_REF", "Z2O_ZOTERO_SKILLS_REF")
$previousEnvironment = @{}
foreach ($name in $environmentNames) {
  $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}

$resultCode = 1
try {
  if ($PSBoundParameters.ContainsKey("SkillsRoot")) { $env:Z2O_SKILLS_ROOT = $SkillsRoot }
  if ($PSBoundParameters.ContainsKey("ObsidianSkillsRef")) { $env:Z2O_OBSIDIAN_SKILLS_REF = $ObsidianSkillsRef }
  if ($PSBoundParameters.ContainsKey("ZoteroSkillsRef")) { $env:Z2O_ZOTERO_SKILLS_REF = $ZoteroSkillsRef }

  $python = $env:Z2O_PYTHON
  if (-not $python) {
    foreach ($candidate in @("python", "python3", "py")) {
      $command = Get-Command $candidate -ErrorAction SilentlyContinue
      if ($command) { $python = $command.Source; break }
    }
  }
  if (-not $python) { throw "Python 3.10+ is required. Set Z2O_PYTHON to the interpreter to use." }

  # Native nonzero exit codes must be propagated by both PowerShell 5.1 and 7.
  & $python (Join-Path $PSScriptRoot "install.py")
  $resultCode = $LASTEXITCODE
}
finally {
  foreach ($name in $environmentNames) {
    [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], "Process")
  }
}
exit $resultCode
