import json
import os
import re
import string
import threading
import time
from collections import OrderedDict
from concurrent import futures
from dataclasses import dataclass
from datetime import datetime, timezone
from fnmatch import fnmatch
from typing import Any

import grpc
import rich_click as click
from opentelemetry.proto.collector.trace.v1 import (
    trace_service_pb2,
    trace_service_pb2_grpc,
)
from rich.console import Console
from rich.text import Text
from rich.tree import Tree

from planoai.consts import PLANO_COLOR

DEFAULT_GRPC_PORT = 4317
MAX_TRACES = 50
MAX_SPANS_PER_TRACE = 500


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


def _parse_filter_patterns(filter_patterns: tuple[str, ...]) -> list[str]:
    parts: list[str] = []
    for raw in filter_patterns:
        for token in raw.split(","):
            part = token.strip()
            if not part:
                raise ValueError("Filter contains empty tokens.")
            parts.append(part)
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
    port = os.environ.get("PLANO_TRACE_PORT", str(DEFAULT_GRPC_PORT))
    target = f"127.0.0.1:{port}"
    try:
        channel = grpc.insecure_channel(target)
        stub = channel.unary_unary(
            "/plano.TraceQuery/GetTraces",
            request_serializer=lambda x: x,
            response_deserializer=lambda x: x,
        )
        response = stub(b"", timeout=3)
        channel.close()
        data = json.loads(response)
        traces = data.get("traces", [])
        if isinstance(traces, list):
            return traces
    except Exception:
        pass
    return []


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


class _TraceStore:
    """Thread-safe in-memory store backed by a fixed-length deque.

    Spans may arrive with **different** ``traceId`` values but are
    linked via ``parentSpanId``.  This store groups them into logical
    traces by following parent-child span relationships, so all
    connected spans end up under a single trace group regardless of
    the ``traceId`` they were emitted with.
    """

    def __init__(self, max_traces: int = MAX_TRACES) -> None:
        self._traces: OrderedDict[str, dict[str, Any]] = OrderedDict()
        self._seen_spans: dict[str, set[str]] = {}
        # span_id → group key (the trace_id used as the dict key)
        self._span_to_group: dict[str, str] = {}
        # parent_span_id → group key for spans whose parent arrived first
        self._parent_to_group: dict[str, str] = {}
        self._max_traces = max_traces
        self._lock = threading.Lock()

    def _evict_oldest(self) -> None:
        """Remove the oldest trace group (caller must hold *_lock*)."""
        if not self._traces:
            return
        oldest_id, oldest = self._traces.popitem(last=False)
        self._seen_spans.pop(oldest_id, None)
        for span in oldest.get("spans", []):
            sid = span.get("spanId", "")
            self._span_to_group.pop(sid, None)
            self._parent_to_group.pop(sid, None)

    def _merge_groups(self, src_key: str, dst_key: str) -> None:
        """Move all spans from *src_key* group into *dst_key* (caller holds lock)."""
        if src_key == dst_key or src_key not in self._traces:
            return
        src = self._traces.pop(src_key)
        dst = self._traces[dst_key]
        dst_seen = self._seen_spans[dst_key]
        src_seen = self._seen_spans.pop(src_key, set())
        for span in src.get("spans", []):
            sid = span.get("spanId", "")
            if sid and sid not in dst_seen:
                dst["spans"].append(span)
                dst_seen.add(sid)
            self._span_to_group[sid] = dst_key
        for sid in src_seen:
            self._span_to_group[sid] = dst_key
        # Update parent→group mappings that pointed to src.
        for pid, gid in list(self._parent_to_group.items()):
            if gid == src_key:
                self._parent_to_group[pid] = dst_key

    def merge_spans(self, trace_id: str, spans: list[dict[str, Any]]) -> None:
        """Merge *spans* into the correct trace group.

        The group is determined by following ``parentSpanId`` /
        ``spanId`` links, falling back to *trace_id* when no link
        exists.
        """
        with self._lock:
            for span in spans:
                span_id = span.get("spanId", "")
                parent_id = span.get("parentSpanId", "")

                # Determine which group this span belongs to.
                group_key: str | None = None

                # 1. Does the parent already live in a group?
                if parent_id and parent_id in self._span_to_group:
                    group_key = self._span_to_group[parent_id]

                # 2. Is this span already known as a parent of another group?
                if group_key is None and span_id and span_id in self._parent_to_group:
                    group_key = self._parent_to_group.pop(span_id)

                # 3. Fall back to the wire trace_id.
                if group_key is None:
                    group_key = trace_id

                # Create the group if needed.
                if group_key not in self._traces:
                    if len(self._traces) >= self._max_traces:
                        self._evict_oldest()
                    self._traces[group_key] = {"trace_id": group_key, "spans": []}
                    self._seen_spans[group_key] = set()
                else:
                    self._traces.move_to_end(group_key)

                # Insert span (deduplicate).
                seen = self._seen_spans[group_key]
                if span_id and span_id in seen:
                    continue
                if span_id:
                    seen.add(span_id)
                    self._span_to_group[span_id] = group_key
                if len(self._traces[group_key]["spans"]) < MAX_SPANS_PER_TRACE:
                    self._traces[group_key]["spans"].append(span)

                # Record parent link so future spans can find this group.
                if parent_id and parent_id not in self._span_to_group:
                    self._parent_to_group[parent_id] = group_key

                # If this span's span_id is the parent of an existing
                # *different* group, merge that group into this one.
                if span_id and span_id in self._parent_to_group:
                    other = self._parent_to_group.pop(span_id)
                    if other != group_key and other in self._traces:
                        self._merge_groups(other, group_key)

    def snapshot(self) -> list[dict[str, Any]]:
        """Return traces ordered newest-first."""
        with self._lock:
            traces = list(self._traces.values())
        traces.reverse()
        return traces


