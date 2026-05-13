"""Agent Skills discovery for Plano.

Parses SKILL.md files from .plano/skills/ (project scope) and ~/.plano/skills/
(user scope) following the Agent Skills specification:
https://agentskills.io/specification.md

The parser is intentionally lenient (per the "Adding skills support" guide):
warn on cosmetic issues but only skip a skill when its YAML is unparseable or
its required `description` field is missing.
"""

from __future__ import annotations

import json
import os
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable

import yaml

from planoai.utils import getLogger

log = getLogger(__name__)

PROJECT_SKILLS_DIR = Path(".plano") / "skills"
USER_SKILLS_DIR = Path(os.path.expanduser("~/.plano/skills"))
# Universal Agent Skills install location used by `npx skills add` (vercel-labs/add-skill).
# Auto-trusted: same security posture as ~/.plano/skills, no project trust needed.
AGENTS_SKILLS_DIR = Path(os.path.expanduser("~/.agents/skills"))

MAX_CATALOG_BYTES = 5 * 1024

MAX_DIRS_SCANNED = 2000

_NAME_PATTERN = re.compile(r"^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$")


def trusted_projects_file() -> Path:
    """Resolve `~/.plano/trusted_projects.json` at call time.

    Lazy so tests can override $HOME and have the new path picked up; module
    import time would freeze it to the developer's actual home directory.
    """
    return Path(os.path.expanduser("~/.plano/trusted_projects.json"))


def is_project_trusted(project_root: Path) -> bool:
    """Return True if `project_root` is listed in `~/.plano/trusted_projects.json`.

    Project-scope skills come from arbitrary repos and are gated on this trust
    decision (set with `planoai skills trust`). Single source of truth, shared
    between the `skills_cmd` CLI surface and the render pipeline.
    """
    path = trusted_projects_file()
    if not path.exists():
        return False
    try:
        with path.open("r", encoding="utf-8") as fh:
            data = json.load(fh)
    except (OSError, json.JSONDecodeError):
        return False
    trusted = data.get("trusted_projects", []) if isinstance(data, dict) else []
    resolved = str(project_root.resolve())
    return resolved in {str(Path(p).resolve()) for p in trusted}


@dataclass(frozen=True)
class SkillDiagnostic:
    severity: str  # "warn" or "error"
    message: str
    path: Path


@dataclass
class Skill:
    name: str
    description: str
    location: Path
    base_dir: Path
    body: str
    scope: str
    compatibility: str | None = None
    license: str | None = None
    metadata: dict = field(default_factory=dict)
    allowed_tools: str | None = None

    def to_dict(self) -> dict:
        """Serialize to a YAML-friendly dict for embedding in rendered config."""
        return {
            "name": self.name,
            "description": self.description,
            "path": str(self.location),
            "base_dir": str(self.base_dir),
            "scope": self.scope,
            "body": self.body,
            "compatibility": self.compatibility,
            "license": self.license,
            "metadata": dict(self.metadata) if self.metadata else None,
            "allowed_tools": self.allowed_tools,
        }


def find_project_root(start: Path | None = None) -> Path:
    """Walk up from `start` looking for `.plano/`, then `.git/`.

    Falls back to `start` (or cwd) if nothing is found. This matches how
    `npx skills add` chooses a project root.
    """
    base = Path(start or Path.cwd()).resolve()
    cur = base
    while cur != cur.parent:
        if (cur / ".plano").exists():
            return cur
        cur = cur.parent

    cur = base
    while cur != cur.parent:
        if (cur / ".git").exists():
            return cur
        cur = cur.parent

    return base


