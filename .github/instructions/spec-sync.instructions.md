# Spec Sync Instructions

These instructions apply to **every** code change session that alters observable behavior of a feature.

## When to Update Specs

After implementing a requirement, bug fix, or behavior change:

1. **Check if a spec exists** in `specs/` for the affected feature.
2. **If a spec exists** and the change alters observable behavior described in it, update the spec to match the new behavior in the same change.
3. **If no spec exists** and the feature is non-trivial (multi-file, user-facing, or has edge cases), create a new spec following `specs/SPEC-TEMPLATE.md`.

## How to Update

- Add new behavior to the **Behavior** section using concrete "when X, then Y" language.
- If a previous behavior was changed, update it in-place and record **why** in the **Rationale** section.
- Move resolved items from **Open Questions** to **Rationale** when a decision is made.
- Add new edge cases discovered during implementation to **Constraints & Edge Cases**.
- Never reference code, modules, or data structures in the spec — describe the contract, not the implementation.

## How to Create a New Spec

Use the template at `specs/SPEC-TEMPLATE.md`. The file should be named with kebab-case matching the feature (e.g., `specs/workspaces-multi-output.md`).

Required sections: Summary, Goals, Non-Goals, Behavior, Constraints & Edge Cases, Rationale, Open Questions.

## Triggering This Agent

When the user states a new requirement, reports a bug, or asks for a behavior change:

1. Identify which spec in `specs/` covers the affected feature (search by topic).
2. Read the current spec.
3. Determine what changed: new behavior, modified behavior, new constraint, or resolved open question.
4. Apply the minimal edit to keep the spec accurate.
5. If no spec covers the feature and it warrants one, create it.

## Rules

- The spec describes **what** the system does, not **how** it is implemented.
- Every behavior statement must be testable or observable by a user.
- Do not remove behavior from a spec unless it was explicitly dropped — add new behavior alongside existing.
- Keep the spec concise. One sentence per behavior rule where possible.
- Status stays `draft` until the feature is fully implemented and stable.
- **Always delegate spec writes to a background sub-agent** (agent_type: `general-purpose`, mode: `background`). Never write specs inline in the main conversation.

## Spec File Mapping

- Workspace management, multi-output, virtual outputs → `specs/workspaces-multi-output.md`
- Dynamic island → `specs/dynamic-island.md`
- Notifications → `specs/notification-daemon.md`
