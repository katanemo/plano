import os
import re
import string
from fnmatch import fnmatch
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any

import rich_click as click
import requests
from rich.console import Console
from rich.text import Text
from rich.tree import Tree

from planoai.consts import PLANO_COLOR


@dataclass
class TraceSummary:
    trace_id: str
    start_ns: int
    end_ns: int

    @property
    def total_ms(self) -> float:
        return max(0, (self.end_ns - self.start_ns) / 1_000_000)

    @property
    def timestamp(self) -> str:
        if self.start_ns <= 0:
            return "unknown"
        dt = datetime.fromtimestamp(self.start_ns / 1_000_000_000, tz=timezone.utc)
        return dt.astimezone().strftime("%Y-%m-%d %H:%M:%S")


def _trace_api_url() -> str:
    return os.environ.get("PLANO_TRACE_API_URL", "http://localhost:9091")


def _split_patterns(value: str | None) -> list[str]:
    if not value:
        return []
    parts = [part.strip() for part in value.split(",")]
    if any(not part for part in parts):
        raise ValueError("Filter contains empty tokens.")
    return parts


def _is_hex(value: str, length: int) -> bool:
    if len(value) != length:
        return False
    return all(char in string.hexdigits for char in value)


def _parse_where_filters(where_filters: tuple[str, ...]) -> list[tuple[str, str]]:
    parsed: list[tuple[str, str]] = []
    invalid: list[str] = []
    key_pattern = re.compile(r"^[A-Za-z0-9_.:-]+$")
    for raw in where_filters:
        if raw.count("=") != 1:
            invalid.append(raw)
            continue
        key, value = raw.split("=", 1)
        key = key.strip()
        value = value.strip()
        if not key or not value or not key_pattern.match(key):
            invalid.append(raw)
            continue
        parsed.append((key, value))
    if invalid:
        invalid_list = ", ".join(invalid)
        raise click.ClickException(
            f"Invalid --where filter(s): {invalid_list}. Use key=value."
        )
    return parsed


def _collect_attr_keys(traces: list[dict[str, Any]]) -> set[str]:
    keys: set[str] = set()
    for trace in traces:
        for span in trace.get("spans", []):
            for item in span.get("attributes", []):
                key = item.get("key")
                if key:
                    keys.add(str(key))
    return keys


def _fetch_traces(endpoint: str, params: dict[str, Any]) -> dict:
    url = f"{_trace_api_url().rstrip('/')}{endpoint}"
    try:
        response = requests.get(url, params=params, timeout=5)
    except requests.RequestException as exc:
        raise click.ClickException(
            f"Trace API not reachable at {url}. "
            "If Plano is running in Docker, expose port 9091 or set PLANO_TRACE_API_URL. "
            "Start Plano with 'planoai up' if needed."
        ) from exc
    if response.status_code >= 400:
        raise click.ClickException(response.text)
    try:
        return response.json()
    except ValueError as exc:
        raise click.ClickException("Trace API returned invalid JSON.") from exc


def _attrs(span: dict[str, Any]) -> dict[str, str]:
    attrs = {}
    for item in span.get("attributes", []):
        key = item.get("key")
        value_obj = item.get("value", {})
        value = value_obj.get("stringValue")
        if value is None and "intValue" in value_obj:
            value = value_obj.get("intValue")
        if value is None and "doubleValue" in value_obj:
            value = value_obj.get("doubleValue")
        if value is None and "boolValue" in value_obj:
            value = value_obj.get("boolValue")
        if key is not None and value is not None:
            attrs[str(key)] = str(value)
    return attrs


def _span_time_ns(span: dict[str, Any], key: str) -> int:
    try:
        return int(span.get(key, 0))
    except (TypeError, ValueError):
        return 0


def _trace_id_short(trace_id: str) -> str:
    return trace_id[:8] if trace_id else "unknown"


def _trace_summary(trace: dict[str, Any]) -> TraceSummary:
    spans = trace.get("spans", [])
    start_ns = min((_span_time_ns(s, "startTimeUnixNano") for s in spans), default=0)
    end_ns = max((_span_time_ns(s, "endTimeUnixNano") for s in spans), default=0)
    return TraceSummary(
        trace_id=trace.get("trace_id", "unknown"),
        start_ns=start_ns,
        end_ns=end_ns,
    )


def _service_color(service: str) -> str:
    service = service.lower()
    if "inbound" in service:
        return "grey50"
    if "orchestrator" in service:
        return PLANO_COLOR
    if "routing" in service:
        return "magenta"
    if "agent" in service:
        return "cyan"
    if "llm" in service:
        return "green"
    return "white"


