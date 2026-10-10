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
  The undefined values 0x9–0xD are `LtpReq::Reserved(value)` rather than an error: LTS:3
  leaves a lane that requests one as it was (as the Xilinx and Intel drivers do),
  records `TrainingEvent::UndefinedLtpRequest` and returns a `TrainingWarning`, so a
  stray value, such as on lane 3 at a 3-lane rate, no longer ends training.
- **`train` and `train_at_rate` return `Trained`**: the `TrainingOutcome` with the
  `TrainingWarning`s the attempt produced, as piaf, cartouche and concordance return
  warnings with their results. Read the outcome as `trained.outcome` and the warnings
  with `trained.iter_warnings()`.
- **`FfeLevels` is a level index, 0–7**, constructed with `FfeLevels::new` instead of
  `Ffe0`–`Ffe7` variants, and written to `Config_1` limited for the rate
  (`FfeLevels::limited_to`: at most 3 up to 12 Gbps). It has `FfeLevels::MAX` and
  `value()`, and derives `Default` (0), `PartialOrd` and `Ord`.
- **`FrlConfig` is `Config_1`**: the rate and FFE levels. `dsc_frl_max` is removed, as is
  `TrainingConfig::dsc_frl_max`.
- **`TrainingConfig` has poll limits per state**: `flt_ready_polls` (default 50),
  `ltp_polls` (100), `frl_start_polls` (100) and `no_timeout_poll_cap` (500), replacing
  `flt_ready_timeout`, `frl_start_timeout` and `ltp_timeout` (each 1000), plus
  `max_retrains` (3). The defaults reproduce the spec's 100 ms and 200 ms timeouts and the
  AMD and Intel drivers' 200 ms `FRL_start` wait at one poll every 2 ms; the values of
  the `FLT_no_timeout` cap and the retrain count follow the AMD driver. `ffe_levels`
  defaults to 3, as the AMD and Intel drivers advertise; set it to 0 for a PHY that
  cannot apply TxFFE.
- **`TrainingOutcome::FallbackRequired` carries a `reason`** (`FallbackReason`:
  `FltReadyTimeout`, `TrainingTimeout`, `FrlStartTimeout`, `RatesExhausted` or
  `RetrainsExhausted`), and every fallback leaves the sink and PHY in TMDS. LTS:L attempts
  every step even when one fails, so an error on one end does not keep the other in FRL.
- **`HdmiPhy` calls follow hdmi-hal's per-lane model**: `send_ltp` receives the full
  per-lane pattern set, `adjust_equalization` the per-lane TxFFE levels, and
  `set_frl_output(GapOnly)` is sent during training and LTS:P. TxFFE levels are reset
  after `set_frl_rate`, not before, so a PHY that resets its lanes on a rate change keeps
  plumbob's levels. plumbob sends no pattern
  in LTS:2: any bring-up the transmitter needs, such as a clock pattern held until its PLL
  locks, belongs to the PHY's `set_frl_rate`.
- **`TrainingEvent` records the new states** (`FltReady`, `RateConfigured`,
  `LtpRequested`, `FfeRaised`, `TrainingPassed`, `RateLowered`, `RatesExhausted`,
  `RetrainRequested`, `RetrainsExhausted`, `FrlStart`, `ExitedToTmds`, the three timeouts and
  `SourceTestConfigRead`). They replace `FltReadyReceived`, `FrlStartReceived`,
  `LtpPatternRequested`, `AllLanesSatisfied` and `LtpLoopTimeout`, and the timeout events
  count `polls` instead of `iterations_elapsed`.
