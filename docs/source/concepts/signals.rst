.. -*- coding: utf-8 -*-

========
Signals™
========

Agentic Signals are behavioral and executions quality indicators that act as early warning signs of agent performance—highlighting both brilliant successes and **severe failures**. These signals are computed directly from conversation traces without requiring manual labeling or domain expertise, making them practical for production observability at scale.

The Problem: Knowing What's "Good"
==================================

One of the hardest parts of building agents is measuring how well they perform in the real world.

**Offline testing** relies on hand-picked examples and happy-path scenarios, missing the messy diversity of real usage. Developers manually prompt models, evaluate responses, and tune prompts by guesswork—a slow, incomplete feedback loop.

**Production debugging** floods developers with traces and logs but provides little guidance on which interactions actually matter. Finding failures means painstakingly reconstructing sessions and manually labeling quality issues.

You can't score every response with an LLM-as-judge (too expensive, too slow) or manually review every trace (doesn't scale). What you need are **behavioral signals**—fast, economical proxies that don’t label quality outright but dramatically shrink the search space, pointing to sessions most likely to be broken or brilliant.

What Are Behavioral Signals?
============================

Behavioral signals are canaries in the coal mine—early, objective indicators that something may have gone wrong (or gone exceptionally well). They don’t explain *why* an agent failed, but they reliably signal *where* attention is needed.

These signals emerge naturally from the rhythm of interaction:

- A user rephrasing the same request
- Sharp increases in conversation length
- Frustrated follow-up messages (ALL CAPS, "this doesn’t work", excessive !!!/???)
- Agent repetition / looping
- Expressions of gratitude or satisfaction
- Requests to speak to a human / contact support

Individually, these clues are shallow; together, they form a fingerprint of agent performance. Embedded directly into traces, they make it easy to spot friction as it happens: where users struggle, where agents loop, and where escalations occur.

Signals vs Response Quality
===========================

Behavioral signals and response quality are complementary.

**Response Quality**
    Domain-specific correctness: did the agent do the right thing given business rules, user intent, and operational context? This often requires subject-matter experts or outcome instrumentation and is time-intensive but irreplaceable.

**Behavioral Signals**
    Observable patterns that correlate with quality: high repair frequency, excessive turns, frustration markers, repetition, escalation, and positive feedback. Fast to compute and valuable for prioritizing which traces deserve inspection.

Used together, signals tell you *where to look*, and quality evaluation tells you *what went wrong (or right)*.

How It Works
============

Signals are computed automatically by the gateway and emitted as **OpenTelemetry trace attributes** to your existing observability stack (Jaeger, Honeycomb, Grafana Tempo, etc.). No additional libraries or instrumentation required—just configure your OTEL collector endpoint.

Each conversation trace is enriched with signal attributes that you can query, filter, and visualize in your observability platform. The gateway analyzes message content (performing text normalization, Unicode handling, and pattern matching) to compute behavioral signals in real-time.

**OTEL Trace Attributes**

Signal data is exported as structured span attributes:

- ``signals.quality`` - Overall assessment (Excellent/Good/Neutral/Poor/Severe)
- ``signals.turn_count`` - Total number of turns in the conversation
- ``signals.efficiency_score`` - Efficiency metric (0.0-1.0)
- ``signals.repair.count`` - Number of repair attempts detected (when present)
- ``signals.repair.ratio`` - Ratio of repairs to user turns (when present)
- ``signals.frustration.count`` - Number of frustration indicators detected
- ``signals.frustration.severity`` - Frustration level (0-3)
- ``signals.repetition.count`` - Number of repetition instances detected
- ``signals.escalation.requested`` - Boolean escalation flag ("true" when present)
- ``signals.positive_feedback.count`` - Number of positive feedback indicators

**Visual Flag Marker**

When concerning signals are detected (frustration, looping, escalation, or poor/severe quality), the flag marker **🚩** is automatically appended to the span's operation name, making problematic traces easy to spot in your trace visualizations.

**Querying in Your Observability Platform**

Example queries:

- Find all severe interactions: ``signals.quality = "Severe"``
- Find flagged traces: search for **🚩** in span names
- Find long conversations: ``signals.turn_count > 10``
- Find inefficient interactions: ``signals.efficiency_score < 0.5``
- Find high repair rates: ``signals.repair.ratio > 0.3``
- Find frustrated users: ``signals.frustration.severity >= 2``
- Find looping agents: ``signals.repetition.count >= 3``
- Find positive interactions: ``signals.positive_feedback.count >= 2``
- Find escalations: ``signals.escalation.requested = "true"``

