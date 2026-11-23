"""Read-only checks for this skill bundle; run as a script or import the function."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from urllib.parse import unquote, urlsplit

import yaml


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
EXPECTED_SKILLS = (
    "literature-zotero2obsidian",
    "literature-inspect-zotero",
    "literature-fulltext-extract",
    "literature-translate-zh",
    "literature-write2obsidian",
    "literature-filename",
    "literature-note-properties",
    "literature-header-format",
    "literature-body-format",
    "literature-internal-links",
    "literature-library-links",
    "literature-supplement-materials",
)
DEPENDENCY_URLS = {
    "obsidian-cli": "https://github.com/obsidianmd/obsidian-skills.git",
    "zotero-skills": "https://github.com/WenyuChiou/zotero-skills.git",
}
LINK = re.compile(r"(?<!!)\[[^\]\n]+\]\((?:<([^>\n]+)>|([^\s)]+))\)")
FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})(.*)$")


@dataclass(frozen=True)
class Finding:
    path: str
    line: int
    code: str
    message: str


class UniqueKeyLoader(yaml.SafeLoader):
    """Reject duplicate YAML keys instead of silently losing the earlier value."""


def _unique_mapping(loader: UniqueKeyLoader, node: yaml.MappingNode, deep: bool = False):
    mapping = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        try:
            duplicate = key in mapping
        except TypeError as error:
            raise yaml.constructor.ConstructorError(
                None, None, "mapping key must be scalar", key_node.start_mark
            ) from error
        if duplicate:
            raise yaml.constructor.ConstructorError(
                None, None, f"duplicate key: {key}", key_node.start_mark
            )
        mapping[key] = loader.construct_object(value_node, deep=deep)
    return mapping


UniqueKeyLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, _unique_mapping
)


def _visible_lines(text: str):
    """Yield Markdown prose outside fenced and inline code, retaining line numbers."""
    fence_character = ""
    fence_size = 0
    for number, line in enumerate(text.splitlines(), 1):
        match = FENCE.match(line)
        if fence_character:
            if (
                match
                and match[1][0] == fence_character
                and len(match[1]) >= fence_size
                and not match[2].strip()
            ):
                fence_character = ""
            continue
        if match:
            fence_character, fence_size = match[1][0], len(match[1])
            continue
        # Documentation examples inside inline code are not file dependencies.
        yield number, re.sub(r"(`+).*?\1(?!`)", "", line)


def _frontmatter(path: Path, root: Path) -> list[Finding]:
    relative = path.relative_to(root).as_posix()
    lines = path.read_text(encoding="utf-8-sig").splitlines()
    if not lines or lines[0] != "---":
        return [Finding(relative, 1, "frontmatter", "Missing opening YAML delimiter.")]
    try:
        end = lines.index("---", 1)
    except ValueError:
        return [Finding(relative, 1, "frontmatter", "Missing closing YAML delimiter.")]
    try:
        metadata = yaml.load("\n".join(lines[1:end]), Loader=UniqueKeyLoader)
    except (yaml.YAMLError, ValueError, OverflowError, RecursionError) as error:
        return [Finding(relative, 1, "frontmatter", str(error))]
    if not isinstance(metadata, dict):
        return [Finding(relative, 1, "frontmatter", "YAML must be a mapping.")]
    findings = []
    if metadata.get("name") != path.parent.name:
        findings.append(Finding(relative, 2, "skill-name", "name must match the skill directory."))
    if not re.fullmatch(r"[a-z0-9-]{1,64}", str(metadata.get("name", ""))):
        findings.append(Finding(relative, 2, "skill-name", "Use 1–64 lowercase letters, digits or hyphens."))
    description = metadata.get("description")
    if not isinstance(description, str) or not description.strip() or len(description) > 1024:
        findings.append(Finding(relative, 3, "description", "Provide a nonempty description of at most 1024 characters."))
    if set(metadata) - {"name", "description"}:
        findings.append(Finding(relative, 1, "metadata", "Bundle skills use name/description only; VERSION is the release source."))
    if len(lines) > 500:
        findings.append(Finding(relative, 1, "skill-size", "Move detailed guidance to references; SKILL.md exceeds 500 lines."))
    return findings


def _links(path: Path, root: Path) -> list[Finding]:
    findings = []
    relative = path.relative_to(root).as_posix()
    external_entries = {root.joinpath(name, "SKILL.md") for name in DEPENDENCY_URLS}
    for number, line in _visible_lines(path.read_text(encoding="utf-8-sig")):
        for match in LINK.finditer(line):
            target = match[1] or match[2]
            parsed = urlsplit(target)
            if parsed.scheme or parsed.netloc or not parsed.path:
                continue
            candidate = path.parent.joinpath(unquote(parsed.path)).resolve()
            if not candidate.is_relative_to(root):
                findings.append(Finding(relative, number, "path-escape", f"Link leaves the bundle: {target}"))
                continue
            if candidate in external_entries and not candidate.exists():
                continue  # These two entries are supplied by install.py.
            if not candidate.is_file():
                findings.append(Finding(relative, number, "broken-link", f"Missing file: {target}"))
    return findings


def _safe_relative(value: object) -> bool:
    if not isinstance(value, str) or not value or "\\" in value or ":" in value:
        return False
    path = PurePosixPath(value)
    return not path.is_absolute() and ".." not in path.parts and value != "."


def _dependencies(root: Path) -> list[Finding]:
    path = root.joinpath("dependencies.lock.json")
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        return [Finding(path.name, 1, "dependency-lock", str(error))]
    if not isinstance(data, dict) or data.get("schema_version") != 1:
        return [Finding(path.name, 1, "dependency-lock", "Expected lock schema_version=1.")]
    dependencies = data.get("dependencies")
    if not isinstance(dependencies, dict) or set(dependencies) != set(DEPENDENCY_URLS):
        return [Finding(path.name, 1, "dependency-lock", "Lock must describe exactly the two dependency skills.")]
    findings = []
    for name, expected_url in DEPENDENCY_URLS.items():
        entry = dependencies[name]
        if not isinstance(entry, dict):
            findings.append(Finding(path.name, 1, "dependency-lock", f"Invalid entry: {name}"))
            continue
        if entry.get("repository") != expected_url:
            findings.append(Finding(path.name, 1, "dependency-source", f"Unexpected upstream repository: {name}"))
        if not re.fullmatch(r"[0-9a-f]{40}", str(entry.get("revision", ""))):
            findings.append(Finding(path.name, 1, "dependency-revision", f"Pin {name} to a full lowercase commit SHA."))
        if not _safe_relative(entry.get("skill_path")):
            findings.append(Finding(path.name, 1, "dependency-path", f"Unsafe skill_path: {name}"))
        extras = entry.get("extra_paths")
        if not isinstance(extras, dict) or extras.get("LICENSE") != "LICENSE":
            findings.append(Finding(path.name, 1, "dependency-license", f"Preserve the upstream LICENSE for {name}."))
            continue
        for source, destination in extras.items():
            if not _safe_relative(source) or not _safe_relative(destination):
                findings.append(Finding(path.name, 1, "dependency-path", f"Unsafe extra path: {name}"))
    return findings


def validate_repository(root: Path = REPOSITORY_ROOT) -> list[Finding]:
    root = root.resolve()
    findings = []
    actual = {path.parent.name for path in root.glob("literature-*/SKILL.md")}
    for name in sorted(set(EXPECTED_SKILLS) - actual):
        findings.append(Finding(name, 1, "missing-skill", "Required SKILL.md is missing."))
    for name in sorted(actual - set(EXPECTED_SKILLS)):
        findings.append(Finding(name, 1, "unknown-skill", "Add new skills explicitly to the bundle inventory."))
    for name in sorted(actual):
        skill = root.joinpath(name)
        findings.extend(_frontmatter(skill.joinpath("SKILL.md"), root))
        for path in sorted(skill.rglob("*.md")):
            findings.extend(_links(path, root))
    readme = root.joinpath("README.md")
    if readme.is_file():
        findings.extend(_links(readme, root))
    version_path = root.joinpath("VERSION")
    if not version_path.is_file() or not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", version_path.read_text(encoding="utf-8").strip()):
        findings.append(Finding("VERSION", 1, "version", "Provide one semantic release version."))
    findings.extend(_dependencies(root))
    return findings


def main() -> int:
    findings = validate_repository()
    for finding in findings:
        print(f"{finding.path}:{finding.line}: {finding.code}: {finding.message}")
    print(f"Repository validation: {len(EXPECTED_SKILLS)} skills, {len(findings)} error(s).")
    return int(bool(findings))


if __name__ == "__main__":
    raise SystemExit(main())
