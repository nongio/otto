# Otto Specs

A spec file is the **source of truth for the requirements of a feature**. It defines *what* a feature must do — not how it is currently implemented. Specs must remain valid regardless of refactors or rewrites; if an implementation detail changes but the behavior is the same, the spec does not change.

The code must match the spec — not the other way around. If they diverge, the spec wins and the code needs fixing (or the spec needs a deliberate update with recorded rationale).

Specs serve two audiences:

- **Agents** — precise enough to implement a feature correctly without guessing
- **Humans** — capture rationale and trade-offs so decisions are never lost

## When to Write a Spec

Write a spec when:
- Adding a new feature
- Designing a cross-cutting system (input handling, rendering pipeline changes, etc.)
- Refactoring a subsystem where the intended behavior isn't obvious from code alone
- An open question needs a recorded decision

## File Naming

```
specs/<feature-slug>.md
```

Use lowercase kebab-case. Group related specs with a shared prefix (e.g., `workspace-focus.md`, `workspace-switcher.md`).

## Spec Format

Every spec must follow [`SPEC-TEMPLATE.md`](./SPEC-TEMPLATE.md). Copy it as a starting point:

```sh
cp specs/SPEC-TEMPLATE.md specs/<feature-slug>.md
```

---

## Status Meanings

| Status | Meaning |
|--------|---------|
| `draft` | Work in progress — not yet implemented or design still evolving |
| `stable` | Implemented and matches the code; update this when behavior changes |
| `deprecated` | Feature removed or superseded; kept for historical context |