_TRACE_STORE = _TraceStore()


def _anyvalue_to_python(value_obj: Any) -> Any:
    """Convert an opentelemetry AnyValue protobuf to a Python primitive."""
    if hasattr(value_obj, "string_value") and value_obj.HasField("value"):
        kind = value_obj.WhichOneof("value")
        if kind == "string_value":
            return value_obj.string_value
        if kind == "int_value":
            return value_obj.int_value
        if kind == "double_value":
            return value_obj.double_value
        if kind == "bool_value":
            return value_obj.bool_value
    return None


def _proto_span_to_dict(span: Any, service_name: str) -> dict[str, Any]:
    """Convert a protobuf Span message to the dict format used internally."""
    span_dict: dict[str, Any] = {
        "traceId": span.trace_id.hex(),
        "spanId": span.span_id.hex(),
        "parentSpanId": span.parent_span_id.hex() if span.parent_span_id else "",
        "name": span.name,
        "startTimeUnixNano": str(span.start_time_unix_nano),
        "endTimeUnixNano": str(span.end_time_unix_nano),
        "service": service_name,
        "attributes": [],
    }
    for kv in span.attributes:
        py_val = _anyvalue_to_python(kv.value)
        if py_val is not None:
            value_dict: dict[str, Any] = {}
            if isinstance(py_val, str):
                value_dict["stringValue"] = py_val
            elif isinstance(py_val, bool):
                value_dict["boolValue"] = py_val
            elif isinstance(py_val, int):
                value_dict["intValue"] = str(py_val)
            elif isinstance(py_val, float):
                value_dict["doubleValue"] = py_val
            span_dict["attributes"].append({"key": kv.key, "value": value_dict})
    return span_dict


