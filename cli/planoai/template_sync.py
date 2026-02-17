from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path

import yaml

from planoai.init_cmd import BUILTIN_TEMPLATES


@dataclass(frozen=True)
class SyncEntry:
    template_id: str
    template_file: str
    demo_configs: tuple[str, ...]
    transform: str = "none"


REPO_ROOT = Path(__file__).resolve().parents[2]
TEMPLATES_DIR = REPO_ROOT / "cli" / "planoai" / "templates"
SYNC_MAP_PATH = TEMPLATES_DIR / "template_sync_map.yaml"


def _load_sync_entries() -> list[SyncEntry]:
    payload = yaml.safe_load(SYNC_MAP_PATH.read_text(encoding="utf-8")) or {}
    rows = payload.get("templates", [])
    entries: list[SyncEntry] = []
    for row in rows:
        entries.append(
            SyncEntry(
                template_id=row["template_id"],
                template_file=row["template_file"],
                demo_configs=tuple(row.get("demo_configs", [])),
                transform=row.get("transform", "none"),
            )
        )
    return entries


def _render_for_demo(template_text: str, transform: str) -> str:
    if transform == "none":
        rendered = template_text
    else:
        raise ValueError(f"Unknown transform profile: {transform}")

    return rendered if rendered.endswith("\n") else f"{rendered}\n"


def _validate_manifest(entries: list[SyncEntry]) -> list[str]:
    errors: list[str] = []
    builtin_ids = {t.id for t in BUILTIN_TEMPLATES}
    manifest_ids = {entry.template_id for entry in entries}

    missing = sorted(builtin_ids - manifest_ids)
    extra = sorted(manifest_ids - builtin_ids)
    if missing:
        errors.append(f"Missing template IDs in sync map: {', '.join(missing)}")
    if extra:
        errors.append(f"Unknown template IDs in sync map: {', '.join(extra)}")

    for entry in entries:
        template_path = TEMPLATES_DIR / entry.template_file
        if not template_path.exists():
            errors.append(
                f"template_file does not exist for '{entry.template_id}': {template_path}"
            )
        for demo_rel_path in entry.demo_configs:
            demo_path = REPO_ROOT / demo_rel_path
            if not demo_path.exists():
                errors.append(
                    f"demo config does not exist for '{entry.template_id}': {demo_path}"
                )

    return errors


def write_mapped_demo_configs(*, verbose: bool = False) -> int:
    entries = _load_sync_entries()
    manifest_errors = _validate_manifest(entries)
    if manifest_errors:
        for error in manifest_errors:
            print(f"[manifest] {error}")
        return 2

    write_count = 0
    for entry in entries:
        template_text = (TEMPLATES_DIR / entry.template_file).read_text(
            encoding="utf-8"
        )
        expected_text = _render_for_demo(template_text, entry.transform)

        for demo_rel_path in entry.demo_configs:
            demo_path = REPO_ROOT / demo_rel_path
            # Keep this as a write-only sync step so CI behavior is deterministic.
            demo_path.write_text(expected_text, encoding="utf-8")
            write_count += 1
            if verbose:
                print(
                    f"[wrote] {demo_rel_path} <- {entry.template_id} ({entry.template_file})"
                )

    print(f"Wrote {write_count} mapped demo config(s) from CLI templates.")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Sync CLI templates to mapped demo config.yaml files (write-only)."
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Print each file written during sync.",
    )
    args = parser.parse_args()

    return write_mapped_demo_configs(verbose=bool(args.verbose))


if __name__ == "__main__":
    raise SystemExit(main())
