# Phase 1 Golden Artifacts

This directory stores deterministic JSON extraction goldens and the benchmark baseline used by the Phase 1 gate.

- `<fixture-stem>.json` files: expected output for each fixture in `tests/corpus/phase1`.
- `benchmarks.baseline.json`: committed benchmark baseline (`kaidoku.phase1.bench.v4`) used by `kaidoku bench phase1 --check`, including runner-class keyed reports captured from release-profile benchmark runs.
