---
name: release
description: Bump the Plano version across all required files. Use when preparing a release.
disable-model-invocation: true
user-invocable: true
---

Prepare a release version bump. The user may provide the new version number as $ARGUMENTS (e.g., `/release 0.4.12`), or a bump type (`major`, `minor`, `patch`).

If no argument is provided, read the current version from `cli/planoai/__init__.py`, auto-increment the patch version (e.g., `0.4.11` → `0.4.12`), and confirm with the user before proceeding.

Update the version string in ALL of these files:

- `.github/workflows/ci.yml`
- `cli/planoai/__init__.py`
- `cli/planoai/consts.py`
- `cli/pyproject.toml`
- `build_filter_image.sh`
- `config/validate_plano_config.sh`
- `docs/source/conf.py`
- `docs/source/get_started/quickstart.rst`
- `docs/source/resources/deployment.rst`
- `apps/www/src/components/Hero.tsx`
- `demos/llm_routing/preference_based_routing/README.md`

Do NOT change version strings in `*.lock` files or `Cargo.lock`.

After updating all version strings, run `cd cli && uv lock` to update the lock file with the new version.

After making changes, show a summary of all files modified and the old → new version.
