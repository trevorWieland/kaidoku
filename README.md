# kaidoku

kaidoku (解読) is a Rust-first PDF deciphering engine focused on structured extraction,
traceable evidence coordinates, and open accessibility compliance workflows.

## Workspace

- `crates/kaidoku-core` — extraction core primitives
- `bin/kaidoku-cli` — CLI entrypoint
- `bin/kaidoku-server` — Axum HTTP service entrypoint
- `crates/kaidoku-python` — PyO3 bridge crate shell

## Quality Gate

Run all checks locally with:

```bash
just ci
```

Parser-adapter policy:

- Any change under `crates/kaidoku-core/src/parse/` must run and pass `just phase1-gate`.
- Parser-adapter PRs are expected to include updated goldens/bench baseline when behavior changes.
- The Phase 1 fixture corpus is pinned; removing or renaming required fixtures is gate-breaking.
- Extraction schema is currently `kaidoku.phase1.v2` and intentionally breaking from v1.
- PR merges to `main` require the new `Fuzz Smoke` check to pass in addition to the Quality Gate. See [docs/ci-required-checks.md](docs/ci-required-checks.md) for branch-protection settings.

## Phase 1 Demo — one command

The canonical demo runs extraction over the Phase-1 fixture corpus, verifies
golden outputs and geometry invariants, checks benchmarks against the
committed baseline, and runs a short fuzz smoke — all in one orchestrated
pass that writes a reproducible transcript:

```bash
just phase1-demo-all
```

Artifacts land under `target/phase1/`:

- `target/phase1/demo/*.json` — canonical extraction output per fixture
- `target/phase1/demo/transcript.txt` — commit SHA, wall-clock, step log
- `target/phase1/benchmarks.current.json` — latest benchmark report

### Component recipes

The orchestrated recipe above composes these smaller recipes; use them
directly if you only want a subset of the gate:

```bash
just phase1-demo         # extraction artifacts only
just phase1-bench        # generate benchmark report
just phase1-bench-refresh # refresh committed baseline (review-gated)
just phase1-gate         # tests + bench --check
just phase1-fuzz-smoke   # 20s per target (4 targets) local smoke
```

`phase1-bench` and `phase1-gate` run benchmarks with `--release` and capture host metadata
(OS/arch/cores/profile/rustc/governor when available) in the benchmark report. Baselines are
stored with runner-class keyed reports (`kaidoku.phase1.bench.v4`) for cross-runner CI checks.

## First-page fast path

For latency-critical previews, call `kaidoku_core::extract_pdf_first_page`
directly. This avoids the full page-tree walk and extracts only the first
page by walking the leftmost `/Kids` path from `/Pages`. The first-page
benchmark (`bench phase1`'s `first_page_ms`) measures this path.

## Opt-in parallel page extraction

Large multi-page PDFs can be extracted in parallel by opting into
`ExtractOptions::builder().parallel_page_extraction(true)`. Page order is
preserved and structural output (element counts, source refs, bboxes) is
byte-stable across runs. Font IDs are assigned by resource-name order
during a deterministic pre-pass; serial and parallel may therefore produce
different font ID numberings, but both are self-consistent and reproducible.