class _OTLPTraceServicer(trace_service_pb2_grpc.TraceServiceServicer):
    """gRPC servicer that receives OTLP ExportTraceServiceRequest and
    merges incoming spans into the global _TRACE_STORE by trace_id."""

    _console = Console(stderr=True)

    def Export(self, request, context):  # noqa: N802
        for resource_spans in request.resource_spans:
            service_name = "unknown"
            for attr in resource_spans.resource.attributes:
                if attr.key == "service.name":
                    val = _anyvalue_to_python(attr.value)
                    if val is not None:
                        service_name = str(val)
                    break

            for scope_spans in resource_spans.scope_spans:
                for span in scope_spans.spans:
                    trace_id = span.trace_id.hex()
                    if not trace_id:
                        continue
                    span_dict = _proto_span_to_dict(span, service_name)
                    _TRACE_STORE.merge_spans(trace_id, [span_dict])
                    short_id = trace_id[:8]
                    short_span = span.span_id.hex()[:8]
                    span_start = (
                        datetime.fromtimestamp(
                            span.start_time_unix_nano / 1_000_000_000, tz=timezone.utc
                        )
                        .astimezone()
                        .strftime("%H:%M:%S.%f")[:-3]
                    )
                    dur_ns = span.end_time_unix_nano - span.start_time_unix_nano
                    dur_s = dur_ns / 1_000_000_000
                    dur_str = f"{dur_s:.3f}".rstrip("0").rstrip(".")
                    dur_str = f"{dur_str}s"
                    self._console.print(
                        f"[dim]{span_start}[/dim], "
                        f"trace=[yellow]{short_id}[/yellow], "
                        f"span=[yellow]{short_span}[/yellow], "
                        f"[bold {_service_color(service_name)}]{service_name}[/bold {_service_color(service_name)}] "
                        f"[cyan]{span.name}[/cyan] "
                        f"[dim]({dur_str})[/dim]"
                    )

        return trace_service_pb2.ExportTraceServiceResponse()


class _TraceQueryHandler(grpc.GenericRpcHandler):
    """gRPC handler that serves stored traces to the CLI show command."""

    def service(self, handler_call_details):
        if handler_call_details.method == "/plano.TraceQuery/GetTraces":
            return grpc.unary_unary_rpc_method_handler(
                self._get_traces,
                request_deserializer=lambda x: x,
                response_serializer=lambda x: x,
            )
        return None

    @staticmethod
    def _get_traces(_request, _context):
        traces = _TRACE_STORE.snapshot()
        return json.dumps({"traces": traces}, separators=(",", ":")).encode("utf-8")


def _create_trace_server(host: str, grpc_port: int) -> grpc.Server:
    """Create, bind, and start an OTLP/gRPC trace-collection server.

    Returns the running ``grpc.Server``.  The caller is responsible
    for calling ``server.stop()`` when done.
    """
    grpc_server = grpc.server(
        futures.ThreadPoolExecutor(max_workers=4),
        handlers=[_TraceQueryHandler()],
    )
    trace_service_pb2_grpc.add_TraceServiceServicer_to_server(
        _OTLPTraceServicer(), grpc_server
    )
    grpc_server.add_insecure_port(f"{host}:{grpc_port}")
    grpc_server.start()
    return grpc_server


def _start_trace_listener(host: str, grpc_port: int) -> None:
    """Start the OTLP/gRPC listener and block until interrupted."""
    console = Console()
    grpc_server = _create_trace_server(host, grpc_port)

    console.print()
    console.print(f"[bold {PLANO_COLOR}]Listening for traces...[/bold {PLANO_COLOR}]")
    console.print(
        f"[green]●[/green] gRPC (OTLP receiver) on [cyan]{host}:{grpc_port}[/cyan]"
    )
    console.print("[dim]Press Ctrl+C to stop.[/dim]")
    console.print()
    try:
        grpc_server.wait_for_termination()
    except KeyboardInterrupt:
        pass
    finally:
        grpc_server.stop(grace=2)


