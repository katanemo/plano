.. -*- coding: utf-8 -*-

========
Signals™
========

Agentic Signals are lightweight, model-free behavioral indicators computed from
live interaction trajectories and attached to your existing
OpenTelemetry traces. They make it possible to triage the small fraction of
trajectories that are most likely to be informative — brilliant successes or
**severe failures** — without running an LLM-as-judge on every session.

The framework implemented here follows the taxonomy and detector design in
*Signals: Trajectory Sampling and Triage for Agentic Interactions* (Chen,
Hafeez, Paracha, 2026; `arXiv:2604.00356
<https://arxiv.org/abs/2604.00356>`_). All detectors are computed without
model calls; the entire pipeline attaches structured attributes and span
events to existing spans so your dashboards and alerts work unmodified.

The Problem: Knowing What's "Good"
==================================

One of the hardest parts of building agents is measuring how well they
perform in the real world.

**Offline testing** relies on hand-picked examples and happy-path scenarios,
missing the messy diversity of real usage. Developers manually prompt models,
evaluate responses, and tune prompts by guesswork — a slow, incomplete
feedback loop.

**Production debugging** floods developers with traces and logs but provides
little guidance on which interactions actually matter. Finding failures means
painstakingly reconstructing sessions and manually labeling quality issues.

You can't score every response with an LLM-as-judge (too expensive, too slow)
or manually review every trace (doesn't scale). What you need are
**behavioral signals** — fast, economical proxies that don't label quality
outright but dramatically shrink the search space, pointing to sessions most
likely to be broken or brilliant.

What Are Behavioral Signals?
============================

Behavioral signals are canaries in the coal mine — early, objective
indicators that something may have gone wrong (or gone exceptionally well).
They don't explain *why* an agent failed, but they reliably signal *where*
attention is needed.

These signals emerge naturally from the rhythm of interaction:

- A user rephrasing or correcting the same request
- Sharp increases in conversation length
- Negative stance markers ("this doesn't work", ALL CAPS, excessive !!! or ???)
- Agent repetition or tool-call loops
- Expressions of gratitude, confirmation, or task success
- Requests for a human agent or explicit quit intent
- Tool errors, timeouts, rate limits, and context-window exhaustion

Individually, these clues are shallow; together, they form a fingerprint of
agent performance. Embedded directly into traces, they make it easy to spot
friction as it happens: where users struggle, where agents loop, where tool
failures cluster, and where escalations occur.

Signals vs Response Quality
===========================

Behavioral signals and response quality are complementary.

**Response Quality**
    Domain-specific correctness: did the agent do the right thing given
    business rules, user intent, and operational context? This often
    requires subject-matter experts or outcome instrumentation and is
    time-intensive but irreplaceable.

**Behavioral Signals**
    Observable patterns that correlate with quality: misalignment,
    stagnation, disengagement, satisfaction, tool failures, loops, and
    environment exhaustion. Fast to compute and valuable for prioritizing
    which traces deserve inspection.

Used together, signals tell you *where to look*, and quality evaluation tells
you *what went wrong (or right)*.

Signal Taxonomy
===============

Signals are organized into three top-level **layers**, each with its own
intent. Every detected signal belongs to exactly one leaf type under one of
seven categories.

Interaction (user ↔ agent conversational quality)
-------------------------------------------------

Covers how the discourse itself is going: is the user being understood, is
the conversation progressing, is the user engaged, is the user satisfied?

.. list-table::
   :header-rows: 1
   :widths: 25 25 50

   * - Category
     - Leaf signal type
     - Meaning
   * - **Misalignment**
     - ``misalignment.correction``
     - User explicitly corrects the agent ("No, I meant Paris, France").
   * -
     - ``misalignment.rephrase``
     - User reformulates a previous request; semantic overlap is high.
   * -
     - ``misalignment.clarification``
     - User signals confusion ("I don't understand", "what do you mean").
   * - **Stagnation**
     - ``stagnation.dragging``
     - Conversation length significantly exceeds the expected baseline.
   * -
     - ``stagnation.repetition``
     - Assistant near-duplicates prior turns (bigram Jaccard similarity).
   * - **Disengagement**
     - ``disengagement.escalation``
     - User asks to speak to a human / supervisor / support.
   * -
     - ``disengagement.quit``
     - User expresses intent to give up or abandon the session.
   * -
     - ``disengagement.negative_stance``
     - User expresses frustration: complaints, ALL CAPS, excessive
       punctuation, agent-directed profanity.
   * - **Satisfaction**
     - ``satisfaction.gratitude``
     - User expresses thanks or appreciation.
   * -
     - ``satisfaction.confirmation``
     - User confirms the outcome ("got it", "sounds good").
   * -
     - ``satisfaction.success``
     - User confirms task success ("that worked", "perfect").

Execution (agent-caused action quality)
---------------------------------------

Covers attempts to act in the world that don't yield usable outcomes.
Requires tool-call traces (``function_call`` / ``observation``) to fire.

.. list-table::
   :header-rows: 1
   :widths: 25 25 50

   * - Category
     - Leaf signal type
     - Meaning
   * - **Failure**
     - ``failure.invalid_args``
     - Tool call rejected due to schema / argument validation failure.
   * -
     - ``failure.bad_query``
     - Downstream query rejected as malformed by the tool.
   * -
     - ``failure.tool_not_found``
     - Agent called a tool that doesn't exist or isn't available.
   * -
     - ``failure.auth_misuse``
     - Authentication / authorization failure on a tool call.
   * -
     - ``failure.state_error``
     - Call-order / state-machine violation (e.g. commit without begin).
   * - **Loops**
     - ``loops.retry``
     - Same tool call repeated with near-identical arguments.
   * -
     - ``loops.parameter_drift``
     - Same tool called with slowly drifting parameters (walk pattern).
   * -
     - ``loops.oscillation``
     - Call A → Call B → Call A → Call B pattern across multiple turns.

Environment (external system / boundary conditions)
---------------------------------------------------

Covers failures **outside** the agent's control that still break the
interaction. Useful for separating agent-caused issues from infrastructure.

.. list-table::
   :header-rows: 1
   :widths: 25 25 50

   * - Category
     - Leaf signal type
     - Meaning
   * - **Exhaustion**
     - ``exhaustion.api_error``
     - Downstream API returned a 5xx or unexpected error.
   * -
     - ``exhaustion.timeout``
     - Tool / API call timed out.
   * -
     - ``exhaustion.rate_limit``
     - Rate-limit response from a tool / API.
   * -
     - ``exhaustion.network``
     - Transient network failure mid-call.
   * -
     - ``exhaustion.malformed_response``
     - Response received but couldn't be parsed.
   * -
     - ``exhaustion.context_overflow``
     - Context window / token budget exceeded.

How It Works
============

Signals are computed automatically by the gateway after each assistant
response and emitted as **OpenTelemetry trace attributes** and **span events**
on your existing spans. No additional libraries or instrumentation are
required — just configure your OTEL collector endpoint as usual.

Each conversation trace is enriched with layered signal attributes
(category-level counts and severities) plus one span event per detected
signal instance (with confidence, snippet, and per-detector metadata).

.. note::
   Signal analysis is enabled by default and runs on the request path. It
   does **not** affect the response sent to the client. Set
   ``overrides.disable_signals: true`` in your Plano config to skip this
   CPU-heavy analysis (see the configuration reference).

OTel Span Attributes
====================

Signal data is exported as structured OTel attributes. There are two tiers:
**top-level** attributes (always emitted on spans that carry signal
analysis) and **layered** attributes (emitted only when the corresponding
category has at least one signal instance).

Top-level attributes
--------------------

Always emitted once signals are computed.

.. list-table::
   :header-rows: 1
   :widths: 40 15 45

   * - Attribute
     - Type
     - Value
   * - ``signals.quality``
     - string
     - One of ``excellent``, ``good``, ``neutral``, ``poor``, ``severe``.
   * - ``signals.quality_score``
     - float
     - Numeric score 0.0 – 100.0 that feeds the quality bucket.
   * - ``signals.turn_count``
     - int
     - Total number of user + assistant turns in the interaction.
   * - ``signals.efficiency_score``
     - float
     - Efficiency metric 0.0 – 1.0 (stays at 1.0 up to baseline turns,
       then decays: ``1 / (1 + 0.3 * (turns - baseline))``).

Layered attributes
------------------

Emitted per category, only when ``count > 0``. One ``.count`` and one
``.severity`` attribute per category. Severity is a 0–3 bucket (see
`Severity levels`_ below).

.. list-table::
   :header-rows: 1
   :widths: 50 50

   * - Attribute (emitted when fired)
     - Source
   * - ``signals.interaction.misalignment.count``
     - Any ``misalignment.*`` leaf type
   * - ``signals.interaction.misalignment.severity``
     - "
   * - ``signals.interaction.stagnation.count``
     - Any ``stagnation.*`` leaf type
   * - ``signals.interaction.stagnation.severity``
     - "
   * - ``signals.interaction.disengagement.count``
     - Any ``disengagement.*`` leaf type
   * - ``signals.interaction.disengagement.severity``
     - "
   * - ``signals.interaction.satisfaction.count``
     - Any ``satisfaction.*`` leaf type
   * - ``signals.interaction.satisfaction.severity``
     - "
   * - ``signals.execution.failure.count``
     - Any ``failure.*`` leaf type
   * - ``signals.execution.failure.severity``
     - "
   * - ``signals.execution.loops.count``
     - Any ``loops.*`` leaf type
   * - ``signals.execution.loops.severity``
     - "
   * - ``signals.environment.exhaustion.count``
     - Any ``exhaustion.*`` leaf type
   * - ``signals.environment.exhaustion.severity``
     - "

Legacy attributes (deprecated, still emitted)
---------------------------------------------

The following aggregate keys pre-date the paper taxonomy and are still
emitted for one release so existing dashboards keep working. They are
derived from the layered counts above and will be removed in a future
release. Migrate to the layered keys when convenient.

.. list-table::
   :header-rows: 1
   :widths: 50 50

   * - Legacy attribute
     - Layered equivalent
   * - ``signals.follow_up.repair.count``
     - ``signals.interaction.misalignment.count``
   * - ``signals.follow_up.repair.ratio``
     - (computed: ``misalignment.count / max(user_turns, 1)``)
   * - ``signals.frustration.count``
     - Count of ``disengagement.negative_stance`` instances
   * - ``signals.frustration.severity``
     - Derived severity bucket of the above
   * - ``signals.repetition.count``
     - ``signals.interaction.stagnation.count``
   * - ``signals.escalation.requested``
     - True if any ``disengagement.escalation`` or ``disengagement.quit`` fired
   * - ``signals.positive_feedback.count``
     - ``signals.interaction.satisfaction.count``

Span Events
===========

In addition to span attributes, every detected signal instance is emitted as
a span event named ``signal.<dotted-type>`` (e.g.
``signal.interaction.satisfaction.gratitude``). Each event carries:

.. list-table::
   :header-rows: 1
   :widths: 30 15 55

   * - Event attribute
     - Type
     - Description
   * - ``signal.type``
     - string
     - Full dotted signal type (same as the event name suffix).
   * - ``signal.message_index``
     - int
     - Zero-based index of the message that triggered the signal.
   * - ``signal.confidence``
     - float
     - Detector confidence in [0.0, 1.0].
   * - ``signal.snippet``
     - string
     - Matched substring from the source message (when available).
   * - ``signal.metadata``
     - string (JSON)
     - Per-detector metadata (pattern name, ratio values, etc.).

Span events are the right surface for drill-down: attribute filters narrow
traces, then events tell you *which messages* fired *which signals* with
*what evidence*.

Visual Flag Marker
------------------

When concerning signals are detected (disengagement present, stagnation
count > 2, any execution failure / loop, or overall quality ``poor``/
``severe``), the marker ``[!]`` is appended to the span's operation name.
This makes flagged sessions immediately visible in trace UIs without
requiring attribute filtering.

Querying in Your Observability Platform
---------------------------------------

Example queries against the layered keys::

    signals.quality = "severe"
    signals.turn_count > 10
    signals.efficiency_score < 0.5
    signals.interaction.disengagement.severity >= 2
    signals.interaction.misalignment.count > 3
    signals.interaction.satisfaction.count > 0 AND signals.quality = "good"
    signals.execution.failure.count > 0
    signals.environment.exhaustion.count > 0

For flagged sessions, search for ``[!]`` in span names.

.. image:: /_static/img/signals_trace.png
   :width: 100%
   :align: center

Severity Levels
===============

Every category aggregates its leaf signal counts into a severity bucket used
by both the layered ``.severity`` attribute and the overall quality score.

- **None (0)**: 0 instances
- **Mild (1)**: 1–2 instances
- **Moderate (2)**: 3–4 instances
- **Severe (3)**: 5+ instances

Severity is always computed per-category. For example, three instances of
``misalignment.rephrase`` plus two of ``misalignment.correction`` yield
``signals.interaction.misalignment.severity = 3`` (5 instances total).

Overall Quality Assessment
==========================

Signals are aggregated into an overall interaction quality on a 5-point
scale. The scoring model starts at 50.0 (neutral), adds positive weight for
satisfaction, and subtracts weight for disengagement, misalignment (when
ratio > 30% of user turns), stagnation (when count > 2), execution failures,
execution loops, and environment exhaustion.

The resulting numeric score maps to the bucket emitted in ``signals.quality``:

**Excellent (75 – 100)**
    Strong positive signals, efficient resolution, low friction.

**Good (60 – 74)**
    Mostly positive with minor clarifications; some back-and-forth but
    successful.

**Neutral (40 – 59)**
    Mixed signals; neither clearly good nor bad.

**Poor (25 – 39)**
    Concerning negative patterns (high friction, multiple misalignments,
    moderate disengagement, tool failures). High abandonment risk.

**Severe (0 – 24)**
    Critical issues — escalation requested, severe disengagement, severe
    stagnation, or compounding failures. Requires immediate attention.

The raw numeric score is available under ``signals.quality_score``.

Sampling and Prioritization
===========================

In production, trace data is overwhelming. Signals provide a lightweight
first layer of triage to select the small fraction of trajectories that are
most likely to be informative. Per the paper, signal-based sampling reaches
82% informativeness on τ-bench versus 54% for random sampling — a 1.52×
efficiency gain per informative trajectory.

Workflow:

1. Gateway captures conversation messages and computes signals
2. Signal attributes and per-instance events are emitted to OTEL spans
3. Your observability platform ingests and indexes the attributes
4. Query / filter by signal attributes to surface outliers and exemplars
5. Review high-information traces to identify improvement opportunities
6. Update prompts, routing, or policies based on findings
7. Redeploy and monitor signal metrics to validate improvements

This creates a reinforcement loop where traces become both diagnostic data
and training signal for prompt engineering, routing policies, and
preference-data construction.

.. note::
   An in-gateway triage sampler that selects informative trajectories
   inline — with configurable per-category weights and budgets — is planned
   as a follow-up to this release. Today, sampling is consumer-side: your
   observability platform filters on the signal attributes described above.

Example Span
============

A concerning session, showing both layered attributes and a per-instance
event::

    # Span name: "POST /v1/chat/completions gpt-5.2 [!]"

    # Top-level
    signals.quality            = "severe"
    signals.quality_score      = 0.0
    signals.turn_count         = 4
    signals.efficiency_score   = 1.0

    # Layered (only non-zero categories are emitted)
    signals.interaction.disengagement.count    = 6
    signals.interaction.disengagement.severity = 3

    # Legacy (deprecated, emitted while dual-emit is on)
    signals.frustration.count     = 4
    signals.frustration.severity  = 2
    signals.escalation.requested  = true

    # Per-instance span events
    event: signal.interaction.disengagement.escalation
      signal.type          = "interaction.disengagement.escalation"
      signal.message_index = 6
      signal.confidence    = 1.0
      signal.snippet       = "get me a human"
      signal.metadata      = {"pattern_type":"escalation"}

Building Dashboards
===================

Use signal attributes to build monitoring dashboards in Grafana, Honeycomb,
Datadog, etc. Prefer the layered keys — they align with the paper taxonomy
and will outlive the legacy keys.

- **Quality distribution**: Count of traces by ``signals.quality``
- **P95 turn count**: 95th percentile of ``signals.turn_count``
- **Average efficiency**: Mean of ``signals.efficiency_score``
- **High misalignment rate**: Percentage where
  ``signals.interaction.misalignment.count > 3``
- **Disengagement rate**: Percentage where
  ``signals.interaction.disengagement.severity >= 2``
- **Satisfaction rate**: Percentage where
  ``signals.interaction.satisfaction.count >= 1``
- **Escalation rate**: Percentage where a ``disengagement.escalation`` or
  ``disengagement.quit`` event fired (via span-event filter)
- **Tool-failure rate**: Percentage where
  ``signals.execution.failure.count > 0``
- **Environment issue rate**: Percentage where
  ``signals.environment.exhaustion.count > 0``

Creating Alerts
===============

Set up alerts based on signal thresholds:

- Alert when ``signals.quality = "severe"`` count exceeds threshold in a
  1-hour window
- Alert on sudden spike in
  ``signals.interaction.disengagement.severity >= 2`` (>2× baseline)
- Alert on sustained ``signals.execution.failure.count > 0`` — agent-caused
  tool issues
- Alert on spikes in ``signals.environment.exhaustion.count`` — external
  system degradation
- Alert on degraded efficiency (P95 ``signals.turn_count`` up > 50%)

Best Practices
==============

Start simple:

- Alert or page on ``severe`` sessions (or on spikes in ``severe`` rate)
- Review ``poor`` sessions within 24 hours
- Sample ``excellent`` sessions as exemplars

Combine multiple signals to infer failure modes:

- **Silent loop**: ``signals.interaction.stagnation.severity >= 2`` +
  ``signals.turn_count`` above baseline
- **User giving up**: ``signals.interaction.disengagement.severity >= 2`` +
  any escalation event
- **Misunderstood intent**:
  ``signals.interaction.misalignment.count / user_turns > 0.3``
- **Agent-caused friction**: ``signals.execution.failure.count > 0`` +
  ``signals.interaction.misalignment.count > 0``
- **External degradation, not agent fault**:
  ``signals.environment.exhaustion.count > 0`` while
  ``signals.execution.failure.count = 0``
- **Working well**: ``signals.interaction.satisfaction.count >= 1`` +
  ``signals.efficiency_score > 0.8`` + no disengagement

Limitations and Considerations
==============================

Signals don't capture:

- Task completion / real outcomes
- Factual or domain correctness
- Silent abandonment (user leaves without expressing frustration)
- Non-English nuance (pattern libraries are English-oriented)

Mitigation strategies:

- Periodically sample flagged sessions and measure false positives / negatives
- Tune baselines per use case and user population
- Add domain-specific phrase libraries where needed
- Combine signals with non-text metrics (tool failures, disconnects, latency)

.. note::
   Behavioral signals complement — but do not replace — domain-specific
   response quality evaluation. Use signals to prioritize which traces to
   inspect, then apply domain expertise and outcome checks to diagnose root
   causes.

.. tip::
   The ``[!]`` marker in the span name provides instant visual feedback in
   trace UIs, while the structured attributes (``signals.quality``,
   ``signals.interaction.disengagement.severity``, etc.) and per-instance
   span events enable powerful querying and drill-down in your observability
   platform.

See Also
========

- `Signals: Trajectory Sampling and Triage for Agentic Interactions
  <https://arxiv.org/abs/2604.00356>`_ — the paper this framework implements
- :doc:`../guides/observability/tracing` — Distributed tracing for agent
  systems
- :doc:`../guides/observability/monitoring` — Metrics and dashboards
- :doc:`../guides/observability/access_logging` — Request / response logging
- :doc:`../guides/observability/observability` — Complete observability guide
