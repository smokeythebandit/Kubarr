# Architecture Decision Records

Architecture Decision Records (ADRs) capture important architectural decisions made during the development of Kubarr, along with their context and consequences.

## ADR Index

| ADR | Status | Date | Summary |
|-----|--------|------|---------|
| [Storage Model Architecture](storage-model-architecture.md) | Proposed | 2026-01-29 | Evaluates storage layer options for Kubarr's single-pod architecture — PostgreSQL, SQLite, hybrid approaches |

## What is an ADR?

An ADR is a short document that captures a single architectural decision. Each ADR describes the context, the decision made, and the consequences of that decision. ADRs are numbered sequentially and are immutable once accepted — superseded decisions are marked as such with a link to the replacement.

## Template

New ADRs should follow this structure:

- **Status** — Proposed, Accepted, Deprecated, or Superseded
- **Context** — What is the issue motivating this decision?
- **Decision** — What is the change being proposed?
- **Consequences** — What becomes easier or harder as a result?
