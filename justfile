set shell := ["bash", "-euo", "pipefail", "-c"]

cargo := env("CARGO", "cargo")
max_lines := "500"
toml_globs := "Cargo.toml bin/*/Cargo.toml crates/*/Cargo.toml .cargo/*.toml .config/*.toml rust-toolchain.toml clippy.toml taplo.toml deny.toml rustfmt.toml lefthook.yml"
foundation_crates := "kaidoku-core"
capability_crates := "kaidoku-cli kaidoku-server kaidoku-python"
phase1_fixtures := "tests/corpus/phase1/doclaynet_simple_text.pdf tests/corpus/phase1/doclaynet_multi_column.pdf tests/corpus/phase1/doclaynet_mixed_content.pdf tests/corpus/phase1/pdfjs_copy_paste_ligatures.pdf tests/corpus/phase1/pdfjs_arabic_cid_true_type.pdf tests/corpus/phase1/pdfjs_identity_to_unicode_map_char_code_of.pdf"

# Default recipe: show available commands

default:
    @just --list

bootstrap:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "==> Checking for rustup..."
    if ! command -v rustup &>/dev/null; then
        echo "Installing rustup..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        echo "Run 'source ~/.cargo/env' or restart your shell, then re-run 'just bootstrap'"
        exit 1
    fi

    echo "==> Ensuring stable toolchain with components..."
    rustup show active-toolchain &>/dev/null || rustup default stable
    rustup component add rustfmt clippy llvm-tools-preview 2>/dev/null || true

    echo "==> Installing cargo-binstall..."
    if ! command -v cargo-binstall &>/dev/null; then
        curl -L --proto '=https' --tlsv1.2 -sSf \
            https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash
    fi

    echo "==> Installing cargo tools (skipping already-installed)..."
    failed=""

    if ! command -v cargo-nextest &>/dev/null; then
        echo "  Installing cargo-nextest..."
        nextest_platform="linux"
        [[ "$(uname -s)" == "Darwin" ]] && nextest_platform="mac"
        if ! curl -LsSf "https://get.nexte.st/latest/${nextest_platform}" | tar zxf - -C "${CARGO_HOME:-$HOME/.cargo}/bin" 2>/dev/null; then
            if ! cargo binstall --no-confirm cargo-nextest; then
                echo "  FAIL: cargo-nextest"
                failed="$failed cargo-nextest"
            fi
        fi
    fi

    tools=(
        "cargo-deny:cargo-deny"
        "cargo-fuzz:cargo-fuzz"
        "cargo-llvm-cov:cargo-llvm-cov"
        "cargo-machete:cargo-machete"
        "cargo-hack:cargo-hack"
        "cargo-insta:cargo-insta"
        "taplo:taplo-cli"
        "just:just"
    )

    for entry in "${tools[@]}"; do
        bin="${entry%%:*}"
        pkg="${entry##*:}"
        if ! command -v "$bin" &>/dev/null; then
            echo "  Installing $pkg..."
            if ! cargo binstall --no-confirm "$pkg"; then
                echo "  FAIL: $pkg"
                failed="$failed $pkg"
            fi
        fi
    done

    echo "==> Ensuring nightly toolchain for fuzzing..."
    if ! rustup toolchain list | grep -q '^nightly'; then
        if ! rustup toolchain install nightly; then
            echo "  FAIL: nightly toolchain"
            failed="$failed nightly-toolchain"
        fi
    fi

    echo "==> Platform-specific setup..."
    if [[ "$(uname -s)" == "Linux" ]]; then
        if command -v apt-get &>/dev/null; then
            echo "Installing mold linker (apt)..."
            sudo apt-get install -y mold clang 2>/dev/null || echo "  (skipped — install mold manually if needed)"
        elif command -v dnf &>/dev/null; then
            echo "Installing mold linker (dnf)..."
            sudo dnf install -y mold clang 2>/dev/null || echo "  (skipped — install mold manually if needed)"
        else
            echo "  Install mold linker manually: https://github.com/rui314/mold"
        fi
    else
        echo "  macOS/Windows detected — using default linker (no mold needed)"
    fi

    echo "==> Installing lefthook..."
    if ! command -v lefthook &>/dev/null; then
        if curl -1sLf 'https://dl.cloudsmith.io/public/evilmartians/lefthook/setup.shell.sh' | bash 2>/dev/null \
            && command -v lefthook &>/dev/null; then
            true
        elif command -v brew &>/dev/null; then
            brew install lefthook
        else
            echo "  FAIL: lefthook"
            failed="$failed lefthook"
        fi
    fi

    if command -v lefthook &>/dev/null; then
        echo "==> Activating git hooks..."
        lefthook install
    fi

    if [[ -n "$failed" ]]; then
        echo ""
        echo "==> Bootstrap completed with failures:"
        echo "    Failed to install:$failed"
        echo "    Install these manually, then re-run 'just bootstrap' to verify."
        exit 1
    fi

    echo "==> Bootstrap complete!"

