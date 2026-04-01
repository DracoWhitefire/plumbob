# Architecture

## Role

`plumbob` implements the Fixed Rate Link (FRL) training state machine defined in the HDMI
2.1 specification. It defines the interface its dependencies must satisfy rather than
depending on any specific SCDC implementation, and it is itself replaceable by any crate
that implements the `LinkTrainer` trait defined by the integration layer above it.

Training establishes actual link capability, not just theoretical negotiation. A
`NegotiatedConfig` from concordance identifies what the hardware should support; link
training determines what it actually achieves. The caller is responsible for deciding what
to do with a `FallbackRequired` outcome — whether to retry at a lower tier, fall back to
TMDS, or surface the failure.

plumbob is usable as a standalone crate without the broader concordance/piaf stack. A
firmware image, an embedded hardware vendor's SDK, or a kernel driver can depend directly
on plumbob, supply their own `ScdcClient` and `HdmiPhy` implementations, and call
`train_at_rate` without any negotiation layer above it. The concordance integration is one
deployment model, not a requirement.

---

## Scope

plumbob covers:

- the FRL link training state machine: four-phase training per HDMI 2.1 §10.x,
- `ScdcClient`: the typed SCDC interface trait, defined here and implemented by SCDC crates,
- `FrlTrainer<C, P>`: the central type, owning an `ScdcClient` and a PHY,
- `TrainingOutcome`: the result of a single training attempt (`Success` or `FallbackRequired`),
- `TrainingConfig`: per-attempt configuration (FFE levels, iteration limits),
- `TrainingError`: hard failures (transport or protocol errors),
- owned protocol types: `LtpReq`, `FfeLevels`, `TrainingStatus`, `CedCounters`,
- simulation support: the training procedure is fully exercisable without real hardware
  by using simulated implementations of `ScdcClient` and `HdmiPhy`.

The following are out of scope:

- **Rate fallback policy** — `train_at_rate` returns `FallbackRequired`; the caller
  decides the retry sequence. plumbob does not maintain a rate table or retry loop.
- **SCDC register decoding** — plumbob reads typed values from `ScdcClient` and does not
  decode raw register bytes or know SCDC register addresses.
- **PHY vendor sequences** — plumbob calls `HdmiPhy` methods; the register sequences
  for lane reconfiguration are in platform PHY backends.
- **Timing** — plumbob is synchronous and poll-based. No sleep, no timers. Timing
  between polls is implicit in the transport; the iteration limits in `TrainingConfig`
  are the only timeout mechanism.
- **TMDS link setup** — plumbob handles FRL training only. TMDS mode is the fallback
  that concordance selects if no FRL tier trains successfully; plumbob has no role in it.
- **CED-driven equalization** — CED counter feedback from the `ScdcClient` will eventually
  inform equalization adjustments during the LTP loop. This is blocked on `EqParams`
  being expanded; the placeholder call exists but does nothing useful yet.

---

## Dependencies

```
display-types  ─┐
hdmi-hal       ─┴─►  plumbob  ◄─  culvert (implements ScdcClient, feature-gated)
                               ◄─  integration layer (defines LinkTrainer, plumbob implements it)
```

- `hdmi-hal` — `HdmiPhy`, `EqParams`
- `display-types` — `HdmiForumFrl`

plumbob does not depend on `culvert`. The relationship runs the other way: culvert
implements `plumbob::ScdcClient` for `Scdc<T>`, gated behind a `plumbob` cargo feature.
Any crate that implements `ScdcClient` can be used in place of culvert.

plumbob does not depend on the integration layer. The integration layer defines a
`LinkTrainer` trait; plumbob implements it. Any crate that implements `LinkTrainer` can
be used in place of plumbob.

plumbob does not depend on `concordance` or `piaf`. It receives a target FRL rate from
the caller and trains at that rate.

---

## Training Procedure (HDMI 2.1 §10.x)

FRL link training has four phases. `train_at_rate` runs the full sequence for a single
rate and returns when it reaches a terminal state.