def parse_skill_md(path: Path) -> tuple[Skill | None, list[SkillDiagnostic]]:
    """Parse a single SKILL.md file leniently."""
    diagnostics: list[SkillDiagnostic] = []
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        diagnostics.append(
            SkillDiagnostic("error", f"failed to read SKILL.md: {exc}", path)
        )
        return None, diagnostics

    frontmatter, body = _split_frontmatter(text)
    if frontmatter is None:
        diagnostics.append(SkillDiagnostic("error", "missing YAML frontmatter", path))
        return None, diagnostics

    data = _parse_yaml_lenient(frontmatter, path, diagnostics)
    if data is None:
        return None, diagnostics

    description = data.get("description")
    if not isinstance(description, str) or not description.strip():
        diagnostics.append(
            SkillDiagnostic(
                "error", "skill is missing required 'description' field", path
            )
        )
        return None, diagnostics

    parent_name = path.parent.name
    name = data.get("name")
    if not isinstance(name, str) or not name.strip():
        diagnostics.append(
            SkillDiagnostic(
                "warn",
                f"missing 'name' field; falling back to parent directory '{parent_name}'",
                path,
            )
        )
        name = parent_name

    name = name.strip()

    if len(name) > 64:
        diagnostics.append(
            SkillDiagnostic("warn", "skill name exceeds 64 characters", path)
        )

    if not _NAME_PATTERN.match(name):
        diagnostics.append(
            SkillDiagnostic(
                "warn",
                f"skill name '{name}' violates spec naming rules "
                "(lowercase alphanumeric + hyphens, no leading/trailing/double hyphens)",
                path,
            )
        )

    if name != parent_name:
        diagnostics.append(
            SkillDiagnostic(
                "warn",
                f"skill name '{name}' does not match parent directory '{parent_name}'",
                path,
            )
        )

    metadata_raw = data.get("metadata")
    metadata = {}
    if isinstance(metadata_raw, dict):
        metadata = {str(k): str(v) for k, v in metadata_raw.items()}

    skill = Skill(
        name=name,
        description=description.strip(),
        location=path.resolve(),
        base_dir=path.parent.resolve(),
        body=body,
        scope="project",  # may be overridden by caller
        compatibility=_string_field(data.get("compatibility")),
        license=_string_field(data.get("license")),
        metadata=metadata,
        allowed_tools=_string_field(data.get("allowed-tools")),
    )
    return skill, diagnostics


def _split_frontmatter(text: str) -> tuple[str | None, str]:
    if not text.startswith("---"):
        return None, text

    m = re.match(r"^---\s*\r?\n(.*?)\r?\n---\s*(?:\r?\n)?(.*)$", text, re.DOTALL)
    if not m:
        return None, text
    return m.group(1), m.group(2).strip("\n")


def _parse_yaml_lenient(
    frontmatter: str, path: Path, diagnostics: list[SkillDiagnostic]
) -> dict | None:
    try:
        data = yaml.safe_load(frontmatter)
    except yaml.YAMLError as exc:
        retried = _retry_quote_problem_fields(frontmatter)
        if retried is None:
            diagnostics.append(
                SkillDiagnostic("error", f"YAML parse error: {exc}", path)
            )
            return None
        try:
            data = yaml.safe_load(retried)
        except yaml.YAMLError as exc2:
            diagnostics.append(
                SkillDiagnostic(
                    "error", f"YAML parse error (after retry): {exc2}", path
                )
            )
            return None

    if not isinstance(data, dict):
        diagnostics.append(
            SkillDiagnostic("error", "frontmatter is not a YAML mapping", path)
        )
        return None
    return data


_PROBLEM_FIELDS = ("description", "compatibility")


def _retry_quote_problem_fields(frontmatter: str) -> str | None:
    """Wrap unquoted values for fields prone to YAML colon-collisions in quotes."""
    lines = frontmatter.splitlines()
    out: list[str] = []
    changed = False
    for line in lines:
        m = re.match(r"^(\w[\w-]*)\s*:\s*(.*)$", line)
        if m and m.group(1) in _PROBLEM_FIELDS:
            key = m.group(1)
            value = m.group(2).rstrip()
            if value and not (
                (value.startswith("'") and value.endswith("'"))
                or (value.startswith('"') and value.endswith('"'))
            ):
                escaped = value.replace("\\", "\\\\").replace('"', '\\"')
                out.append(f'{key}: "{escaped}"')
                changed = True
                continue
        out.append(line)
    if not changed:
        return None
    return "\n".join(out)


def _string_field(value) -> str | None:
    if value is None:
        return None
    s = str(value).strip()
    return s or None