build:
    @{{ cargo }} build --workspace --quiet

check:
    @{{ cargo }} check --workspace --all-targets --quiet

test *args:
    @{{ cargo }} nextest run --workspace --profile ci --no-tests=pass {{ args }}

phase1-bench:
    @{{ cargo }} run --release -p kaidoku-cli -- bench phase1 --iterations 15 --warmup-iterations 4 --fixtures tests/corpus/phase1 --output tests/golden/phase1/benchmarks.current.json

phase1-bench-refresh:
    @{{ cargo }} run --release -p kaidoku-cli -- bench phase1 --iterations 15 --warmup-iterations 4 --fixtures tests/corpus/phase1 --output tests/golden/phase1/benchmarks.baseline.json

phase1-gate:
    @{{ cargo }} nextest run -p kaidoku-core --profile ci --no-tests=pass
    @{{ cargo }} run --release -p kaidoku-cli -- bench phase1 --iterations 15 --warmup-iterations 4 --check --fixtures tests/corpus/phase1 --baseline tests/golden/phase1/benchmarks.baseline.json --output target/phase1/benchmarks.current.json

phase1-demo:
    @{{ cargo }} run -p kaidoku-cli -- extract --input tests/corpus/phase1/doclaynet_simple_text.pdf --input tests/corpus/phase1/doclaynet_multi_column.pdf --input tests/corpus/phase1/doclaynet_mixed_content.pdf --input tests/corpus/phase1/pdfjs_copy_paste_ligatures.pdf --input tests/corpus/phase1/pdfjs_arabic_cid_true_type.pdf --input tests/corpus/phase1/pdfjs_identity_to_unicode_map_char_code_of.pdf --output target/phase1/demo

# Fixed LibFuzzer seeds (-seed=0xN) make transcripts reproducible: the
# generated corpus mutations are deterministic for the same seed + time
# budget, so a regression that showed up in CI can be replayed verbatim
# locally with `just phase1-fuzz-smoke`.
phase1-fuzz-smoke:
    @cd fuzz && cargo +nightly fuzz run decode_filters -- -max_total_time=20 -seed=1
    @cd fuzz && cargo +nightly fuzz run content_ops -- -max_total_time=20 -seed=2
    @cd fuzz && cargo +nightly fuzz run geometry_normalization -- -max_total_time=20 -seed=3
    @cd fuzz && cargo +nightly fuzz run content_parser -- -max_total_time=20 -seed=4
    @cd fuzz && cargo +nightly fuzz run decode_predictor -- -max_total_time=20 -seed=5

# PR-blocking fuzz-smoke invoked from .github/workflows/ci.yml — a slightly
# longer run than the local 20 s smoke so transient regressions surface before
# merge. Still capped short enough (~2 min total) not to dominate CI time.
# Seeds match phase1-fuzz-smoke so CI and local runs explore the same paths.
phase1-fuzz-smoke-ci:
    @cd fuzz && cargo +nightly fuzz run decode_filters -- -max_total_time=30 -seed=1
    @cd fuzz && cargo +nightly fuzz run content_ops -- -max_total_time=30 -seed=2
    @cd fuzz && cargo +nightly fuzz run geometry_normalization -- -max_total_time=30 -seed=3
    @cd fuzz && cargo +nightly fuzz run content_parser -- -max_total_time=30 -seed=4
    @cd fuzz && cargo +nightly fuzz run decode_predictor -- -max_total_time=30 -seed=5

