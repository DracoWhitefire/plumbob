# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.1] - 2026-10-17

### Fixed

- **`lts::run`'s future is `Send` when its I/O is.** Since 0.2.0 it kept the sink's
  `FLT_no_timeout` in a `Cell` shared with the closure that records events, across
  `.await`s, so the future was never `Send`, whatever the `TrainingIo`: an async driver
  built on it, such as plumbob-async's `FrlTrainer`, could not be spawned on a
  multi-threaded executor. The flag is now a field read between the attempt's steps.
  Nothing else changes; a test checks at compile time that `lts::run` and
  `lts::exit_to_tmds` give `Send` futures over a `Send` I/O.

## [0.3.0] - 2026-10-13

### Breaking changes

- **Every struct variant of `TrainingOutcome`, `TrainingError`, `TrainingWarning` and
  `TrainingEvent` is `#[non_exhaustive]`**, so a later release can add a field to one
  without breaking callers. Patterns need `..` (`TrainingOutcome::Success { achieved_rate,
  .. }`), and code outside plumbob can no longer build these variants: tests that compare
  an outcome, error or trace with `assert_eq!` against a constructed value match on it
  instead (`matches!`, or a pattern with `..`).
- **`Trained`'s warnings are private, and its API no longer depends on the `alloc`
  feature.** The public `warnings` field (a `Vec` with `alloc`, an
  `[Option<TrainingWarning>; MAX_WARNINGS]` without) and `num_warnings` (without `alloc`)
  are gone; read the warnings with the new `warnings()` (a slice, in every build) or
  `iter_warnings()`. `Trained` is no longer `Copy` without `alloc`. Cargo enables a
  feature for every crate in a build that one of them asks for, so code written against a
  bare `no_std` plumbob used to stop compiling as soon as another dependency enabled
  `alloc`; now both builds have the same API and traits.
- **`FrlConfig::rate` is renamed `frl_rate`**, after the `Config_1` field (`FRL_Rate`) and
  culvert's `FrlConfig`, which plumbob's SCDC types now mirror.
- **`LtpRequests`, `CedCounters` and `FrlConfig` are `#[non_exhaustive]`**, with `new(…)`
  constructors (`LtpRequests::new` and `CedCounters::new` take the lanes in order,
  `FrlConfig::new` the rate and the FFE levels), mirroring culvert's types; `LtpRequests`
  also gains `all_trained()`, as in culvert. Code outside plumbob builds them with `new`
  instead of a struct expression (an `ScdcClient` returns `LtpRequests` and
  `CedCounters`); reading the fields is unchanged.
- **`UpdateFlags` has all seven `Update_0` flags, and clears take a `ClearableUpdateFlags`**,
  mirroring culvert. `UpdateFlags` gains `status_update`, `ced_update`, `rr_test` and
  `rsed_update`, so an `ScdcClient` reports the whole register; it is `#[non_exhaustive]`
  (room for a later flag in bit 7), with `UpdateFlags::new` (flags in bit order) and
  `clearable()`. `ScdcClient::clear_update_flags` takes the new `ClearableUpdateFlags`:
  every flag but `RR_Test`, which the source never clears. plumbob still acts on, and
  clears, only `source_test_update`, `frl_start` and `flt_update`. Implementations of
  `ScdcClient` change `clear_update_flags`'s signature, and fill every `UpdateFlags` field
  in `read_update_flags` (with `new`, or by setting fields on the default). The public
  `lts::TrainingIo` changes the same way: `clear_update_flags` takes a
  `ClearableUpdateFlags`.
- **`SourceTestConfig` has all six `Source_Test_Configuration` bits, and the trace records
  them**, mirroring culvert. It gains `txffe_pre_shoot_only`, `txffe_de_emphasis_only`,
  `txffe_no_ffe`, `dsc_frl_max` and `frl_max`, is `#[non_exhaustive]`, and has
  `SourceTestConfig::new` (bits in bit order). `TrainingEvent::SourceTestConfigRead` carries
  the register as read (`config`) instead of `flt_no_timeout` alone, so the trace shows
  everything a compliance tester set. plumbob acts on `flt_no_timeout` as before;
  `frl_max` and `dsc_frl_max` are for the layer that builds the rate list.

