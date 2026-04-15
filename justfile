set shell := ["bash", "-euo", "pipefail", "-c"]

cargo := env("CARGO", "cargo")
max_lines := "500"
toml_globs := "Cargo.toml bin/*/Cargo.toml crates/*/Cargo.toml .cargo/*.toml .config/*.toml rust-toolchain.toml clippy.toml taplo.toml deny.toml rustfmt.toml lefthook.yml"
foundation_crates := "kaidoku-core"
capability_crates := "kaidoku-cli kaidoku-server kaidoku-python"

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

ci: fmt lint check test coverage deny machete doc check-lines check-suppression check-deps
    @echo "==> All CI checks passed!"
