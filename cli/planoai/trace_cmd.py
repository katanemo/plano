import json
import os
import re
import string
import threading
import time
from collections import deque
from dataclasses import dataclass
from datetime import datetime, timezone
from fnmatch import fnmatch
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any
from urllib.parse import urlparse

import rich_click as click
import requests
from rich.console import Console
from rich.text import Text
from rich.tree import Tree

from planoai.consts import PLANO_COLOR

DEFAULT_TRACE_API_URL = "http://127.0.0.1:4318"
MAX_TRACE_BODY_BYTES = 5_000_000


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
    return os.environ.get("PLANO_TRACE_API_URL", DEFAULT_TRACE_API_URL)


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


def _fetch_traces_raw() -> list[dict[str, Any]]:
    url = f"{_trace_api_url().rstrip('/')}/v1/traces"
    try:
        response = requests.get(url, timeout=5)
    except requests.RequestException as exc:
        raise click.ClickException(
            f"Trace listener not reachable at {url}. "
            "Start it with 'planoai trace listen' or set PLANO_TRACE_API_URL."
        ) from exc
    if response.status_code == 404:
        raise click.ClickException(
            f"An error occurred while fetching traces: {response.text}"
            f"Make sure the trace listener is running and PLANO_TRACE_API_URL is set correctly."
        )
    try:
        payload = response.json()
    except ValueError as exc:
        raise click.ClickException("Trace API returned invalid JSON.") from exc
    if not isinstance(payload, dict) or "traces" not in payload:
        raise click.ClickException("Trace API returned invalid traces.")
    if not isinstance(payload["traces"], list):
        raise click.ClickException("Trace API returned invalid traces.")
    return payload["traces"]


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


def _safe_int(value: Any, default: int = 0) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


def _parse_since_seconds(value: str | None) -> int | None:
    if not value:
        return None
    value = value.strip()
    if not value:
        return None
    if len(value) < 2:
        return None
    number, unit = value[:-1], value[-1]
    try:
        qty = int(number)
    except ValueError:
        return None
    multiplier = {"m": 60, "h": 60 * 60, "d": 60 * 60 * 24}.get(unit)
    if multiplier is None:
        return None
    return qty * multiplier


def _matches_pattern(value: str, pattern: str) -> bool:
    if pattern == "*":
        return True
    if "*" not in pattern:
        return value == pattern
    parts = [part for part in pattern.split("*") if part]
    if not parts:
        return True
    remaining = value
    for idx, part in enumerate(parts):
        pos = remaining.find(part)
        if pos == -1:
            return False
        if idx == 0 and not pattern.startswith("*") and pos != 0:
            return False
        remaining = remaining[pos + len(part) :]
    if not pattern.endswith("*") and remaining:
        return False
    return True


def _attribute_map(span: dict[str, Any]) -> dict[str, str]:
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


def _filter_attributes(span: dict[str, Any], patterns: list[str]) -> dict[str, Any]:
    if not patterns:
        return span
    attributes = span.get("attributes", [])
    filtered = [
        item
        for item in attributes
        if any(
            _matches_pattern(str(item.get("key", "")), pattern) for pattern in patterns
        )
    ]
    cloned = dict(span)
    cloned["attributes"] = filtered
    return cloned


def _build_traces_from_payloads(
    payloads: list[dict[str, Any]],
) -> tuple[list[dict[str, Any]], list[str]]:
    traces: dict[str, dict[str, Any]] = {}
    trace_order: list[str] = []

    for payload in payloads:
        resource_spans = payload.get("resourceSpans", []) or []
        for resource_span in resource_spans:
            if not isinstance(resource_span, dict):
                continue
            service_name = "unknown"
            resource = resource_span.get("resource", {}) or {}
            for attr in resource.get("attributes", []) or []:
                if not isinstance(attr, dict):
                    continue
                if attr.get("key") == "service.name":
                    value_obj = attr.get("value", {}) or {}
                    service_name = value_obj.get("stringValue", service_name)
                    break

            for scope_span in resource_span.get("scopeSpans", []) or []:
                if not isinstance(scope_span, dict):
                    continue
                for span in scope_span.get("spans", []) or []:
                    if not isinstance(span, dict):
                        continue
                    trace_id = str(span.get("traceId") or "")
                    if not trace_id:
                        continue
                    if trace_id not in traces:
                        traces[trace_id] = {"trace_id": trace_id, "spans": []}
                        trace_order.append(trace_id)

                    span_obj = dict(span)
                    span_obj["service"] = service_name
                    traces[trace_id]["spans"].append(span_obj)

    trace_list = [traces[trace_id] for trace_id in trace_order if trace_id in traces]
    trace_list.reverse()

    trace_ids = [trace["trace_id"] for trace in trace_list]
    return trace_list, trace_ids


def _filter_traces(
    traces: list[dict[str, Any]],
    filter_patterns: list[str],
    where_filters: list[tuple[str, str]],
    since_seconds: int | None,
) -> tuple[list[dict[str, Any]], list[str]]:
    now_nanos = int(time.time() * 1_000_000_000)
    since_nanos = now_nanos - (since_seconds * 1_000_000_000) if since_seconds else None

    filtered_traces: list[dict[str, Any]] = []
    for trace in traces:
        spans = trace.get("spans", []) or []
        if since_nanos is not None:
            spans = [
                span
                for span in spans
                if _safe_int(span.get("startTimeUnixNano", 0)) >= since_nanos
            ]
        if filter_patterns:
            spans = [_filter_attributes(span, filter_patterns) for span in spans]
        if not spans:
            continue

        candidate = dict(trace)
        candidate["spans"] = spans
        filtered_traces.append(candidate)

    if where_filters:

        def matches_where(trace: dict[str, Any]) -> bool:
            for key, value in where_filters:
                if not any(
                    _attribute_map(span).get(key) == value
                    for span in trace.get("spans", [])
                ):
                    return False
            return True

        filtered_traces = [trace for trace in filtered_traces if matches_where(trace)]

    trace_ids = [trace.get("trace_id", "") for trace in filtered_traces]
    return filtered_traces, trace_ids


