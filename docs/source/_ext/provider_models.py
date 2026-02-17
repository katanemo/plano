"""Sphinx extension to copy provider_models.yaml to build output."""
from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING
import shutil

if TYPE_CHECKING:
    from sphinx.application import Sphinx


def _on_build_finished(app: Sphinx, exception: Exception | None) -> None:
    """Copy provider_models.yaml to the build output after build completes."""
    if exception is not None:
        return

    # Only generate for HTML-like builders where app.outdir is a website root.
    if getattr(app.builder, "format", None) != "html":
        return

    # Source path: provider_models.yaml is copied into the Docker image at /docs/provider_models.yaml
    # This follows the pattern used for config templates like envoy.template.yaml and plano_config_schema.yaml
    docs_root = Path(app.srcdir).parent  # Goes from source/ to docs/
    source_path = docs_root / "provider_models.yaml"

    if not source_path.exists():
        # Silently skip if source file doesn't exist
        return

    # Per repo convention, place generated artifacts under an `includes/` folder.
    out_path = Path(app.outdir) / "includes" / "provider_models.yaml"
    out_path.parent.mkdir(parents=True, exist_ok=True)

    shutil.copy2(source_path, out_path)


def setup(app: Sphinx) -> dict[str, object]:
    """Register the extension with Sphinx."""
    app.connect("build-finished", _on_build_finished)
    return {
        "version": "0.1.0",
        "parallel_read_safe": True,
        "parallel_write_safe": True,
    }
