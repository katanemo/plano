import os
from importlib import resources
from dataclasses import dataclass
from pathlib import Path

import rich_click as click
from rich.console import Console
from rich.panel import Panel

from planoai.consts import PLANO_COLOR


@dataclass(frozen=True)
class Template:
    """
    A Plano config template.

    - id: stable identifier used by --template
    - title/description: UI strings
    - yaml_text: embedded template contents (works in PyPI installs)
    """

    id: str
    title: str
    description: str
    yaml_text: str


_TEMPLATE_PACKAGE = "planoai.templates"


def _load_template_yaml(filename: str) -> str:
    return resources.files(_TEMPLATE_PACKAGE).joinpath(filename).read_text("utf-8")


BUILTIN_TEMPLATES: list[Template] = [
    Template(
        id="sub_agent_orchestration",
        title="Sub Agent Orchestration",
        description="multi-agent routing across specialized agents",
        yaml_text=_load_template_yaml("sub_agent_orchestration.yaml"),
    ),
    Template(
        id="coding_agent_routing",
        title="Coding Agent Routing",
        description="routing preferences + model aliases for coding tasks",
        yaml_text=_load_template_yaml("coding_agent_routing.yaml"),
    ),
    Template(
        id="preference_aware_routing",
        title="Preference-aware LLM routing",
        description="automatic LLM routing based on preferences",
        yaml_text=_load_template_yaml("preference_aware_routing.yaml"),
    ),
    Template(
        id="filter_chain_guardrails",
        title="Guardrails via Filter Chains",
        description="input guards, query rewrite, and context building",
        yaml_text=_load_template_yaml("filter_chain_guardrails.yaml"),
    ),
    Template(
        id="conversational_state_v1_responses",
        title="Conversational State via v1/responses",
        description="stateful responses with memory-backed storage",
        yaml_text=_load_template_yaml("conversational_state_v1_responses.yaml"),
    ),
]


def _get_templates() -> list[Template]:
    return list(BUILTIN_TEMPLATES)


def _resolve_template(template_id: str | None) -> Template | None:
    if not template_id:
        return None

    templates = _get_templates()
    for t in templates:
        if t.id == template_id:
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

    path.write_text(template.yaml_text, encoding="utf-8")
    return "builtin"


def _print_config_preview(console: Console, text: str, max_lines: int = 28) -> None:
    lines = text.strip("\n").splitlines()
    preview_lines = lines[:max_lines]
    if len(lines) > max_lines:
        preview_lines.append("... (truncated)")
    preview = "\n".join(preview_lines).strip("\n")
    if not preview:
        preview = "(empty)"
    console.print(
        Panel(
            preview,
            title="Config preview",
            border_style="dim",
            title_align="left",
        )
    )


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
    help="Create config.yaml from a built-in template id.",
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
    "--list-templates",
    is_flag=True,
    help="List available template ids and exit.",
)
@click.pass_context
def init(ctx, template_id_or_path, clean, output_path, force, list_templates):
    """Initialize a Plano config quickly (arrow-key interactive wizard by default)."""
    import sys

    console = Console()

    if clean and template_id_or_path:
        raise click.UsageError("Use either --clean or --template, not both.")

    templates = _get_templates()

    if list_templates:
        console.print(f"[bold {PLANO_COLOR}]Available templates[/bold {PLANO_COLOR}]\n")
        for t in templates:
            console.print(f"  [bold]{t.id}[/bold]  - {t.description}")
        return

    out_path = Path(output_path).expanduser()

    # Non-interactive fast paths
    if clean or template_id_or_path:
        if clean:
            try:
                _write_clean_config(out_path, force=force)
            except FileExistsError:
                raise click.ClickException(
                    f"Refusing to overwrite existing file: {out_path} (use --force)"
                )
            console.print(f"[green]✓[/green] Wrote [bold]{out_path}[/bold]")
            _print_config_preview(console, out_path.read_text(encoding="utf-8"))
            return

        template = _resolve_template(template_id_or_path)
        if not template:
            raise click.ClickException(
                f"Unknown template: {template_id_or_path}\n"
                f"Run: planoai init --list-templates"
            )
        try:
            _write_template_config(out_path, template, force=force)
        except FileExistsError:
            raise click.ClickException(
                f"Refusing to overwrite existing file: {out_path} (use --force)"
            )
        console.print(
            f"[green]✓[/green] Wrote [bold]{out_path}[/bold] [dim]({template.id})[/dim]"
        )
        _print_config_preview(console, template.yaml_text)
        return

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

    # Step 1: choose template (or clean)
    template_choices: list[Choice] = [
        Choice("Create a clean config.yaml (empty)", value="clean"),
    ]
    for t in templates:
        label = f"{t.title} — {t.description}"
        template_choices.append(Choice(label, value=t))

    selected = questionary.select(
        "Choose a template",
        choices=template_choices,
        style=_questionary_style(),
        pointer="❯",
        use_indicator=True,
    ).ask()
    if not selected:
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

    if selected == "clean":
        _write_clean_config(out_path, force=True)
        console.print(f"[green]✓[/green] Wrote [bold]{out_path}[/bold]")
        _print_config_preview(console, out_path.read_text(encoding="utf-8"))
        return

    template = selected
    _write_template_config(out_path, template, force=True)
    console.print(
        f"[green]✓[/green] Wrote [bold]{out_path}[/bold] [dim]({template.id})[/dim]"
    )
    _print_config_preview(console, template.yaml_text)