.. image:: /_static/img/signals_trace.png
   :width: 100%
   :align: center


Core Signal Types
=================

The signals system tracks six categories of behavioral indicators.

Turn Count & Efficiency
-----------------------

**What it measures**
    Number of user–assistant exchanges.

**Why it matters**
    Long conversations often indicate unclear intent resolution, confusion, or inefficiency. Very short conversations can correlate with crisp resolution.

**Key metrics**

- Total turn count
- Warning thresholds (concerning: >7 turns, excessive: >12 turns)
- Efficiency score (0.0–1.0)

**Efficiency scoring**
    Baseline expectation is ~5 turns (tunable). Efficiency stays at 1.0 up to the baseline, then declines with an inverse penalty as turns exceed baseline::

        efficiency = 1 / (1 + 0.3 * (turns - baseline))

Follow-Up & Repair Frequency
----------------------------

**What it measures**
    How often users clarify, correct, or rephrase requests. This is a **user signal** tracking query reformulation behavior—when users must repair or rephrase their requests because the agent didn't understand or respond appropriately.

**Why it matters**
    High repair frequency is a proxy for misunderstanding or intent drift. When users repeatedly rephrase the same request, it indicates the agent is failing to grasp or act on the user's intent.

**Key metrics**

- Repair count and ratio (repairs / user turns)
- Concerning threshold: >30% repair ratio
- Detected repair phrases (exact or fuzzy)

**Common patterns detected**

- Explicit corrections: "I meant", "correction"
- Negations: "No, I...", "that's not"
- Rephrasing: "let me rephrase", "to clarify"
- Mistake acknowledgment: "my mistake", "I was wrong"
- "Similar rephrase" heuristic based on token overlap (with stopwords downweighted)

User Frustration
----------------

**What it measures**
    Observable frustration indicators and emotional escalation.

**Why it matters**
    Catching frustration early enables intervention before users abandon or escalate.

**Detection patterns**

- **Complaints**: "this doesn't work", "not helpful", "waste of time"
- **Confusion**: "I don't understand", "makes no sense", "I'm confused"
- **Tone markers**:

  - ALL CAPS (>=10 alphabetic chars and >=80% uppercase)
  - Excessive punctuation (>=3 exclamation marks or >=3 question marks)

- **Profanity**: token-based (avoids substring false positives like "absolute" -> "bs")

**Severity levels**

- **None (0)**: no indicators
- **Mild (1)**: 1–2 indicators
- **Moderate (2)**: 3–4 indicators
- **Severe (3)**: 5+ indicators

Repetition & Looping
--------------------

**What it measures**
    Assistant repetition / degenerative loops. This is an **assistant signal** tracking when the agent repeats itself, fails to follow instructions, or gets stuck in loops—indicating the agent is not making progress or adapting its responses.

**Why it matters**
    Often indicates missing state tracking, broken tool integration, prompt issues, or the agent ignoring user corrections. High repetition means the agent is not learning from the conversation context.

**Detection method**

- Compare assistant messages using **bigram Jaccard similarity**
- Classify:

  - **Exact**: similarity >= 0.85
  - **Near-duplicate**: similarity >= 0.50

- Looping is flagged when repetition instances exceed 2 in a session.

**Severity levels**

- **None (0)**: 0 instances
- **Mild (1)**: 1–2 instances
- **Moderate (2)**: 3–4 instances
- **Severe (3)**: 5+ instances

Positive Feedback
-----------------

**What it measures**
    User expressions of satisfaction, gratitude, and success.

**Why it matters**
    Strong positive signals identify exemplar traces for prompt engineering and evaluation.

**Detection patterns**

- Gratitude: "thank you", "appreciate it"
- Satisfaction: "that's great", "awesome", "love it"
- Success confirmation: "got it", "that worked", "perfect"

**Confidence scoring**

- 1 indicator: 0.6
- 2 indicators: 0.8
- 3+ indicators: 0.95

Escalation Requests
-------------------

**What it measures**
    Requests for human help/support or threats to quit.

**Why it matters**
    Escalation is a strong signal that the agent failed to resolve the interaction.

**Detection patterns**

- Human requests: "speak to a human", "real person", "live agent"
- Support: "contact support", "customer service", "help desk"
- Quit threats: "I'm done", "forget it", "I give up"

Overall Quality Assessment
==========================

Signals are aggregated into an overall interaction quality on a 5-point scale.

**Excellent**
    Strong positive signals, efficient resolution, low friction.

**Good**
    Mostly positive with minor clarifications; some back-and-forth but successful.