def start_trace_listener_background(
    host: str = "0.0.0.0", grpc_port: int = DEFAULT_GRPC_PORT
) -> grpc.Server:
    """Start the trace listener in the background (non-blocking).

    Returns the running ``grpc.Server`` so the caller can call
    ``server.stop()`` later.
    """
    return _create_trace_server(host, grpc_port)


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
        return "white"
    if "outbound" in service:
        return "white"
    if "orchestrator" in service:
        return PLANO_COLOR
    if "routing" in service:
        return "magenta"
    if "agent" in service:
        return "cyan"
    if "llm" in service:
        return "green"
    return "white"


# Attributes to show for inbound/outbound spans when not verbose (trimmed view).
_INBOUND_OUTBOUND_ATTR_KEYS = (
    "http.method",
    "http.target",
    "http.status_code",
    "url.scheme",
    "guid:x-request-id",
    "request_size",
    "response_size",
)


def _trim_attrs_for_display(
    attrs: dict[str, str], service: str, verbose: bool
) -> dict[str, str]:
    if verbose:
        return attrs
    if "inbound" in service.lower() or "outbound" in service.lower():
        attrs = {k: v for k, v in attrs.items() if k in _INBOUND_OUTBOUND_ATTR_KEYS}
    return {k: v for k, v in attrs.items() if k != "service.name.override"}


def _sorted_attr_items(attrs: dict[str, str]) -> list[tuple[str, str]]:
    priority = [
        "http.method",
        "http.target",
        "http.status_code",
        "guid:x-request-id",
        "request_size",
        "response_size",
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


def _display_attr_value(key: str, value: str) -> str:
    if key == "http.status_code" and value != "200":
        return f"{value} ⚠️"
    return value


def _build_tree(trace: dict[str, Any], console: Console, verbose: bool = False) -> None:
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

    spans.sort(key=lambda s: _span_time_ns(s, "startTimeUnixNano"))
    tree = Tree("", guide_style="dim")

    for span in spans:
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

        node = tree.add(label)
        attrs = _trim_attrs_for_display(_attrs(span), service, verbose)
        sorted_items = list(_sorted_attr_items(attrs))
        for idx, (key, value) in enumerate(sorted_items):
            attr_line = Text()
            attr_line.append(f"{key}: ", style="white")
            attr_line.append(
                _display_attr_value(key, str(value)),
                style=f"{PLANO_COLOR}",
            )
            if idx == len(sorted_items) - 1:
                attr_line.append("\n")
            node.add(attr_line)

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
    multiple=True,
    help=(
        "Limit displayed attributes to matching keys "
        "(wildcards supported). Repeatable."
    ),
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
@click.option(
    "--verbose",
    "-v",
    is_flag=True,
    help="Show all span attributes; default trims inbound/outbound to a few keys.",
)
def _run_trace_show(
    target,
    filter_patterns,
    where_filters,
    list_only,
    no_interactive,
    limit,
    since,
    json_out,
    verbose,
):
    """Trace requests from the local OTLP listener."""
    console = Console()

    try:
        patterns = _parse_filter_patterns(filter_patterns)
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
                _build_tree(selected, console, verbose=verbose)
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
    _build_tree(trace_obj, console, verbose=verbose)


@click.group(invoke_without_command=True)
@click.argument("target", required=False)
@click.option(
    "--filter",
    "filter_patterns",
    multiple=True,
    help=(
        "Limit displayed attributes to matching keys "
        "(wildcards supported). Repeatable."
    ),
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
@click.option(
    "--verbose",
    "-v",
    is_flag=True,
    help="Show all span attributes; default trims inbound/outbound to a few keys.",
)
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
    verbose,
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
            verbose,
        ]
    ):
        _start_trace_listener("0.0.0.0", DEFAULT_GRPC_PORT)
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
        verbose,
    )


@trace.command("listen")
@click.option("--host", default="0.0.0.0", show_default=True)
@click.option(
    "--port",
    type=int,
    default=DEFAULT_GRPC_PORT,
    show_default=True,
    help="gRPC port for receiving OTLP traces.",
)
def trace_listen(host: str, port: int) -> None:
    """Listen for OTLP/gRPC traces."""
    _start_trace_listener(host, port)
