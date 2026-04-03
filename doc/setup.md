# Development Setup

**Requirements:** Rust 1.85+ (stable). Install via [rustup](https://rustup.rs/).

## Clone and build

```sh
git clone https://github.com/DracoWhitefire/plumbob.git
cd plumbob
cargo build
```

## Running checks

```sh
cargo fmt --check
cargo clippy --features std -- -D warnings
cargo rustdoc --features std -- -D missing_docs
```

## Running tests

```sh
cargo test --features std                            # std (default for tests)
cargo build --no-default-features --features alloc  # alloc-only build check
cargo build --no-default-features                   # bare no_std build check
```

## Measuring coverage

Coverage requires [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov):

```sh
cargo install cargo-llvm-cov
cargo llvm-cov --features std
```

The current baseline is stored in `.coverage-baseline`. CI fails if coverage drops more
than 0.1% below it. On pushes to `main` or `develop`, an improvement automatically opens
a `ci/coverage-ratchet` PR to commit the new baseline.

## Running the simulate example

```sh
cd examples/simulate
cargo run
```
