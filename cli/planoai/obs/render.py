"""Rich TUI renderer for the observability console."""

from __future__ import annotations

from collections import Counter
from dataclasses import dataclass
from datetime import datetime, timezone

from rich.box import SIMPLE
from rich.columns import Columns
from rich.console import Group
from rich.panel import Panel
from rich.table import Table
from rich.text import Text

from planoai.obs.collector import LLMCall


@dataclass
class AggregateStats:
    count: int
    total_cost_usd: float
    total_input_tokens: int
    total_output_tokens: int
    distinct_sessions: int
    current_session: str | None


@dataclass
class ModelRollup:
    model: str
    requests: int
    input_tokens: int
    output_tokens: int
    cache_write: int
    cache_read: int
    cost_usd: float


def _now() -> datetime:
    return datetime.now(tz=timezone.utc).astimezone()


def aggregates(calls: list[LLMCall]) -> AggregateStats:
    total_cost = sum((c.cost_usd or 0.0) for c in calls)
    total_input = sum(int(c.prompt_tokens or 0) for c in calls)
    total_output = sum(int(c.completion_tokens or 0) for c in calls)
    session_ids = {c.session_id for c in calls if c.session_id}
    current = next(
        (c.session_id for c in reversed(calls) if c.session_id is not None), None
    )
    return AggregateStats(
        count=len(calls),
        total_cost_usd=total_cost,
        total_input_tokens=total_input,
        total_output_tokens=total_output,
        distinct_sessions=len(session_ids),
        current_session=current,
    )


def model_rollups(calls: list[LLMCall]) -> list[ModelRollup]:
    buckets: dict[str, dict[str, float | int]] = {}
    for c in calls:
        key = c.model
        b = buckets.setdefault(
            key,
            {
                "requests": 0,
                "input": 0,
                "output": 0,
                "cache_write": 0,
                "cache_read": 0,
                "cost": 0.0,
            },
        )
        b["requests"] = int(b["requests"]) + 1
        b["input"] = int(b["input"]) + int(c.prompt_tokens or 0)
        b["output"] = int(b["output"]) + int(c.completion_tokens or 0)
        b["cache_write"] = int(b["cache_write"]) + int(c.cache_creation_tokens or 0)
        b["cache_read"] = int(b["cache_read"]) + int(c.cached_input_tokens or 0)
        b["cost"] = float(b["cost"]) + (c.cost_usd or 0.0)

    rollups: list[ModelRollup] = []
    for model, b in buckets.items():
        rollups.append(
            ModelRollup(
                model=model,
                requests=int(b["requests"]),
                input_tokens=int(b["input"]),
                output_tokens=int(b["output"]),
                cache_write=int(b["cache_write"]),
                cache_read=int(b["cache_read"]),
                cost_usd=float(b["cost"]),
            )
        )
    rollups.sort(key=lambda r: r.cost_usd, reverse=True)
    return rollups


def route_hits(calls: list[LLMCall]) -> list[tuple[str, int, float]]:
    counts: Counter[str] = Counter()
    for c in calls:
        if c.route_name:
            counts[c.route_name] += 1
    total = sum(counts.values())
    if total == 0:
        return []
    return [(r, n, (n / total) * 100.0) for r, n in counts.most_common()]


def _fmt_cost(v: float | None) -> str:
    if v is None:
        return "—"
    if v == 0:
        return "$0"
    # Adaptive precision so tiny costs ($3.8e-5) remain readable.
    if abs(v) < 0.0001:
        return f"${v:.8f}".rstrip("0").rstrip(".")
    if abs(v) < 0.01:
        return f"${v:.6f}".rstrip("0").rstrip(".")
    return f"${v:.4f}"


def _fmt_ms(v: float | None) -> str:
    if v is None:
        return "—"
    if v >= 1000:
        return f"{v / 1000:.1f}s"
    return f"{v:.0f}ms"


def _fmt_int(v: int | None) -> str:
    if v is None or v == 0:
        return "—"
    return f"{v:,}"


def _fmt_tokens(v: int | None) -> str:
    if v is None:
        return "—"
    return f"{v:,}"


def _request_panel(last: LLMCall | None) -> Panel:
    if last is None:
        body = Text("no requests yet", style="dim")
    else:
        t = Table.grid(padding=(0, 1))
        t.add_column(style="bold cyan")
        t.add_column()
        t.add_row("Endpoint", "chat/completions")
        status = "—" if last.status_code is None else str(last.status_code)
        t.add_row("Status", status)
        t.add_row("Model", last.model)
        if last.request_model and last.request_model != last.model:
            t.add_row("Req model", last.request_model)
        if last.route_name:
            t.add_row("Route", last.route_name)
        body = t
    return Panel(body, title="[bold]Request[/]", border_style="cyan", box=SIMPLE)


