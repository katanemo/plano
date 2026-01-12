import re
import os
from dataclasses import dataclass
from pathlib import Path

import rich_click as click
from rich.console import Console
from rich.panel import Panel

from planoai.consts import PLANO_COLOR
from planoai.utils import get_llm_provider_access_keys, find_repo_root


@dataclass(frozen=True)
class Template:
    """
    A Plano config template.

    - id: stable identifier used by --template
    - title/description: UI strings
    - yaml_text: embedded template contents (works in PyPI installs)
    - repo_path: optional path to a real demos/.../config.yaml when running in-repo
    """

    id: str
    title: str
    description: str
    yaml_text: str | None = None
    repo_path: str | None = None


BUILTIN_TEMPLATES: list[Template] = [
    Template(
        id="samples_python/weather_forecast",
        title="samples_python/weather_forecast",
        description="prompt targets + multiple LLMs (OpenAI/Groq/Anthropic)",
        yaml_text="""version: v0.1.0

listeners:
  ingress_traffic:
    address: 0.0.0.0
    port: 10000
    message_format: openai
    timeout: 30s

  egress_traffic:
    address: 0.0.0.0
    port: 12000
    message_format: openai
    timeout: 30s

endpoints:
  weather_forecast_service:
    endpoint: host.docker.internal:18083
    connect_timeout: 0.005s

overrides:
  prompt_target_intent_matching_threshold: 0.6

llm_providers:
  - access_key: $GROQ_API_KEY
    model: groq/llama-3.2-3b-preview

  - access_key: $OPENAI_API_KEY
    model: openai/gpt-4o
    default: true

  - access_key: $OPENAI_API_KEY
    model: openai/gpt-4o-mini

  - access_key: $ANTHROPIC_API_KEY
    model: anthropic/claude-sonnet-4-20250514

system_prompt: |
  You are a helpful assistant.

prompt_targets:
  - name: get_current_weather
    description: Get current weather at a location.
    parameters:
      - name: location
        description: The location to get the weather for
        required: true
        type: string
        format: City, State
      - name: days
        description: the number of days for the request
        required: true
        type: int
    endpoint:
      name: weather_forecast_service
      path: /weather
      http_method: POST

  - name: default_target
    default: true
    description: This is the default target for all unmatched prompts.
    endpoint:
      name: weather_forecast_service
      path: /default_target
      http_method: POST
    system_prompt: |
      You are a helpful assistant! Summarize the user's request and provide a helpful response.
    auto_llm_dispatch_on_response: false

tracing:
  random_sampling: 100
  trace_arch_internal: true
""",
    ),
    Template(
        id="samples_python/stock_quote",
        title="samples_python/stock_quote",
        description="external API headers ($TWELVEDATA_API_KEY) + prompt targets",
        yaml_text="""version: v0.1.0

listeners:
  ingress_traffic:
    address: 0.0.0.0
    port: 10000
    message_format: openai
    timeout: 30s

llm_providers:
  - access_key: $OPENAI_API_KEY
    model: openai/gpt-4o

endpoints:
  twelvedata_api:
    endpoint: api.twelvedata.com
    protocol: https

system_prompt: |
  You are a helpful assistant.

prompt_targets:
  - name: stock_quote
    description: get current stock exchange rate for a given symbol
    parameters:
      - name: symbol
        description: Stock symbol
        required: true
        type: str
    endpoint:
      name: twelvedata_api
      path: /quote
      http_headers:
        Authorization: "apikey $TWELVEDATA_API_KEY"
    system_prompt: |
      You are a helpful stock exchange assistant. Parse the JSON and present it in a human-readable format. Be concise.

  - name: stock_quote_time_series
    description: get historical stock exchange rate for a given symbol
    parameters:
      - name: symbol
        description: Stock symbol
        required: true
        type: str
      - name: interval
        description: Time interval
        default: 1day
        enum:
          - 1h
          - 1day
        type: str
    endpoint:
      name: twelvedata_api
      path: /time_series
      http_headers:
        Authorization: "apikey $TWELVEDATA_API_KEY"
    system_prompt: |
      You are a helpful stock exchange assistant. Parse the JSON and present it in a human-readable format. Be concise.

tracing:
  random_sampling: 100
  trace_arch_internal: true
""",
    ),
    Template(
        id="use_cases/claude_code_router",
        title="use_cases/claude_code_router",
        description="multi-model routing preferences + model_aliases (good for CLI agents)",
        yaml_text="""version: v0.1

listeners:
  egress_traffic:
    address: 0.0.0.0
    port: 12000
    message_format: openai
    timeout: 30s

llm_providers:
  - model: openai/gpt-5-2025-08-07
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: code generation
        description: generating new code snippets, functions, or boilerplate based on user prompts or requirements

  - model: openai/gpt-4.1-2025-04-14
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: code understanding
        description: understand and explain existing code snippets, functions, or libraries

  - model: anthropic/claude-sonnet-4-5
    default: true
    access_key: $ANTHROPIC_API_KEY

  - model: anthropic/claude-haiku-4-5
    access_key: $ANTHROPIC_API_KEY

  - model: ollama/llama3.1
    base_url: http://host.docker.internal:11434

model_aliases:
  arch.claude.code.small.fast:
    target: claude-haiku-4-5

tracing:
  random_sampling: 100
""",
    ),
    Template(
        id="use_cases/ollama",
        title="use_cases/ollama",
        description="local LLM via base_url (OpenAI-compatible provider_interface)",
        yaml_text="""version: v0.1.0

listeners:
  egress_traffic:
    address: 0.0.0.0
    port: 12000
    message_format: openai
    timeout: 30s

llm_providers:
  - model: my_llm_provider/llama3.2
    provider_interface: openai
    base_url: http://host.docker.internal:11434
    default: true

system_prompt: |
  You are a helpful assistant.

tracing:
  random_sampling: 100
  trace_arch_internal: true
""",
    ),
]