### Added

- **The tester's TxFFE restrictions reach the PHY.** The three TxFFE bits of
  `Source_Test_Configuration` (pre-shoot only, de-emphasis only, no FFE) go to
  `HdmiPhy::adjust_equalization` as hdmi-hal's `TxFfeRestrictions`, with every
  equalization of the attempt; when a `Source_Test_Update` seen on any LTS:3 or LTS:P
  poll changes them, plumbob re-sends the lanes' levels with the new restrictions at once.
  A `Source_Test_Update` is now handled on every such poll, also without `FLT_update` and
  in LTS:P before `FRL_start` can end training; previously LTS:3 only looked at it
  together with `FLT_update`, and LTS:P not at all, which also delayed `FLT_no_timeout`
  changes. How to combine the bits is
  the PHY's (see hdmi-hal 0.5.1). `TxFfeRestrictions` is re-exported. Requires hdmi-hal
  0.5.1.

### Changed

- **Stopping training early is documented as the `ScdcClient`'s job.** A wall-clock
  deadline, a cancel flag or any other reason to stop belongs in the polled reads, which
  already wait the poll interval: fail one of them with an error of your own and let
  every later call through, and `train` stops, returns the link to TMDS through LTS:L and
  reports `TrainingError::Scdc` with that error (unless `exit_to_tmds_on_error` is off, or
  the sink sets `FLT_no_timeout`). LTS:L's own `read_update_flags` is the same method, so a
  client never fails a read made after `write_frl_config` with `NotSupported`, LTS:L's
  first write. `ScdcClient`'s documentation, the README and architecture.md ("How long one
  `train` call can take") describe it, and tests cover it, including a deadline that
  lands on LTS:L's read. plumbob gets no separate stop hook: the poll interval, the poll limits,
  `no_timeout_poll_cap` and `max_retrains` already bound a call.
- **The documentation describes the SCDC types as mirrored**: architecture.md ("Type
  ownership and the culvert boundary") and model.md no longer list differences from
  culvert's types, and architecture.md says where `FRL_Max` and `DSC_FRL_Max` are handled
  (rate selection, before training; with both set, Amlogic and NVIDIA act on neither and
  AMD trains at the maximum rate). `doc/roadmap.md` drops its "Released" section (this changelog is the record)
  and the finished "Mirrored SCDC types" entry.

## [0.2.0] - 2026-10-11

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
  plumbob's levels; LTS:4 finishes as LTS:2 does, restoring the patterns, gap characters
  and `Config_0` after the rate change as well. plumbob sends no pattern
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
  each end's LTS:L error, `Skipped`, or `HeldUnderTest` while the sink sets
  `FLT_no_timeout`, since the test equipment is then in control). A fallback whose LTS:L fails returns the new
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
- **`display-types` updated to 0.4** — tracks DisplayID 2.x support added in `piaf` 0.4.1.
- **`hdmi-hal` updated to 0.5** — the release with the per-lane `send_ltp`,
  `set_frl_output` and the HDMI 2.1 `LtpPattern` values that training uses (see
  above). plumbob 0.1.3 does not build against hdmi-hal 0.4.1, which moved
  to display-types 0.4; this release uses display-types 0.4 throughout.

### Added

- **`LtpReq::from_value`** — the request for a raw 4-bit value (`None` above 0xF), for
  `ScdcClient` implementations and simulators: it gives every value exactly one request,
  `Reserved` only for 0x9–0xD.
- **`FrlTrainer::phy`, `phy_mut`, `scdc` and `scdc_mut`** — access to the PHY and SCDC
  client the trainer holds, for example to start video after `Success` without
  `into_parts`.
- Re-exports of `HdmiForumFrl`, `HdmiPhy` and the PHY types (`EqParams`, `LaneEqParams`,
  `LanePatterns`, `FrlOutput`, `LtpPattern`, `TxFfeLevel`), so callers and PHY
  implementations need no direct `display-types` or `hdmi-hal` dependency and cannot end
  up with versions other than plumbob's.
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

### Internal