### Phase 1 — Configuration

1. Call `ScdcClient::write_frl_config` with the target rate and FFE level count.
2. Call `HdmiPhy::set_frl_rate` to configure the physical lanes for this rate.

Configuration is a write-then-forget step. The sink detects the rate change and begins its
own internal preparation.

### Phase 2 — Readiness

Poll `ScdcClient::read_training_status` until the sink asserts `flt_ready`. This flag
signals that the sink has completed its internal preparation for Fixed Link Training at the
requested rate.

If `flt_ready` does not assert within `TrainingConfig::flt_ready_timeout` iterations, the
attempt terminates with `TrainingOutcome::FallbackRequired`.

### Phase 3 — Initiation

Poll `ScdcClient::read_training_status` until the sink asserts `frl_start`. This flag
signals that the sink is ready for the LTP training loop to begin.

If `frl_start` does not assert within `TrainingConfig::frl_start_timeout` iterations, the
attempt terminates with `TrainingOutcome::FallbackRequired`.

### Phase 4 — LTP Loop

On each iteration:
1. Read `TrainingStatus::ltp_req` via `ScdcClient::read_training_status`.
2. If `LtpReq::None` — all lanes are satisfied. Training succeeded.
3. Otherwise — drive the requested Link Training Pattern on the PHY lanes and iterate.

If `ltp_req` does not reach `None` within `TrainingConfig::ltp_timeout` iterations, the
attempt terminates with `TrainingOutcome::FallbackRequired`.