def _discover_repo_demo_templates(repo_root: str | None) -> dict[str, str]:
    """
    Returns mapping from template id -> absolute config.yaml path for repo demos.
    This is best-effort and should be fast; built-in templates remain the default.
    """
    if not repo_root:
        return {}
    demos_dir = Path(repo_root) / "demos"
    if not demos_dir.exists():
        return {}

    result: dict[str, str] = {}
    # keep it bounded: just walk demos and match config.yaml (small tree)
    for cfg in demos_dir.rglob("config.yaml"):
        try:
            rel = cfg.relative_to(demos_dir).as_posix()
        except Exception:
            continue
        template_id = rel.removesuffix("/config.yaml")
        result[template_id] = str(cfg)
    return result


def _get_templates() -> list[Template]:
    repo_root = find_repo_root()
    repo_templates = _discover_repo_demo_templates(repo_root)
    templates: list[Template] = []
    for t in BUILTIN_TEMPLATES:
        repo_path = repo_templates.get(t.id)
        templates.append(
            Template(
                id=t.id,
                title=t.title,
                description=t.description,
                yaml_text=t.yaml_text,
                repo_path=repo_path,
            )
        )

    # Add any extra demo configs not represented by built-ins (no embedded yaml).
    builtin_ids = {t.id for t in templates}
    for template_id, path in sorted(repo_templates.items()):
        if template_id in builtin_ids:
            continue
        templates.append(
            Template(
                id=template_id,
                title=template_id,
                description="(repo demo)",
                yaml_text=None,
                repo_path=path,
            )
        )
    return templates


def _resolve_template(template_id_or_path: str | None) -> Template | None:
    if not template_id_or_path:
        return None

    # 1) explicit path
    p = Path(template_id_or_path)
    if p.exists() and p.is_file():
        return Template(
            id=str(p),
            title=str(p),
            description="(file)",
            yaml_text=None,
            repo_path=str(p.resolve()),
        )

    # 2) known id
    templates = _get_templates()
    for t in templates:
        if t.id == template_id_or_path:
            return t

    return None


def _ensure_parent_dir(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)


def _write_clean_config(path: Path, force: bool) -> None:
    _ensure_parent_dir(path)
    if path.exists() and not force:
        raise FileExistsError(str(path))
    # user asked for NOTHING in it: empty file, with just a newline for POSIX friendliness
    path.write_text("\n", encoding="utf-8")


def _write_template_config(path: Path, template: Template, force: bool) -> str:
    _ensure_parent_dir(path)
    if path.exists() and not force:
        raise FileExistsError(str(path))

    if template.repo_path:
        src = Path(template.repo_path)
        text = src.read_text(encoding="utf-8")
        path.write_text(text, encoding="utf-8")
        return f"repo:{template.repo_path}"

    if template.yaml_text is None:
        raise ValueError(f"Template '{template.id}' is not available in this install.")

    path.write_text(template.yaml_text, encoding="utf-8")
    return "builtin"


_ENV_VAR_PATTERN = re.compile(r"\$\{?([A-Z_][A-Z0-9_]*)\}?")


def _extract_env_vars(config_path: Path) -> list[str]:
    """
    Extract env vars referenced by the config so we can offer .env placeholders.
    Uses existing logic (headers/model providers/etc) plus a regex fallback.
    """
    keys: set[str] = set()
    try:
        extracted = get_llm_provider_access_keys(str(config_path))
        for item in extracted:
            if not item:
                continue
            if item.startswith("$"):
                keys.add(item[1:])
            else:
                # some cases may return raw vars
                keys.add(item)
    except Exception:
        # best-effort; still run regex scan
        pass

    try:
        text = config_path.read_text(encoding="utf-8")
        for m in _ENV_VAR_PATTERN.findall(text):
            keys.add(m)
    except Exception:
        pass

    # Filter obvious false positives if any ever appear
    keys.discard("HOST")
    keys.discard("PORT")
    return sorted(keys)


