from click.testing import CliRunner

from planoai.init_cmd import init


def test_init_clean_writes_empty_config(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)

    runner = CliRunner()
    result = runner.invoke(init, ["--clean"])

    assert result.exit_code == 0, result.output
    config_path = tmp_path / "config.yaml"
    assert config_path.exists()
    assert config_path.read_text(encoding="utf-8") == "\n"


def test_init_template_builtin_writes_config(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)

    runner = CliRunner()
    result = runner.invoke(init, ["--template", "coding_agent_routing"])

    assert result.exit_code == 0, result.output

    config_path = tmp_path / "config.yaml"
    assert config_path.exists()
    config_text = config_path.read_text(encoding="utf-8")
    assert "model_providers:" in config_text


def test_init_refuses_overwrite_without_force(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    (tmp_path / "config.yaml").write_text("hello", encoding="utf-8")

    runner = CliRunner()
    result = runner.invoke(init, ["--clean"])

    assert result.exit_code != 0
    assert "Refusing to overwrite" in result.output


def test_init_force_overwrites(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    (tmp_path / "config.yaml").write_text("hello", encoding="utf-8")

    runner = CliRunner()
    result = runner.invoke(init, ["--clean", "--force"])

    assert result.exit_code == 0, result.output
    assert (tmp_path / "config.yaml").read_text(encoding="utf-8") == "\n"
