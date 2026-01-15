"""
Travel Agents - LangChain Demo

LangChain-powered travel agents that integrate with Plano gateway.
Each agent uses LangChain's tool-calling capabilities for a clean, modular design.

Agents:
- Weather Agent (port 10510): Real-time weather and forecasts
- Flight Agent (port 10520): Flight search and information

Usage:
    # Start weather agent
    python -m travel_agents.weather_agent

    # Start flight agent
    python -m travel_agents.flight_agent

    # Or use the CLI
    travel_agents weather --port 10510
    travel_agents flight --port 10520
"""

import click


@click.group()
def cli():
    """Travel Agents - LangChain demo for Plano integration."""
    pass


@cli.command()
@click.option("--host", default="0.0.0.0", help="Host to bind to")
@click.option("--port", default=10510, help="Port to listen on")
def weather(host: str, port: int):
    """Start the Weather Agent (LangChain)."""
    from travel_agents.weather_agent import start_server

    click.echo(f"🌤️ Starting Weather Agent on {host}:{port}")
    start_server(host=host, port=port)


@cli.command()
@click.option("--host", default="0.0.0.0", help="Host to bind to")
@click.option("--port", default=10520, help="Port to listen on")
def flight(host: str, port: int):
    """Start the Flight Agent (LangChain)."""
    from travel_agents.flight_agent import start_server

    click.echo(f"✈️ Starting Flight Agent on {host}:{port}")
    start_server(host=host, port=port)


def main():
    """Entry point for the CLI."""
    cli()


if __name__ == "__main__":
    main()
