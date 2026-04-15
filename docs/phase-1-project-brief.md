# Phase 1 Project Brief — Parsing Foundation

Date: April 15, 2026
Owner: kaidoku core team
Phase window: 1 sprint (target 1-2 weeks)

## 1) Purpose

Phase 1 establishes the technical foundation for every later phase by delivering a reliable PDF parsing vertical slice with reproducible outputs, fixtures, and baseline measurements.

At the end of this phase, we should be able to point to a deterministic `PDF -> structured JSON` path with page-level coordinates and a repeatable demo command.

## 2) Phase Outcome (Definition of Done)

Phase 1 is complete when all of the following are true:

1. `kaidoku-core` can parse a PDF and emit raw page primitives (chars/spans/images metadata) with bounding boxes.
2. `kaidoku-cli` can run a stable `extract` command over fixture PDFs and write JSON artifacts.
3. Golden fixtures are versioned and diffable in CI.
4. Baseline performance metrics are captured for the same fixture set.
5. A single demo script/command reproduces the above on a clean clone.

## 3) Scope and Non-Goals

In scope:

- Parser adapter and extraction contracts
- Raw extraction entities and JSON serialization
- Fixture corpus setup and golden outputs
- CLI demo path for extraction artifacts
- Baseline performance and correctness checks

Out of scope:

- Heading/list/table semantic detection
- Reading-order optimization (XY-Cut++)
- Hybrid backends
- Tagged PDF/PDF-UA writing
- Annotated PDF highlighting

## 4) Key Deliverables (Groundwork-First)

## D1. Parse Adapter Contract and Core Entities

Description:

- Introduce `parse` module boundary in `kaidoku-core` that isolates parser implementation details.
- Define raw extraction entities for Phase 1 (page index, content kind, bbox, text/glyph payload, source reference).
- Keep API stable enough for downstream semantic pipeline stages.

Acceptance criteria:

1. Public core API exposes a single extraction entrypoint returning deterministic structured output.
2. Parser internals are not leaked into callers (adapter boundary enforced).
3. Unit tests cover entity invariants (bbox validity, page numbering, source ref stability).

Demo checkpoint:

- `cargo test -p kaidoku-core` passes with entity + adapter tests visible in output.

## D2. Fixture Corpus + Golden Output Harness

Description:

- Add a minimal but representative fixture set under `tests/corpus/phase1/`.
- Generate and commit golden JSON outputs under `tests/golden/phase1/`.
- Add snapshot/diff checks so output regressions are explicit.

Acceptance criteria:

1. At least 3 fixture PDFs (simple text, multi-column, mixed content).
2. Golden comparison test fails on output drift and reports actionable diff.
3. Fixture/golden naming conventions documented in brief README.

Demo checkpoint:

- `cargo nextest run -p kaidoku-core` includes golden-output checks.

## D3. CLI Extraction Demo Path

Description:

- Implement CLI extraction command to read input PDF(s) and emit JSON artifacts to an output directory.
- Include deterministic formatting and metadata fields for downstream tooling.

Acceptance criteria:

1. CLI supports `--input`, `--output`, and optional `--pages` range.
2. Exit codes are reliable (`0` success, non-zero on parse/IO failure).
3. `README` includes one canonical demo command.

Demo checkpoint:

- `cargo run -p kaidoku-cli -- extract --input <fixture.pdf> --output <dir>` produces a JSON artifact that matches golden expectations.

## D4. Baseline Performance Harness

Description:

- Add baseline benchmark/report command for Phase 1 fixture corpus.
- Record parse throughput and latency-to-first-page for local mode.

Acceptance criteria:

1. Benchmark command runs in CI (or CI-adjacent job) without mutating tracked files.
2. Report format is machine-readable (JSON or markdown table).
3. Baseline numbers are checked in and referenced in docs.

Demo checkpoint:

- `just phase1-bench` (or equivalent) prints/stores baseline results for each fixture.

## D5. Phase Gate in CI

Description:

- Add a Phase 1 gate recipe (sub-gate inside `just ci`) that validates parser extraction behavior, fixture integrity, and baseline metrics format.
- Ensure the gate is PR-blocking through existing `Quality Gate` workflow.

Acceptance criteria:

1. CI fails if fixture outputs drift unexpectedly.
2. CI fails if phase benchmark report generation breaks.
3. Gate is documented as required for all parser-adapter changes.

Demo checkpoint:

- Open PR with intentional output drift -> `Quality Gate` fails.
- Restore expected output -> `Quality Gate` passes.

## 5) Delivery Sequence and Handoff Units

1. D1 Parse adapter and entities
2. D2 Fixtures and golden harness
3. D3 CLI extraction command
4. D4 Baseline benchmark harness
5. D5 CI phase gate and demo script hardening

Each deliverable should land as its own PR-sized unit with:

- implementation
- tests
- docs update
- explicit demo command in PR description

## 6) Demo Package for Phase Review

At phase close, present:

1. Demo command transcript from clean clone.
2. Output JSON artifact(s) from fixture set.
3. Golden diff test run output.
4. Baseline performance report.
5. Link to passing CI run (`Quality Gate`).

## 7) Risks and Mitigations (Phase 1)

1. Parser API churn risk
- Mitigation: keep adapter trait internal and isolate dependency-specific code.

2. Fixture fragility risk
- Mitigation: start with a small curated corpus and stable normalization rules.

3. Benchmark noise risk
- Mitigation: pin fixture set and report median over multiple runs.

## 8) Immediate Next Tickets (First 5)

1. Define `RawElement` and `SourceRef` v0 schema in `kaidoku-core`.
2. Add parse adapter trait + first implementation skeleton.
3. Add `extract` CLI command shape and IO contract.
4. Add `tests/corpus/phase1` + `tests/golden/phase1` harness.
5. Add phase benchmark command and wire into `just ci` sub-gate.
