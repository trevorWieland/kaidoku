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
