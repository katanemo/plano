"""CLI tests for the `planoai skills` command group."""

from __future__ import annotations

import json
import os
from pathlib import Path
from unittest import mock

import pytest
from click.testing import CliRunner

from planoai.skills_cmd import skills


def _seed_project(tmp_path: Path) -> Path:
    """Create a project that find_project_root will pick up via .plano/."""
    project = tmp_path / "project"
    project.mkdir()
    (project / ".plano").mkdir()
    return project


@pytest.fixture(autouse=True)
def _isolate_user_scopes(tmp_path, monkeypatch):
    """Default both user-tier scopes to non-existent dirs so the dev's real
    ~/.plano/skills and ~/.agents/skills cannot bleed into the test sandbox.
    Individual tests can override these via further monkeypatching.
    """
    monkeypatch.setattr("planoai.skills.USER_SKILLS_DIR", tmp_path / "no-user-skills")
    monkeypatch.setattr(
        "planoai.skills.AGENTS_SKILLS_DIR", tmp_path / "no-agents-skills"
    )


def _write_skill(base: Path, name: str, description: str = "demo skill") -> None:
    skill_dir = base / name
    skill_dir.mkdir(parents=True, exist_ok=True)
    (skill_dir / "SKILL.md").write_text(
        f"---\nname: {name}\ndescription: {description}\n---\n\nbody",
        encoding="utf-8",
    )


def test_list_empty(tmp_path, monkeypatch):
    project = _seed_project(tmp_path)
    monkeypatch.chdir(project)
    # Isolate user-scope skills dir.
    monkeypatch.setattr("planoai.skills.USER_SKILLS_DIR", tmp_path / "no-such-home")

    runner = CliRunner()
    result = runner.invoke(skills, ["list"])

    assert result.exit_code == 0, result.output
    assert "No skills installed" in result.output


def test_list_shows_project_skills(tmp_path, monkeypatch):
    project = _seed_project(tmp_path)
    monkeypatch.chdir(project)
    monkeypatch.setattr("planoai.skills.USER_SKILLS_DIR", tmp_path / "no-such-home")

    _write_skill(project / ".plano" / "skills", "pdf-processing")
    _write_skill(project / ".plano" / "skills", "code-review")

    runner = CliRunner()
    result = runner.invoke(skills, ["list", "--no-user-scope"])

    assert result.exit_code == 0, result.output
    assert "pdf-processing" in result.output
    assert "code-review" in result.output


def test_remove_deletes_skill_dir(tmp_path, monkeypatch):
    project = _seed_project(tmp_path)
    monkeypatch.chdir(project)
    monkeypatch.setattr("planoai.skills.USER_SKILLS_DIR", tmp_path / "no-such-home")

    skills_dir = project / ".plano" / "skills"
    _write_skill(skills_dir, "pdf-processing")
    (skills_dir / ".skills.json").write_text(
        json.dumps(
            {
                "skills": {
                    "pdf-processing": {
                        "source": "git",
                        "repo": "owner/pdf-processing",
                    }
                }
            }
        ),
        encoding="utf-8",
    )

    runner = CliRunner()
    result = runner.invoke(skills, ["remove", "pdf-processing"])

    assert result.exit_code == 0, result.output
    assert not (skills_dir / "pdf-processing").exists()
    manifest = json.loads((skills_dir / ".skills.json").read_text(encoding="utf-8"))
    assert "pdf-processing" not in manifest["skills"]


def test_remove_unknown_skill_errors(tmp_path, monkeypatch):
    project = _seed_project(tmp_path)
    monkeypatch.chdir(project)
    monkeypatch.setattr("planoai.skills.USER_SKILLS_DIR", tmp_path / "no-such-home")
    (project / ".plano" / "skills").mkdir()

    runner = CliRunner()
    result = runner.invoke(skills, ["remove", "nope"])
    assert result.exit_code != 0