def _read_env_file_keys(env_path: Path) -> set[str]:
    if not env_path.exists():
        return set()
    keys: set[str] = set()
    for line in env_path.read_text(encoding="utf-8").splitlines():
        s = line.strip()
        if not s or s.startswith("#") or "=" not in s:
            continue
        k = s.split("=", 1)[0].strip()
        if k:
            keys.add(k)
    return keys


def _upsert_env_placeholders(env_path: Path, keys: list[str]) -> list[str]:
    """
    Create or append missing keys with blank values. Returns the keys actually added.
    """
    _ensure_parent_dir(env_path)
    existing = _read_env_file_keys(env_path)
    missing = [k for k in keys if k not in existing]
    if not missing:
        return []

    header = ""
    if env_path.exists():
        header = "\n# Added by `planoai init`\n"

    addition = header + "\n".join([f"{k}=" for k in missing]) + "\n"
    with env_path.open("a", encoding="utf-8") as f:
        f.write(addition)
    return missing


def _questionary_style():
    # prompt_toolkit style string format
    from prompt_toolkit.styles import Style

    return Style.from_dict(
        {
            "qmark": f"fg:{PLANO_COLOR} bold",
            "question": "bold",
            "answer": f"fg:{PLANO_COLOR} bold",
            "pointer": f"fg:{PLANO_COLOR} bold",
            "highlighted": f"fg:{PLANO_COLOR} bold",
            "selected": f"fg:{PLANO_COLOR}",
            "instruction": "fg:#888888",
            "text": "",
            "disabled": "fg:#666666",
        }
    )


def _force_truecolor_for_prompt_toolkit() -> None:
    """
    Ensure prompt_toolkit uses truecolor so our brand hex (#969FF4) renders correctly.
    Without this, some terminals or environments downgrade to 8-bit and the color
    can look like a generic blue.
    """
    # Only set if user hasn't explicitly chosen a depth.
    os.environ.setdefault("PROMPT_TOOLKIT_COLOR_DEPTH", "DEPTH_24_BIT")