- **`TrainingError` reports what happened to the link.** An SCDC or PHY error is now
  followed by LTS:L, returning both ends to TMDS as a fallback does, and the variants
  are struct variants carrying the outcome: `Scdc { error, exit }` and
  `Phy { error, exit }`, with `exit` a `TmdsExit` (`Exited`, `Failed(ExitError)` with
  each end's LTS:L error, or `Skipped`). A fallback whose LTS:L fails returns the new
  `ExitFailed { reason, error }` instead of `FallbackRequired`. A rate list the
  procedure cannot run — `NotSupported`, or a rate not strictly lower than the one before
  it, since LTS:4 steps down — is rejected before any I/O with the new
  `InvalidRates { index, rate }`, and an empty list with the new `NoRates` (it used to
  return `FallbackRequired { reason: RatesExhausted }` without touching the sink, so
  `RatesExhausted` now always means the sink asked past the end of the list, and every
  `FallbackRequired` leaves both ends in TMDS). `TrainingError` is now
  `#[non_exhaustive]`.
- **The traced methods return the trace whatever the result**: `train_traced` and
  `train_at_rate_traced` return `(Result<Trained, TrainingError>, TrainingTrace)` instead
  of `Result<(TrainingOutcome, TrainingTrace), TrainingError>`, so an attempt that ended
  in an error can be explained from its events.
- **`TrainingTrace` records the list of rates**: its `rate` field is replaced by
  `rates: Vec<HdmiForumFrl>`, and `TrainingTrace::new` takes the rates instead of a
  single rate.

### Added

- **`FrlTrainer::train(rates, config)`** — trains over a caller-supplied list of rates,
  stepping down when the sink requests a lower rate (LTS:4). `train_at_rate` is `train`
  with one rate. `train_traced` is its traced form, alongside `train_at_rate_traced`.
- **Per-lane training** — plumbob tracks each lane's pattern and TxFFE level: pattern
  requests apply per lane, 0xE raises the lane's TxFFE level up to the advertised maximum
  and holds it there, and a Nyquist clock request is honoured only under `FLT_no_timeout`.
- **`FLT_no_timeout` support** — when the sink sets it in `Source_Test_Configuration`, it
  is under compliance test: the LTS:2, LTS:3 and LTS:P limits are replaced by
  `no_timeout_poll_cap`, and when that runs out the attempt returns the new
  `TrainingOutcome::NoTimeoutHold { rate }` (event `NoTimeoutCapReached`) without leaving
  FRL, so plumbob never ends a test link on its own timer. The retrain bound
  (`max_retrains`) still applies and still falls back: it is not a timer but what ends
  the LTS:P ↔ LTS:3 cycle. The register is
  read at the start of every attempt, so a setting left in place applies to retries and
  retrains as well, not only to the attempt that saw `Source_Test_Update`.
- **Retraining from LTS:P** — a `FLT_update` before `FRL_start` returns to LTS:3, up to
  `TrainingConfig::max_retrains` times per call (default 3); the next request ends the
  attempt with `RetrainsExhausted`, so training always terminates. LTS:3 resumes from the
  state LTS:P left: no pattern on any lane, TxFFE levels kept. If the sink sets
  `FRL_start` together with `FLT_update`, the retrain wins (`FrlStartWithRetrain`).
- **`plumbob::lts`** — the state machine as one I/O-free `async fn`, `lts::run`, over
  the `TrainingIo` trait (the SCDC and PHY operations training performs). `FrlTrainer`
  drives it synchronously without an async runtime (one poll with `Waker::noop`), and
  `plumbob-async` drives the same function asynchronously, so the two cannot drift.
  Implementing `TrainingIo` drives training from any other environment.
- **`TrainingEvent` is available without the `alloc` feature**: `train_with_events`
  calls a callback with each event as it occurs (as does `lts::run`), so the full
  reasoning is available on targets without an allocator; only `TrainingTrace` and the
  traced methods need `alloc`. `TrainingEvent` now derives `Copy`.
- `LtpReq::value()` — a request's 4-bit value.
- **`TrainingWarning`**, **`Trained`** and **`MAX_WARNINGS`** (5) — non-fatal anomalies
  returned with the outcome, starting with `UndefinedLtpRequest { lane, value, count,
  in_use }`. Repeats are merged, so an attempt produces at most five and none are
  dropped without `alloc`.
- **`TrainingConfig::exit_to_tmds_on_error`** (default `true`) — turn the LTS:L after an
  error off to leave the sink and PHY as the error left them (`TmdsExit::Skipped`).
- **`FrlTrainer::exit_to_tmds`** — LTS:L on demand, to take an FRL link down when the
  display is disabled or unplugged, before a mode change, or after an error with
  `exit_to_tmds_on_error` off. `lts::exit_to_tmds` is the same over `TrainingIo`, with
  events.
- `TmdsExit`, `ExitError` (each end's first LTS:L error) and
  `TrainingEvent::ExitToTmdsFailed { scdc, phy }`, recording which ends' LTS:L steps
  failed.
- `LtpRequests`, `UpdateFlags` and `SourceTestConfig` — the per-lane requests, the
  `Update_0` flags and the `Source_Test_Configuration` field the state machine uses.

### Changed

- **`display-types` updated to 0.4** — tracks DisplayID 2.x support added in `piaf` 0.4.1.
- **`hdmi-hal` updated to 0.5** — the release with the per-lane `send_ltp`,
  `set_frl_output` and the HDMI 2.1 `LtpPattern` values that training uses (see
  *Breaking changes*). plumbob 0.1.3 does not build against hdmi-hal 0.4.1, which moved
  to display-types 0.4; this release uses display-types 0.4 throughout.

### Internal

- **The training future has a size budget** — a test keeps the `lts::run` future
  within 408 bytes without `alloc` (384 with it), and CI and the publish workflow now also
  run the tests with no features, so both budgets are checked.
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
