"""`planoai obs` — live observability TUI."""

from __future__ import annotations

import logging
import os
import time

import rich_click as click
import yaml
from rich.console import Console
from rich.live import Live

from planoai.consts import PLANO_COLOR
from planoai.obs.collector import (
    DEFAULT_CAPACITY,
    DEFAULT_GRPC_PORT,
    LLMCallStore,
    ObsCollector,
)
from planoai.obs.pricing import DEFAULT_PRICING_PROVIDER, PricingCatalog
from planoai.obs.render import render
from planoai.utils import find_config_file

logger = logging.getLogger(__name__)


def _resolve_pricing_source(
    config_file: str | None,
    provider_override: str | None,
    url_override: str | None,
) -> tuple[str, str | None]:
    """Pick the cost pricing source.

    Precedence: explicit CLI overrides > the first ``type: cost`` entry in
    ``model_metrics_sources`` from the Plano config > the DigitalOcean default.
    """
    provider = DEFAULT_PRICING_PROVIDER
    url: str | None = None

    config_path = find_config_file(file=config_file)
    if config_path and os.path.exists(config_path):
        try:
            with open(config_path, "r") as f:
                config = yaml.safe_load(f) or {}
            sources = config.get("model_metrics_sources") or []
            for source in sources:
                if isinstance(source, dict) and source.get("type") == "cost":
                    if source.get("provider"):
                        provider = str(source["provider"])
                    if source.get("url"):
                        url = str(source["url"])
                    break
        except Exception as exc:  # noqa: BLE001 — config is optional for obs
            logger.warning(
                "could not read pricing source from %s: %s", config_path, exc
            )

    if provider_override:
        provider = provider_override
    if url_override:
        url = url_override

    return provider, url


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
@click.option(
    "--config",
    "config_file",
    type=str,
    default=None,
    help="Path to the Plano config to read the pricing source from "
    "(defaults to ./config.yaml or ./plano_config.yaml).",
)
@click.option(
    "--pricing-provider",
    type=click.Choice(["digitalocean", "models.dev"]),
    default=None,
    help="Override the cost pricing provider (otherwise read from config).",
)
@click.option(
    "--pricing-url",
    type=str,
    default=None,
    help="Override the pricing catalog URL (otherwise read from config / provider default).",
)
def obs(
    port: int,
    host: str,
    capacity: int,
    refresh_ms: int,
    config_file: str | None,
    pricing_provider: str | None,
    pricing_url: str | None,
) -> None:
    console = Console()
    provider, url = _resolve_pricing_source(config_file, pricing_provider, pricing_url)
    console.print(
        f"[bold {PLANO_COLOR}]planoai obs[/] — loading {provider} pricing catalog...",
        end="",
    )
    pricing = PricingCatalog.fetch(provider=provider, url=url)
    if len(pricing):
        sample = ", ".join(pricing.sample_models(3))
        console.print(
            f" [green]{len(pricing)} models loaded[/] [dim]({sample}, ...)[/]"
        )
    else:
        console.print(
            " [yellow]no pricing loaded[/] — "
            f"[dim]cost column will be blank ({provider} catalog unreachable)[/]"
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
