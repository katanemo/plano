import os
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any

import rich_click as click
import requests
import json
from rich.console import Console
from rich.text import Text
from rich.tree import Tree

from planoai.consts import PLANO_COLOR


@dataclass
class TraceSummary:
    trace_id: str
    request_id: str
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
    return [part.strip() for part in value.split(",") if part.strip()]


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
    return response.json()


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


def _extract_request_id(trace: dict[str, Any]) -> str:
    request_ids = trace.get("request_ids") or []
    if request_ids:
        return request_ids[0]
    for span in trace.get("spans", []):
        attrs = _attrs(span)
        if "x-request-id" in attrs:
            return attrs["x-request-id"]
        if "guid:x-request-id" in attrs:
            return attrs["guid:x-request-id"]
    return "unknown"


def _trace_summary(trace: dict[str, Any]) -> TraceSummary:
    spans = trace.get("spans", [])
    start_ns = min((_span_time_ns(s, "startTimeUnixNano") for s in spans), default=0)
    end_ns = max((_span_time_ns(s, "endTimeUnixNano") for s in spans), default=0)
    return TraceSummary(
        trace_id=trace.get("trace_id", "unknown"),
        request_id=_extract_request_id(trace),
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
    remaining = [
        (k, v) for k, v in attrs.items() if k not in {k for k, _ in prioritized}
    ]
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

    request_id = _extract_request_id(trace)
    console.print(
        f"\n[bold]Request:[/bold] {request_id} [dim]({total_ms:.0f}ms total)[/dim]\n"
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


def _select_request(
    console: Console, traces: list[dict[str, Any]]
) -> dict[str, Any] | None:
    import questionary
    from questionary import Choice
    from prompt_toolkit.styles import Style

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
        label = f"{summary.request_id} ({summary.total_ms:.0f}ms total • {summary.timestamp})"
        choices.append(Choice(label, value=trace))

    selected = questionary.select(
        "Select a request to trace:",
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
@click.option("--list", "list_only", is_flag=True, help="List request IDs only.")
@click.option("--limit", type=int, default=None, help="Limit results.")
@click.option("--since", default=None, help="Look back window (e.g. 5m, 2h, 1d).")
@click.option("--json", "json_out", is_flag=True, help="Output raw JSON.")
def trace(target, filter_patterns, where_filters, list_only, limit, since, json_out):
    """Trace requests from the local OTLP log."""
    console = Console()

    patterns = _split_patterns(filter_patterns)
    params: dict[str, Any] = {}
    if patterns:
        params["filter"] = ",".join(patterns)
    for item in where_filters:
        params.setdefault("where", [])
        params["where"].append(item)
    if list_only:
        params["list"] = "true"
    if limit is not None:
        params["limit"] = str(limit)
    if since:
        params["since"] = since

    if target is None:
        target = "any" if list_only or since or limit or where_filters else "last"

    if target == "last":
        endpoint = "/debug/traces/last"
    elif target == "any":
        endpoint = "/debug/traces/any"
    else:
        endpoint = f"/debug/traces/by-request/{target}"

    # For interactive listing, fetch full trace details to show timing info
    if list_only and console.is_terminal:
        params.pop("list", None)

    data = _fetch_traces(endpoint, params)

    if json_out:
        console.print_json(data=data)
        return

    traces = data.get("traces", [])
    if list_only:
        if traces and console.is_terminal:
            selected = _select_request(console, traces)
            if selected:
                _build_tree(selected, console)
            return

        if traces:
            request_ids = [_trace_summary(t).request_id for t in traces]
        else:
            request_ids = data.get("request_ids", [])

        if not request_ids:
            console.print("[yellow]No request IDs found.[/yellow]")
            return

        console.print("\n[bold]Request IDs:[/bold]")
        for req_id in request_ids:
            console.print(f"  [dim]-[/dim] {req_id}")
        return

    if not traces:
        console.print("[yellow]No traces found.[/yellow]")
        return

    trace_obj = traces[0]
    _build_tree(trace_obj, console)
