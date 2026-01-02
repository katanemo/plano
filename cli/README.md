## Setup Instructions(User): plano CLI

This guide will walk you through the steps to set up the plano cli on your local machine

### Step 1: Create a Python virtual environment

In the tools directory, create a Python virtual environment by running:

```bash
python -m venv venv
```

### Step 2: Activate the virtual environment
* On Linux/MacOS:

```bash
source venv/bin/activate
```

### Step 3: Run the build script
```bash
pip install planoai==0.4.1
```

## Uninstall Instructions: plano CLI
```bash
pip uninstall planoai
```

## Setup Instructions (Dev): plano CLI

This guide will walk you through the steps to set up the plano cli on your local machine when you want to develop the plano CLI

### Step 1: Install uv

Install uv if you haven't already:

```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
```

### Step 2: Create a Python virtual environment and install dependencies

In the cli directory, run:

```bash
uv sync
```

This will create a virtual environment and install all dependencies.

### Step 3: Activate the virtual environment (optional)

uv will automatically use the virtual environment, but if you need to activate it manually:

* On Linux/MacOS:

```bash
source .venv/bin/activate
```

### Step 4: build Arch
```bash
uv run planoai build
```

### Logs
`plano` command can also view logs from the gateway. Use following command to view logs,

```bash
uv run planoai logs --follow
```

## Uninstall Instructions: plano CLI
```bash
pip uninstall planoai
