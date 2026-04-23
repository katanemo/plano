"""Rich TUI renderer for the observability console."""

from __future__ import annotations

from collections import Counter
from dataclasses import dataclass
from datetime import datetime
from http import HTTPStatus

from rich.align import Align
from rich.box import SIMPLE, SIMPLE_HEAVY
from rich.console import Group
from rich.panel import Panel
from rich.table import Table
from rich.text import Text

MAX_WIDTH = 160

from planoai.obs.collector import LLMCall


@dataclass
class AggregateStats:
    count: int
    total_cost_usd: float
    total_input_tokens: int
    total_output_tokens: int
    distinct_sessions: int
    current_session: str | None
    p50_latency_ms: float | None = None
    p95_latency_ms: float | None = None
    p99_latency_ms: float | None = None
    p50_ttft_ms: float | None = None
    p95_ttft_ms: float | None = None
    p99_ttft_ms: float | None = None
    error_count: int = 0
    errors_4xx: int = 0
    errors_5xx: int = 0
    has_cost: bool = False


@dataclass
class ModelRollup:
    model: str
    requests: int
    input_tokens: int
    output_tokens: int
    cache_write: int
    cache_read: int
    cost_usd: float
    has_cost: bool = False
    avg_tokens_per_sec: float | None = None


def _percentile(values: list[float], pct: float) -> float | None:
    if not values:
        return None
    s = sorted(values)
    k = max(0, min(len(s) - 1, int(round((pct / 100.0) * (len(s) - 1)))))
    return s[k]


def aggregates(calls: list[LLMCall]) -> AggregateStats:
    total_cost = sum((c.cost_usd or 0.0) for c in calls)
    total_input = sum(int(c.prompt_tokens or 0) for c in calls)
    total_output = sum(int(c.completion_tokens or 0) for c in calls)
    session_ids = {c.session_id for c in calls if c.session_id}
    current = next(
        (c.session_id for c in reversed(calls) if c.session_id is not None), None
    )
    durations = [c.duration_ms for c in calls if c.duration_ms is not None]
    ttfts = [c.ttft_ms for c in calls if c.ttft_ms is not None]
    errors_4xx = sum(
        1 for c in calls if c.status_code is not None and 400 <= c.status_code < 500
    )
    errors_5xx = sum(
        1 for c in calls if c.status_code is not None and c.status_code >= 500
    )
    has_cost = any(c.cost_usd is not None for c in calls)
    return AggregateStats(
        count=len(calls),
        total_cost_usd=total_cost,
        total_input_tokens=total_input,
        total_output_tokens=total_output,
        distinct_sessions=len(session_ids),
        current_session=current,
        p50_latency_ms=_percentile(durations, 50),
        p95_latency_ms=_percentile(durations, 95),
        p99_latency_ms=_percentile(durations, 99),
        p50_ttft_ms=_percentile(ttfts, 50),
        p95_ttft_ms=_percentile(ttfts, 95),
        p99_ttft_ms=_percentile(ttfts, 99),
        error_count=errors_4xx + errors_5xx,
        errors_4xx=errors_4xx,
        errors_5xx=errors_5xx,
        has_cost=has_cost,
    )


def model_rollups(calls: list[LLMCall]) -> list[ModelRollup]:
    buckets: dict[str, dict[str, float | int | bool]] = {}
    tps_samples: dict[str, list[float]] = {}
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
                "has_cost": False,
            },
        )
        b["requests"] = int(b["requests"]) + 1
        b["input"] = int(b["input"]) + int(c.prompt_tokens or 0)
        b["output"] = int(b["output"]) + int(c.completion_tokens or 0)
        b["cache_write"] = int(b["cache_write"]) + int(c.cache_creation_tokens or 0)
        b["cache_read"] = int(b["cache_read"]) + int(c.cached_input_tokens or 0)
        b["cost"] = float(b["cost"]) + (c.cost_usd or 0.0)
        if c.cost_usd is not None:
            b["has_cost"] = True
        tps = c.tokens_per_sec
        if tps is not None:
            tps_samples.setdefault(key, []).append(tps)

    rollups: list[ModelRollup] = []
    for model, b in buckets.items():
        samples = tps_samples.get(model)
        avg_tps = (sum(samples) / len(samples)) if samples else None
        rollups.append(
            ModelRollup(
                model=model,
                requests=int(b["requests"]),
                input_tokens=int(b["input"]),
                output_tokens=int(b["output"]),
                cache_write=int(b["cache_write"]),
                cache_read=int(b["cache_read"]),
                cost_usd=float(b["cost"]),
                has_cost=bool(b["has_cost"]),
                avg_tokens_per_sec=avg_tps,
            )
        )
    rollups.sort(key=lambda r: (r.cost_usd, r.requests), reverse=True)
    return rollups


