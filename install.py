"""Install the literature skills and pinned dependencies without deleting old copies.

Edit the configuration below and run this script in your Python environment.
The shell/PowerShell wrappers pass their existing arguments through environment
variables. Import ``InstallConfig`` and ``install`` for scripted use.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import uuid
from contextlib import contextmanager
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from typing import Callable, Iterator, Mapping, Sequence


# User-editable configuration. Empty refs select dependencies.lock.json.
REPOSITORY_ROOT = Path(__file__).resolve().parent
SKILLS_ROOT = REPOSITORY_ROOT
OBSIDIAN_SKILLS_REF = ""
ZOTERO_SKILLS_REF = ""
INSTALL_PYTHON_DEPENDENCIES = True

LITERATURE_SKILLS = (
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
DEPENDENCY_NAMES = ("obsidian-cli", "zotero-skills")
DEPENDENCY_REPOSITORIES = {
    "obsidian-cli": "https://github.com/obsidianmd/obsidian-skills.git",
    "zotero-skills": "https://github.com/WenyuChiou/zotero-skills.git",
}


class InstallationError(RuntimeError):
    """An installation failed; existing skills are preserved or recoverable."""


@dataclass(frozen=True)
class InstallConfig:
    skills_root: Path = SKILLS_ROOT
    source_root: Path = REPOSITORY_ROOT
    obsidian_ref: str = OBSIDIAN_SKILLS_REF
    zotero_ref: str = ZOTERO_SKILLS_REF
    install_python_dependencies: bool = INSTALL_PYTHON_DEPENDENCIES
    python_executable: str = sys.executable
    git_executable: str = "git"


@dataclass(frozen=True)
class Dependency:
    name: str
    repository: str
    revision: str
    requested_ref: str
    skill_path: PurePosixPath
    extra_paths: tuple[tuple[PurePosixPath, PurePosixPath], ...]


@dataclass(frozen=True)
class PreparedSkill:
    name: str
    path: Path
    fingerprint: str


@dataclass(frozen=True)
class InstallResult:
    installed: tuple[str, ...]
    skipped: tuple[str, ...]
    backup_root: Path | None
    dependency_revisions: Mapping[str, str]
    python_dependencies: str


def _relative_path(value: object) -> PurePosixPath:
    if not isinstance(value, str) or not value or "\\" in value or ":" in value:
        raise InstallationError(f"Invalid relative dependency path: {value!r}")
    path = PurePosixPath(value)
    if path.is_absolute() or any(part in (".", "..") for part in value.split("/")):
        raise InstallationError(f"Unsafe relative dependency path: {value!r}")
    return path


def load_dependencies(path: Path, overrides: Mapping[str, str]) -> tuple[Dependency, ...]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema_version") != 1:
        raise InstallationError("Unsupported dependencies.lock.json schema_version")
    entries = data.get("dependencies")
    if not isinstance(entries, dict) or set(entries) != set(DEPENDENCY_NAMES):
        raise InstallationError("The lock file must contain exactly the two dependency skills")
    dependencies = []
    for name in DEPENDENCY_NAMES:
        entry = entries[name]
        if not isinstance(entry, dict):
            raise InstallationError(f"Invalid dependency record: {name}")
        revision = entry.get("revision", "")
        if not isinstance(revision, str) or not re.fullmatch(r"[0-9a-f]{40}", revision):
            raise InstallationError(f"{name}: the lock revision must be a full commit SHA")
        repository = entry.get("repository")
        if repository != DEPENDENCY_REPOSITORIES[name]:
            raise InstallationError(f"{name}: unexpected upstream repository")
        requested_ref = overrides.get(name, "") or revision
        if requested_ref.startswith("-") or any(char.isspace() for char in requested_ref):
            raise InstallationError(f"{name}: invalid revision override")
        extra_paths = entry.get("extra_paths")
        if not isinstance(extra_paths, dict) or not extra_paths:
            raise InstallationError(f"{name}: missing extra_paths (including LICENSE)")
        if extra_paths.get("LICENSE") != "LICENSE":
            raise InstallationError(f"{name}: the upstream LICENSE must be retained")
        dependencies.append(
            Dependency(
                name=name,
                repository=repository,
                revision=revision,
                requested_ref=requested_ref,
                skill_path=_relative_path(entry.get("skill_path")),
                extra_paths=tuple(
                    (_relative_path(source), _relative_path(target))
                    for source, target in extra_paths.items()
                ),
            )
        )
    return tuple(dependencies)


def _run(command: Sequence[str], *, cwd: Path | None = None, capture: bool = False) -> str:
    result = subprocess.run(
        list(command), cwd=cwd, check=True, text=True,
        stdout=subprocess.PIPE if capture else None,
    )
    return result.stdout.strip() if capture else ""


def fetch_repository(dependency: Dependency, destination: Path, git_executable: str) -> str:
    """Fetch a branch, tag or full SHA without treating the ref as a branch name."""
    destination.mkdir()
    _run([git_executable, "init", "--quiet", str(destination)])
    _run([git_executable, "remote", "add", "origin", dependency.repository], cwd=destination)
    _run(
        [git_executable, "fetch", "--depth", "1", "--", "origin", dependency.requested_ref],
        cwd=destination,
    )
    _run([git_executable, "checkout", "--quiet", "--detach", "FETCH_HEAD"], cwd=destination)
    actual = _run([git_executable, "rev-parse", "HEAD"], cwd=destination, capture=True)
    if not re.fullmatch(r"[0-9a-f]{40}", actual):
        raise InstallationError(f"{dependency.name}: Git did not return a full commit SHA")
    if re.fullmatch(r"[0-9a-fA-F]{40}", dependency.requested_ref):
        if actual != dependency.requested_ref.lower():
            raise InstallationError(f"{dependency.name}: fetched commit differs from requested SHA")
    return actual


def _is_link(path: Path) -> bool:
    if path.is_symlink():
        return True
    try:
        attributes = getattr(path.lstat(), "st_file_attributes", 0)
    except FileNotFoundError:
        return False
    # Windows junctions/reparse points are not reported by Path.is_symlink().
    return bool(attributes & 0x400)


def _check_tree(path: Path) -> None:
    if _is_link(path):
        raise InstallationError(f"Refusing symbolic link: {path}")
    if not path.exists():
        raise InstallationError(f"Required source is missing: {path}")
    if not path.is_dir():
        if not path.is_file():
            raise InstallationError(f"Refusing non-regular source: {path}")
        return
    for child in path.rglob("*"):
        if _is_link(child) or not (child.is_file() or child.is_dir()):
            raise InstallationError(f"Refusing linked or special file: {child}")


def validate_skill(path: Path, expected_name: str) -> None:
    _check_tree(path)
    if not path.is_dir():
        raise InstallationError(f"Skill is not a directory: {path}")
    skill_file = path.joinpath("SKILL.md")
    if not skill_file.is_file():
        raise InstallationError(f"Refusing a directory without SKILL.md: {path}")
    text = skill_file.read_text(encoding="utf-8-sig")
    frontmatter = re.match(r"\A---\r?\n(.*?)\r?\n---(?:\r?\n|$)", text, re.DOTALL)
    if frontmatter is None:
        raise InstallationError(f"Invalid skill frontmatter: {skill_file}")
    name_match = re.search(r"^name:\s*([^\r\n]+)$", frontmatter.group(1), re.MULTILINE)
    actual_name = name_match.group(1).strip().strip("\"'") if name_match else ""
    if actual_name != expected_name:
        raise InstallationError(f"Unexpected skill name in {skill_file}: {actual_name!r}")


def tree_fingerprint(path: Path) -> str:
    """Hash paths and contents in a stable order, reading files in bounded chunks."""
    _check_tree(path)
    digest = hashlib.sha256()
    for entry in sorted(path.rglob("*"), key=lambda item: item.relative_to(path).as_posix()):
        relative_path = entry.relative_to(path)
        if "__pycache__" in relative_path.parts or entry.suffix in (".pyc", ".pyo"):
            continue
        relative = relative_path.as_posix().encode("utf-8")
        digest.update(b"D" if entry.is_dir() else b"F")
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        if entry.is_dir():
            continue
        digest.update((entry.stat().st_mode & 0o111).to_bytes(2, "big"))
        digest.update(entry.stat().st_size.to_bytes(8, "big"))
        with entry.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
    return digest.hexdigest()


def _snapshot_target(path: Path, expected_name: str) -> str | None:
    if _is_link(path):
        raise InstallationError(f"Refusing to replace a symbolic-link target: {path}")
    if not path.exists():
        return None
    validate_skill(path, expected_name)
    return tree_fingerprint(path)


@contextmanager
def _installation_lock(root: Path) -> Iterator[None]:
    lock = root.joinpath(".z2o-install.lock")
    token = uuid.uuid4().hex
    try:
        with lock.open("x", encoding="utf-8") as stream:
            stream.write(json.dumps({"pid": os.getpid(), "token": token}))
    except FileExistsError as error:
        raise InstallationError(
            f"An install lock already exists: {lock}. Check the other installer before removing a stale lock."
        ) from error
    try:
        yield
    finally:
        if not _is_link(lock) and lock.is_file():
            data = json.loads(lock.read_text(encoding="utf-8"))
            if data.get("token") == token:
                lock.unlink()


def _copy_source(source: Path, target: Path) -> None:
    _check_tree(source)
    target.parent.mkdir(parents=True, exist_ok=True)
    if source.is_dir():
        shutil.copytree(source, target, dirs_exist_ok=True, ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "*.pyo"))
        return
    shutil.copy2(source, target)


def _move_path(source: Path, target: Path) -> None:
    source.replace(target)


def _write_receipt(path: Path, data: Mapping[str, object]) -> None:
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def _publish(
    prepared: Sequence[PreparedSkill],
    target_root: Path,
    snapshots: Mapping[str, str | None],
    revisions: Mapping[str, str],
    progress: Callable[[str], None],
    installation_details: Mapping[str, object],
) -> Path:
    backups = target_root.joinpath(".z2o-backups")
    if _is_link(backups) or (backups.exists() and not backups.is_dir()):
        raise InstallationError(f"Refusing unsafe backup directory: {backups}")
    backup_root = backups.joinpath(datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:12])
    previous = backup_root.joinpath("previous")
    previous.mkdir(parents=True)
    receipt_path = backup_root.joinpath("installation.json")
    receipt = {
        "status": "installing",
        "target_root": str(target_root),
        "dependency_revisions": dict(revisions),
        "skills": [skill.name for skill in prepared],
        "fingerprints": {skill.name: skill.fingerprint for skill in prepared},
        **installation_details,
    }
    _write_receipt(receipt_path, receipt)
    actions: list[tuple[str, bool]] = []
    try:
        for index, skill in enumerate(prepared, start=1):
            target = target_root.joinpath(skill.name)
            if _snapshot_target(target, skill.name) != snapshots[skill.name]:
                raise InstallationError(f"Target changed during preparation; preserved without overwrite: {target}")
            had_previous = target.exists()
            if had_previous:
                _move_path(target, previous.joinpath(skill.name))
            actions.append((skill.name, had_previous))
            _move_path(skill.path, target)
            progress(f"  [{index}/{len(prepared)}] Installed {skill.name}")
        receipt["status"] = "completed"
        _write_receipt(receipt_path, receipt)
        return backup_root
    except BaseException as error:
        rollback_errors = []
        failed = backup_root.joinpath("failed")
        installed_fingerprints = {skill.name: skill.fingerprint for skill in prepared}
        for name, had_previous in reversed(actions):
            target = target_root.joinpath(name)
            try:
                if target.exists() or _is_link(target):
                    if _is_link(target) or tree_fingerprint(target) != installed_fingerprints[name]:
                        rollback_errors.append(f"{name}: destination changed; current path and previous backup preserved")
                        continue
                    failed.mkdir(exist_ok=True)
                    _move_path(target, failed.joinpath(name))
                if had_previous:
                    _move_path(previous.joinpath(name), target)
            except (OSError, InstallationError) as rollback_error:
                rollback_errors.append(f"{name}: {rollback_error}")
        receipt["status"] = "rollback_incomplete" if rollback_errors else "rolled_back"
        receipt["error"] = str(error)
        receipt["rollback_errors"] = rollback_errors
        try:
            _write_receipt(receipt_path, receipt)
        except OSError:
            pass
        detail = "; ".join(rollback_errors) if rollback_errors else "Previous skill directories restored"
        raise InstallationError(f"Install failed. {detail}. Recovery files: {backup_root}") from error


def _progress(message: str) -> None:
    print(message, flush=True)


def install(
    config: InstallConfig,
    *,
    fetcher: Callable[[Dependency, Path, str], str] = fetch_repository,
    progress: Callable[[str], None] = _progress,
) -> InstallResult:
    source_root = config.source_root.expanduser().resolve()
    target_root = config.skills_root.expanduser().absolute()
    if _is_link(target_root):
        raise InstallationError(f"Refusing a symbolic-link skills root: {target_root}")
    if target_root.exists() and not target_root.is_dir():
        raise InstallationError(f"Skills root is not a directory: {target_root}")
    target_root = target_root.resolve()
    for name in (*LITERATURE_SKILLS, *DEPENDENCY_NAMES):
        if target_root.is_relative_to(source_root.joinpath(name).resolve()):
            raise InstallationError(f"Skills root cannot be inside a source skill: {target_root}")
        if source_root.is_relative_to(target_root.joinpath(name).resolve()):
            raise InstallationError(f"Source repository cannot be inside a skill being replaced: {source_root}")
    dependencies = load_dependencies(
        source_root.joinpath("dependencies.lock.json"),
        {"obsidian-cli": config.obsidian_ref, "zotero-skills": config.zotero_ref},
    )
    progress("[1/5] Validating local skills and destination")
    for name in LITERATURE_SKILLS:
        validate_skill(source_root.joinpath(name), name)
    target_root.mkdir(parents=True, exist_ok=True)
    target_root = target_root.resolve()
    with _installation_lock(target_root), tempfile.TemporaryDirectory(prefix=".z2o-staging-", dir=target_root) as temporary:
        stage = Path(temporary)
        names = (*LITERATURE_SKILLS, *DEPENDENCY_NAMES)
        snapshots = {name: _snapshot_target(target_root.joinpath(name), name) for name in names}
        backup_parent = target_root.joinpath(".z2o-backups")
        if _is_link(backup_parent) or (backup_parent.exists() and not backup_parent.is_dir()):
            raise InstallationError(f"Refusing unsafe backup directory: {backup_parent}")
        packages = stage.joinpath("packages")
        packages.mkdir()
        source_cache = stage.joinpath("sources")
        source_cache.mkdir()
        prepared = []
        skipped = []
        progress("[2/5] Staging literature skills")
        for index, name in enumerate(LITERATURE_SKILLS, start=1):
            source = source_root.joinpath(name)
            if source == target_root.joinpath(name):
                skipped.append(name)
                continue
            destination = packages.joinpath(name)
            _copy_source(source, destination)
            prepared.append(PreparedSkill(name, destination, tree_fingerprint(destination)))
            progress(f"  [{index}/{len(LITERATURE_SKILLS)}] Prepared {name}")
        progress("[3/5] Fetching and validating pinned external skills")
        revisions = {}
        for dependency in dependencies:
            checkout = source_cache.joinpath(dependency.name)
            revisions[dependency.name] = fetcher(dependency, checkout, config.git_executable)
            source = checkout.joinpath(*dependency.skill_path.parts)
            validate_skill(source, dependency.name)
            for extra_source, _ in dependency.extra_paths:
                _check_tree(checkout.joinpath(*extra_source.parts))
            destination = packages.joinpath(dependency.name)
            _copy_source(source, destination)
            for extra_source, extra_target in dependency.extra_paths:
                _copy_source(checkout.joinpath(*extra_source.parts), destination.joinpath(*extra_target.parts))
            validate_skill(destination, dependency.name)
            prepared.append(PreparedSkill(dependency.name, destination, tree_fingerprint(destination)))
            progress(f"  Prepared {dependency.name} at {revisions[dependency.name]}")
        requirements = (
            source_root.joinpath("literature-zotero2obsidian", "requirements.txt"),
            packages.joinpath("zotero-skills", "requirements.txt"),
        )
        for requirement in requirements:
            if not requirement.is_file():
                raise InstallationError(f"Required dependency list is missing: {requirement}")
        progress("[4/5] Installing Python dependencies" if config.install_python_dependencies else "[4/5] Python dependency installation explicitly skipped")
        if config.install_python_dependencies:
            command = [config.python_executable, "-m", "pip", "install"]
            for requirement in requirements:
                command.extend(["-r", str(requirement)])
            _run(command)
        changed = []
        for skill in prepared:
            if _snapshot_target(target_root.joinpath(skill.name), skill.name) != snapshots[skill.name]:
                raise InstallationError(f"Target changed during preparation; preserved without overwrite: {skill.name}")
            if skill.fingerprint == snapshots[skill.name]:
                skipped.append(skill.name)
                continue
            changed.append(skill)
        progress("[5/5] Publishing prepared skills with recoverable backups")
        python_status = "installed" if config.install_python_dependencies else "skipped"
        version_file = source_root.joinpath("VERSION")
        source_version = version_file.read_text(encoding="utf-8").strip() if version_file.is_file() else "unknown"
        details = {"python_dependencies": python_status, "skipped_skills": skipped, "source_version": source_version}
        backup_root = _publish(changed, target_root, snapshots, revisions, progress, details) if changed else None
        return InstallResult(tuple(skill.name for skill in changed), tuple(skipped), backup_root, revisions, python_status)


def _environment_bool(name: str, default: bool) -> bool:
    value = os.environ.get(name)
    if value is None:
        return default
    if value.lower() in ("1", "true", "yes"):
        return True
    if value.lower() in ("0", "false", "no"):
        return False
    raise InstallationError(f"{name} must be true/false or 1/0")


def main() -> int:
    if sys.version_info < (3, 10):
        print("Installation requires Python 3.10 or newer.", file=sys.stderr)
        return 1
    try:
        config = InstallConfig(
            skills_root=Path(os.environ.get("Z2O_SKILLS_ROOT") or str(SKILLS_ROOT)),
            obsidian_ref=os.environ.get("Z2O_OBSIDIAN_SKILLS_REF") or OBSIDIAN_SKILLS_REF,
            zotero_ref=os.environ.get("Z2O_ZOTERO_SKILLS_REF") or ZOTERO_SKILLS_REF,
            install_python_dependencies=_environment_bool("Z2O_INSTALL_PYTHON_DEPENDENCIES", INSTALL_PYTHON_DEPENDENCIES),
        )
        result = install(config)
    except (InstallationError, OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Installation failed: {error}", file=sys.stderr)
        print("If pip ran, changes to the selected Python environment cannot be rolled back by this installer.", file=sys.stderr)
        return 1
    print(f"Done: {len(result.installed)} installed, {len(result.skipped)} unchanged.")
    print(f"Python dependencies: {result.python_dependencies}.")
    if result.backup_root is not None:
        print(f"Recoverable backups and installation receipt: {result.backup_root}")
    print("Open Zotero and Obsidian, and verify their CLI/API settings before using the skills.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
