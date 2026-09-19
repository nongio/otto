# Review Instructions

These instructions apply to **every** change that is about to be merged —
a branch, a pull request, or a working tree the author considers finished.

## How to review

Use the `otto-reviewer` agent (`.claude/agents/otto-reviewer.md`). It holds the
full checklist; this file exists so the repository points at it.

```
Agent(subagent_type: "otto-reviewer", prompt: "review this branch before merge")
```

It reviews seven dimensions and reports findings with evidence:

1. **Dead code and dangling leftovers** — orphaned symbols, debug scaffolding,
   half-migrations, and docs, specs, locales or config examples left describing
   behaviour the change removed.
2. **Duplication** — logic written twice, or written here when `otto-kit`,
   `src/utils` or a sibling component already has it.
3. **Reinventing the wheel** — hand-rolling what `lay-rs`, Smithay, Skia or
   `otto-kit` already provide; and the inverse, a new dependency for something
   small Otto should own.
4. **Security** — privilege, the lock screen and greeter as a boundary,
   client-controlled input, portal and screenshare consent, secrets, `unsafe`,
   resource leaks, IPC surfaces.
5. **Linux and freedesktop standards** — XDG paths, desktop entries, D-Bus
   naming, Wayland protocol semantics, systemd integration, packaging.
6. **Design** — layering, coordinate spaces, ownership and lock order, error
   handling, structural performance, tests.
7. **Rust craft** — the Microsoft Pragmatic Rust Guidelines, via the `ms-rust`
   skill. The reviewer loads the guideline files that match what the diff does
   and cites rule ids (`M-PANIC-IS-STOP`, `M-STRONG-TYPES`, …).

## What the reviewer does not do

It does not fix the code, and it does not add the `// Rust guideline compliant`
marker. It reports; the author decides what to change.

## The bar

**Blocker** means a security hole, a crash reachable from a running desktop,
data loss, or a broken session path. Everything else is "should fix" or "worth
considering". A clean diff is reported as clean — findings are never invented
to look thorough.

## Before the review

Run the cheap gates yourself so the reviewer spends its attention on judgment:

```sh
cargo fmt --all -- --check
cargo clippy --features "default" -- -D warnings
cargo test --lib
python3 scripts/check-locales.py
```

Also check spec sync (`.github/instructions/spec-sync.instructions.md`) — a
behaviour change with a stale spec is a finding the reviewer will raise anyway.
