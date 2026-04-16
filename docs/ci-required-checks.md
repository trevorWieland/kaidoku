# Required CI Checks (Branch Protection)

The `main` branch must require the following GitHub Actions checks to pass
before a pull request can be merged. Claude Code cannot modify repository
settings from inside the workspace, so this configuration is a manual
follow-up for the repository admin.

## Required checks

1. **`Quality Gate / Quality Gate (ubuntu-latest)`** — runs `just ci`
   (fmt, lint, check, phase1-gate, test, coverage ≥ 80%, deny, machete,
   doc, check-lines, check-suppression, check-deps).
2. **`Quality Gate / Quality Gate (macos-latest)`** — same, on macOS.
3. **`Quality Gate / Fuzz Smoke`** — runs `just phase1-fuzz-smoke-ci` on
   nightly + cargo-fuzz. 30 s per target × 4 targets ≈ 2 minutes.
4. **Linear history**: require "Require linear history" to prevent
   out-of-order merges muddying benchmark attribution.
5. **Signed commits**: require verified commits so benchmark baselines can
   be trusted.

## Applying the configuration

As a repository admin:

```
Settings → Branches → Branch protection rules → main →
  Require status checks to pass before merging:
    - Quality Gate / Quality Gate (ubuntu-latest)
    - Quality Gate / Quality Gate (macos-latest)
    - Quality Gate / Fuzz Smoke
  Require branches to be up to date before merging: ✓
  Require linear history: ✓
  Require signed commits: ✓
```

## Why fuzz-smoke is blocking

Audit finding P2 (`F8`) flagged that fuzzing only ran on a weekly schedule,
so parser-hardening regressions could merge before the fuzzer could catch
them. The new 30-second-per-target PR check forces every change on the
parser path (content streams, inline images, stream decoding, geometry
normalization) through the same fuzz harness locally exercised in
`just phase1-fuzz-smoke`. A single 30 s run of `content_parser` would have
surfaced the pre-audit depth-recursion vulnerability in the first second.