def _iter_skill_dirs(root: Path) -> Iterable[Path]:
    if not root.exists() or not root.is_dir():
        return

    try:
        children = sorted(root.iterdir(), key=lambda p: p.name)
    except OSError:
        return

    count = 0
    for child in children:
        count += 1
        if count > MAX_DIRS_SCANNED:
            log.warning(
                "exceeded max scan budget (%d) while looking for skills in %s",
                MAX_DIRS_SCANNED,
                root,
            )
            break
        if not child.is_dir():
            continue
        if child.name.startswith("."):
            continue
        yield child


def discover_skills(
    project_root: Path | None = None,
    include_user_scope: bool = True,
) -> tuple[list[Skill], list[SkillDiagnostic]]:
    """Discover all skills available to the current project.

    Precedence (highest first): project > user > agents. Project-scope
    skills shadow lower tiers with the same name; user-scope shadows
    agents-scope. Both ``~/.plano/skills/`` (Plano-native) and
    ``~/.agents/skills/`` (the universal Agent Skills install location used
    by ``npx skills add``) are treated as auto-trusted user-tier scopes.

    Returns ``(skills, diagnostics)`` sorted by name.
    """
    project_root = find_project_root(project_root)
    project_dir = project_root / PROJECT_SKILLS_DIR

    skills_by_name: dict[str, Skill] = {}
    diagnostics: list[SkillDiagnostic] = []

    if include_user_scope:
        # Load lowest precedence first so higher tiers shadow.
        for skill_dir in _iter_skill_dirs(AGENTS_SKILLS_DIR):
            skill_md = skill_dir / "SKILL.md"
            if not skill_md.exists():
                continue
            skill, diags = parse_skill_md(skill_md)
            diagnostics.extend(diags)
            if skill is not None:
                skill = _set_scope(skill, "agents")
                skills_by_name[skill.name] = skill

        for skill_dir in _iter_skill_dirs(USER_SKILLS_DIR):
            skill_md = skill_dir / "SKILL.md"
            if not skill_md.exists():
                continue
            skill, diags = parse_skill_md(skill_md)
            diagnostics.extend(diags)
            if skill is None:
                continue
            skill = _set_scope(skill, "user")
            existing = skills_by_name.get(skill.name)
            if existing is not None and existing.scope == "agents":
                diagnostics.append(
                    SkillDiagnostic(
                        "warn",
                        f"user-scope skill '{skill.name}' shadows ~/.agents/skills entry at {existing.location}",
                        skill.location,
                    )
                )
            skills_by_name[skill.name] = skill

    for skill_dir in _iter_skill_dirs(project_dir):
        skill_md = skill_dir / "SKILL.md"
        if not skill_md.exists():
            continue
        skill, diags = parse_skill_md(skill_md)
        diagnostics.extend(diags)
        if skill is None:
            continue
        skill = _set_scope(skill, "project")
        existing = skills_by_name.get(skill.name)
        if existing is not None and existing.scope in ("user", "agents"):
            diagnostics.append(
                SkillDiagnostic(
                    "warn",
                    f"project-scope skill '{skill.name}' shadows {existing.scope}-scope skill at {existing.location}",
                    skill.location,
                )
            )
        skills_by_name[skill.name] = skill

    return sorted(skills_by_name.values(), key=lambda s: s.name), diagnostics


def _set_scope(skill: Skill, scope: str) -> Skill:
    return Skill(
        name=skill.name,
        description=skill.description,
        location=skill.location,
        base_dir=skill.base_dir,
        body=skill.body,
        scope=scope,
        compatibility=skill.compatibility,
        license=skill.license,
        metadata=skill.metadata,
        allowed_tools=skill.allowed_tools,
    )


def total_catalog_size(skills: Iterable[Skill]) -> int:
    """Approximate byte size of the catalog the orchestrator will receive."""
    return sum(len(s.name) + len(s.description) for s in skills)


def filter_skills_by_allow_list(
    skills: Iterable[Skill], allow_list: Iterable[str] | None
) -> list[Skill]:
    """Filter skills to those whose `name` appears in `allow_list`.

    If `allow_list` is None, returns all skills. Unknown names are silently
    dropped — callers warn at config-validation time.
    """
    if allow_list is None:
        return list(skills)
    allowed = set(allow_list)
    return [s for s in skills if s.name in allowed]