def test_add_falls_back_to_git_when_no_npx(tmp_path, monkeypatch):
    project = _seed_project(tmp_path)
    monkeypatch.chdir(project)

    # Force the npx branch off and stub git clone to create a SKILL.md.
    monkeypatch.setattr("planoai.skills_cmd._has_npx", lambda: False)
    monkeypatch.setattr("planoai.skills_cmd._has_git", lambda: True)

    def fake_subprocess_run(cmd, **kwargs):
        # cmd is like ["git", "clone", ..., url, dest]
        dest = Path(cmd[-1])
        dest.mkdir(parents=True, exist_ok=True)
        (dest / "SKILL.md").write_text(
            "---\nname: my-skill\ndescription: example\n---\n\nbody",
            encoding="utf-8",
        )
        (dest / ".git").mkdir()
        return mock.Mock(returncode=0)

    monkeypatch.setattr("planoai.skills_cmd.subprocess.run", fake_subprocess_run)

    runner = CliRunner()
    result = runner.invoke(skills, ["add", "owner/my-skill"])
    assert result.exit_code == 0, result.output
    assert (project / ".plano" / "skills" / "my-skill" / "SKILL.md").exists()
    # Trust hint should be shown for untrusted projects with project-scope installs.
    assert "planoai skills trust" in result.output


def test_add_discovers_skill_installed_into_agents_scope_by_npx(tmp_path, monkeypatch):
    """`npx skills add` writes to ~/.agents/skills/<name>; planoai must
    pick it up from that universal scope and *not* nag about trust.
    """
    project = _seed_project(tmp_path)
    monkeypatch.chdir(project)

    agents_dir = tmp_path / "agents" / "skills"
    agents_dir.mkdir(parents=True)
    monkeypatch.setattr("planoai.skills.AGENTS_SKILLS_DIR", agents_dir)

    # Pretend npx is on $PATH and succeeds, dropping the skill in ~/.agents/skills
    # rather than .plano/skills (which is what the upstream CLI actually does).
    monkeypatch.setattr("planoai.skills_cmd._has_npx", lambda: True)

    def fake_install_via_npx(target, project_root, console):
        skill_dir = agents_dir / "pdf"
        skill_dir.mkdir(parents=True, exist_ok=True)
        (skill_dir / "SKILL.md").write_text(
            "---\nname: pdf\ndescription: process pdfs\n---\n\nbody",
            encoding="utf-8",
        )
        return True

    monkeypatch.setattr("planoai.skills_cmd._install_via_npx", fake_install_via_npx)

    runner = CliRunner()
    result = runner.invoke(skills, ["add", "openai/skills"])

    assert result.exit_code == 0, result.output
    assert "scope=agents" in result.output
    assert "planoai skills trust" not in result.output


def test_list_includes_agents_scope_entries(tmp_path, monkeypatch):
    project = _seed_project(tmp_path)
    monkeypatch.chdir(project)

    agents_dir = tmp_path / "agents-skills"
    agents_dir.mkdir()
    monkeypatch.setattr("planoai.skills.AGENTS_SKILLS_DIR", agents_dir)
    _write_skill(agents_dir, "pdf")

    runner = CliRunner()
    result = runner.invoke(skills, ["list"])

    assert result.exit_code == 0, result.output
    assert "pdf" in result.output
    assert "agents" in result.output


def test_remove_rejects_agents_scope_skill(tmp_path, monkeypatch):
    project = _seed_project(tmp_path)
    monkeypatch.chdir(project)
    (project / ".plano" / "skills").mkdir()

    agents_dir = tmp_path / "agents-skills"
    agents_dir.mkdir()
    monkeypatch.setattr("planoai.skills.AGENTS_SKILLS_DIR", agents_dir)
    _write_skill(agents_dir, "pdf")

    runner = CliRunner()
    result = runner.invoke(skills, ["remove", "pdf"])

    assert result.exit_code != 0
    assert "npx skills remove" in result.output


def test_trust_marks_project(tmp_path, monkeypatch):
    project = _seed_project(tmp_path)
    monkeypatch.chdir(project)

    fake_home = tmp_path / "home"
    fake_home.mkdir()
    monkeypatch.setenv("HOME", str(fake_home))

    runner = CliRunner()
    result = runner.invoke(skills, ["trust"])

    assert result.exit_code == 0, result.output
    trusted_file = fake_home / ".plano" / "trusted_projects.json"
    assert trusted_file.exists()
    data = json.loads(trusted_file.read_text(encoding="utf-8"))
    assert str(project.resolve()) in data["trusted_projects"]


def test_add_rejects_invalid_target(tmp_path, monkeypatch):
    project = _seed_project(tmp_path)
    monkeypatch.chdir(project)
    runner = CliRunner()
    result = runner.invoke(skills, ["add", "not-a-spec"])
    assert result.exit_code != 0