@dataclass
class RouteHit:
    route: str
    hits: int
    pct: float
    p95_latency_ms: float | None
    error_count: int


def route_hits(calls: list[LLMCall]) -> list[RouteHit]:
    counts: Counter[str] = Counter()
    per_route_latency: dict[str, list[float]] = {}
    per_route_errors: dict[str, int] = {}
    for c in calls:
        if not c.route_name:
            continue
        counts[c.route_name] += 1
        if c.duration_ms is not None:
            per_route_latency.setdefault(c.route_name, []).append(c.duration_ms)
        if c.status_code is not None and c.status_code >= 400:
            per_route_errors[c.route_name] = per_route_errors.get(c.route_name, 0) + 1
    total = sum(counts.values())
    if total == 0:
        return []
    return [
        RouteHit(
            route=r,
            hits=n,
            pct=(n / total) * 100.0,
            p95_latency_ms=_percentile(per_route_latency.get(r, []), 95),
            error_count=per_route_errors.get(r, 0),
        )
        for r, n in counts.most_common()
    ]


def _fmt_cost(v: float | None, *, zero: str = "—") -> str:
    if v is None:
        return "—"
    if v == 0:
        return zero
    if abs(v) < 0.0001:
        return f"${v:.8f}".rstrip("0").rstrip(".")
    if abs(v) < 0.01:
        return f"${v:.6f}".rstrip("0").rstrip(".")
    if abs(v) < 1:
        return f"${v:.4f}"
    return f"${v:,.2f}"


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


def _fmt_tps(v: float | None) -> str:
    if v is None or v <= 0:
        return "—"
    if v >= 100:
        return f"{v:.0f}/s"
    return f"{v:.1f}/s"


def _latency_style(v: float | None) -> str:
    if v is None:
        return "dim"
    if v < 500:
        return "green"
    if v < 2000:
        return "yellow"
    return "red"


def _ttft_style(v: float | None) -> str:
    if v is None:
        return "dim"
    if v < 300:
        return "green"
    if v < 1000:
        return "yellow"
    return "red"


def _truncate_model(name: str, limit: int = 32) -> str:
    if len(name) <= limit:
        return name
    return name[: limit - 1] + "…"


def _status_text(code: int | None) -> Text:
    if code is None:
        return Text("—", style="dim")
    if 200 <= code < 300:
        return Text("● ok", style="green")
    if 300 <= code < 400:
        return Text(f"● {code}", style="yellow")
    if 400 <= code < 500:
        return Text(f"● {code}", style="yellow bold")
    return Text(f"● {code}", style="red bold")


