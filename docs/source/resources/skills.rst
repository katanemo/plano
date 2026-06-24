.. _agent_skills:

Agent Skills
============

Plano can load `Agent Skills <https://agentskills.io>`_ — lightweight,
markdown-defined capabilities — and let Plano-Orchestrator decide *per request*
which skills to attach to the downstream LLM call. Skills attach to entries in
``routing_preferences``: when the orchestrator picks a route, it also picks
zero or more skills from that route's allow-list, and brightstaff injects
each selected ``SKILL.md`` body into the upstream system prompt before
forwarding the request.

Why use this?
-------------

- **Modular instructions.** Ship a skill (markdown + scripts + assets) rather
  than baking 500-token instructions into every system prompt.
- **Progressive disclosure.** Skill names and one-line descriptions are
  always visible to the orchestrator; full instructions load only when a
  skill is activated.
- **Per-route scoping.** A ``skills:`` list on a ``routing_preferences``
  entry constrains which skills can be activated for that route.

Install a skill
---------------

.. code-block:: bash

   # via the upstream Agent Skills CLI (recommended for multi-skill repos)
   planoai skills add openai/skills

   # planoai falls back to a direct git clone if `npx` is unavailable; this
   # path expects a single-skill repo with a SKILL.md at the root.
   planoai skills add owner/code-review

Where do skills end up?
~~~~~~~~~~~~~~~~~~~~~~~

Plano looks for skills across three scopes (highest precedence first):

============== ============================== =========================== =====================================
Scope          Location                       Trust required              Typical installer
============== ============================== =========================== =====================================
``project``    ``.plano/skills/<name>/``      Yes — ``planoai skills      ``planoai skills add`` (git fallback)
                                              trust``
``user``       ``~/.plano/skills/<name>/``    No (auto-trusted)           manual
``agents``     ``~/.agents/skills/<name>/``   No (auto-trusted)           ``npx skills add`` / upstream CLI
============== ============================== =========================== =====================================

The ``agents`` scope is the universal Agent Skills install location used by
``npx skills add`` (see https://github.com/vercel-labs/add-skill). Because
``npx skills add`` doesn't know about Plano, it never writes into
``.plano/skills/``; instead it drops the skill under ``~/.agents/skills/<name>``
and symlinks it into every recognised agent (Claude Code, Cursor, …). Plano
treats that directory as an auto-trusted user-tier scope, so anything
installed there is picked up automatically — no ``planoai skills trust``
needed.

A ``.plano/skills/.skills.json`` manifest is maintained only for installs
that land in project scope (the git fallback). The ``agents`` scope owns
its own bookkeeping in ``~/.agents/``.

Trust the project
-----------------

Project-level skills are loaded only after you mark the project trusted. This
matches the recommendation in the `Adding skills support guide
<https://agentskills.io/client-implementation/adding-skills-support.md>`_:

.. code-block:: bash

   planoai skills trust

   # revoke trust later if needed
   planoai skills trust --revoke

Skills under ``~/.plano/skills/`` and ``~/.agents/skills/`` are always
trusted and ignore this setting.

Discover and remove
-------------------

.. code-block:: bash

   planoai skills list
   planoai skills remove pdf-processing

Configure routing
-----------------

Reference installed skills from your ``config.yaml`` in two places:

1. The top-level ``skills:`` catalog (optional — omit to auto-include every
   discovered skill).
2. Each ``routing_preferences`` entry that should make a skill eligible for
   activation. The orchestrator's ``<skills>`` block is built from the union
   of every ``routing_preferences[].skills`` list; skills not referenced by
   any route are silently dropped.

.. code-block:: yaml

   skills:
     - pdf-processing
     - code-review

   routing_preferences:
     - name: code review
       description: |
         Reviewing pull requests, analyzing diffs, and suggesting improvements
         to existing code.
       models:
         - anthropic/claude-sonnet-4-5
         - openai/gpt-4.1-2025-04-14
       skills:
         - code-review

     - name: document understanding
       description: |
         Summarizing PDFs and other long-form documents, extracting structured
         data such as tables, line items, or signatures.
       models:
         - anthropic/claude-sonnet-4-5
       skills:
         - pdf-processing
       selection_policy:
         prefer: cheapest

When ``planoai up`` runs, the CLI walks ``.plano/skills/`` and
``~/.plano/skills/``, parses each ``SKILL.md``, and inlines the markdown body
into the rendered Plano config so the brightstaff orchestrator can attach it
to the request without any filesystem access.

How routing works
-----------------

At request time:

1. The brightstaff routing service builds an ``<skills>`` block in the
   Plano-Orchestrator prompt — alongside the existing ``<routes>`` block —
   listing every skill referenced by ``routing_preferences[].skills`` with
   its name and short description.
2. The orchestrator replies with JSON of the form
   ``{"route": ["..."], "skills": ["..."]}``.
3. brightstaff resolves each selected skill name against the chosen route's
   ``skills:`` allow-list. Names that aren't allowed for that route (or are
   not in the catalog) are dropped.
4. The activated ``SkillRef`` bodies are prepended to the upstream request's
   system prompt — wrapped in
   ``<skill_content name="..." base_dir="...">…</skill_content>`` tags — and
   the request is forwarded to the chosen model.

If the orchestrator picks only skills and no route, the request falls back
to the originally-requested model (or the default) and the skill bodies are
injected the same way.

Bootstrap from the template
---------------------------

A ready-made template wires the moving pieces together:

.. code-block:: bash

   planoai init --template skills_routing

Out of scope
------------

- Hot-reload of ``.plano/skills/`` while Plano is running — re-run
  ``planoai up`` to pick up new skills.
- Server-side execution of bundled ``scripts/`` from skills. The upstream
  client runs scripts as part of the progressive-disclosure model from the
  `specification <https://agentskills.io/specification.md>`_.
- Subagent delegation per skill. See the
  `client-implementation guide
  <https://agentskills.io/client-implementation/adding-skills-support.md>`_
  for the advanced pattern.
