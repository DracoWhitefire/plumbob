# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **SLSA Build Level 2 provenance** — release artifacts are attested via
  `actions/attest-build-provenance` and verified with
  `gh attestation verify <file> --repo DracoWhitefire/plumbob`.

### Changed

- Updated `hdmi-hal` dependency from `0.3.0` to `0.4.0`.

## [0.1.2] - 2026-04-04

- `TrainingTrace::new(rate, config, events)` — readded constructor.


## [0.1.1] - 2026-04-04

### Added

- `TrainingTrace::new(rate, config, events)` — constructor for `TrainingTrace`, required
  because the struct is `#[non_exhaustive]` and cannot be created by expression outside this
  crate. Companion crates such as `plumbob-async` that run their own training loop and produce
  a trace depend on this constructor.

## [0.1.0] - 2026-04-03

### Added

**FRL training state machine**
- `FrlTrainer<C, P>` — central type owning an `ScdcClient` and `HdmiPhy`; reusable across
  rate-fallback attempts without reconstruction between calls
- `FrlTrainer::train_at_rate` — runs the full four-phase FRL training sequence at a given
  `HdmiForumFrl` rate, returning a `TrainingOutcome` or a hard `TrainingError`
- `TrainingConfig` — per-attempt configuration covering FFE levels, `dsc_frl_max` flag, and
  independent iteration-count timeouts for each polling phase
- `TrainingOutcome` — `Success { achieved_rate }` on convergence;
  `FallbackRequired` when any phase times out without satisfying its condition
- `TrainingError<ScdcErr, PhyErr>` — hard I/O error, distinct from a soft fallback outcome
- Four sequential training phases: configuration write (phase 1), `flt_ready` readiness polling
  (phase 2), `frl_start` initiation polling (phase 3), LTP pattern loop (phase 4)
- LTP transition detection: `LtpPatternRequested` is emitted only when `ltp_req` changes, not
  on every poll iteration

**`ScdcClient` trait**
- `write_frl_config`, `read_training_status`, `read_ced` — the three register-group operations
  the training procedure requires, with an associated `Error` type
- Bus-level error handling, register-to-field mapping, validity-bit interpretation, and
  inter-poll delay are all delegated to the implementer

**Types**
- `LtpReq` — sink LTP pattern request (`None`, `Lfsr0`–`Lfsr3`) with `From<LtpReq> for
  LtpPattern` conversion for direct use with `HdmiPhy::send_ltp`
- `FfeLevels` — FFE level count (`Ffe0`–`Ffe7`) advertised in Config_0
- `FrlConfig` — rate, FFE levels, and `dsc_frl_max` flag written to the sink in phase 1
- `TrainingStatus` — `flt_ready`, `frl_start`, and `ltp_req` fields decoded per status poll
- `CedCount` — 15-bit character error count; `new` masks off the validity flag (`bits[14:0]`)
- `CedCounters` — per-lane `Option<CedCount>`; `lane3` is only populated in 4-lane FRL mode

**Training trace** (`alloc` feature)
- `TrainingTrace` — records the `HdmiForumFrl` rate, `TrainingConfig`, and full ordered
  `TrainingEvent` sequence for a single training attempt
- `FrlTrainer::train_at_rate_traced` — traced variant returning `(TrainingOutcome, TrainingTrace)`
- `TrainingEvent` variants: `RateConfigured`, `FltReadyReceived`, `FltReadyTimeout`,
  `FrlStartReceived`, `FrlStartTimeout`, `LtpPatternRequested`, `AllLanesSatisfied`,
  `LtpLoopTimeout`

**`no_std` support**
- `#![no_std]` throughout; the core training path requires no heap allocation
- `alloc` feature enables `TrainingTrace` and `train_at_rate_traced`; `std` implies `alloc`

**Robustness and safety**
- `#![forbid(unsafe_code)]`
- All polling loops bounded by configurable `u32` iteration counts with an explicit per-iteration
  check; no unbounded loops
- `#![deny(missing_docs)]` with full rustdoc coverage enforced in CI

**Developer experience**
- Simulation example (`examples/simulate`) demonstrating rate fallback with scripted SCDC and
  PHY stubs
- CI: `cargo test`, `cargo clippy -D warnings`, `cargo rustdoc -D missing_docs`,
  `cargo fmt --check`, and `cargo build` across all feature flag combinations
- Coverage ratchet: line coverage measured with `cargo-llvm-cov`; baseline stored in
  `.coverage-baseline`; CI fails on regression
- Dependency audit: `rustsec/audit-check` runs on every push
