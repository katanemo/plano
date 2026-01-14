# Contribution

We would love feedback on our [Roadmap](https://github.com/orgs/katanemo/projects/1) and we welcome contributions to **Plano**!
Whether you're fixing bugs, adding new features, improving documentation, or creating tutorials, your help is much appreciated.

## How to Contribute

### 1. Fork the Repository

Fork the repository to create your own version of **Plano**:

- Navigate to the [Plano GitHub repository](https://github.com/katanemo/plano).
- Click the "Fork" button in the upper right corner.
- This will create a copy of the repository under your GitHub account.

### 2. Clone Your Fork

Once you've forked the repository, clone it to your local machine:

```bash
$ git clone https://github.com/katanemo/plano.git
$ cd plano
```

### 3. Install Prerequisites

**Install uv** (Python package manager for the planoai CLI):

```bash
$ curl -LsSf https://astral.sh/uv/install.sh | sh
```

**Install pre-commit hooks:**

Pre-commit hooks help maintain code quality by running automated checks before each commit. Install them with:

```bash
$ pip install pre-commit
$ pre-commit install
```

The pre-commit hooks will automatically run:
- YAML validation
- Code formatting checks (Rust with `cargo fmt`, Python with `black`)
- Linting checks (Rust with `cargo clippy`)
- Rust unit tests

### 4. Setup the planoai CLI

The planoai CLI is used to build, run, and manage Plano locally:

```bash
$ cd cli
$ uv sync
```

This creates a virtual environment in `.venv` and installs all dependencies.

Optionally, install planoai globally in editable mode:

```bash
$ uv tool install --editable .
```

Now you can use `planoai` commands from anywhere, or use `uv run planoai` from the `cli` directory.

### 5. Create a Branch

Use a descriptive name for your branch (e.g., fix-bug-123, add-feature-x).

```bash
$ git checkout -b <your-branch-name>
```

### 6. Make Your Changes

Make your changes in the relevant files. If you're adding new features or fixing bugs, please include tests where applicable.

### 7. Test Your Changes Locally

**Run Rust tests:**

```bash
$ cd crates
$ cargo test
```

For library tests only:
```bash
$ cargo test --lib
```

**Run Python CLI tests:**

```bash
$ cd cli
$ uv run pytest
```

Or with verbose output:
```bash
$ uv run pytest -v
```

**Run pre-commit checks manually:**

Before committing, you can run all pre-commit checks manually:

```bash
$ pre-commit run --all-files
```

This ensures your code passes all checks before you commit.

### 8. Push Changes and Create a Pull Request

Once your changes are tested and committed:

```bash
$ git push origin <your-branch-name>
```

Go back to the original Plano repository, and you should see a "Compare & pull request" button. Click that to submit a Pull Request (PR). In your PR description, clearly explain the changes you made and why they are necessary.

We will review your pull request and provide feedback. Once approved, your contribution will be merged into the main repository!

## Contribution Guidelines

- Ensure that all existing tests pass.
- Write clear commit messages.
- Add tests for any new functionality.
- Follow the existing coding style (enforced by pre-commit hooks).
- Update documentation as needed.
- Pre-commit hooks must pass before committing.

To get in touch with us, please join our [discord server](https://discord.gg/pGZf2gcwEc). We will be monitoring that actively and offering support there.
