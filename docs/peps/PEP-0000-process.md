# PEP-0000: Plano Enhancement Proposal Process

| Field | Value |
|---|---|
| **PEP** | 0000 |
| **Title** | Plano Enhancement Proposal Process |
| **Status** | Active |
| **Authors** | Plano Maintainers |
| **Created** | 2026-04-07 |

## What is a PEP?

A **Plano Enhancement Proposal (PEP)** is a design document that describes a significant change to the Plano project. PEPs provide a structured way to propose, discuss, and track major features, architectural changes, and process improvements.

PEPs are inspired by [Kafka's KIP process](https://cwiki.apache.org/confluence/display/KAFKA/Kafka+Improvement+Proposals), [Kubernetes KEPs](https://github.com/kubernetes/enhancements/tree/master/keps), and [Envoy's design document process](https://github.com/envoyproxy/envoy/blob/main/CONTRIBUTING.md).

## When is a PEP Required?

A PEP is required for changes that:

- Introduce a new user-facing feature or capability
- Change existing user-facing behavior in a breaking way
- Add a new subsystem or architectural component
- Modify the configuration schema in a significant way
- Add a new LLM provider with non-standard API patterns
- Change the project's processes or governance

A PEP is **not** required for:

- Bug fixes
- Documentation improvements
- Refactoring that doesn't change behavior
- Adding models to an existing provider
- Minor CLI improvements
- Test improvements
- Dependency updates

When in doubt, open a GitHub issue or Discussion first. A maintainer will let you know if a PEP is warranted.

## PEP Lifecycle

```
Draft → Under Review → Accepted → Implementing → Complete
                  ↘ Declined
                  ↘ Deferred
                  ↘ Withdrawn
```

### States

| State | Description |
|---|---|
| **Draft** | Author is writing the proposal. Not yet ready for formal review. |
| **Under Review** | PR is open. Maintainers and community are discussing the design. |
| **Accepted** | Maintainers have approved the design. Implementation can begin. |
| **Declined** | Maintainers have decided not to pursue this proposal. The PEP remains in the repo for historical reference with an explanation of the decision. |
| **Deferred** | Good idea, but not the right time. Will be reconsidered later. |
| **Withdrawn** | Author has decided not to pursue this proposal. |
| **Implementing** | Accepted and actively being built. Linked to tracking issue(s). |
| **Complete** | Fully implemented and released. |

## How to Submit a PEP

### 1. Discuss First (Recommended)

Before writing a full PEP, validate the idea:

- Open a [GitHub Discussion](https://github.com/katanemo/plano/discussions) describing the problem and your proposed approach
- Or bring it up in a [community meeting](https://discord.gg/pGZf2gcwEc)
- Or open a GitHub issue tagged `enhancement`

This step saves time by catching fundamental objections early.

### 2. Write the PEP

Copy `docs/peps/PEP-TEMPLATE.md` to `docs/peps/PEP-XXXX-short-title.md` (use the next available number). Fill in all sections. The template is deliberately structured — each section exists for a reason.

Key guidelines:

- **Be specific.** "Add caching" is too vague. "Add exact-match response cache with configurable TTL keyed by model + message hash" is actionable.
- **Show the config.** If the feature involves user-facing configuration, include the YAML snippet users would write.
- **Address trade-offs.** Every design has trade-offs. Acknowledging them strengthens the proposal.
- **Include alternatives.** Explain what other approaches you considered and why you chose this one.

### 3. Submit as a Pull Request

Open a PR adding your PEP file to `docs/peps/`. The PR title should be `PEP-XXXX: Short Title`. Set the status to `Draft` or `Under Review` depending on readiness.

### 4. Review and Discussion

- At least **two maintainers** must review the PEP
- Community members are encouraged to comment on the PR
- The author is expected to respond to feedback and revise the proposal
- Discussion should focus on the **design**, not implementation details (those belong in code review)
- Complex PEPs may be discussed in a community meeting

### 5. Decision

Maintainers aim to provide **initial feedback within two weeks** of a PEP entering `Under Review`. Complex proposals may take longer, but the author should never be left without a response.

A PEP is **accepted** when at least two maintainers approve the PR and there are no unresolved objections. The accepting maintainer merges the PR with the status set to `Accepted`.

A PEP is **declined** when maintainers determine the proposal doesn't align with the project's direction or has fundamental issues that can't be resolved. The PR is merged (not closed) with the status set to `Declined` and a rationale recorded — declined PEPs remain in the repo as a record.

**Resolving disagreements:** If maintainers disagree on a PEP, the proposal is discussed in a community meeting. If consensus still can't be reached, the project lead makes the final call and records the rationale in the PEP.

### 6. Implementation

Once accepted:

- Create a tracking GitHub issue (or issues) for the implementation
- Link the issue(s) in the PEP header
- Update the PEP status to `Implementing`
- Implementation PRs should reference the PEP number (e.g., "Part of PEP-0042")
- When all implementation work is merged and released, update status to `Complete`

## PEP Numbering

- PEPs are numbered sequentially starting from 0001
- PEP-0000 is reserved for this process document
- The author picks the next available number when submitting

## Roles

| Role | Responsibility |
|---|---|
| **Author** | Writes the PEP, responds to review feedback, drives the proposal to a decision |
| **Sponsor** | A maintainer who shepherds the PEP through review. Required for PEPs from non-maintainers. Find a sponsor by asking in Discord or a community meeting. |
| **Reviewers** | Maintainers and community members who provide feedback on the design |

## Amending Accepted PEPs

If an accepted PEP needs material changes during implementation:

- For minor adjustments (implementation details, clarifications): update the PEP in-place via a PR
- For significant design changes: open a new PEP that supersedes the original, linking back to it

## Index

| PEP | Title | Status | Author |
|---|---|---|---|
| [0000](PEP-0000-process.md) | Plano Enhancement Proposal Process | Active | Plano Maintainers |