def _summary_panel(last: LLMCall | None, stats: AggregateStats) -> Panel:
    # Content-sized columns with a fixed gutter keep the two blocks close
    # together instead of stretching across the full terminal on wide screens.
    grid = Table.grid(padding=(0, 4))
    grid.add_column(no_wrap=True)
    grid.add_column(no_wrap=True)

    # Left: latest request snapshot.
    left = Table.grid(padding=(0, 1))
    left.add_column(style="dim", no_wrap=True)
    left.add_column(no_wrap=True)
    if last is None:
        left.add_row("latest", Text("waiting for spans…", style="dim italic"))
    else:
        model_text = Text(_truncate_model(last.model, 48), style="bold cyan")
        if last.is_streaming:
            model_text.append("  ⟳ stream", style="dim")
        left.add_row("model", model_text)
        if last.request_model and last.request_model != last.model:
            left.add_row(
                "requested", Text(_truncate_model(last.request_model, 48), style="cyan")
            )
        if last.route_name:
            left.add_row("route", Text(last.route_name, style="yellow"))
        left.add_row("status", _status_text(last.status_code))
        tokens = Text()
        tokens.append(_fmt_tokens(last.prompt_tokens))
        tokens.append(" in", style="dim")
        tokens.append("  ·  ", style="dim")
        tokens.append(_fmt_tokens(last.completion_tokens), style="green")
        tokens.append(" out", style="dim")
        if last.cached_input_tokens:
            tokens.append("  ·  ", style="dim")
            tokens.append(_fmt_tokens(last.cached_input_tokens), style="yellow")
            tokens.append(" cached", style="dim")
        left.add_row("tokens", tokens)
        timing = Text()
        timing.append("TTFT ", style="dim")
        timing.append(_fmt_ms(last.ttft_ms), style=_ttft_style(last.ttft_ms))
        timing.append("  ·  ", style="dim")
        timing.append("lat ", style="dim")
        timing.append(_fmt_ms(last.duration_ms), style=_latency_style(last.duration_ms))
        tps = last.tokens_per_sec
        if tps:
            timing.append("  ·  ", style="dim")
            timing.append(_fmt_tps(tps), style="green")
        left.add_row("timing", timing)
        left.add_row("cost", Text(_fmt_cost(last.cost_usd), style="green bold"))

    # Right: lifetime totals.
    right = Table.grid(padding=(0, 1))
    right.add_column(style="dim", no_wrap=True)
    right.add_column(no_wrap=True)
    right.add_row(
        "requests",
        Text(f"{stats.count:,}", style="bold"),
    )
    if stats.error_count:
        err_text = Text()
        err_text.append(f"{stats.error_count:,}", style="red bold")
        parts: list[str] = []
        if stats.errors_4xx:
            parts.append(f"{stats.errors_4xx} 4xx")
        if stats.errors_5xx:
            parts.append(f"{stats.errors_5xx} 5xx")
        if parts:
            err_text.append(f"  ({' · '.join(parts)})", style="dim")
        right.add_row("errors", err_text)
    cost_str = _fmt_cost(stats.total_cost_usd) if stats.has_cost else "—"
    right.add_row("total cost", Text(cost_str, style="green bold"))
    tokens_total = Text()
    tokens_total.append(_fmt_tokens(stats.total_input_tokens))
    tokens_total.append(" in", style="dim")
    tokens_total.append("  ·  ", style="dim")
    tokens_total.append(_fmt_tokens(stats.total_output_tokens), style="green")
    tokens_total.append(" out", style="dim")
    right.add_row("tokens", tokens_total)
    lat_text = Text()
    lat_text.append("p50 ", style="dim")
    lat_text.append(
        _fmt_ms(stats.p50_latency_ms), style=_latency_style(stats.p50_latency_ms)
    )
    lat_text.append("  ·  ", style="dim")
    lat_text.append("p95 ", style="dim")
    lat_text.append(
        _fmt_ms(stats.p95_latency_ms), style=_latency_style(stats.p95_latency_ms)
    )
    lat_text.append("  ·  ", style="dim")
    lat_text.append("p99 ", style="dim")
    lat_text.append(
        _fmt_ms(stats.p99_latency_ms), style=_latency_style(stats.p99_latency_ms)
    )
    right.add_row("latency", lat_text)
    ttft_text = Text()
    ttft_text.append("p50 ", style="dim")
    ttft_text.append(_fmt_ms(stats.p50_ttft_ms), style=_ttft_style(stats.p50_ttft_ms))
    ttft_text.append("  ·  ", style="dim")
    ttft_text.append("p95 ", style="dim")
    ttft_text.append(_fmt_ms(stats.p95_ttft_ms), style=_ttft_style(stats.p95_ttft_ms))
    ttft_text.append("  ·  ", style="dim")
    ttft_text.append("p99 ", style="dim")
    ttft_text.append(_fmt_ms(stats.p99_ttft_ms), style=_ttft_style(stats.p99_ttft_ms))
    right.add_row("TTFT", ttft_text)
    sess = Text()
    sess.append(f"{stats.distinct_sessions}")
    if stats.current_session:
        sess.append("  ·  current ", style="dim")
        sess.append(stats.current_session, style="magenta")
    right.add_row("sessions", sess)

    grid.add_row(left, right)
    return Panel(
        grid,
        title="[bold]live LLM traffic[/]",
        border_style="cyan",
        box=SIMPLE_HEAVY,
        padding=(0, 1),
    )