**Open item:** Step 3 requires a `send_ltp(req: LtpReq)` method on `HdmiPhy`. The trait
does not currently have this method. Until it is added, the LTP phase falls back to calling
`adjust_equalization` as a placeholder. See [Open Items](#open-items).

---

## Key Types

### Owned protocol types

These types are defined in plumbob because they are the vocabulary of the `ScdcClient`
interface and the training state machine. SCDC implementations convert to them; the
training state machine uses them directly.

```rust
/// Link Training Pattern requested by the sink via Status_Flags_1 bits[7:4].
#[non_exhaustive]
pub enum LtpReq {
    None  = 0,
    Lfsr0 = 1,
    Lfsr1 = 2,
    Lfsr2 = 3,
    Lfsr3 = 4,
}

/// FFE (Feed-Forward Equalization) level count advertised to the sink in Config_0.
pub enum FfeLevels {
    Ffe0 = 0,
    Ffe1 = 1,
    // ... through Ffe7
}

/// FRL configuration written to Config_0.
///
/// `dsc_frl_max` is sourced from `TrainingConfig` and reflects whether the
/// negotiated configuration requires DSC transport (`NegotiatedConfig.resolved.dsc_required`).
/// plumbob passes it through without interpreting it.
pub struct FrlConfig {
    pub rate: HdmiForumFrl,
    pub ffe_levels: FfeLevels,
    pub dsc_frl_max: bool,
}

/// The subset of SCDC status that the training state machine reads on each poll.
pub struct TrainingStatus {
    pub flt_ready: bool,
    pub frl_start: bool,
    pub ltp_req: LtpReq,
}

/// A 15-bit per-lane character error count.
pub struct CedCount(u16);

/// Per-lane character error counts used for equalization feedback.
pub struct CedCounters {
    pub lane0: Option<CedCount>,
    pub lane1: Option<CedCount>,
    pub lane2: Option<CedCount>,
    pub lane3: Option<CedCount>,  // None in 3-lane FRL mode
}
```

### `ScdcClient`

The typed SCDC interface required by the link training state machine. Defined here so
that the state machine has no dependency on any specific SCDC implementation.

```rust
pub trait ScdcClient {
    type Error;

    /// Write FRL rate and configuration to Config_0.
    fn write_frl_config(&mut self, config: FrlConfig) -> Result<(), Self::Error>;

    /// Read flt_ready, frl_start, and ltp_req from Status_Flags.
    fn read_training_status(&mut self) -> Result<TrainingStatus, Self::Error>;

    /// Read per-lane character error counts for equalization feedback.
    fn read_ced(&mut self) -> Result<CedCounters, Self::Error>;
}
```

Implementations are provided by SCDC crates. culvert implements this for `Scdc<T>` via a
`plumbob` cargo feature. A simulated implementation for testing requires only a struct with
a register array.

### Training types

```rust
/// Outcome of a training attempt at a single FRL rate.
pub enum TrainingOutcome {
    /// All lanes satisfied. The link is ready at this rate.
    Success { achieved_rate: HdmiForumFrl },
    /// Training did not converge within the timeout.
    /// Caller should retry at a lower rate or fall back to TMDS.
    FallbackRequired,
}

/// Per-attempt training configuration.
#[non_exhaustive]
#[derive(Clone, Copy)]
pub struct TrainingConfig {
    /// FFE levels advertised to the sink in Config_0.
    pub ffe_levels: FfeLevels,
    /// Whether to set DSC_FRL_Max in Config_0.
    ///
    /// Set to `true` when the negotiated configuration requires DSC transport
    /// (`NegotiatedConfig.resolved.dsc_required`). The integration layer is
    /// responsible for supplying this value; plumbob passes it through into
    /// `FrlConfig` without interpreting it. Defaults to `false`.
    pub dsc_frl_max: bool,
    /// Maximum poll iterations waiting for flt_ready (phase 2).
    pub flt_ready_timeout: u32,
    /// Maximum poll iterations waiting for frl_start (phase 3).
    pub frl_start_timeout: u32,
    /// Maximum poll iterations in the LTP training loop (phase 4).
    pub ltp_timeout: u32,
}

/// Hard error that terminated a training attempt.
///
/// Distinct from FallbackRequired: this means something failed at the I/O level,
/// not that the link simply didn't train at this rate.
pub enum TrainingError<ScdcErr, PhyErr> {
    /// The ScdcClient returned an error.
    Scdc(ScdcErr),
    /// The PHY returned an error.
    Phy(PhyErr),
}

/// The central training type. Owns an ScdcClient and an HdmiPhy.
///
/// `FrlTrainer` is reusable across multiple `train_at_rate` calls. A caller
/// performing rate fallback calls `train_at_rate` repeatedly on the same trainer,
/// stepping down through FRL tiers, without reconstructing it between attempts.
/// `into_parts` is for when training is finished entirely and the caller wants
/// the SCDC client and PHY back for ongoing use (link monitoring, re-training
/// on degradation, etc.).
pub struct FrlTrainer<C, P> { ... }

impl<C: ScdcClient, P: HdmiPhy> FrlTrainer<C, P> {
    pub fn new(scdc: C, phy: P) -> Self;
    pub fn into_parts(self) -> (C, P);

    /// Run the full four-phase training sequence at the given FRL rate.
    pub fn train_at_rate(
        &mut self,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
    ) -> Result<TrainingOutcome, TrainingError<C::Error, P::Error>>;

    /// Like `train_at_rate`, but also returns a `TrainingTrace` recording the
    /// full event sequence. Requires the `alloc` feature.
    #[cfg(feature = "alloc")]
    pub fn train_at_rate_traced(
        &mut self,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
    ) -> Result<(TrainingOutcome, TrainingTrace), TrainingError<C::Error, P::Error>>;
}
```

`TrainingConfig` is `#[non_exhaustive]` and implements `Default`. The default values are
`ffe_levels: FfeLevels::Ffe0`, `dsc_frl_max: false`, `flt_ready_timeout: 1000`,
`frl_start_timeout: 1000`, and `ltp_timeout: 1000`. These are reasonable for hardware use
but are not tuned for any specific platform; callers should adjust the timeout values for
their polling cadence.

---

## Diagnostics

Concordance records every decision it considers in a `ReasoningTrace`. plumbob is a
sequential state machine rather than a decision pipeline, so the diagnostic equivalent is
an ordered event log: a record of what the sink signaled, when each phase completed, and
why training succeeded or did not. A driver or diagnostic tool must be able to reconstruct
the full training sequence from the trace without inspecting internal state or reading logs.

### `TrainingEvent`

Each phase transition and significant sink signal produces a `TrainingEvent`. Events are
recorded in order from phase 1 through the terminal state.

```rust
#[non_exhaustive]
pub enum TrainingEvent {
    /// Phase 1: Config_0 was written with this rate and FFE level count.
    RateConfigured {
        rate: HdmiForumFrl,
        ffe_levels: FfeLevels,
    },

    /// Phase 2: the sink asserted flt_ready after this many poll iterations.
    FltReadyReceived { after_iterations: u32 },

    /// Phase 2: timed out waiting for flt_ready.
    FltReadyTimeout { iterations_elapsed: u32 },

    /// Phase 3: the sink asserted frl_start after this many poll iterations.
    FrlStartReceived { after_iterations: u32 },

    /// Phase 3: timed out waiting for frl_start.
    FrlStartTimeout { iterations_elapsed: u32 },

    /// Phase 4: the sink changed its LTP request to this pattern.
    ///
    /// Recorded each time ltp_req transitions to a new value, not on every poll.
    /// The sequence of these events shows how the sink's pattern requests evolved
    /// during training.
    LtpPatternRequested { pattern: LtpReq },

    /// Phase 4: ltp_req reached None on all lanes. Training succeeded.
    AllLanesSatisfied { after_iterations: u32 },

    /// Phase 4: timed out in the LTP loop before ltp_req reached None.
    LtpLoopTimeout { iterations_elapsed: u32 },
}
```

Only transitions in `ltp_req` are recorded as `LtpPatternRequested`, not every poll. A
sink that holds the same pattern for 50 iterations produces one event, not 50. This keeps
the trace compact while preserving the information a diagnostic tool actually needs: what
patterns were requested, and in what order.

### `TrainingTrace`

```rust
/// Full event log for a single training attempt.
#[non_exhaustive]
pub struct TrainingTrace {
    /// The FRL rate that was attempted.
    pub rate: HdmiForumFrl,
    /// The configuration in force during this attempt.
    ///
    /// Recorded so that a trace is fully self-describing: timeout counts in
    /// the events (e.g. `FltReadyTimeout { iterations_elapsed: 47 }`) are only
    /// meaningful when read against the limit that was configured.
    pub config: TrainingConfig,
    /// Ordered event log from phase 1 through the terminal state.
    pub events: Vec<TrainingEvent>,
}
```

`TrainingTrace` uses `Vec` and requires the `alloc` feature. The non-allocating
`train_at_rate` is always available; `train_at_rate_traced` is alloc-gated.

`TrainingConfig` derives `Clone` and `Copy` (all its fields are `Copy`); storing it
in `TrainingTrace` is a value copy with no allocation.

### Interpreting the trace

A complete successful trace looks like:

```
RateConfigured { rate: Rate9Gbps3Lanes, ffe_levels: Ffe0 }
FltReadyReceived { after_iterations: 5 }
FrlStartReceived { after_iterations: 12 }
LtpPatternRequested { pattern: Lfsr0 }
LtpPatternRequested { pattern: Lfsr2 }
LtpPatternRequested { pattern: None }   ← not recorded; AllLanesSatisfied is emitted instead
AllLanesSatisfied { after_iterations: 47 }
```

A trace that timed out in phase 2 — the sink never completed internal preparation:

```
RateConfigured { rate: Rate12Gbps4Lanes, ffe_levels: Ffe0 }
FltReadyTimeout { iterations_elapsed: 1000 }
```

A trace that timed out in phase 3 — the sink became ready but never signalled to begin:

```
RateConfigured { rate: Rate12Gbps4Lanes, ffe_levels: Ffe0 }
FltReadyReceived { after_iterations: 4 }
FrlStartTimeout { iterations_elapsed: 1000 }
```

A trace that timed out in phase 4 — the sink requested patterns but lanes never converged:

```
RateConfigured { rate: Rate12Gbps4Lanes, ffe_levels: Ffe0 }
FltReadyReceived { after_iterations: 4 }
FrlStartReceived { after_iterations: 3 }
LtpPatternRequested { pattern: Lfsr1 }
LtpPatternRequested { pattern: Lfsr3 }
LtpLoopTimeout { iterations_elapsed: 1000 }
```

Each timeout phase tells the caller something distinct about what went wrong: a phase 2
timeout means the sink did not complete internal preparation at this rate; a phase 3 timeout
means it prepared but did not initiate training; a phase 4 timeout means the sink entered
the LTP loop but lanes failed to lock, which is more likely a signal integrity or
equalization issue.

---

## Interface Boundaries

plumbob sits between two interfaces, and defines one of them.

### Below: `ScdcClient` (defined here, implemented by SCDC crates)

**The SCDC implementation's responsibility:** typed register access. Given a desired FRL
rate, write it into `Config_0`. Given a status register, decode it into `TrainingStatus`.
The SCDC implementation does not know what to do with a `TrainingStatus`; it only knows
how to read one.

**plumbob's responsibility toward the SCDC layer:** sequence the calls. Write `Config_0`.
Poll for `frl_start`. Read `ltp_req`. Declare success or fall back. That sequencing, the
iteration limits, and the fallback signal live here.

The rule: if it touches state across multiple register accesses, timeout logic, or the
decision of what to do with a register value, it belongs in plumbob. If it reads or writes
registers and returns typed results, it belongs in the SCDC implementation.

#### Type ownership and the culvert boundary

plumbob owns the types that form the vocabulary of `ScdcClient`: `LtpReq`, `FfeLevels`,
`FrlConfig`, `TrainingStatus`, `CedCount`, `CedCounters`. These are the types the state
machine reasons about.

culvert independently defines its own register-layer types (`culvert::LtpReq`,
`culvert::FfeLevels`, etc.) for its own purposes — they are the output of SCDC register
decoding, not the input to a training state machine. The two sets of types happen to be
structurally identical today but exist at different layers and can evolve independently.

When culvert implements `ScdcClient` (via its `plumbob` cargo feature), it converts between
its own types and plumbob's at the impl boundary:

```rust
#[cfg(feature = "plumbob")]
impl<T: ScdcTransport> plumbob::ScdcClient for Scdc<T> {
    fn read_training_status(&mut self) -> Result<plumbob::TrainingStatus, ...> {
        let flags = self.read_status_flags()?;
        Ok(plumbob::TrainingStatus {
            frl_start: flags.frl_start,
            ltp_req: flags.ltp_req.into(), // culvert::LtpReq → plumbob::LtpReq
        })
    }
    // ...
}
```

`From` impls between the corresponding types live in the same feature-gated module.
culvert's own types are unchanged; the conversion is confined to the impl.

This approach is intentional. The alternative — making culvert's types re-exports of
plumbob's when the feature is active — would make `culvert::LtpReq` mean different things
depending on the feature set, breaking crates that use culvert without plumbob. Making the
dependency unconditional would force plumbob into every culvert user's dependency graph.
The boundary conversion is small, explicit, and keeps both crates independently usable.

### Above: `LinkTrainer` (defined by the integration layer, implemented here)

The integration layer defines the interface it needs from link training. plumbob
implements it. This means the integration layer has no dependency on plumbob specifically
— any crate that implements `LinkTrainer` is substitutable.

The `LinkTrainer` trait is defined in the integration layer crate (not yet built). Its
surface will be driven by what the DRM/KMS integration actually needs to call: at minimum,
`train_at_rate` and the ability to recover the SCDC client and PHY on completion.

---

## `no_std`, `alloc`, and `async`

plumbob declares `#![no_std]` and `#![forbid(unsafe_code)]`. Three capability tiers are
available depending on the target environment:

**`no_std` (no allocator)**

The full training state machine is available. `FrlTrainer<C, P>` is stack-allocated;
`TrainingConfig`, `TrainingOutcome`, `TrainingError`, and all owned protocol types
(`LtpReq`, `FfeLevels`, `TrainingStatus`, `CedCounters`) are stack-allocated. No heap
use anywhere in the training loop. This tier covers bare-metal and firmware targets.

**`no_std` + `alloc` feature**

Adds `TrainingTrace` and `train_at_rate_traced`. The trace requires `Vec` to accumulate
events; everything else is unchanged. Enable with:

```toml
plumbob = { version = "0.1", features = ["alloc"] }
```

**`std` feature**

Implies `alloc`. No additional API surface beyond what `alloc` provides; `std` exists as
a convenience for targets where it is available and for host-side tooling.

**Async**

The sync `ScdcClient` trait is blocking. Async link training follows the same pattern as
`hdmi-hal` / `hdmi-hal-async`: a companion crate `plumbob-async` will mirror `ScdcClient`
and `FrlTrainer` with `async fn` methods, depend on `plumbob` for shared data types, and
be implemented against `culvert-async`. This is out of scope for the current phase; the
sync API is designed so that adding the async companion requires no changes to this crate.

---

## Design Principles

- **Interfaces owned by consumers.** plumbob defines the interface its dependencies
  must satisfy (`ScdcClient`) rather than depending on a concrete implementation.
  The integration layer above defines the interface plumbob must satisfy (`LinkTrainer`).
  Each layer is substitutable independently.
- **Deterministic and testable.** The training procedure runs identically against a
  simulated `ScdcClient` and real hardware. Implement `ScdcClient` with a register
  array, pre-load it with the values a sink would produce at each phase, run the state
  machine, assert on the outcome. No hardware required for any test.
- **State machine, not scattered logic.** The four phases are an explicit sequence.
  Phase transitions are clear, terminal states are explicit, and every exit point
  produces a typed result. No implicit control flow, no silent completion.
- **Policy at the right layer.** plumbob implements the spec, not strategy. Rate
  fallback order, retry counts above the per-attempt limit, and the decision of whether
  to surface a `FallbackRequired` to the user are the caller's concerns.
- **Transport and PHY errors are distinct.** A caller diagnosing a training failure
  needs to know whether it came from the I²C bus, the PHY, or the protocol. `TrainingError`
  keeps them separate.
- **No unsafe code.** `#![forbid(unsafe_code)]`.
- **Stable consumer types.** `TrainingOutcome`, `TrainingConfig`, and `TrainingTrace` are
  `#[non_exhaustive]` where appropriate. Callers are insulated from internal expansions.

---

## Open Items

**`HdmiPhy::send_ltp`** — Phase 3 of training requires the source to drive a specific Link
Training Pattern on the physical lanes, as requested by the sink via `LtpReq`. This is a
PHY operation. `HdmiPhy` currently has `set_frl_rate`, `adjust_equalization`, and
`set_scrambling`, but no method for driving LTP patterns.

The method will be added to `hdmi-hal` as `send_ltp(pattern: LtpPattern)`, where
`LtpPattern` is a newtype defined in hdmi-hal. This keeps hdmi-hal free of any dependency
on plumbob. plumbob converts from its `LtpReq` to `hdmi_hal::LtpPattern` before calling
the PHY; `LtpReq::None` is the exit condition for the LTP loop and never reaches the call.

Until `send_ltp` and `LtpPattern` are added to hdmi-hal, the LTP loop calls
`adjust_equalization` as a placeholder. The state machine structure is complete; only this
one call is a stub.

**`EqParams` expansion** — `EqParams` in hdmi-hal is currently an empty placeholder struct.
`ScdcClient::read_ced` returns `CedCounters` (defined here) which will feed equalization
adjustments during the LTP loop once `EqParams` is expanded to carry the relevant fields.
The training loop already calls `read_ced`; the equalization call is the stub.

---

## Implementation Plan

Steps are ordered by dependency. Each step should leave the crate compiling and tests
passing before the next begins. This section will be removed once implementation is
complete.

### Step 1 — Crate scaffolding

- Replace the stub `src/lib.rs` with the real module skeleton.
- Configure `Cargo.toml`: add dependencies on `hdmi-hal` and `display-types`; declare
  `alloc` and `std` feature flags (`std` implies `alloc`).
- Add crate-level attributes: `#![no_std]`, `#![forbid(unsafe_code)]`,
  `#![deny(missing_docs)]`.
- Add `extern crate alloc;` gated on the `alloc` feature.

### Step 2 — CI scaffolding

Add the three workflow files used by all sibling projects:

- `.github/workflows/ci.yml` — `test`, `build (alloc only)`, `build (no_std)`,
  `build simulate example`, and a `coverage` job using `cargo-llvm-cov`. The coverage
  job measures line coverage and enforces a ratcheting baseline stored in
  `.coverage-baseline`. Coverage is measured with `--features std` (the default).
- `.github/workflows/audit.yml` — runs `rustsec/audit-check` on `Cargo.toml` /
  `Cargo.lock` changes.
- `.github/workflows/publish.yml` — triggers on `v*.*.*` tags on `main`; runs the full
  CI check sequence then `cargo publish`.

Add `.coverage-baseline` containing `0.00` as a placeholder; it will be ratcheted
upward automatically as tests are added.

### Step 3 — Owned protocol types

Implement all types that form the vocabulary of `ScdcClient` and the state machine:
`LtpReq`, `FfeLevels`, `FrlConfig`, `TrainingStatus`, `CedCount`, `CedCounters`.

These are pure data types with no logic. Derive `Debug`, `Clone`, `Copy`, `PartialEq`,
`Eq` where appropriate. Mark `LtpReq` `#[non_exhaustive]`. `CedCount::new` masks to
15 bits (matching culvert's existing behaviour).

Tests: each type's constructor, field accessors, and any derived trait behaviour
(`PartialEq`, `Clone`). Verify `CedCount` masks the validity bit.

### Step 4 — `ScdcClient` trait

Define the trait with its three methods: `write_frl_config`, `read_training_status`,
`read_ced`. No implementations in this step.

### Step 5 — Training types

Implement `TrainingOutcome`, `TrainingError`, and `TrainingConfig`.

`TrainingConfig` implements `Default` with `ffe_levels: Ffe0`, `dsc_frl_max: false`,
and all three timeouts set to `1000`. Mark `TrainingConfig` and `TrainingOutcome`
`#[non_exhaustive]`.

Tests: verify `TrainingConfig::default()` field values; verify both `TrainingOutcome`
variants and both `TrainingError` variants are constructible and match correctly.

### Step 6 — `FrlTrainer` and `train_at_rate`

Implement the `FrlTrainer<C, P>` struct with `new` and `into_parts`.

Implement `train_at_rate`: the four-phase sequence in full. In phase 4, call
`phy.adjust_equalization(EqParams::default())` as the placeholder for `send_ltp` — the
state machine structure is complete, only this call is a stub.

### Step 7 — State machine tests

Write a `SimScdc` test helper: a struct holding a scripted response queue that returns
pre-set `TrainingStatus` values in sequence, simulating a sink's phase-by-phase
behaviour. Write a `MockPhy` that records calls.

Full coverage requires a test for every branch in `train_at_rate`:

- Successful training: `flt_ready` asserts on poll N, `frl_start` asserts on poll M,
  `ltp_req` transitions through one or more patterns before reaching `None`.
- `flt_ready` never asserts: phase 2 times out, returns `FallbackRequired`.
- `flt_ready` asserts immediately (iteration 0).
- `frl_start` never asserts: phase 3 times out, returns `FallbackRequired`.
- `frl_start` asserts immediately.
- LTP loop times out: `ltp_req` never reaches `None`, returns `FallbackRequired`.
- LTP loop succeeds on the first iteration (`ltp_req` is `None` on first read).
- `ltp_req` transitions through all four LFSR variants before reaching `None`.
- `TrainingError::Scdc` propagates from each of the three `ScdcClient` calls.
- `TrainingError::Phy` propagates from `set_frl_rate` and from the LTP call.
- `into_parts` recovers the SCDC client and PHY after a completed attempt.

### Step 8 — Diagnostics (`alloc` feature)

Implement `TrainingEvent`, `TrainingTrace`, and `train_at_rate_traced`, all gated on
`#[cfg(feature = "alloc")]`. `TrainingEvent` and `TrainingTrace` are `#[non_exhaustive]`.

`train_at_rate_traced` runs the same state machine logic as `train_at_rate`; the shared
logic should be factored so the trace variant adds event recording without duplicating
the phase code.

### Step 9 — Diagnostics tests

Full coverage requires a test for every event variant and every trace shape:

- Successful run: assert `RateConfigured`, `FltReadyReceived`, `FrlStartReceived`,
  one or more `LtpPatternRequested`, `AllLanesSatisfied` in order.
- Phase 2 timeout: assert `RateConfigured`, `FltReadyTimeout`.
- Phase 3 timeout: assert `RateConfigured`, `FltReadyReceived`, `FrlStartTimeout`.
- Phase 4 timeout: assert `RateConfigured`, `FltReadyReceived`, `FrlStartReceived`,
  one or more `LtpPatternRequested`, `LtpLoopTimeout`.
- `LtpPatternRequested` is emitted on transition only: a sink that holds the same
  pattern for multiple iterations produces exactly one event, not one per poll.
- `AllLanesSatisfied` and `LtpLoopTimeout` carry correct `after_iterations` /
  `iterations_elapsed` counts.
- `TrainingTrace.config` matches the `TrainingConfig` passed to `train_at_rate_traced`;
  verify that `iterations_elapsed` in timeout events is interpretable against the
  corresponding limit in the recorded config.

### Step 10 — hdmi-hal: `LtpPattern` and `send_ltp`

In hdmi-hal:
- Add `LtpPattern` newtype.
- Add `send_ltp(pattern: LtpPattern)` to the `HdmiPhy` trait.

In plumbob:
- Add a `From<LtpReq>` conversion to `hdmi_hal::LtpPattern` (all variants except
  `None`, which never reaches the call).
- Replace the `adjust_equalization` placeholder in phase 4 with `phy.send_ltp(pattern)`.
- Update `MockPhy` in tests to implement `send_ltp`; add a test that asserts the correct
  `LtpPattern` is passed for each `LtpReq` variant.

### Step 11 — culvert: `plumbob` feature

In culvert:
- Add a `plumbob` cargo feature that depends on this crate.
- Add `From` impls between culvert's register-layer types and plumbob's protocol types
  (`LtpReq`, `FfeLevels`, `CedCount`, `CedCounters`).
- Implement `plumbob::ScdcClient` for `Scdc<T>` in a feature-gated module. The
  `read_training_status` impl reads `StatusFlags` and maps `flt_ready`, `frl_start`,
  and `ltp_req` into `plumbob::TrainingStatus`.
- Add integration tests in culvert that exercise the `ScdcClient` impl against
  `TestTransport`: verify each method delegates correctly and that `From` conversions
  round-trip all variants.

### Step 12 — Example

Add `examples/simulate/` as a standalone crate (matching the pattern in hdmi-hal).

The example demonstrates the full `FrlTrainer` usage pattern:
- A `SimScdc` implementation backed by an in-memory register array, with a helper
  to pre-load it with a scripted successful training sequence.
- A `SimPhy` that prints each PHY call to stdout.
- A `main` that constructs an `FrlTrainer`, calls `train_at_rate_traced`, and prints
  the outcome and the full `TrainingTrace`.

The example should show all four phases completing and the trace output, so a reader
can see the intended usage and verify the event sequence by inspection.

Add `examples/simulate/Cargo.toml` with `publish = false` and a path dependency on
this crate with `features = ["std"]`.