def _cost_panel(last: LLMCall | None) -> Panel:
    if last is None:
        body = Text("—", style="dim")
    else:
        t = Table.grid(padding=(0, 1))
        t.add_column(style="bold green")
        t.add_column()
        t.add_row("Request", _fmt_cost(last.cost_usd))
        t.add_row("Input", _fmt_tokens(last.prompt_tokens))
        t.add_row("Output", _fmt_tokens(last.completion_tokens))
        if last.cached_input_tokens:
            t.add_row("Cached", _fmt_tokens(last.cached_input_tokens))
        body = t
    return Panel(body, title="[bold]Cost[/]", border_style="green", box=SIMPLE)


def _totals_panel(stats: AggregateStats) -> Panel:
    t = Table.grid(padding=(0, 1))
    t.add_column(style="bold magenta")
    t.add_column()
    t.add_column(style="bold magenta")
    t.add_column()
    t.add_row(
        "Total cost",
        _fmt_cost(stats.total_cost_usd),
        "Requests",
        str(stats.count),
    )
    t.add_row(
        "Input",
        _fmt_tokens(stats.total_input_tokens),
        "Output",
        _fmt_tokens(stats.total_output_tokens),
    )
    t.add_row(
        "Sessions",
        str(stats.distinct_sessions),
        "Current session",
        stats.current_session or "—",
    )
    return Panel(t, title="[bold]Totals[/]", border_style="magenta", box=SIMPLE)


def _model_rollup_table(rollups: list[ModelRollup]) -> Table:
    table = Table(
        title="Totals by model",
        box=SIMPLE,
        header_style="bold",
        expand=True,
    )
    table.add_column("Model", style="cyan")
    table.add_column("Req", justify="right")
    table.add_column("Input", justify="right")
    table.add_column("Output", justify="right", style="green")
    table.add_column("Cache write", justify="right", style="yellow")
    table.add_column("Cache read", justify="right", style="yellow")
    table.add_column("Cost", justify="right", style="green")
    if not rollups:
        table.add_row("—", "—", "—", "—", "—", "—", "—")
    for r in rollups:
        table.add_row(
            r.model,
            str(r.requests),
            _fmt_tokens(r.input_tokens),
            _fmt_tokens(r.output_tokens),
            _fmt_int(r.cache_write),
            _fmt_int(r.cache_read),
            _fmt_cost(r.cost_usd),
        )
    return table


def _route_hit_table(hits: list[tuple[str, int, float]]) -> Table:
    table = Table(
        title="Route hit %",
        box=SIMPLE,
        header_style="bold",
        expand=True,
    )
    table.add_column("Route", style="cyan")
    table.add_column("Hits", justify="right")
    table.add_column("%", justify="right")
    for route, n, pct in hits:
        table.add_row(route, str(n), f"{pct:.1f}")
    return table


def _recent_table(calls: list[LLMCall], limit: int = 15) -> Table:
    show_route = any(c.route_name for c in calls)
    table = Table(
        title="Recent requests",
        box=SIMPLE,
        header_style="bold",
        expand=True,
    )
    table.add_column("time")
    table.add_column("model", style="cyan")
    if show_route:
        table.add_column("route", style="yellow")
    table.add_column("in", justify="right")
    table.add_column("cache", justify="right", style="yellow")
    table.add_column("out", justify="right", style="green")
    table.add_column("rsn", justify="right")
    table.add_column("cost", justify="right", style="green")
    table.add_column("TTFT", justify="right")
    table.add_column("lat", justify="right")
    table.add_column("st")

    recent = list(reversed(calls))[:limit]
    for c in recent:
        status_cell = "ok" if c.status_code and 200 <= c.status_code < 400 else str(c.status_code or "—")
        row = [
            c.timestamp.strftime("%H:%M:%S"),
            c.model,
        ]
        if show_route:
            row.append(c.route_name or "—")
        row.extend(
            [
                _fmt_tokens(c.prompt_tokens),
                _fmt_int(c.cached_input_tokens),
                _fmt_tokens(c.completion_tokens),
                _fmt_int(c.reasoning_tokens),
                _fmt_cost(c.cost_usd),
                _fmt_ms(c.ttft_ms),
                _fmt_ms(c.duration_ms),
                status_cell,
            ]
        )
        table.add_row(*row)
    if not recent:
        table.add_row(*(["no requests yet"] + ["—"] * (10 if show_route else 9)))
    return table


def render(calls: list[LLMCall]) -> Group:
    last = calls[-1] if calls else None
    stats = aggregates(calls)
    rollups = model_rollups(calls)
    hits = route_hits(calls)

    header = Columns(
        [_request_panel(last), _cost_panel(last), _totals_panel(stats)],
        expand=True,
        equal=True,
    )
    parts = [
        header,
        _model_rollup_table(rollups),
    ]
    if hits:
        parts.append(_route_hit_table(hits))
    parts.append(_recent_table(calls))
    parts.append(
        Text(
            "q quit · c clear · waiting for spans on OTLP :4317 — brightstaff needs "
            "tracing.opentracing_grpc_endpoint=localhost:4317",
            style="dim",
        )
    )
    return Group(*parts)
