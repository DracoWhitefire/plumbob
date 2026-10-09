# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Breaking changes

- **The training state machine follows the HDMI 2.1 link training states.** Training now
  runs LTS:2 (prepare) → LTS:3 (train) → LTS:P (pass), with LTS:4 (lower rate) on the
  sink's request and LTS:L (exit to TMDS) on every failure. The previous four-phase
  sequence waited for `FRL_start` before the LTP loop, which a real sink never completes.
- **`ScdcClient` has one method per register operation**: `read_flt_ready`,
  `read_update_flags`, `clear_update_flags`, `read_ltp_requests`,
  `read_source_test_config`, `write_config_0_defaults`, `write_frl_config` and `read_ced`.
  `read_training_status` and `TrainingStatus` are removed. Implementations still enforce
  the poll interval (2 ms by default) inside the polled methods.
- **`LtpReq` has the HDMI 2.1 request values** — 0x0 (none), 0x1–0x8 (all ones, all
  zeros, Nyquist clock, DDE compliance, LFSR 0–3), 0xE (`FfeChange`) and 0xF
  (`RateChange`) — and is read per lane as `LtpRequests`. The previous values (1–4 =
  LFSR 0–3) were wrong. `From<LtpReq> for LtpPattern` is replaced by `LtpReq::pattern()`.
- **`FfeLevels` is a level index, 0–7**, constructed with `FfeLevels::new` instead of
  `Ffe0`–`Ffe7` variants, and written to `Config_1` limited for the rate
  (`FfeLevels::limited_to`: at most 3 up to 12 Gbps).
- **`FrlConfig` is `Config_1`**: the rate and FFE levels. `dsc_frl_max` is removed, as is
  `TrainingConfig::dsc_frl_max`.
- **`TrainingConfig` has poll limits per state**: `flt_ready_polls` (default 50),
  `ltp_polls` (100), `frl_start_polls` (100) and `no_timeout_poll_cap` (500), replacing
  `flt_ready_timeout`, `frl_start_timeout` and `ltp_timeout` (each 1000), plus
  `max_retrains` (3). The defaults reproduce the spec's 100 ms and 200 ms timeouts and the
  AMD and Intel drivers' 200 ms `FRL_start` wait at one poll every 2 ms; the
  `FLT_no_timeout` cap and the retrain count follow the AMD driver.
- **`TrainingOutcome::FallbackRequired` carries a `reason`** (`FallbackReason`:
  `FltReadyTimeout`, `TrainingTimeout`, `FrlStartTimeout`, `RatesExhausted` or
  `RetrainsExhausted`), and every fallback leaves the sink and PHY in TMDS.
- **`HdmiPhy` calls follow hdmi-hal's per-lane model**: `send_ltp` receives the full
  per-lane pattern set, `adjust_equalization` the per-lane TxFFE levels, and
  `set_frl_output(GapOnly)` is sent during training and LTS:P.
- **`TrainingEvent` records the new states** (`FltReady`, `RateConfigured`,
  `LtpRequested`, `FfeRaised`, `TrainingPassed`, `RateLowered`, `RatesExhausted`,
  `RetrainRequested`, `RetrainsExhausted`, `FrlStart`, `ExitedToTmds`, the three timeouts and
  `SourceTestConfigRead`), and `TrainingTrace` records the list of rates instead of a
  single `rate`.

### Added

- **`FrlTrainer::train(rates, config)`** — trains over a caller-supplied list of rates,
  stepping down when the sink requests a lower rate (LTS:4). `train_at_rate` is `train`
  with one rate. `train_traced` is its traced form, alongside `train_at_rate_traced`.
- **Per-lane training** — plumbob tracks each lane's pattern and TxFFE level: pattern
  requests apply per lane, 0xE raises the lane's TxFFE level up to the advertised maximum
  and holds it there, and a Nyquist clock request is honoured only under `FLT_no_timeout`.
- **`FLT_no_timeout` support** — when the sink sets it in `Source_Test_Configuration`, the
  LTS:2 and LTS:3 poll limits are suspended, up to `no_timeout_poll_cap`.
- **Retraining from LTS:P** — a `FLT_update` before `FRL_start` returns to LTS:3, up to
  `TrainingConfig::max_retrains` times per call (default 3); the next request ends the
  attempt with `RetrainsExhausted`, so training always terminates.
- **`plumbob::lts`** — the state machine as one I/O-free `async fn`, `lts::run`, over
  the `TrainingIo` trait (the SCDC and PHY operations training performs). `FrlTrainer`
  drives it synchronously without an async runtime (one poll with `Waker::noop`), and
  `plumbob-async` drives the same function asynchronously, so the two cannot drift.
  Implementing `TrainingIo` drives training from any other environment.
- **`TrainingEvent` is available without the `alloc` feature**, for `lts::run`'s event
  callback; only `TrainingTrace` and the traced methods need `alloc`.
- `LtpRequests`, `UpdateFlags` and `SourceTestConfig` — the per-lane requests, the
  `Update_0` flags and the `Source_Test_Configuration` field the state machine uses.

### Changed

- **`display-types` updated to 0.4** — tracks DisplayID 2.x support added in `piaf` 0.4.1.
- **`hdmi-hal` updated to 0.5** — the release with the per-lane `send_ltp`,
  `set_frl_output` and the HDMI 2.1 `LtpPattern` values that training uses (see
  *Breaking changes*). plumbob 0.1.3 does not build against hdmi-hal 0.4.1, which moved
  to display-types 0.4; this release uses display-types 0.4 throughout.

### Internal

- **CI builds for a `no_std` target** — the `Build (no_std)` and `Build (alloc only)`
  steps now build for `thumbv7em-none-eabi`. They previously built for the host, where
  `std` is always available, so they could not catch a dependency that enables `std` (as
  hdmi-hal did through `display-types`).
  The publish workflow runs the same build steps.
- **Automated publish can be triggered by `release-tag`** — `publish.yml` gains a
  `workflow_dispatch` trigger. Tags pushed with `GITHUB_TOKEN` do not start push-triggered
  workflows, so `release-tag`'s "Trigger publish workflow" step
  (`gh workflow run publish.yml`) could not start a publish run. Dispatches against a
  non-tag ref (e.g. `main`) are skipped, so they cannot publish or create a release.

## [0.1.3] - 2026-04-13

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
