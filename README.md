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

## Phase 1 Demo

Run the canonical extraction demo over the committed Phase 1 fixture corpus:

```bash
cargo run -p kaidoku-cli -- extract \
  --input tests/corpus/phase1/doclaynet_simple_text.pdf \
  --input tests/corpus/phase1/doclaynet_multi_column.pdf \
  --input tests/corpus/phase1/doclaynet_mixed_content.pdf \
  --output target/phase1/demo
```

Generate baseline benchmark output:

```bash
just phase1-bench
```

Refresh the committed benchmark baseline after approved parser/benchmark methodology changes:

```bash
just phase1-bench-refresh
```

Run the full Phase 1 parser gate locally:

```bash
just phase1-gate
```
