#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
COVERAGE_DIR="$PROJECT_ROOT/coverage"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

print_step() {
    echo -e "${YELLOW}▶ $1${NC}"
}

print_success() {
    echo -e "${GREEN}✓ $1${NC}"
}

print_error() {
    echo -e "${RED}✗ $1${NC}"
}

# Add llvm tools to PATH
export PATH="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's|host: ||p')/bin:$PATH"

# Check if llvm-tools are available
if ! command -v llvm-profdata &> /dev/null; then
    print_error "llvm-profdata not found. Install with: rustup component add llvm-tools-preview"
    exit 1
fi

# Create coverage directory
mkdir -p "$COVERAGE_DIR"

case "${1:-full}" in
    --summary)
        print_step "Running tests with coverage instrumentation..."
        cd "$PROJECT_ROOT"
        RUSTFLAGS="-C instrument-coverage" LLVM_PROFILE_FILE="$COVERAGE_DIR/turbo-%m-%p.profraw" \
            cargo test --tests --workspace --exclude "*example*" --exclude "*turborepo-tests*"
        
        print_step "Merging coverage data..."
        llvm-profdata merge -sparse "$COVERAGE_DIR"/*.profraw -o "$COVERAGE_DIR/turbo.profdata"
        
        print_step "Generating coverage summary..."
        BINARIES=$(RUSTFLAGS="-C instrument-coverage" cargo test --tests --no-run --message-format=json \
            --workspace --exclude "*example*" --exclude "*turborepo-tests*" \
            | jq -r "select(.profile.test == true) | .filenames[]" | grep -v dSYM || true)
        
        OBJECT_ARGS=""
        while IFS= read -r binary; do
            if [[ -f "$binary" ]]; then
                OBJECT_ARGS="$OBJECT_ARGS --object $binary"
            fi
        done <<< "$BINARIES"
        
        llvm-cov report --instr-profile="$COVERAGE_DIR/turbo.profdata" \
            --ignore-filename-regex='/examples/' \
            --ignore-filename-regex='/turborepo-tests/' \
            $OBJECT_ARGS
        ;;
    *)
        print_step "Running tests with coverage instrumentation..."
        cd "$PROJECT_ROOT"
        RUSTFLAGS="-C instrument-coverage" LLVM_PROFILE_FILE="$COVERAGE_DIR/turbo-%m-%p.profraw" \
            cargo test --tests --workspace --exclude "*example*" --exclude "*turborepo-tests*"
        
        print_step "Merging coverage data..."
        llvm-profdata merge -sparse "$COVERAGE_DIR"/*.profraw -o "$COVERAGE_DIR/turbo.profdata"
        
        print_step "Generating HTML coverage report..."
        BINARIES=$(RUSTFLAGS="-C instrument-coverage" cargo test --tests --no-run --message-format=json \
            --workspace --exclude "*example*" --exclude "*turborepo-tests*" \
            | jq -r "select(.profile.test == true) | .filenames[]" | grep -v dSYM || true)
        
        OBJECT_ARGS=""
        while IFS= read -r binary; do
            if [[ -f "$binary" ]]; then
                OBJECT_ARGS="$OBJECT_ARGS --object $binary"
            fi
        done <<< "$BINARIES"
        
        llvm-cov show --format=html --output-dir="$COVERAGE_DIR/html" \
            --instr-profile="$COVERAGE_DIR/turbo.profdata" \
            --ignore-filename-regex='/examples/' \
            --ignore-filename-regex='/turborepo-tests/' \
            $OBJECT_ARGS
        
        print_success "Coverage report generated at $COVERAGE_DIR/html/index.html"
        ;;
esac