# contract/ — source of truth for the Anecho API

Versioned protobuf schemas. Rust types (prost, `backend/crates/anecho-contract`) and
TypeScript types (`protoc-gen-es`, `frontend/src/gen/`) are **generated** from these files —
never written by hand on either side.

Rules (see CLAUDE.md):
- Additive evolution only: add optional fields, never rename or remove before a major version.
- `make contract-check` (`buf lint` + `buf breaking`) is mandatory before any merge.
- Each phase ends with a tag `contract-v0.x`; breaking checks run against the latest tag.

Tooling: `buf` (https://buf.build) for lint, breaking-change detection and TypeScript
generation. Rust generation needs nothing installed (vendored protoc via `build.rs`).