class _TraceBuffer:
    def __init__(self, max_payloads: int = 10) -> None:
        self._payloads: deque[dict[str, Any]] = deque(maxlen=max_payloads)
        self._lock = threading.Lock()

    def push(self, payload: dict[str, Any]) -> None:
        with self._lock:
            self._payloads.append(payload)

    def snapshot(self) -> list[dict[str, Any]]:
        with self._lock:
            return list(self._payloads)


_TRACE_BUFFER = _TraceBuffer(max_payloads=10)


class _TraceListenerHandler(BaseHTTPRequestHandler):
    server_version = "plano-trace-listener/1.0"

    def log_message(self, format, *args) -> None:
        return

    def _send_json(self, status_code: int, payload: dict[str, Any]) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(status_code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self) -> None:
        if self.path != "/v1/traces":
            self._send_json(404, {"error": "not_found"})
            return
        length = _safe_int(self.headers.get("Content-Length", "0"))
        if length <= 0:
            self._send_json(400, {"error": "empty_body"})
            return
        if length > MAX_TRACE_BODY_BYTES:
            self._send_json(413, {"error": "payload_too_large"})
            return
        body = self.rfile.read(length)
        try:
            payload = json.loads(body.decode("utf-8"))
        except json.JSONDecodeError:
            self._send_json(400, {"error": "invalid_json"})
            return
        if not isinstance(payload, dict):
            self._send_json(400, {"error": "invalid_payload"})
            return
        _TRACE_BUFFER.push(payload)
        self._send_json(200, {})

    def do_GET(self) -> None:
        parsed = urlparse(self.path)
        if parsed.path != "/v1/traces":
            self._send_json(404, {"error": "not_found"})
            return
        traces, _ = _build_traces_from_payloads(_TRACE_BUFFER.snapshot())
        self._send_json(200, {"traces": traces})


def _start_trace_listener(host: str, port: int) -> None:
    server = ThreadingHTTPServer((host, port), _TraceListenerHandler)
    console = Console()
    console.print()
    console.print(f"[bold {PLANO_COLOR}]Listening for traces...[/bold {PLANO_COLOR}]")
    console.print(
        f"[green]●[/green] Running trace listener on [cyan]http://{host}:{port}/v1/traces[/cyan]"
    )
    console.print("[dim]Press Ctrl+C to stop.[/dim]")
    console.print()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


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
def _run_trace_show(
    target,
    filter_patterns,
    where_filters,
    list_only,
    no_interactive,
    limit,
    since,
    json_out,
):
    """Trace requests from the local OTLP listener."""
    console = Console()

    try:
        patterns = _split_patterns(filter_patterns)
    except ValueError as exc:
        raise click.ClickException(str(exc)) from exc

    parsed_where = _parse_where_filters(where_filters)
    if limit is not None and limit < 0:
        raise click.ClickException("Limit must be greater than or equal to 0.")
    since_seconds = _parse_since_seconds(since)

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

    traces_raw = _fetch_traces_raw()
    if traces_raw:
        available_keys = _collect_attr_keys(traces_raw)
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

    traces, trace_ids = _filter_traces(
        traces_raw, patterns, parsed_where, since_seconds
    )

    if target == "last":
        traces = traces[:1]
        trace_ids = trace_ids[:1]
    elif target not in (None, "any") and short_target is None:
        traces = [trace for trace in traces if trace.get("trace_id") == target]
        trace_ids = [trace.get("trace_id") for trace in traces]
    if short_target:
        traces = [
            trace
            for trace in traces
            if trace.get("trace_id", "").lower().startswith(short_target)
        ]
        trace_ids = [trace.get("trace_id") for trace in traces]

    if limit is not None:
        if list_only:
            trace_ids = trace_ids[:limit]
        else:
            traces = traces[:limit]

    if json_out:
        if list_only:
            console.print_json(data={"trace_ids": trace_ids})
        else:
            console.print_json(data={"traces": traces})
        return

    if list_only:
        if traces and console.is_terminal and not no_interactive:
            selected = _select_request(console, traces)
            if selected:
                _build_tree(selected, console)
            return

        if traces:
            trace_ids = [_trace_id_short(_trace_summary(t).trace_id) for t in traces]

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


@click.group(invoke_without_command=True)
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
@click.pass_context
def trace(
    ctx,
    target,
    filter_patterns,
    where_filters,
    list_only,
    no_interactive,
    limit,
    since,
    json_out,
):
    """Trace requests from the local OTLP listener."""
    if ctx.invoked_subcommand:
        return
    if target == "listen" and not any(
        [
            filter_patterns,
            where_filters,
            list_only,
            no_interactive,
            limit,
            since,
            json_out,
        ]
    ):
        _start_trace_listener("127.0.0.1", 4318)
        return
    _run_trace_show(
        target,
        filter_patterns,
        where_filters,
        list_only,
        no_interactive,
        limit,
        since,
        json_out,
    )


@trace.command("listen")
@click.option("--host", default="127.0.0.1", show_default=True)
@click.option("--port", type=int, default=4318, show_default=True)
def trace_listen(host: str, port: int) -> None:
    """Listen for OTLP/HTTP traces and serve traces."""
    _start_trace_listener(host, port)