def _sorted_attr_items(attrs: dict[str, str]) -> list[tuple[str, str]]:
    priority = [
        "http.method",
        "http.target",
        "http.status_code",
        "routing.determination_ms",
        "route.selected_model",
        "selection.agents",
        "selection.agent_count",
        "agent.name",
        "agent.sequence",
        "duration_ms",
        "llm.model",
        "llm.is_streaming",
        "llm.time_to_first_token",
        "llm.duration_ms",
        "llm.response_bytes",
    ]
    prioritized = [(k, attrs[k]) for k in priority if k in attrs]
    prioritized_keys = {k for k, _ in prioritized}
    remaining = [(k, v) for k, v in attrs.items() if k not in prioritized_keys]
    remaining.sort(key=lambda item: item[0])
    return prioritized + remaining


def _build_tree(trace: dict[str, Any], console: Console) -> None:
    spans = trace.get("spans", [])
    if not spans:
        console.print("[yellow]No spans found for this trace.[/yellow]")
        return

    start_ns = min((_span_time_ns(s, "startTimeUnixNano") for s in spans), default=0)
    end_ns = max((_span_time_ns(s, "endTimeUnixNano") for s in spans), default=0)
    total_ms = max(0, (end_ns - start_ns) / 1_000_000)

    trace_id = trace.get("trace_id", "unknown")
    console.print(
        f"\n[bold]Trace:[/bold] {trace_id} [dim]({total_ms:.0f}ms total)[/dim]\n"
    )

    span_by_id = {s.get("spanId"): s for s in spans if s.get("spanId")}
    children: dict[str, list[dict[str, Any]]] = {}
    roots: list[dict[str, Any]] = []

    for span in spans:
        parent_id = span.get("parentSpanId")
        if parent_id and parent_id in span_by_id:
            children.setdefault(parent_id, []).append(span)
        else:
            roots.append(span)

    for items in children.values():
        items.sort(key=lambda s: _span_time_ns(s, "startTimeUnixNano"))
    roots.sort(key=lambda s: _span_time_ns(s, "startTimeUnixNano"))
    tree = Tree("", guide_style="dim")

    def add_node(parent: Tree, span: dict[str, Any]) -> None:
        service = span.get("service", "plano(unknown)")
        name = span.get("name", "")
        offset_ms = max(
            0, (_span_time_ns(span, "startTimeUnixNano") - start_ns) / 1_000_000
        )
        color = _service_color(service)
        label = Text(f"{offset_ms:.0f}ms ", style="yellow")
        label.append(service, style=f"bold {color}")
        if name:
            label.append(f" {name}", style="dim white")

        node = parent.add(label)
        attrs = _attrs(span)
        for key, value in _sorted_attr_items(attrs):
            attr_line = Text()
            attr_line.append(f"{key}: ", style="white")
            attr_line.append(str(value), style=f"{PLANO_COLOR}")
            node.add(attr_line)

        for child in children.get(span.get("spanId"), []):
            add_node(node, child)

    for root in roots:
        add_node(tree, root)

    console.print(tree)
    console.print()


