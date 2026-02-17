from __future__ import annotations

import argparse
import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any

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


def _normalize_yaml(text: str) -> Any:
    return yaml.safe_load(text) if text.strip() else None


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


def run_sync(*, write: bool, verbose: bool = False) -> int:
    entries = _load_sync_entries()
    manifest_errors = _validate_manifest(entries)
    if manifest_errors:
        for error in manifest_errors:
            print(f"[manifest] {error}")
        return 2

    drift_count = 0
    for entry in entries:
        template_text = (TEMPLATES_DIR / entry.template_file).read_text(
            encoding="utf-8"
        )
        expected_text = _render_for_demo(template_text, entry.transform)
        expected_yaml = _normalize_yaml(expected_text)

        for demo_rel_path in entry.demo_configs:
            demo_path = REPO_ROOT / demo_rel_path
            actual_text = demo_path.read_text(encoding="utf-8")
            actual_yaml = _normalize_yaml(actual_text)

            if actual_yaml == expected_yaml:
                if verbose:
                    print(f"[ok] {demo_rel_path}")
                continue

            drift_count += 1
            print(
                f"[drift] {demo_rel_path} differs from template '{entry.template_id}' "
                f"({entry.template_file})"
            )

            if write:
                demo_path.write_text(expected_text, encoding="utf-8")
                print(f"[fixed] wrote {demo_rel_path}")
            elif verbose:
                actual_repr = json.dumps(actual_yaml, indent=2, sort_keys=True)
                expected_repr = json.dumps(expected_yaml, indent=2, sort_keys=True)
                print(f"[actual]\n{actual_repr}\n[expected]\n{expected_repr}")

    if drift_count == 0:
        print("All mapped demo configs are in sync with CLI templates.")
        return 0

    if write:
        print(f"Updated {drift_count} out-of-sync demo config(s).")
        return 0

    print(
        f"Found {drift_count} out-of-sync demo config(s). "
        "Run `python -m planoai.template_sync --write` to update."
    )
    return 1


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Check or sync CLI templates to demo config.yaml files."
    )
    mode_group = parser.add_mutually_exclusive_group()
    mode_group.add_argument(
        "--write",
        action="store_true",
        help="Write template content to mapped demo configs when drift is found.",
    )
    mode_group.add_argument(
        "--check",
        action="store_true",
        help="Check for drift and return non-zero if any mapped demos are out of sync.",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Print per-file status and parsed YAML when drift is detected.",
    )
    args = parser.parse_args()

    write_mode = bool(args.write)
    return run_sync(write=write_mode, verbose=bool(args.verbose))


if __name__ == "__main__":
    raise SystemExit(main())
