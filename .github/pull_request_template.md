## Summary

-

## Validation

- [ ] `just ci`

## Parser Adapter Checklist (Required for `crates/kaidoku-core/src/parse/` changes)

- [ ] I ran `just phase1-gate` locally and it passed.
- [ ] I kept the required phase1 fixture set pinned (simple_text, multi_column, mixed_content).
- [ ] I verified strict corpus/provenance/golden/benchmark parity.
- [ ] I committed updated goldens and benchmark baseline (if extraction behavior or performance baseline changed).
