.. _local-agent-providers:

Local-Agent Providers
=====================

Plano draws a hard line between two very different kinds of "providers"
that can sit behind a ``model_providers`` entry:

1. **Network LLM providers** — ``openai``, ``anthropic``, ``gemini``,
   ``vercel``, ``openrouter``, ``mistral``, ``groq``, ``digitalocean``,
   ``together_ai``, etc. These are stateless HTTPS APIs. The trust
   boundary is the network call: Plano forwards the request to the
   provider's server, the provider does whatever it does, and the
   response comes back. The host never executes provider code.

2. **Local-agent providers** — currently ``claude-cli`` (and, by design,
   any future ``codex-cli`` / ``chatgpt-cli`` / ``opencode`` /
   ``hermes`` integration). These are not LLMs; they are *agent
   integrations*. Plano implements them as a localhost bridge inside
   ``brightstaff`` that **spawns a local CLI binary as a subprocess**
   for every request and pipes the conversation through it.

These two classes of provider have fundamentally different security
properties, and conflating them in production is the kind of mistake
that turns into a postmortem. This page exists so the boundary is
explicit.

Why ``planoai up`` warns about them
-----------------------------------

When ``planoai up`` loads a config that contains a local-agent provider
(matched on ``provider_interface`` or on a ``<interface>/...`` prefix in
``model:``/``name:``), it prints a single warning panel listing the
triggering entries and refusing to proceed silently until the operator
acknowledges. This is intentional. The warning fires exactly once per
``planoai up`` run, regardless of how many local-agent entries the
config has.

Trust model
-----------

Spawning a local CLI binary as the operator's user is a very different
thing from making an HTTPS call. The subprocess inherits everything the
operator can do:

.. list-table::
   :header-rows: 1
   :widths: 30 35 35

   * - Capability
     - Network LLM provider
     - Local-agent provider
   * - Filesystem read
     - No
     - **Yes** — anything ``$USER`` can read
   * - Filesystem write
     - No
     - **Yes** — anything ``$USER`` can write
   * - Shell command execution
     - No
     - **Yes** — full shell as ``$USER``
   * - Auth / credentials
     - Per-provider API key
     - **Host login keychain** (no per-tenant isolation)
   * - Outbound network
     - To the provider only
     - **Anywhere the host can reach**
   * - Reproducibility
     - Deterministic given inputs
     - Depends on local FS, env, CWD, installed tools
   * - Suitable for production
     - Yes
     - **No — local development only**

Concretely, when a request hits a ``claude-cli/*`` model, brightstaff
runs (roughly):

.. code-block:: bash

   claude -p --output-format stream-json --input-format stream-json \
       --permission-mode bypassPermissions ...

Whatever Claude Code decides to do with the working directory, the
shell, ``rm``, ``git``, your SSH keys, your ``~/.aws/credentials``, your
production database connection strings — all of that is reachable. This
is the *correct* trust model for a single-developer workstation; it is
the *wrong* trust model for anything multi-tenant.

Local-agent providers are in the same category as standalone agent
runtimes like `OpenClaw`_, `OpenCode`_, and `Hermes`_: they are agent
integrations that happen to expose an LLM-shaped HTTP API, not
LLM providers that happen to run locally.

.. _OpenClaw: https://github.com/openclaw/openclaw
.. _OpenCode: https://github.com/sst/opencode
.. _Hermes: https://github.com/HermesAI/hermes

Recommended setup
-----------------

If you are using a local-agent provider, treat it like any other
developer-machine agent runtime:

- **Bind to loopback only.** Do not expose the bridge or the Plano
  listener to a network interface. ``127.0.0.1`` only.
- **Single-developer use.** One operator, one host. Do not put a
  load balancer in front of it. Do not share the deployment.
- **Opt-in.** Don't add a local-agent provider to a config that other
  people deploy. Keep it in a config file that's clearly scoped to one
  workstation.
- **Don't run as root** and don't run inside a container that mounts
  more of the host filesystem than necessary. The subprocess inherits
  the launching process's capabilities verbatim.
- **Audit the spawned binary** the same way you would audit anything
  with ``sudo`` access. If the operator's ``claude`` (or future
  ``codex``) binary is compromised, so is the host.

Dismissing the warning
----------------------

The warning is dismissable per-host. The recommended path is the CLI
flag:

.. code-block:: bash

   planoai up --ack-local-agents

That writes an ack file at ``~/.plano/state/local_agent_ack.json``
containing every triggering provider interface and the timestamp. On
subsequent ``planoai up`` runs, the warning is suppressed silently as
long as the ack covers every local-agent interface in the config.

If you prefer an environment variable (e.g. inside a personal
``direnv`` setup), set ``PLANO_ACK_LOCAL_AGENTS=1`` instead. Truthy
values are ``1``, ``true``, ``yes``, ``on`` (case-insensitive). Setting
the env var has the same effect as passing the flag — it writes the
ack file.

If a *new* local-agent interface appears later (e.g. you add a
hypothetical ``codex-cli/*`` after acknowledging ``claude-cli/*``), the
warning re-fires for the un-acked interface only.

Undoing the dismissal
~~~~~~~~~~~~~~~~~~~~~

To undo the dismissal — for example, when handing the host to another
developer or running through a security review — simply remove the
file:

.. code-block:: bash

   rm ~/.plano/state/local_agent_ack.json

The next ``planoai up`` run will print the full warning panel again.

Adding a new local-agent provider type
--------------------------------------

The set of local-agent provider interfaces lives in
``cli/planoai/local_agent_warning.py`` as
``LOCAL_AGENT_PROVIDER_INTERFACES``. Adding a new entry — say, a future
``codex-cli`` bridge that spawns the OpenAI Codex CLI — is a one-line
change:

.. code-block:: python

   LOCAL_AGENT_PROVIDER_INTERFACES = ("claude-cli", "codex-cli")

Detection automatically covers ``provider_interface: codex-cli`` as
well as ``model: codex-cli/...`` and ``name: codex-cli/...``, so users
who rely on the Python-side autofill for short-form configs are still
warned.

.. note::

   At the time of writing, the only network ``provider_interface`` that
   shares any naming with a local agent runtime is ``chatgpt`` — but
   that is a stateless HTTPS provider against
   ``https://chatgpt.com/backend-api/codex``, **not** a local CLI
   bridge. It is correctly excluded from
   ``LOCAL_AGENT_PROVIDER_INTERFACES``. The ``codex`` value accepted by
   ``planoai cli_agent codex`` is a *client* helper that points the
   Codex CLI at a running Plano listener; it does not introduce a
   provider into the config.
