"""Tests for cli/planoai/skills.py and the config-rendering hooks that
materialize SKILL.md bodies into the rendered plano config.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
from unittest import mock

import pytest

from planoai.skills import (
    AGENTS_SKILLS_DIR,
    PROJECT_SKILLS_DIR,
    USER_SKILLS_DIR,
    Skill,
    discover_skills,
    parse_skill_md,
    total_catalog_size,
)


@pytest.fixture(autouse=True)
def _isolate_user_scopes(tmp_path, monkeypatch):
    """Default both user-tier scopes to non-existent dirs so the dev's real
    ~/.plano/skills and ~/.agents/skills cannot bleed into tests.
    """
    monkeypatch.setattr("planoai.skills.USER_SKILLS_DIR", tmp_path / "_no_user_skills")
    monkeypatch.setattr(
        "planoai.skills.AGENTS_SKILLS_DIR", tmp_path / "_no_agents_skills"
    )


def _write_skill(
    base: Path,
    name: str,
    description: str = "Process PDFs. Use when handling PDF files.",
    body: str = "# Body\n\nDo the thing.",
    extra_frontmatter: str = "",
) -> Path:
    skill_dir = base / name
    skill_dir.mkdir(parents=True, exist_ok=True)
    frontmatter = f"name: {name}\ndescription: {description}\n{extra_frontmatter}"
    (skill_dir / "SKILL.md").write_text(
        f"---\n{frontmatter}---\n\n{body}",
        encoding="utf-8",
    )
    return skill_dir / "SKILL.md"


def test_parse_skill_md_minimal(tmp_path):
    skill_md = _write_skill(tmp_path, "pdf-processing")

    skill, diagnostics = parse_skill_md(skill_md)

    assert skill is not None
    assert skill.name == "pdf-processing"
    assert skill.description.startswith("Process PDFs")
    assert "Do the thing." in skill.body
    assert skill.location == skill_md.resolve()
    assert skill.base_dir == skill_md.parent.resolve()
    # No warnings or errors for a well-formed skill.
    assert diagnostics == []


def test_parse_skill_md_lenient_when_name_mismatches_directory(tmp_path):
    skill_dir = tmp_path / "some-dir"
    skill_dir.mkdir()
    (skill_dir / "SKILL.md").write_text(
        "---\nname: different-name\ndescription: ok\n---\n\nbody",
        encoding="utf-8",
    )

    skill, diagnostics = parse_skill_md(skill_dir / "SKILL.md")

    assert skill is not None
    assert skill.name == "different-name"
    assert any(
        "does not match parent directory" in d.message for d in diagnostics
    ), diagnostics


def test_parse_skill_md_warns_on_invalid_name(tmp_path):
    skill_dir = tmp_path / "Bad-Name"
    skill_dir.mkdir()
    (skill_dir / "SKILL.md").write_text(
        "---\nname: Bad-Name\ndescription: ok\n---\n\nbody",
        encoding="utf-8",
    )

    skill, diagnostics = parse_skill_md(skill_dir / "SKILL.md")
    assert skill is not None
    assert any("violates spec naming rules" in d.message for d in diagnostics)


def test_parse_skill_md_recovers_from_unquoted_colons(tmp_path):
    skill_dir = tmp_path / "code-review"
    skill_dir.mkdir()
    (skill_dir / "SKILL.md").write_text(
        "---\nname: code-review\n"
        "description: Use this skill when: the user asks about code review\n"
        "---\n\nbody",
        encoding="utf-8",
    )

    skill, diagnostics = parse_skill_md(skill_dir / "SKILL.md")
    # Lenient parse retries with quoted values and succeeds.
    assert skill is not None
    assert skill.name == "code-review"
    assert "Use this skill when:" in skill.description


def test_parse_skill_md_rejects_when_description_missing(tmp_path):
    skill_dir = tmp_path / "broken"
    skill_dir.mkdir()
    (skill_dir / "SKILL.md").write_text(
        "---\nname: broken\n---\n\nbody",
        encoding="utf-8",
    )

    skill, diagnostics = parse_skill_md(skill_dir / "SKILL.md")
    assert skill is None
    assert any(d.severity == "error" for d in diagnostics)


def test_parse_skill_md_rejects_when_frontmatter_missing(tmp_path):
    skill_dir = tmp_path / "no-frontmatter"
    skill_dir.mkdir()
    (skill_dir / "SKILL.md").write_text("just markdown", encoding="utf-8")

    skill, diagnostics = parse_skill_md(skill_dir / "SKILL.md")
    assert skill is None
    assert any("frontmatter" in d.message for d in diagnostics)


def test_discover_skills_project_only(tmp_path, monkeypatch):
    (tmp_path / ".plano").mkdir()
    project_skills_dir = tmp_path / ".plano" / "skills"
    project_skills_dir.mkdir()
    _write_skill(project_skills_dir, "pdf-processing")
    _write_skill(project_skills_dir, "code-review")

    skills, diagnostics = discover_skills(
        project_root=tmp_path, include_user_scope=False
    )

    names = sorted(s.name for s in skills)
    assert names == ["code-review", "pdf-processing"]
    assert all(s.scope == "project" for s in skills)


def test_discover_skills_picks_up_agents_scope(tmp_path, monkeypatch):
    """`npx skills add` writes into ~/.agents/skills/<name>. That directory
    must be discovered as a user-tier (auto-trusted) scope so the upstream
    CLI works without Plano-specific awareness.
    """
    agents_dir = tmp_path / "fake-home" / ".agents" / "skills"
    agents_dir.mkdir(parents=True)
    _write_skill(agents_dir, "pdf", description="agents-scope description")

    monkeypatch.setattr("planoai.skills.AGENTS_SKILLS_DIR", agents_dir)

    (tmp_path / ".plano").mkdir()
    (tmp_path / ".plano" / "skills").mkdir()

    skills, _ = discover_skills(project_root=tmp_path, include_user_scope=True)
    by_name = {s.name: s for s in skills}
    assert "pdf" in by_name
    assert by_name["pdf"].scope == "agents"
    assert by_name["pdf"].description == "agents-scope description"


def test_discover_skills_user_scope_shadows_agents_scope(tmp_path, monkeypatch):
    """When the same skill name exists in both ~/.plano/skills and
    ~/.agents/skills, the Plano-native one wins and a diagnostic is emitted.

    Project root lives in its own subtree (with no .plano/ ancestor) so it
    cannot collide with the patched user-scope dir.
    """
    home = tmp_path / "fake-home"
    home.mkdir()
    agents_dir = home / ".agents" / "skills"
    agents_dir.mkdir(parents=True)
    _write_skill(agents_dir, "pdf", description="agents copy")
    monkeypatch.setattr("planoai.skills.AGENTS_SKILLS_DIR", agents_dir)

    user_dir = home / ".plano" / "skills"
    user_dir.mkdir(parents=True)
    _write_skill(user_dir, "pdf", description="user copy")
    monkeypatch.setattr("planoai.skills.USER_SKILLS_DIR", user_dir)

    project_root = tmp_path / "elsewhere" / "proj"
    project_root.mkdir(parents=True)
    skills, diagnostics = discover_skills(
        project_root=project_root, include_user_scope=True
    )
    by_name = {s.name: s for s in skills}
    assert by_name["pdf"].scope == "user"
    assert by_name["pdf"].description == "user copy"
    assert any("shadows ~/.agents/skills" in d.message for d in diagnostics)


def test_discover_skills_project_overrides_user_scope(tmp_path, monkeypatch):
    user_skills_dir = tmp_path / "fake-home" / ".plano" / "skills"
    user_skills_dir.mkdir(parents=True)
    _write_skill(user_skills_dir, "shared", description="user-scope description")

    monkeypatch.setattr("planoai.skills.USER_SKILLS_DIR", user_skills_dir)

    (tmp_path / ".plano").mkdir()
    project_skills_dir = tmp_path / ".plano" / "skills"
    project_skills_dir.mkdir()
    _write_skill(project_skills_dir, "shared", description="project-scope description")
    _write_skill(project_skills_dir, "only-project")

    skills, diagnostics = discover_skills(
        project_root=tmp_path, include_user_scope=True
    )

    by_name = {s.name: s for s in skills}
    assert by_name["shared"].scope == "project"
    assert by_name["shared"].description == "project-scope description"
    assert by_name["only-project"].scope == "project"
    assert any("shadows user-scope skill" in d.message for d in diagnostics)


def test_total_catalog_size_counts_name_and_description():
    skills = [
        Skill(
            name="a",
            description="d1",
            location=Path("/x"),
            base_dir=Path("/"),
            body="b",
            scope="project",
        ),
        Skill(
            name="bb",
            description="dd",
            location=Path("/y"),
            base_dir=Path("/"),
            body="b",
            scope="project",
        ),
    ]
    assert total_catalog_size(skills) == (1 + 2) + (2 + 2)


def test_materialize_skills_in_config_default_inlines_bodies(tmp_path, monkeypatch):
    project_root = tmp_path
    (project_root / ".plano").mkdir()
    project_skills_dir = project_root / ".plano" / "skills"
    project_skills_dir.mkdir()
    _write_skill(
        project_skills_dir,
        "pdf-processing",
        description="Process PDFs.",
        body="# Body\nfollow these steps.",
    )

    fake_home = tmp_path / "home"
    fake_home.mkdir()
    monkeypatch.setenv("HOME", str(fake_home))
    # Mark the project trusted so .plano/skills is loaded.
    trusted = fake_home / ".plano" / "trusted_projects.json"
    trusted.parent.mkdir(parents=True, exist_ok=True)
    trusted.write_text(
        json.dumps({"trusted_projects": [str(project_root.resolve())]}),
        encoding="utf-8",
    )
    monkeypatch.setattr(
        "planoai.skills.USER_SKILLS_DIR",
        fake_home / ".plano" / "skills",
    )

    from planoai.config_generator import materialize_skills_in_config

    config_yaml = {"version": "v0.4.0"}
    materialize_skills_in_config(config_yaml, project_root)

    assert "skills" in config_yaml
    materialized = config_yaml["skills"]
    assert len(materialized) == 1
    entry = materialized[0]
    assert entry["name"] == "pdf-processing"
    assert "follow these steps." in entry["body"]
    assert entry["scope"] == "project"


def test_materialize_skills_in_config_skips_untrusted_project_skills(
    tmp_path, monkeypatch
):
    project_root = tmp_path
    (project_root / ".plano").mkdir()
    project_skills_dir = project_root / ".plano" / "skills"
    project_skills_dir.mkdir()
    _write_skill(project_skills_dir, "pdf-processing")

    fake_home = tmp_path / "home"
    fake_home.mkdir()
    monkeypatch.setenv("HOME", str(fake_home))
    monkeypatch.setattr(
        "planoai.skills.USER_SKILLS_DIR",
        fake_home / ".plano" / "skills",
    )

    from planoai.config_generator import materialize_skills_in_config

    config_yaml = {"version": "v0.4.0"}
    materialize_skills_in_config(config_yaml, project_root)
    # Untrusted -> project skills are not loaded.
    assert "skills" not in config_yaml


def test_materialize_skills_in_config_loads_agents_scope_without_trust(
    tmp_path, monkeypatch
):
    """Even with no project trust, skills installed by `npx skills add` into
    ~/.agents/skills/<name> must materialize into the rendered config — that
    directory is the universal Agent Skills install location and is
    user-tier, not project-tier.
    """
    project_root = tmp_path / "proj"
    (project_root / ".plano").mkdir(parents=True)

    fake_home = tmp_path / "home"
    fake_home.mkdir()
    monkeypatch.setenv("HOME", str(fake_home))
    monkeypatch.setattr(
        "planoai.skills.USER_SKILLS_DIR",
        fake_home / ".plano" / "skills",
    )

    agents_dir = fake_home / ".agents" / "skills"
    agents_dir.mkdir(parents=True)
    _write_skill(agents_dir, "pdf", body="# Body\nhandle the pdf.")
    monkeypatch.setattr("planoai.skills.AGENTS_SKILLS_DIR", agents_dir)

    from planoai.config_generator import materialize_skills_in_config

    config_yaml = {"version": "v0.4.0"}
    materialize_skills_in_config(config_yaml, project_root)

    assert "skills" in config_yaml
    materialized = config_yaml["skills"]
    assert len(materialized) == 1
    assert materialized[0]["name"] == "pdf"
    assert materialized[0]["scope"] == "agents"
    assert "handle the pdf." in materialized[0]["body"]


def test_materialize_skills_in_config_respects_allow_list(tmp_path, monkeypatch):
    project_root = tmp_path
    (project_root / ".plano").mkdir()
    skills_dir = project_root / ".plano" / "skills"
    skills_dir.mkdir()
    _write_skill(skills_dir, "skill-a")
    _write_skill(skills_dir, "skill-b")

    fake_home = tmp_path / "home"
    fake_home.mkdir()
    monkeypatch.setenv("HOME", str(fake_home))
    trusted = fake_home / ".plano" / "trusted_projects.json"
    trusted.parent.mkdir(parents=True, exist_ok=True)
    trusted.write_text(
        json.dumps({"trusted_projects": [str(project_root.resolve())]}),
        encoding="utf-8",
    )
    monkeypatch.setattr(
        "planoai.skills.USER_SKILLS_DIR",
        fake_home / ".plano" / "skills",
    )

    from planoai.config_generator import materialize_skills_in_config

    config_yaml = {
        "version": "v0.4.0",
        "skills": ["skill-a"],
        "routing_preferences": [
            {
                "name": "demo route",
                "description": "demo",
                "models": ["openai/gpt-4o"],
                "skills": ["skill-a", "does-not-exist"],
            }
        ],
    }
    materialize_skills_in_config(config_yaml, project_root)

    assert [s["name"] for s in config_yaml["skills"]] == ["skill-a"]
    # Unknown allow-list entries are pruned but the known one is kept.
    assert config_yaml["routing_preferences"][0]["skills"] == ["skill-a"]
