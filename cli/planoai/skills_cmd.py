"""`planoai skills` command group.

Installs Agent Skills (https://agentskills.io) and surfaces them to Plano.

Three discovery scopes are supported, in descending precedence:

* ``<project>/.plano/skills/`` -- repo-pinned skills. Loaded only when the
  project has been marked trusted via ``planoai skills trust`` (skill content
  is injected into the orchestrator prompt, so we gate on trust).
* ``~/.plano/skills/`` -- Plano-native user-scope. Always trusted.
* ``~/.agents/skills/`` -- universal Agent Skills location used by
  ``npx skills add``. Always trusted; lets the upstream skills CLI work
  out of the box without any Plano-specific awareness.

``planoai skills add`` tries ``npx skills add`` first (the upstream CLI from
https://github.com/vercel-labs/add-skill), which writes to
``~/.agents/skills/<name>`` and is picked up automatically thanks to the
agents-scope above. Falls back to ``git clone`` into ``.plano/skills/`` when
``npx`` is unavailable.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

import rich_click as click
from rich.console import Console
from rich.table import Table

from planoai.consts import PLANO_COLOR
from planoai.skills import (
    PROJECT_SKILLS_DIR,
    Skill,
    discover_skills,
    find_project_root,
    is_project_trusted,
    trusted_projects_file,
)
from planoai.utils import getLogger

log = getLogger(__name__)

_OWNER_REPO_PATTERN = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")


@dataclass
class _InstallTarget:
    owner: str
    repo: str
    ref: str | None = None  # optional branch / tag / commit (e.g. "owner/repo@v1")

    @property
    def slug(self) -> str:
        return f"{self.owner}/{self.repo}"

    @property
    def url(self) -> str:
        return f"https://github.com/{self.owner}/{self.repo}.git"


def _console() -> Console:
    return Console()


def _ensure_skills_dir(project_root: Path) -> Path:
    skills_dir = project_root / PROJECT_SKILLS_DIR
    skills_dir.mkdir(parents=True, exist_ok=True)
    return skills_dir


def _parse_install_target(raw: str) -> _InstallTarget:
    spec = raw.strip()
    ref: str | None = None
    if "@" in spec:
        spec, _, ref_value = spec.partition("@")
        ref = ref_value.strip() or None
    if not _OWNER_REPO_PATTERN.match(spec):
        raise click.BadParameter(
            f"expected '<owner>/<repo>' (optionally suffixed with '@<ref>'), got '{raw}'"
        )
    owner, repo = spec.split("/", 1)
    return _InstallTarget(owner=owner, repo=repo, ref=ref)


def _has_npx() -> bool:
    return shutil.which("npx") is not None


def _has_git() -> bool:
    return shutil.which("git") is not None


def _mark_project_trusted(project_root: Path) -> None:
    path = trusted_projects_file()
    path.parent.mkdir(parents=True, exist_ok=True)
    existing: dict = {}
    if path.exists():
        try:
            with path.open("r", encoding="utf-8") as fh:
                existing = json.load(fh) or {}
        except (OSError, json.JSONDecodeError):
            existing = {}
    trusted = set(existing.get("trusted_projects", []) or [])
    trusted.add(str(project_root.resolve()))
    existing["trusted_projects"] = sorted(trusted)
    with path.open("w", encoding="utf-8") as fh:
        json.dump(existing, fh, indent=2)


def _read_manifest(skills_dir: Path) -> dict:
    manifest_path = skills_dir / ".skills.json"
    if not manifest_path.exists():
        return {"skills": {}}
    try:
        with manifest_path.open("r", encoding="utf-8") as fh:
            data = json.load(fh)
    except (OSError, json.JSONDecodeError):
        return {"skills": {}}
    if not isinstance(data, dict):
        return {"skills": {}}
    data.setdefault("skills", {})
    return data


def _write_manifest(skills_dir: Path, manifest: dict) -> None:
    manifest_path = skills_dir / ".skills.json"
    with manifest_path.open("w", encoding="utf-8") as fh:
        json.dump(manifest, fh, indent=2, sort_keys=True)


def _record_install(
    skills_dir: Path, name: str, target: _InstallTarget, source: str
) -> None:
    manifest = _read_manifest(skills_dir)
    manifest["skills"][name] = {
        "source": source,
        "repo": target.slug,
        "ref": target.ref,
        "installed_at": datetime.now(timezone.utc).isoformat(),
    }
    _write_manifest(skills_dir, manifest)


def _remove_from_manifest(skills_dir: Path, name: str) -> None:
    manifest = _read_manifest(skills_dir)
    manifest["skills"].pop(name, None)
    _write_manifest(skills_dir, manifest)


def _install_via_npx(
    target: _InstallTarget, project_root: Path, console: Console
) -> bool:
    """Try to install with `npx skills add`. Returns True on success."""
    env = os.environ.copy()
    env.setdefault("SKILLS_NO_TELEMETRY", "1")
    arg = target.slug if target.ref is None else f"{target.slug}@{target.ref}"
    cmd = ["npx", "--yes", "skills", "add", arg]
    console.print(
        f"[dim]Running:[/dim] [cyan]{' '.join(cmd)}[/cyan] [dim](cwd={project_root})[/dim]"
    )
    try:
        result = subprocess.run(
            cmd,
            cwd=project_root,
            env=env,
            check=False,
        )
    except FileNotFoundError:
        return False
    return result.returncode == 0


def _install_via_git(
    target: _InstallTarget,
    project_root: Path,
    skills_dir: Path,
    console: Console,
) -> bool:
    if not _has_git():
        console.print("[red]X[/red] git is not installed; cannot fall back from npx")
        return False

    dest = skills_dir / target.repo
    if dest.exists():
        console.print(
            f"[yellow]![/yellow] {dest} already exists. "
            "Remove it first with [cyan]planoai skills remove[/cyan] before reinstalling."
        )
        return False

    cmd = ["git", "clone", "--depth", "1"]
    if target.ref:
        cmd.extend(["--branch", target.ref])
    cmd.extend([target.url, str(dest)])
    console.print(
        f"[dim]Running:[/dim] [cyan]{' '.join(cmd)}[/cyan] [dim](cwd={project_root})[/dim]"
    )
    try:
        result = subprocess.run(cmd, cwd=project_root, check=False)
    except FileNotFoundError:
        console.print("[red]X[/red] git binary not found")
        return False
    if result.returncode != 0:
        return False

    shutil.rmtree(dest / ".git", ignore_errors=True)

    if not (dest / "SKILL.md").exists():
        console.print(
            f"[red]X[/red] {target.slug} does not contain a SKILL.md at its repo root; "
            "this does not appear to be a valid Agent Skill."
        )
        shutil.rmtree(dest, ignore_errors=True)
        return False
    return True


def _print_skills_table(console: Console, skills: list[Skill]) -> None:
    if not skills:
        console.print(
            f"[dim]No skills installed.[/dim] Try [cyan]planoai skills add owner/repo[/cyan]."
        )
        return
    table = Table(title="Installed Agent Skills", border_style="dim")
    table.add_column("Name", style=f"bold {PLANO_COLOR}")
    table.add_column("Scope")
    table.add_column("Description")
    table.add_column("Path", style="dim")
    for s in skills:
        desc = s.description.splitlines()[0]
        if len(desc) > 80:
            desc = desc[:77] + "..."
        table.add_row(s.name, s.scope, desc, str(s.location))
    console.print(table)


@click.group(name="skills")
def skills():
    """Manage Agent Skills (agentskills.io) for this Plano project."""


@skills.command(name="add")
@click.argument("target", required=True)
@click.option(
    "--path",
    default=".",
    help="Project directory (defaults to the directory containing .plano/ or .git/).",
)
def add_cmd(target: str, path: str):
    """Install an Agent Skill from a GitHub repo into .plano/skills/.

    TARGET should be `owner/repo` (optionally suffixed with `@ref` for a branch
    or tag).
    """
    console = _console()
    install_target = _parse_install_target(target)
    project_root = find_project_root(Path(path).resolve())
    skills_dir = _ensure_skills_dir(project_root)

    console.print(
        f"[bold {PLANO_COLOR}]Installing skill[/bold {PLANO_COLOR}] "
        f"[cyan]{install_target.slug}[/cyan] -> [dim]{skills_dir}[/dim]"
    )

    # Snapshot what's already discoverable so we can diff after install and
    # surface every newly-added skill regardless of which scope it landed in
    # (project for git fallback, agents for `npx skills add`, etc.) and
    # regardless of how the installed skill name maps to the repo name (e.g.
    # multi-skill repos like openai/skills -> ~/.agents/skills/pdf).
    before, _ = discover_skills(project_root=project_root, include_user_scope=True)
    before_keys = {(s.name, str(s.base_dir)) for s in before}

    used_source: str
    success = False
    if _has_npx():
        success = _install_via_npx(install_target, project_root, console)
        used_source = "npx-skills"
        if not success:
            console.print(
                "[yellow]![/yellow] npx skills add did not succeed; "
                "falling back to direct git clone."
            )
    if not success:
        success = _install_via_git(install_target, project_root, skills_dir, console)
        used_source = "git"

    if not success:
        console.print(
            f"[red]X[/red] Failed to install [cyan]{install_target.slug}[/cyan]"
        )
        sys.exit(1)

    discovered, diagnostics = discover_skills(
        project_root=project_root, include_user_scope=True
    )
    for diag in diagnostics:
        if diag.severity == "error":
            console.print(f"[red]X[/red] {diag.path}: {diag.message}")
        else:
            console.print(f"[yellow]![/yellow] {diag.path}: {diag.message}")

    newly_installed = [
        s for s in discovered if (s.name, str(s.base_dir)) not in before_keys
    ]
    if newly_installed:
        for s in newly_installed:
            if s.scope == "project":
                _record_install(skills_dir, s.name, install_target, used_source)
            try:
                display_path = s.location.relative_to(project_root)
            except ValueError:
                display_path = s.location
            console.print(
                f"[green]+[/green] Installed [bold]{s.name}[/bold] "
                f"[dim]({display_path}, scope={s.scope})[/dim]"
            )
        if any(
            s.scope == "project" for s in newly_installed
        ) and not is_project_trusted(project_root):
            console.print(
                "\n[dim]Project-scope skills are not auto-loaded until this project is "
                "trusted. Run[/dim] [cyan]planoai skills trust[/cyan] [dim]to enable them.[/dim]"
            )
    else:
        console.print(
            "[yellow]![/yellow] Install reported success but no new SKILL.md "
            "was discovered under .plano/skills, ~/.plano/skills, or "
            "~/.agents/skills. Check the repo structure or pass a "
            "single-skill repo."
        )


@skills.command(name="list")
@click.option(
    "--path",
    default=".",
    help="Project directory (defaults to the directory containing .plano/ or .git/).",
)
@click.option(
    "--no-user-scope",
    is_flag=True,
    default=False,
    help="Skip user-scope skills under ~/.plano/skills and ~/.agents/skills.",
)
def list_cmd(path: str, no_user_scope: bool):
    """List discovered Agent Skills across project / user / agents scopes."""
    console = _console()
    project_root = find_project_root(Path(path).resolve())
    discovered, diagnostics = discover_skills(
        project_root=project_root, include_user_scope=not no_user_scope
    )
    _print_skills_table(console, discovered)

    if diagnostics:
        console.print()
        for diag in diagnostics:
            color = "red" if diag.severity == "error" else "yellow"
            marker = "X" if diag.severity == "error" else "!"
            console.print(f"[{color}]{marker}[/{color}] {diag.path}: {diag.message}")


@skills.command(name="remove")
@click.argument("name", required=True)
@click.option(
    "--path",
    default=".",
    help="Project directory (defaults to the directory containing .plano/ or .git/).",
)
def remove_cmd(name: str, path: str):
    """Remove a project-scope skill from .plano/skills/.

    User-scope skills under ~/.plano/skills or ~/.agents/skills must be
    removed with their respective installer (`npx skills remove <name>` for
    the latter); planoai will not touch directories outside the project.
    """
    console = _console()
    project_root = find_project_root(Path(path).resolve())
    skills_dir = project_root / PROJECT_SKILLS_DIR
    if not skills_dir.exists():
        console.print(f"[red]X[/red] no skills directory at {skills_dir}")
        sys.exit(1)

    target_dir = skills_dir / name
    if not target_dir.exists():
        discovered, _ = discover_skills(
            project_root=project_root, include_user_scope=True
        )
        project_match = next(
            (s for s in discovered if s.name == name and s.scope == "project"), None
        )
        if project_match is None:
            other = next((s for s in discovered if s.name == name), None)
            if other is not None:
                console.print(
                    f"[red]X[/red] '{name}' is installed in {other.scope} scope at "
                    f"{other.base_dir}; planoai only removes project-scope skills. "
                    "Use the upstream installer (e.g. `npx skills remove`) for that one."
                )
            else:
                console.print(
                    f"[red]X[/red] no project-scope skill named '{name}' under {skills_dir}"
                )
            sys.exit(1)
        target_dir = project_match.base_dir

    if not target_dir.resolve().is_relative_to(skills_dir.resolve()):
        console.print(
            f"[red]X[/red] refusing to delete {target_dir} (outside {skills_dir})"
        )
        sys.exit(1)

    shutil.rmtree(target_dir, ignore_errors=False)
    _remove_from_manifest(skills_dir, name)
    console.print(f"[green]+[/green] Removed [bold]{name}[/bold]")


@skills.command(name="trust")
@click.option(
    "--path",
    default=".",
    help="Project directory to mark as trusted.",
)
@click.option(
    "--revoke",
    is_flag=True,
    default=False,
    help="Revoke trust instead of granting it.",
)
def trust_cmd(path: str, revoke: bool):
    """Mark this project's .plano/skills/ as trusted for auto-loading.

    Project-scope skills come from the working directory's repo, which may
    be untrusted. Plano refuses to inject their contents into the
    orchestrator prompt until you trust the project.
    """
    console = _console()
    project_root = find_project_root(Path(path).resolve())

    if revoke:
        path = trusted_projects_file()
        if not path.exists():
            console.print("[dim]No trusted projects to revoke.[/dim]")
            return
        try:
            with path.open("r", encoding="utf-8") as fh:
                data = json.load(fh) or {}
        except (OSError, json.JSONDecodeError):
            data = {}
        trusted = {
            str(Path(p).resolve()) for p in data.get("trusted_projects", []) or []
        }
        trusted.discard(str(project_root.resolve()))
        data["trusted_projects"] = sorted(trusted)
        with path.open("w", encoding="utf-8") as fh:
            json.dump(data, fh, indent=2)
        console.print(f"[green]+[/green] Revoked trust for [bold]{project_root}[/bold]")
        return

    _mark_project_trusted(project_root)
    console.print(
        f"[green]+[/green] Trusted [bold]{project_root}[/bold].\n"
        f"[dim]Project-scope skills under .plano/skills/ will now be loaded at startup.[/dim]"
    )