def _model_rollup_table(rollups: list[ModelRollup]) -> Table:
    table = Table(
        title="by model",
        title_justify="left",
        title_style="bold dim",
        caption="cost via DigitalOcean Gradient catalog",
        caption_justify="left",
        caption_style="dim italic",
        box=SIMPLE,
        header_style="bold",
        pad_edge=False,
        padding=(0, 1),
    )
    table.add_column("model", style="cyan", no_wrap=True)
    table.add_column("req", justify="right")
    table.add_column("input", justify="right")
    table.add_column("output", justify="right", style="green")
    table.add_column("cache wr", justify="right", style="yellow")
    table.add_column("cache rd", justify="right", style="yellow")
    table.add_column("tok/s", justify="right")
    table.add_column("cost", justify="right", style="green")
    if not rollups:
        table.add_row(
            Text("no requests yet", style="dim italic"),
            *(["—"] * 7),
        )
        return table
    for r in rollups:
        cost_cell = _fmt_cost(r.cost_usd) if r.has_cost else "—"
        table.add_row(
            _truncate_model(r.model),
            f"{r.requests:,}",
            _fmt_tokens(r.input_tokens),
            _fmt_tokens(r.output_tokens),
            _fmt_int(r.cache_write),
            _fmt_int(r.cache_read),
            _fmt_tps(r.avg_tokens_per_sec),
            cost_cell,
        )
    return table


def _route_hit_table(hits: list[RouteHit]) -> Table:
    table = Table(
        title="route share",
        title_justify="left",
        title_style="bold dim",
        box=SIMPLE,
        header_style="bold",
        pad_edge=False,
        padding=(0, 1),
    )
    table.add_column("route", style="cyan")
    table.add_column("hits", justify="right")
    table.add_column("%", justify="right")
    table.add_column("p95", justify="right")
    table.add_column("err", justify="right")
    for h in hits:
        err_cell = (
            Text(f"{h.error_count:,}", style="red bold") if h.error_count else "—"
        )
        table.add_row(
            h.route,
            f"{h.hits:,}",
            f"{h.pct:5.1f}%",
            Text(_fmt_ms(h.p95_latency_ms), style=_latency_style(h.p95_latency_ms)),
            err_cell,
        )
    return table


def _recent_table(calls: list[LLMCall], limit: int = 15) -> Table:
    show_route = any(c.route_name for c in calls)
    show_cache = any((c.cached_input_tokens or 0) > 0 for c in calls)
    show_rsn = any((c.reasoning_tokens or 0) > 0 for c in calls)

    caption_parts = ["in·new = fresh prompt tokens"]
    if show_cache:
        caption_parts.append("in·cache = cached read")
    if show_rsn:
        caption_parts.append("rsn = reasoning")
    caption_parts.append("lat = total latency")

    table = Table(
        title=f"recent · last {min(limit, len(calls)) if calls else 0}",
        title_justify="left",
        title_style="bold dim",
        caption="  ·  ".join(caption_parts),
        caption_justify="left",
        caption_style="dim italic",
        box=SIMPLE,
        header_style="bold",
        pad_edge=False,
        padding=(0, 1),
    )
    table.add_column("time", no_wrap=True)
    table.add_column("model", style="cyan", no_wrap=True)
    if show_route:
        table.add_column("route", style="yellow", no_wrap=True)
    table.add_column("in·new", justify="right")
    if show_cache:
        table.add_column("in·cache", justify="right", style="yellow")
    table.add_column("out", justify="right", style="green")
    if show_rsn:
        table.add_column("rsn", justify="right")
    table.add_column("tok/s", justify="right")
    table.add_column("TTFT", justify="right")
    table.add_column("lat", justify="right")
    table.add_column("cost", justify="right", style="green")
    table.add_column("status")

    if not calls:
        cols = len(table.columns)
        table.add_row(
            Text("waiting for spans…", style="dim italic"),
            *(["—"] * (cols - 1)),
        )
        return table

    recent = list(reversed(calls))[:limit]
    for idx, c in enumerate(recent):
        is_newest = idx == 0
        time_style = "bold white" if is_newest else None
        model_style = "bold cyan" if is_newest else "cyan"
        row: list[object] = [
            (
                Text(c.timestamp.strftime("%H:%M:%S"), style=time_style)
                if time_style
                else c.timestamp.strftime("%H:%M:%S")
            ),
            Text(_truncate_model(c.model), style=model_style),
        ]
        if show_route:
            row.append(c.route_name or "—")
        row.append(_fmt_tokens(c.prompt_tokens))
        if show_cache:
            row.append(_fmt_int(c.cached_input_tokens))
        row.append(_fmt_tokens(c.completion_tokens))
        if show_rsn:
            row.append(_fmt_int(c.reasoning_tokens))
        row.extend(
            [
                _fmt_tps(c.tokens_per_sec),
                Text(_fmt_ms(c.ttft_ms), style=_ttft_style(c.ttft_ms)),
                Text(_fmt_ms(c.duration_ms), style=_latency_style(c.duration_ms)),
                _fmt_cost(c.cost_usd),
                _status_text(c.status_code),
            ]
        )
        table.add_row(*row)
    return table