# Single orchestrated Phase-1 demo: produces artifacts, verifies goldens,
# checks benchmarks against baseline, runs a short fuzz smoke, and writes a
# transcript capturing commit SHA + wall-clock so reviewers can reproduce.
phase1-demo-all:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p target/phase1/demo
    TRANSCRIPT=target/phase1/demo/transcript.txt
    {
        echo "kaidoku Phase-1 orchestrated demo"
        echo "commit: $(git rev-parse HEAD 2>/dev/null || echo unknown)"
        echo "started: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo
    } >"$TRANSCRIPT"
    echo "==> Step 1/5: extract Phase-1 fixture corpus" | tee -a "$TRANSCRIPT"
    just phase1-demo 2>&1 | tee -a "$TRANSCRIPT"
    echo "==> Step 2/5: verify goldens, determinism, geometry, EI/depth invariants" | tee -a "$TRANSCRIPT"
    {{ cargo }} nextest run -p kaidoku-core --profile ci --no-tests=pass 2>&1 | tee -a "$TRANSCRIPT"
    echo "==> Step 3/5: check benchmarks against baseline" | tee -a "$TRANSCRIPT"
    {{ cargo }} run --release -p kaidoku-cli -- bench phase1 --iterations 15 --warmup-iterations 4 --check --fixtures tests/corpus/phase1 --baseline tests/golden/phase1/benchmarks.baseline.json --output target/phase1/benchmarks.current.json 2>&1 | tee -a "$TRANSCRIPT"
    echo "==> Step 4/5: short fuzz smoke (4 targets, 20s each)" | tee -a "$TRANSCRIPT"
    just phase1-fuzz-smoke 2>&1 | tee -a "$TRANSCRIPT"
    echo "==> Step 5/5: ok" | tee -a "$TRANSCRIPT"
    echo "finished: $(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$TRANSCRIPT"
    echo "transcript: $TRANSCRIPT"

coverage:
    @{{ cargo }} llvm-cov nextest -p kaidoku-core --profile ci --lcov --output-path lcov.info --fail-under-lines 80 --no-tests=pass

lint:
    @{{ cargo }} clippy --workspace --all-targets --quiet -- -D warnings

fmt:
    @{{ cargo }} fmt --check
    @RUST_LOG=error taplo fmt --check {{ toml_globs }}

fmt-fix:
    @{{ cargo }} fmt
    @RUST_LOG=error taplo fmt {{ toml_globs }}

fix:
    @{{ cargo }} fmt
    @RUST_LOG=error taplo fmt {{ toml_globs }}
    @{{ cargo }} clippy --workspace --all-targets --fix --allow-dirty --allow-staged --quiet -- -D warnings

deny:
    @{{ cargo }} deny --log-level error check

machete:
    @{{ cargo }} machete

doc:
    @RUSTDOCFLAGS="-D warnings" {{ cargo }} doc --workspace --no-deps --quiet

check-lines:
    #!/usr/bin/env bash
    set -euo pipefail
    failed=0
    while IFS= read -r -d '' file; do
        lines=$(wc -l < "$file")
        if [[ "$lines" -gt {{ max_lines }} ]]; then
            echo "FAIL: $file has $lines lines (max {{ max_lines }})"
            failed=1
        fi
    done < <(find crates bin -name '*.rs' -print0 2>/dev/null)
    if [[ "$failed" -eq 1 ]]; then
        echo "check-lines: exceeded {{ max_lines }}-line ceiling"
        exit 1
    fi

check-suppression:
    #!/usr/bin/env bash
    set -euo pipefail
    found=0
    if grep -rn '#\[allow(' crates bin --include='*.rs' 2>/dev/null; then
        echo "FAIL: Found #[allow(...)] in source. Move to workspace [lints] table."
        found=1
    fi
    if grep -rn '#\[expect(' crates bin --include='*.rs' 2>/dev/null; then
        echo "FAIL: Found #[expect(...)] in source. Move to workspace [lints] table."
        found=1
    fi
    if grep -rn '#!\[allow(' crates bin --include='*.rs' 2>/dev/null; then
        echo "FAIL: Found #![allow(...)] in source. Move to workspace [lints] table."
        found=1
    fi
    if grep -rn '#\[ignore\]' crates bin --include='*.rs' 2>/dev/null; then
        echo "FAIL: Found #[ignore] in tests."
        found=1
    fi
    if [[ "$found" -eq 1 ]]; then
        exit 1
    fi

check-deps:
    #!/usr/bin/env bash
    set -euo pipefail
    foundation=({{ foundation_crates }})
    capability=({{ capability_crates }})
    failed=0

    for fcrate in "${foundation[@]}"; do
        deps=$({{ cargo }} tree -p "$fcrate" --depth 1 --prefix none 2>/dev/null | tail -n +2 | awk '{print $1}')
        for ccrate in "${capability[@]}"; do
            if echo "$deps" | grep -q "^${ccrate}$"; then
                echo "FAIL: foundation crate '$fcrate' depends on capability crate '$ccrate'"
                failed=1
            fi
        done
    done

    if [[ "$failed" -eq 1 ]]; then
        echo "check-deps: foundation -> capability dependency violation"
        exit 1
    fi

ci: fmt lint check phase1-gate test coverage deny machete doc check-lines check-suppression check-deps
    @echo "==> All CI checks passed!"
