# Contributing

## Required Local Checks

Before opening or updating a PR, run:

```bash
just ci
```

## Parser-Adapter Change Policy (Required)

Any change that touches parser-adapter code (`crates/kaidoku-core/src/parse/`) must also run:

```bash
just phase1-gate
```

Parser-adapter changes are incomplete unless all of the following are true:

- `just phase1-gate` passes locally.
- Required phase1 fixture set remains pinned:
  - `doclaynet_simple_text.pdf`
  - `doclaynet_multi_column.pdf`
  - `doclaynet_mixed_content.pdf`
- Corpus, provenance manifest, goldens, and benchmark baseline remain in strict parity.
- Golden and benchmark baseline files are committed when behavior or performance baseline changes.
- Fuzz smoke targets are exercised for hostile-input paths (`just phase1-fuzz-smoke`) before merge when parser internals are touched.