def _last_error(calls: list[LLMCall]) -> LLMCall | None:
    for c in reversed(calls):
        if c.status_code is not None and c.status_code >= 400:
            return c
    return None


def _http_reason(code: int) -> str:
    try:
        return HTTPStatus(code).phrase
    except ValueError:
        return ""


def _fmt_ago(ts: datetime) -> str:
    # `ts` is produced in collector.py via datetime.now(tz=...), but fall back
    # gracefully if a naive timestamp ever sneaks in.
    now = datetime.now(tz=ts.tzinfo) if ts.tzinfo else datetime.now()
    delta = (now - ts).total_seconds()
    if delta < 0:
        delta = 0
    if delta < 60:
        return f"{int(delta)}s ago"
    if delta < 3600:
        return f"{int(delta // 60)}m ago"
    return f"{int(delta // 3600)}h ago"


def _error_banner(call: LLMCall) -> Panel:
    code = call.status_code or 0
    border = "red" if code >= 500 else "yellow"
    header = Text()
    header.append(f"● {code}", style=f"{border} bold")
    reason = _http_reason(code)
    if reason:
        header.append(f" {reason}", style=border)
    header.append("  ·  ", style="dim")
    header.append(_truncate_model(call.model, 48), style="cyan")
    if call.route_name:
        header.append("  ·  ", style="dim")
        header.append(call.route_name, style="yellow")
    header.append("  ·  ", style="dim")
    header.append(_fmt_ago(call.timestamp), style="dim")
    if call.request_id:
        header.append("  ·  req ", style="dim")
        header.append(call.request_id, style="magenta")
    return Panel(
        header,
        title="[bold]last error[/]",
        title_align="left",
        border_style=border,
        box=SIMPLE,
        padding=(0, 1),
    )


def _footer(stats: AggregateStats) -> Text:
    waiting = stats.count == 0
    text = Text()
    text.append("Ctrl-C ", style="bold")
    text.append("exit", style="dim")
    text.append("  ·  OTLP :4317", style="dim")
    text.append("  ·  pricing: DigitalOcean ", style="dim")
    if waiting:
        text.append("waiting for spans", style="yellow")
        text.append(
            " — set tracing.opentracing_grpc_endpoint=localhost:4317", style="dim"
        )
    else:
        text.append(f"receiving · {stats.count:,} call(s) buffered", style="green")
    return text


def render(calls: list[LLMCall]) -> Align:
    last = calls[-1] if calls else None
    stats = aggregates(calls)
    rollups = model_rollups(calls)
    hits = route_hits(calls)

    parts: list[object] = [_summary_panel(last, stats)]
    err = _last_error(calls)
    if err is not None:
        parts.append(_error_banner(err))
    if hits:
        split = Table.grid(padding=(0, 2))
        split.add_column(no_wrap=False)
        split.add_column(no_wrap=False)
        split.add_row(_model_rollup_table(rollups), _route_hit_table(hits))
        parts.append(split)
    else:
        parts.append(_model_rollup_table(rollups))
    parts.append(_recent_table(calls))
    parts.append(_footer(stats))
    # Cap overall width so wide terminals don't stretch the layout into a
    # mostly-whitespace gap between columns.
    return Align.left(Group(*parts), width=MAX_WIDTH)