def _select_request(
    console: Console, traces: list[dict[str, Any]]
) -> dict[str, Any] | None:
    try:
        import questionary
        from questionary import Choice
        from prompt_toolkit.styles import Style
    except ImportError as exc:
        raise click.ClickException(
            "Interactive selection requires 'questionary'. "
            "Install it or rerun with --json."
        ) from exc

    if not traces:
        return None

    style = Style.from_dict(
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

    choices = []
    for trace in traces:
        summary = _trace_summary(trace)
        label = f"{_trace_id_short(summary.trace_id)} ({summary.total_ms:.0f}ms total • {summary.timestamp})"
        choices.append(Choice(label, value=trace))

    selected = questionary.select(
        "Select a trace to view:",
        choices=choices,
        style=style,
        pointer="❯",
    ).ask()

    if not selected:
        console.print("[dim]Cancelled.[/dim]")
        return None
    return selected


@click.command()
@click.argument("target", required=False)
@click.option(
    "--filter",
    "filter_patterns",
    default="",
    help="Limit displayed attributes to matching keys (wildcards supported).",
)
@click.option(
    "--where",
    "where_filters",
    multiple=True,
    help="Match traces that contain key=value. Repeatable (AND semantics).",
)
@click.option("--list", "list_only", is_flag=True, help="List trace IDs only.")
@click.option(
    "--no-interactive",
    is_flag=True,
    help="Disable interactive prompts and selections.",
)
@click.option("--limit", type=int, default=None, help="Limit results.")
@click.option("--since", default=None, help="Look back window (e.g. 5m, 2h, 1d).")
@click.option("--json", "json_out", is_flag=True, help="Output raw JSON.")
def trace(
    target,
    filter_patterns,
    where_filters,
    list_only,
    no_interactive,
    limit,
    since,
    json_out,
):
    """Trace requests from the local OTLP log."""
    console = Console()

    try:
        patterns = _split_patterns(filter_patterns)
    except ValueError as exc:
        raise click.ClickException(str(exc)) from exc
    params: dict[str, Any] = {}
    parsed_where = _parse_where_filters(where_filters)
    if patterns:
        params["filter"] = ",".join(patterns)
    for key, value in parsed_where:
        params.setdefault("where", [])
        params["where"].append(f"{key}={value}")
    if list_only:
        params["list"] = "true"
    if limit is not None and limit < 0:
        raise click.ClickException("Limit must be greater than or equal to 0.")
    if limit is not None:
        params["limit"] = str(limit)
    if since:
        params["since"] = since

    if target is None:
        target = "any" if list_only or since or limit else "last"

    if list_only and target not in (None, "last", "any"):
        raise click.ClickException("Target and --list cannot be used together.")

    short_target = None
    if isinstance(target, str) and target not in ("last", "any"):
        target_lower = target.lower()
        if len(target_lower) == 8:
            if not _is_hex(target_lower, 8) or target_lower == "00000000":
                raise click.ClickException("Short trace ID must be 8 hex characters.")
            short_target = target_lower
        elif len(target_lower) == 32:
            if not _is_hex(target_lower, 32) or target_lower == "0" * 32:
                raise click.ClickException("Trace ID must be 32 hex characters.")
        else:
            raise click.ClickException("Trace ID must be 8 or 32 hex characters.")
    if target == "last":
        endpoint = "/debug/traces/last"
    elif target == "any":
        endpoint = "/debug/traces/any"
    elif short_target:
        endpoint = "/debug/traces/any"
    else:
        endpoint = f"/debug/traces/{target}"

    # For interactive listing, fetch full trace details to show timing info
    if list_only and console.is_terminal and not no_interactive:
        params.pop("list", None)

    data = _fetch_traces(endpoint, params)
    validation_params = {k: v for k, v in params.items() if k not in {"where", "list"}}
    validation_data = _fetch_traces(endpoint, validation_params)
    validation_traces = validation_data.get("traces", [])
    if short_target and validation_traces:
        validation_traces = [
            trace
            for trace in validation_traces
            if trace.get("trace_id", "").lower().startswith(short_target)
        ]
    if validation_traces:
        available_keys = _collect_attr_keys(validation_traces)
        if parsed_where:
            missing_keys = [key for key, _ in parsed_where if key not in available_keys]
            if missing_keys:
                missing_list = ", ".join(missing_keys)
                raise click.ClickException(f"Unknown --where key(s): {missing_list}")
        if patterns:
            unmatched = [
                pattern
                for pattern in patterns
                if not any(fnmatch(key, pattern) for key in available_keys)
            ]
            if unmatched:
                unmatched_list = ", ".join(unmatched)
                console.print(
                    f"[yellow]Warning:[/yellow] Filter key(s) not found: {unmatched_list}. "
                    "Returning unfiltered traces."
                )
    if short_target and "traces" in data:
        data["traces"] = [
            trace
            for trace in data.get("traces", [])
            if trace.get("trace_id", "").lower().startswith(short_target)
        ]

    if json_out:
        console.print_json(data=data)
        return

    traces = data.get("traces", [])
    if list_only:
        if traces and console.is_terminal and not no_interactive:
            selected = _select_request(console, traces)
            if selected:
                _build_tree(selected, console)
            return

        if traces:
            trace_ids = [_trace_id_short(_trace_summary(t).trace_id) for t in traces]
        else:
            trace_ids = data.get("trace_ids", [])

        if not trace_ids:
            console.print("[yellow]No trace IDs found.[/yellow]")
            return

        console.print("\n[bold]Trace IDs:[/bold]")
        for trace_id in trace_ids:
            console.print(f"  [dim]-[/dim] {trace_id}")
        return

    if not traces:
        console.print("[yellow]No traces found.[/yellow]")
        return

    trace_obj = traces[0]
    _build_tree(trace_obj, console)
