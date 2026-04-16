# Required CI Checks (Branch Protection)

The `main` branch is locked down by GitHub repository ruleset
`main-protection` (ID `15115594`). This document is the canonical record of
what's enforced, why, and how to audit it.

## Applied rules

The ruleset is **active** on `~DEFAULT_BRANCH` and enforces:

1. **Deletion** — blocked.
2. **Non-fast-forward pushes** — blocked.
3. **Required linear history** — PR merges only, no merge commits.
4. **Required signatures** — every commit on main must be verifiably signed.
5. **Pull-request required** — squash-only merge, no force-approval bypass,
   `required_reviewers = []` (solo-maintainer posture), stale reviews not
   auto-dismissed.
6. **Required status checks** (strict: PRs must be up to date before merge):
   - `Quality Gate` — runs `just ci` (fmt, lint, check, phase1-gate,
     test, coverage ≥ 80 %, deny, machete, doc, check-lines,
     check-suppression, check-deps) on `ubuntu-latest` and `macos-latest`.
   - `Fuzz Smoke` — runs `just phase1-fuzz-smoke-ci` on nightly +
     cargo-fuzz (4 targets × 30 s ≈ 2 min).

## Audit / verify

```
gh api repos/trevorWieland/kaidoku/rulesets/15115594 \
  --jq '{name, enforcement, rules: [.rules[] | {type, params: .parameters}]}'
```

## Why fuzz-smoke is blocking

Audit finding P2 (`F8`) flagged that fuzzing only ran on a weekly schedule,
so parser-hardening regressions could merge before the fuzzer could catch
them. The new 30-second-per-target PR check forces every change on the
parser path (content streams, inline images, stream decoding, geometry
normalization) through the same fuzz harness locally exercised in
`just phase1-fuzz-smoke`. A single 30 s run of `content_parser` would have
surfaced the pre-audit depth-recursion vulnerability in the first second.

## Changing the ruleset

Direct `PUT` to `repos/trevorWieland/kaidoku/rulesets/15115594` with the
full payload. Partial updates aren't supported by the GitHub ruleset API —
always send the complete `rules[]` list.
