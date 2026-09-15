# otto-ahp

Rust server for the Agent Host Protocol (AHP).

- The spec lives in `spec/upstream/` and is vendored and read-only. Change it only with
  `scripts/sync-spec.sh`. The normative rules are in `spec/upstream/specification/`, and
  the type source of truth is `spec/upstream/types/*.ts`.
- Use `ahp-types` for wire types. Its version must equal `AHP_PROTOCOL_VERSION` in
  `spec/upstream.env`.
- otto-ahp is the agents service for Otto: it runs ACP agents and serves them over AHP,
  taking the service role in `../otto/specs/agents.md`. `otto-agents` is the GUI app.
  Otto surfaces are AHP clients (`docs/plans/0003`, `0007`, `0009`).
- Read the relevant plan in `docs/plans/` before starting a milestone, and update its
  status when work lands.
- Run `scripts/ci.sh` before calling work done.
