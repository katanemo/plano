import rich_click as click


def configure_rich_click(plano_color: str) -> None:
    click.rich_click.USE_RICH_MARKUP = True
    click.rich_click.USE_MARKDOWN = False
    click.rich_click.SHOW_ARGUMENTS = True
    click.rich_click.GROUP_ARGUMENTS_OPTIONS = True
    click.rich_click.STYLE_ERRORS_SUGGESTION = "dim italic"
    click.rich_click.ERRORS_SUGGESTION = (
        "Try running the '--help' flag for more information."
    )
    click.rich_click.ERRORS_EPILOGUE = ""

    # Custom colors matching Plano brand.
    click.rich_click.STYLE_OPTION = f"dim {plano_color}"
    click.rich_click.STYLE_ARGUMENT = f"dim {plano_color}"
    click.rich_click.STYLE_COMMAND = f"bold {plano_color}"
    click.rich_click.STYLE_SWITCH = "bold green"
    click.rich_click.STYLE_METAVAR = "bold yellow"
    click.rich_click.STYLE_USAGE = "bold"
    click.rich_click.STYLE_USAGE_COMMAND = f"bold dim {plano_color}"
    click.rich_click.STYLE_HELPTEXT_FIRST_LINE = "white italic"
    click.rich_click.STYLE_HELPTEXT = ""
    click.rich_click.STYLE_HEADER_TEXT = "bold"
    click.rich_click.STYLE_FOOTER_TEXT = "dim"
    click.rich_click.STYLE_OPTIONS_PANEL_BORDER = "dim"
    click.rich_click.ALIGN_OPTIONS_PANEL = "left"
    click.rich_click.MAX_WIDTH = 100

    # Option groups for better organization.
    click.rich_click.OPTION_GROUPS = {
        "planoai up": [
            {
                "name": "Configuration",
                "options": ["--path", "file"],
            },
            {
                "name": "Runtime Options",
                "options": ["--foreground", "--with-tracing", "--tracing-port"],
            },
        ],
        "planoai logs": [
            {
                "name": "Log Options",
                "options": ["--debug", "--follow"],
            },
        ],
    }

    # Command groups for main help.
    click.rich_click.COMMAND_GROUPS = {
        "planoai": [
            {
                "name": "Gateway Commands",
                "commands": ["up", "down", "build", "logs"],
            },
            {
                "name": "Agent Commands",
                "commands": ["cli-agent"],
            },
            {
                "name": "Observability",
                "commands": ["trace"],
            },
            {
                "name": "Utilities",
                "commands": ["generate-prompt-targets"],
            },
        ],
    }
