## plano CLI - Local Development

This guide will walk you through setting up the plano CLI for local development using uv.

### Install uv

First, install the uv package manager. This is required for managing dependencies and running the development version of planoai.

**On macOS and Linux:**
```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
```

**On Windows:**
```powershell
powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"
```

### Setup

1. **Install dependencies**

   In the cli directory, run:

   ```bash
   uv sync
   ```

   This will create a virtual environment in `.venv` and install all dependencies from `pyproject.toml`.

2. **Install the CLI tool globally (optional)**

   To install planoai as a global tool on your system:

   ```bash
   uv tool install --editable .
   ```

   This installs planoai globally in editable mode, allowing you to run `planoai` commands from anywhere while still using the source code from this directory. Any changes you make to the code will be reflected immediately.

3. **Run plano commands**

   Use `uv run` to execute plano commands with the development version:

   ```bash
   uv run planoai build
   ```

   Or, if you installed globally with `uv tool install .`:

   ```bash
   planoai build
   ```

   Note: `uv run` automatically uses the virtual environment - no activation needed.

### Development Workflow

**Build plano:**
```bash
uv run planoai build
```

**View logs:**
```bash
uv run planoai logs --follow
```

**Run other plano commands:**
```bash
uv run planoai <command> [options]
```

### Optional: Manual Virtual Environment Activation

While `uv run` handles the virtual environment automatically, you can activate it manually if needed:

```bash
source .venv/bin/activate
planoai build  # No need for 'uv run' when activated
```

**Note:** For end-user installation instructions, see the [plano documentation](https://docs.planoai.dev).