**Neutral**
    Mixed signals; neither clearly good nor bad.

**Poor**
    Concerning negative patterns (high friction, multiple repairs, moderate frustration). High abandonment risk.

**Severe**
    Critical issues—escalation requested, severe frustration, severe looping, or excessive turns (>12). Requires immediate attention.

This assessment uses a scoring model that weighs positive factors (efficiency, positive feedback) against negative ones (frustration, repairs, repetition, escalation).

Sampling and Prioritization
===========================

In production, trace data is overwhelming. Signals provide a lightweight first layer of analysis to prioritize which sessions deserve review.

Workflow:

1. Gateway captures conversation messages and computes signals
2. Signal attributes are emitted to OTEL spans automatically
3. Your observability platform ingests and indexes the attributes
4. Query/filter by signal attributes to surface outliers (poor/severe and exemplars)
5. Review high-information traces to identify improvement opportunities
6. Update prompts, routing, or policies based on findings
7. Redeploy and monitor signal metrics to validate improvements

This creates a reinforcement loop where traces become both diagnostic data and training signal.

Trace Filtering and Telemetry
=============================

Signal attributes are automatically added to OpenTelemetry spans, making them immediately queryable in your observability platform.

**Visual Filtering**

When concerning signals are detected, the flag marker **🚩** (U+1F6A9) is automatically appended to the span's operation name. This makes flagged sessions immediately visible in trace visualizations without requiring attribute filtering.

**Example Span Attributes**::

    # Span name: "POST /v1/chat/completions gpt-4 🚩"
    signals.quality = "Severe"
    signals.turn_count = 15
    signals.efficiency_score = 0.234
    signals.repair.count = 4
    signals.repair.ratio = 0.571
    signals.frustration.severity = 3
    signals.frustration.count = 5
    signals.escalation.requested = "true"
    signals.repetition.count = 4

**Building Dashboards**

Use signal attributes to build monitoring dashboards in Grafana, Honeycomb, Datadog, etc.:

- **Quality distribution**: Count of traces by ``signals.quality``
- **P95 turn count**: 95th percentile of ``signals.turn_count``
- **Average efficiency**: Mean of ``signals.efficiency_score``
- **High repair rate**: Percentage where ``signals.repair.ratio > 0.3``
- **Frustration rate**: Percentage where ``signals.frustration.severity >= 2``
- **Escalation rate**: Percentage where ``signals.escalation.requested = "true"``
- **Looping rate**: Percentage where ``signals.repetition.count >= 3``
- **Positive feedback rate**: Percentage where ``signals.positive_feedback.count >= 1``

**Creating Alerts**

Set up alerts based on signal thresholds:

- Alert when severe interaction count exceeds threshold in 1-hour window
- Alert on sudden spike in frustration rate (>2x baseline)
- Alert when escalation rate exceeds 5% of total conversations
- Alert on degraded efficiency (P95 turn count increases >50%)

Best Practices
==============

Start simple:

- Alert or page on **Severe** sessions (or on spikes in Severe rate)
- Review **Poor** sessions within 24 hours
- Sample **Excellent** sessions as exemplars

Combine multiple signals to infer failure modes:

- Looping: repetition severity >= 2 + excessive turns
- User giving up: frustration severity >= 2 + escalation requested
- Misunderstood intent: repair ratio > 30% + excessive turns
- Working well: positive feedback + high efficiency + no frustration

Limitations and Considerations
==============================

Signals don’t capture:

- Task completion / real outcomes
- Factual or domain correctness
- Silent abandonment (user leaves without expressing frustration)
- Non-English nuance (pattern libraries are English-oriented)

Mitigation strategies:

- Periodically sample flagged sessions and measure false positives/negatives
- Tune baselines per use case and user population
- Add domain-specific phrase libraries where needed
- Combine signals with non-text metrics (tool failures, disconnects, latency)

.. note::
   Behavioral signals complement—but do not replace—domain-specific response quality evaluation. Use signals to prioritize which traces to inspect, then apply domain expertise and outcome checks to diagnose root causes.

.. tip::
   The flag marker in the span name provides instant visual feedback in trace UIs, while the structured attributes (``signals.quality``, ``signals.frustration.severity``, etc.) enable powerful querying and aggregation in your observability platform.

See Also
========

- :doc:`../guides/observability/tracing` - Distributed tracing for agent systems
- :doc:`../guides/observability/monitoring` - Metrics and dashboards
- :doc:`../guides/observability/access_logging` - Request/response logging
- :doc:`../guides/observability/observability` - Complete observability guide