@click.command()
@click.option(
    "--template",
    "template_id_or_path",
    default=None,
    help="Create config.yaml from a template id (e.g. use_cases/claude_code_router) or a path to a YAML file.",
)
@click.option(
    "--clean",
    is_flag=True,
    help="Create an empty config.yaml with no contents.",
)
@click.option(
    "--output",
    "-o",
    "output_path",
    default="config.yaml",
    show_default=True,
    help="Where to write the generated config.",
)
@click.option(
    "--force",
    is_flag=True,
    help="Overwrite existing config file if it already exists.",
)
@click.option(
    "--no-env",
    is_flag=True,
    help="Do not create/update a .env file.",
)
@click.option(
    "--yes",
    "-y",
    is_flag=True,
    help="Skip interactive prompts and accept defaults (will NOT overwrite without --force).",
)
@click.option(
    "--list-templates",
    is_flag=True,
    help="List available template ids and exit.",
)
@click.pass_context
def init(
    ctx, template_id_or_path, clean, output_path, force, no_env, yes, list_templates
):
    """Initialize a Plano config quickly (arrow-key interactive wizard by default)."""
    import sys

    console = Console()

    if clean and template_id_or_path:
        raise click.UsageError("Use either --clean or --template, not both.")

    templates = _get_templates()

    if list_templates:
        console.print(f"[bold {PLANO_COLOR}]Available templates[/bold {PLANO_COLOR}]\n")
        for t in templates:
            origin = (
                "repo" if t.repo_path else "builtin" if t.yaml_text else "repo-only"
            )
            console.print(
                f"  [bold]{t.id}[/bold]  [dim]({origin})[/dim]  - {t.description}"
            )
        return

    out_path = Path(output_path).expanduser()

    # Non-interactive fast paths
    if yes or clean or template_id_or_path:
        if clean:
            try:
                _write_clean_config(out_path, force=force)
            except FileExistsError:
                raise click.ClickException(
                    f"Refusing to overwrite existing file: {out_path} (use --force)"
                )
            console.print(f"[green]✓[/green] Wrote [bold]{out_path}[/bold]")
            return

        if template_id_or_path:
            template = _resolve_template(template_id_or_path)
            if not template:
                raise click.ClickException(
                    f"Unknown template: {template_id_or_path}\n"
                    f"Run: planoai init --list-templates"
                )
            try:
                origin = _write_template_config(out_path, template, force=force)
            except FileExistsError:
                raise click.ClickException(
                    f"Refusing to overwrite existing file: {out_path} (use --force)"
                )
            console.print(
                f"[green]✓[/green] Wrote [bold]{out_path}[/bold] [dim]({template.id}, {origin})[/dim]"
            )

            if no_env:
                return

            env_vars = _extract_env_vars(out_path)
            if env_vars:
                env_path = out_path.parent / ".env"
                added = _upsert_env_placeholders(env_path, env_vars)
                if added:
                    console.print(
                        f"[green]✓[/green] Updated [bold]{env_path}[/bold] [dim](added: {', '.join(added)})[/dim]"
                    )
                else:
                    console.print(f"[dim]✓ .env already contains required keys[/dim]")
            return

        # yes without clean/template means: do nothing useful
        raise click.UsageError(
            "Non-interactive mode requires --template or --clean (or omit --yes for the interactive wizard)."
        )

    # Interactive wizard
    if not (sys.stdin.isatty() and sys.stdout.isatty()):
        raise click.ClickException(
            "Interactive mode requires a TTY.\n"
            "Use one of:\n"
            "  planoai init --template <id>\n"
            "  planoai init --clean\n"
            "  planoai init --list-templates"
        )

    _force_truecolor_for_prompt_toolkit()

    # Lazy import so non-interactive users don't pay the import/compat cost
    import questionary
    from questionary import Choice

    # Step 1: mode
    mode = questionary.select(
        "Welcome to Plano! Pick a starting point:",
        choices=[
            Choice("Start from a demo template (recommended)", value="template"),
            Choice("Create a clean config.yaml (empty)", value="clean"),
            Choice("Cancel", value="cancel"),
        ],
        style=_questionary_style(),
        pointer="❯",
    ).ask()

    if mode in (None, "cancel"):
        console.print("[dim]Cancelled.[/dim]")
        return

    # Step 2: output path (default: config.yaml)
    out_answer = questionary.text(
        "Where should I write the config?",
        default=str(out_path),
        style=_questionary_style(),
    ).ask()
    if not out_answer:
        console.print("[dim]Cancelled.[/dim]")
        return
    out_path = Path(out_answer).expanduser()

    if out_path.exists() and not force:
        overwrite = questionary.confirm(
            f"{out_path} already exists. Overwrite?",
            default=False,
            style=_questionary_style(),
        ).ask()
        if not overwrite:
            console.print("[dim]Cancelled.[/dim]")
            return
        force = True

    if mode == "clean":
        _write_clean_config(out_path, force=True)
        console.print(f"[green]✓[/green] Wrote [bold]{out_path}[/bold]")
        return

    # Step 3: choose template (curated at top, plus any repo-only demos)
    # Keep the list compact and readable.
    template_choices: list[Choice] = []
    for t in templates:
        label = f"{t.title} — {t.description}"
        template_choices.append(Choice(label, value=t))

    template = questionary.select(
        "Choose a template",
        choices=template_choices,
        style=_questionary_style(),
        pointer="❯",
        use_indicator=True,
    ).ask()
    if not template:
        console.print("[dim]Cancelled.[/dim]")
        return

    origin = _write_template_config(out_path, template, force=True)
    console.print(
        f"[green]✓[/green] Wrote [bold]{out_path}[/bold] [dim]({template.id}, {origin})[/dim]"
    )

    # Step 4: .env placeholders (recommended, fast)
    if not no_env:
        env_vars = _extract_env_vars(out_path)
        if env_vars:
            env_path = out_path.parent / ".env"
            do_env = questionary.confirm(
                "Create/update a .env file with placeholders for required keys?",
                default=True,
                style=_questionary_style(),
            ).ask()
            if do_env:
                added = _upsert_env_placeholders(env_path, env_vars)
                if added:
                    console.print(
                        f"[green]✓[/green] Updated [bold]{env_path}[/bold] [dim](added: {', '.join(added)})[/dim]"
                    )
                else:
                    console.print(f"[dim]✓ .env already contains required keys[/dim]")

    # Step 5: next step shortcuts (validate/up/done)
    next_step = questionary.select(
        "Next step",
        choices=[
            Choice(f"Run: planoai validate {out_path}", value="validate"),
            Choice(f"Run: planoai up {out_path}", value="up"),
            Choice("Done", value="done"),
        ],
        default="validate",
        style=_questionary_style(),
        pointer="❯",
    ).ask()

    if next_step == "validate":
        # Reuse existing click command implementation
        from planoai.main import validate as validate_cmd

        ctx.invoke(validate_cmd, config_file=str(out_path), path=".", quiet=False)
    elif next_step == "up":
        from planoai.main import up as up_cmd

        ctx.invoke(up_cmd, file=str(out_path), path=".", foreground=False)
