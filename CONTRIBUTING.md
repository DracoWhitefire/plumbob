# Contributing to plumbob

Thanks for your interest in contributing. This document covers the basics.

## Getting started

Relevant docs for contributors:

- [`doc/setup.md`](doc/setup.md) — build, test, and fuzzing commands
- [`doc/testing.md`](doc/testing.md) — testing strategy, sim harness, and CI expectations
- [`doc/architecture.md`](doc/architecture.md) — pipeline structure, scope, and design principles
- [`doc/model.md`](doc/model.md) — data types, field conventions, and the ownership boundary between `ScdcClient` and the state machine
- [`doc/roadmap.md`](doc/roadmap.md) — planned features and known gaps

## Issues and pull requests

**Open an issue first** if you're unsure whether something is a bug or if you want to
discuss a change before implementing it. For small, self-contained fixes a PR on its own
is fine.

- Bug reports: include a `TrainingTrace` if available, or describe the SCDC register sequence and the phase where training failed.
- Feature requests: a brief description of what you need and why is enough to start a
  conversation.
- PRs: keep them focused. One logical change per PR makes review faster and keeps history
  readable.

## Coding standards

- Run `cargo fmt` and `cargo clippy --all-targets --features std -- -D warnings` before
  pushing; CI runs clippy in every feature set ([doc/setup.md](doc/setup.md) lists the
  commands).
- Public items need rustdoc comments, and intra-doc links must resolve
  (`cargo rustdoc --all-features -- -D warnings` must pass).
- Follow the existing patterns in the codebase — see [`doc/architecture.md`](doc/architecture.md)
  and [`doc/model.md`](doc/model.md) for the design principles behind them.
- `#![forbid(unsafe_code)]` is enforced; no unsafe code.
- A claim about HDMI behaviour (a rule, a default, a threshold, a register encoding) cites
  at least two independent public implementations (repository, commit, file and line).
  Where implementations differ, the behaviour is a setting with the majority as default.
- Keep `no_std` compatibility. `FrlTrainer` and all owned protocol types must compile
  without `alloc` or `std`.

## Commit and PR expectations

- Write commit messages in the imperative mood ("Add support for …", not "Added …").
- Keep commits logically atomic. A PR that touches three unrelated things should be three
  commits (or three PRs).
- Tests are expected for new decoding logic. A unit test with a handcrafted byte slice is
  usually sufficient; a real fixture capture is a bonus.
- CI must be green before a PR can merge: fmt, clippy, docs, the tests in every feature
  set, the simulate example, the bare-metal `no_std` builds, the same tests and builds on
  the MSRV (Rust 1.85), and coverage must not drop more than 0.1 percentage points below
  the baseline (stored in `.coverage-baseline`). New decoding logic without tests will
  likely trip this.

## Publishing and downstream dependencies

plumbob builds on [hdmi-hal](https://github.com/ashlar-display/hdmi-hal) and
[display-types](https://github.com/ashlar-display/display-types) and re-exports the types
its API uses (the PHY traits and value types, `HdmiForumFrl`), so a breaking release of
either is a breaking release of plumbob. Other
[Ashlar crates](https://ashlar-display.org/crates.html) depend on plumbob as a versioned
crate dependency. Changes that add or modify public API must be merged, tagged, and
published to crates.io before downstream crates can adopt them. The workflow is:

1. Merge and tag a release in this repo (`vX.Y.Z`).
2. The publish CI workflow uploads the new version to crates.io.
3. Update the version constraint in the downstream crate's `Cargo.toml`.

## Review process

PRs are reviewed on a best-effort basis. Expect feedback within a few days; if you haven't
heard back in a week feel free to ping the thread. Reviews aim to be constructive — if
something needs to change, the reviewer will explain why. Approval from the maintainer is
required to merge.

## Code of Conduct

This project follows the [Contributor Covenant 3.0](https://github.com/ashlar-display/.github/blob/HEAD/CODE_OF_CONDUCT.md),
the code of conduct of every [Ashlar](https://ashlar-display.org) repository. Please read it
before participating.
