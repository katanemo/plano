"""
CLI commands for ChatGPT subscription management.

Usage:
    planoai chatgpt login    - Authenticate with ChatGPT via device code flow
    planoai chatgpt status   - Check authentication status
    planoai chatgpt logout   - Remove stored credentials
"""

import datetime

import click
from rich.console import Console

from planoai import chatgpt_auth

console = Console()


@click.group()
def chatgpt():
    """ChatGPT subscription management."""
    pass


@chatgpt.command()
def login():
    """Authenticate with your ChatGPT subscription using device code flow."""
    try:
        auth_record = chatgpt_auth.login()
        account_id = auth_record.get("account_id", "unknown")
        console.print(
            f"\n[green]Successfully authenticated with ChatGPT![/green]"
            f"\nAccount ID: {account_id}"
            f"\nCredentials saved to: {chatgpt_auth.CHATGPT_AUTH_FILE}"
        )
    except Exception as e:
        console.print(f"\n[red]Authentication failed:[/red] {e}")
        raise SystemExit(1)


@chatgpt.command()
def status():
    """Check ChatGPT authentication status."""
    auth_data = chatgpt_auth.load_auth()
    if not auth_data or not auth_data.get("access_token"):
        console.print(
            "[yellow]Not authenticated.[/yellow] Run 'planoai chatgpt login'."
        )
        return

    account_id = auth_data.get("account_id", "unknown")
    expires_at = auth_data.get("expires_at")

    if expires_at:
        expiry_time = datetime.datetime.fromtimestamp(
            expires_at, tz=datetime.timezone.utc
        )
        now = datetime.datetime.now(tz=datetime.timezone.utc)
        if expiry_time > now:
            remaining = expiry_time - now
            console.print(
                f"[green]Authenticated[/green]"
                f"\n  Account ID: {account_id}"
                f"\n  Token expires: {expiry_time.strftime('%Y-%m-%d %H:%M:%S UTC')}"
                f" ({remaining.seconds // 60}m remaining)"
            )
        else:
            console.print(
                f"[yellow]Token expired[/yellow]"
                f"\n  Account ID: {account_id}"
                f"\n  Expired at: {expiry_time.strftime('%Y-%m-%d %H:%M:%S UTC')}"
                f"\n  Will auto-refresh on next use, or run 'planoai chatgpt login'."
            )
    else:
        console.print(
            f"[green]Authenticated[/green] (no expiry info)"
            f"\n  Account ID: {account_id}"
        )


@chatgpt.command()
def logout():
    """Remove stored ChatGPT credentials."""
    chatgpt_auth.delete_auth()
    console.print("[green]ChatGPT credentials removed.[/green]")