- **The training future has a size budget** — a test keeps the `lts::run` future
  within 416 bytes without `alloc` (392 with it), and CI and the publish workflow now also
  run the tests with no features, so both budgets are checked.
- **CI builds for a `no_std` target** — the `Build (no_std)` and `Build (alloc only)`
  steps now build for `thumbv7em-none-eabi`. They previously built for the host, where
  `std` is always available, so they could not catch a dependency that enables `std` (as
  hdmi-hal did through `display-types`).
  The publish workflow runs the same build steps.
- **Automated publish can be triggered by `release-tag`** — `publish.yml` gains a
  `workflow_dispatch` trigger. Tags pushed with `GITHUB_TOKEN` do not start push-triggered
  workflows, so the tag `release-tag` pushes never started a publish run; `release-tag`
  now dispatches it (see below). Dispatches against a non-tag ref (e.g. `main`) are
  skipped, so they cannot publish or create a release.
- **Publishing uses crates.io trusted publishing** — `publish.yml` obtains a short-lived
  token through OIDC (`rust-lang/crates-io-auth-action`, revoked at the end of the job, and
  short-lived regardless) instead of the long-lived `CARGO_REGISTRY_TOKEN` secret. The
  upload job runs in the `release` environment, which only `v*` tags can deploy to and
  which requires a maintainer's approval. Quality gates moved to a separate `verify` job,
  so no dependency build scripts run and no cache is restored where publishing credentials
  exist. The attested `.crate` is kept as a separate copy; after upload the job fails if
  crates.io's checksum differs from it, and that same copy is attached to the GitHub
  release. Re-running a partially failed publish reuses an existing GitHub release.
  `publish.yml` refuses tags that do not match the `Cargo.toml` version.
- **Release summary before approval** — `publish.yml` gains a `summary` job that shows the
  crate, version and commit, changes since the previous published release (ignoring
  pre-release or malformed tags, and flagging a baseline that is not on crates.io), changes
  to workflow, toolchain and build configuration, manifest changes, the changelog section,
  and the `.crate` sha256 with a command to reproduce it, on the run page while the publish
  job waits for approval. The publish job refuses to attest a package whose sha256 differs
  from the reviewed one.
- **`release-tag` no longer tags after a failed promotion** — `release-tag` now promotes
  `develop` to `main` through a `release-promote/v*` branch and PR. A failed merge of the
  promote PR used to go unnoticed, after which the previous `main` commit was tagged with
  the new version. The step now fails, and tagging requires `main` to hold exactly the
  promoted tree. It then dispatches `publish.yml` for the tag (it needed `actions: write`
  for that), and it also accepts a squash-merged release commit (`Release vX.Y.Z (#N)`).
- **Release workflows only run from `develop`** — `release-prep` and `release-tag` fail when
  started from another branch, so an outdated copy of the workflow is not run against
  `develop` by mistake.
- **`release-prep` no longer edits the README** — its "Update README.md dependency pin" step
  matched only one of the four version pins in the README and docs. They are `cargo add`
  commands now, which need no version, and the step was removed.
- **No CI run on promotion PRs** — CI no longer runs on pull requests into `main`. The only
  ones are `release-tag`'s `release-promote/*` PRs, whose content already passed CI on
  `develop`; opened by `github-actions[bot]`, their runs would be held for approval and
  listed as `action_required` or failed.
- **Supply-chain hardening for CI** — every third-party action is pinned to a commit SHA
  of a release tag, with `dependabot.yml` keeping the pins (and both `Cargo.lock`s, the
  crate's and the `simulate` example's) current. The Rust toolchain itself still follows
  `stable`. `cargo-semver-checks` is installed as a checksum-verified prebuilt binary (the
  version listed by the pinned `install-action`) instead of being restored from a cached
  `~/.cargo/bin`. The security audit also runs weekly.
- **README documents release provenance** — a badge shows whether crates.io requires trusted
  publishing for this crate, and "Verifying releases" now states which versions carry SLSA
  provenance (from 0.1.3, not every release as it previously claimed), which are uploaded
  through trusted publishing (from 0.2.0) or were uploaded with an API token (up to 0.1.3),
  and how to look up a version's publishing details on crates.io.

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
