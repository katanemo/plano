"""`planoai obs` — live observability TUI."""

from __future__ import annotations

import time

import rich_click as click
from rich.console import Console
from rich.live import Live

from planoai.consts import PLANO_COLOR
from planoai.obs.collector import (
    DEFAULT_CAPACITY,
    DEFAULT_GRPC_PORT,
    LLMCallStore,
    ObsCollector,
)
from planoai.obs.pricing import PricingCatalog
from planoai.obs.render import render


@click.command(name="obs", help="Live observability console for Plano LLM traffic.")
@click.option(
    "--port",
    type=int,
    default=DEFAULT_GRPC_PORT,
    show_default=True,
    help="OTLP/gRPC port to listen on. Must match the brightstaff tracing endpoint.",
)
@click.option(
    "--host",
    type=str,
    default="0.0.0.0",
    show_default=True,
    help="Host to bind the OTLP listener.",
)
@click.option(
    "--capacity",
    type=int,
    default=DEFAULT_CAPACITY,
    show_default=True,
    help="Max LLM calls kept in memory; older calls evicted FIFO.",
)
@click.option(
    "--refresh-ms",
    type=int,
    default=500,
    show_default=True,
    help="TUI refresh interval.",
)
def obs(port: int, host: str, capacity: int, refresh_ms: int) -> None:
    console = Console()
    console.print(
        f"[bold {PLANO_COLOR}]planoai obs[/] — loading DO pricing catalog...",
        end="",
    )
    pricing = PricingCatalog.fetch()
    if len(pricing):
        sample = ", ".join(pricing.sample_models(3))
        console.print(
            f" [green]{len(pricing)} models loaded[/] [dim]({sample}, ...)[/]"
        )
    else:
        console.print(
            " [yellow]no pricing loaded[/] — "
            "[dim]cost column will be blank (DO catalog unreachable)[/]"
        )

    store = LLMCallStore(capacity=capacity)
    collector = ObsCollector(store=store, pricing=pricing, host=host, port=port)
    try:
        collector.start()
    except OSError as exc:
        console.print(f"[red]{exc}[/]")
        raise SystemExit(1)

    console.print(
        f"Listening for OTLP spans on [bold]{host}:{port}[/]. "
        "Ensure plano config has [cyan]tracing.opentracing_grpc_endpoint: http://localhost:4317[/] "
        "and [cyan]tracing.random_sampling: 100[/] (or run [bold]planoai up[/] "
        "with no config — it wires this automatically)."
    )
    console.print("Press [bold]Ctrl-C[/] to exit.\n")

    refresh = max(0.05, refresh_ms / 1000.0)
    try:
        with Live(
            render(store.snapshot()),
            console=console,
            refresh_per_second=1.0 / refresh,
            screen=False,
        ) as live:
            while True:
                time.sleep(refresh)
                live.update(render(store.snapshot()))
    except KeyboardInterrupt:
        console.print("\n[dim]obs stopped[/]")
    finally:
        collector.stop()
