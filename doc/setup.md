# Development Setup

**Requirements:** Rust 1.85+ (stable). Install via [rustup](https://rustup.rs/).

## Clone and build

```sh
git clone https://github.com/ashlar-display/plumbob.git
cd plumbob
cargo build
```

## Running checks

```sh
cargo fmt --check
cargo clippy --all-targets --features std -- -D warnings
cargo clippy --all-targets --features alloc -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo rustdoc --all-features -- -D warnings
```

## Running tests

```sh
cargo test --features std         # with the traced variant and the sync driver's std parts
cargo test --features alloc       # alloc only
cargo test --no-default-features  # bare no_std
```

The README's examples compile as doctests with the others. See [testing.md](testing.md)
for what the tests cover.

## `no_std` builds

The crate is `no_std`, with or without `alloc`. CI checks both on a bare target, which has
no `std` for a dependency to pull in:

```sh
rustup target add thumbv7em-none-eabi
cargo build --no-default-features --features alloc --target thumbv7em-none-eabi
cargo build --no-default-features --target thumbv7em-none-eabi
```

## Checking the MSRV

The MSRV is `rust-version` in `Cargo.toml`. CI checks it with
[`cargo-msrv`](https://github.com/foresterre/cargo-msrv), which runs the checks on that
toolchain:

```sh
cargo install cargo-msrv --locked
cargo msrv verify -- cargo test --locked --features std
cargo msrv verify -- cargo test --locked --features alloc
cargo msrv verify -- cargo test --locked --no-default-features
cargo msrv verify --target thumbv7em-none-eabi \
  -- cargo build --locked --no-default-features --features alloc --target thumbv7em-none-eabi
cargo msrv verify --target thumbv7em-none-eabi \
  -- cargo build --locked --no-default-features --target thumbv7em-none-eabi
```

## Measuring coverage

Coverage requires [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov):

```sh
cargo install cargo-llvm-cov --locked
cargo llvm-cov --locked --features std
```

The current baseline is stored in `.coverage-baseline`. CI fails if coverage drops more
than 0.1 percentage points below it. On pushes to `main` or `develop`, an improvement
automatically opens a `ci/coverage-ratchet` PR to commit the new baseline.

## Running the simulate example

```sh
cargo run --locked --manifest-path examples/simulate/Cargo.toml
```

The example is its own workspace with a tracked `Cargo.lock`, which records plumbob's
version and dependencies through the path dependency. After changing `Cargo.toml`, refresh
it and commit it, or the `--locked` run (here and in CI) fails:

```sh
cargo check --manifest-path examples/simulate/Cargo.toml
```
