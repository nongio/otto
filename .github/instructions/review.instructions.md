# Code Review Instructions

These guidelines apply when reviewing code changes (PRs, diffs, or agent-produced patches).

## Spec Compliance

- If a change alters the **observable behavior** of a feature that has a spec in `specs/`, check that the spec was updated in the same change.
- If the spec was **not** updated, flag it as a required change — the spec is the source of truth and must stay in sync.
- A behavior change without a spec update is treated as an incomplete change, not just a style issue.

## Regressions

- If a change causes a **regression** (behavior that contradicts an existing spec), reject it and point to the relevant spec section.
- Regressions are only acceptable if the spec is explicitly updated and the rationale for the behavior change is recorded in the spec's **Rationale** section.
