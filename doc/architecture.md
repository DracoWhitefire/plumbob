# Architecture

## Role

`plumbob` implements the Fixed Rate Link (FRL) training state machine defined in the HDMI
2.1 specification. It defines the interface its dependencies must satisfy rather than
depending on any specific SCDC implementation, and it is itself replaceable by any crate
that implements the `LinkTrainer` trait defined by the integration layer above it.

Training establishes actual link capability, not just theoretical negotiation. A
`NegotiatedConfig` from concordance identifies what the hardware should support; link
training determines what it actually achieves. The caller supplies the rates to try, in
order; plumbob steps down through them when the sink asks for a lower rate. The caller is
responsible for deciding what to do with a `FallbackRequired` outcome — whether to retry
with other rates, fall back to TMDS, or surface the failure.

plumbob is usable as a standalone crate without the broader concordance/piaf stack. A
firmware image, an embedded hardware vendor's SDK, or a kernel driver can depend directly
on plumbob, supply their own `ScdcClient` and `HdmiPhy` implementations, and call
`train` or `train_at_rate` without any negotiation layer above it. The concordance integration is one
deployment model, not a requirement.

---

## Scope

plumbob covers:

- the FRL link training state machine: LTS:2 → LTS:3 → LTS:P, with LTS:4 (lower rate)
  on the sink's request and LTS:L (exit to TMDS) on failure, per HDMI 2.1 §6,
- `ScdcClient`: the typed SCDC interface trait, defined here and implemented by SCDC crates,
- `FrlTrainer<C, P>`: the central type, owning an `ScdcClient` and a PHY,
- `lts`: the state machine as one I/O-free `async fn` (`lts::run`) over the `TrainingIo`
  trait, which `FrlTrainer` drives synchronously and `plumbob-async` asynchronously,
- `TrainingOutcome`: the result of a training attempt (`Success`, or `FallbackRequired`
  with a `FallbackReason`),
- `TrainingConfig`: per-attempt configuration (maximum FFE level, poll limits),
- `TrainingError`: hard SCDC or PHY failures, with what LTS:L did afterwards (`TmdsExit`),
- owned protocol types: `LtpReq`, `LtpRequests`, `FfeLevels`, `FrlConfig`,
  `UpdateFlags`, `SourceTestConfig`, `CedCounters`,
- simulation support: the training procedure is fully exercisable without real hardware
  by using simulated implementations of `ScdcClient` and `HdmiPhy`.

The following are out of scope:

- **Rate fallback policy** — the caller decides which rates to try and in what order (for
  example from concordance's ranking, keeping only rates the chosen mode still fits in).
  plumbob walks that list when the sink requests a lower rate, and does not maintain a
  rate table of its own. Timeouts end the attempt; they do not step down.
- **SCDC register decoding** — plumbob reads typed values from `ScdcClient` and does not
  decode raw register bytes or know SCDC register addresses.
- **PHY vendor sequences** — plumbob calls `HdmiPhy` methods; the register sequences
  for lane reconfiguration are in platform PHY backends.
- **Timing** — plumbob is synchronous and poll-based. No sleep, no timers. The spec's
  timeouts (100 ms for `FLT_ready`, 200 ms for LTS:3) are expressed as poll limits in
  `TrainingConfig`; their defaults assume one poll every 2 ms, which the caller's
  transport or loop must provide.
- **TMDS link setup** — plumbob handles FRL training only. TMDS mode is the fallback
  that concordance selects if no FRL tier trains successfully; plumbob has no role in it.
- **CED-driven equalization** — equalization during training follows the sink's FFE
  change requests (0xE). Using CED counters to tune equalization beyond that is not part
  of the training procedure and is left to the caller.

---

## Dependencies

```
display-types  ─┐
hdmi-hal       ─┴─►  plumbob  ◄─  culvert (implements ScdcClient, feature-gated)
                               ◄─  integration layer (defines LinkTrainer, plumbob implements it)
```

- `hdmi-hal` — `HdmiPhy`, `LanePatterns`, `LtpPattern`, `FrlOutput`, `EqParams`,
  `LaneEqParams`, `TxFfeLevel`
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

## Training Procedure (HDMI 2.1 §6, link training states)

`train` runs the source side of FRL link training over a caller-supplied list of rates, in
order, and returns when it reaches a terminal state; `train_at_rate` is the same with a
single rate. An empty list returns `FallbackRequired { reason: RatesExhausted }` without
touching the sink. The sequence follows the link training states (LTS) of the
HDMI 2.1 specification as implemented by open-source HDMI 2.1 transmitters: the
AMD/Xilinx `v_hdmitx1` driver (`xv_hdmitx1_frl.c`) and the Intel `xe` FRL series
(`intel_hdmi_train_lanes`). LTS:1 (reading the sink's EDID and SCDC capability) is the
caller's job; training starts at LTS:2.

The number of active lanes follows from the rate: 3 for `Rate3Gbps3Lanes` and
`Rate6Gbps3Lanes`, 4 otherwise. In 3-lane mode lane 3 is ignored everywhere below.

### LTS:2 — Prepare

1. If `UpdateFlags::source_test_update` is set, read `SourceTestConfig` and clear the flag.
   `flt_no_timeout` suspends the poll limits of LTS:2 and LTS:3 (compliance testing).
2. Poll `StatusFlags::flt_ready` until the sink asserts it. If it does not within
   `TrainingConfig::flt_ready_polls`, go to LTS:L and return
   `FallbackRequired { reason: FltReadyTimeout }`.
3. Clear `FLT_update`. Reset every lane's TxFFE level to 0 on the PHY.
4. Configure the PHY for the rate (`set_frl_rate`) and drive the Nyquist clock pattern on
   all lanes while the sink's receiver locks.
5. Stop the patterns (no pattern on any lane) and send gap characters only
   (`set_frl_output(GapOnly)`). LTS:3 starts from this state.
6. Write `Config_0` (no read requests, `FLT_no_retrain` clear) and `Config_1` (the first
   rate in the list, and the highest TxFFE level: `TrainingConfig::ffe_levels`, limited to
   the maximum for that rate).

### LTS:3 — Train

plumbob keeps the current pattern and TxFFE level of every lane. Each lane starts with no
pattern and TxFFE level 0. Repeat, at most `TrainingConfig::ltp_polls` times:

1. Read `UpdateFlags`. If `flt_update` is not set, poll again.
2. If `source_test_update` is set, re-read `SourceTestConfig` and clear the flag.
3. Read the per-lane requests (`LtpRequests`) and act on the active lanes:
   - **all 0x0** — training passed: go to LTS:P.
   - **all 0xF** — the sink requests a lower rate: go to LTS:4.
   - otherwise, per lane, update the lane's state:
     - **0x1, 0x2, 0x4–0x8** — that lane's pattern becomes the requested one.
     - **0x3** (Nyquist clock) — becomes the lane's pattern only when `flt_no_timeout` is
       set. Otherwise the lane keeps its previous pattern, as the Xilinx driver does
       (citing the spec's Table 6-32, LTP3 row) and AMD's driver does equivalently.
     - **0xE** — the lane's TxFFE level rises by one, up to the advertised maximum, and is
       held at the maximum if the sink keeps asking (the Intel series does this; the
       Xilinx driver wraps to 0 instead).
     - **0x0** — the lane keeps its pattern and level.

     Then send the full per-lane pattern set to the PHY (`send_ltp`) and, if any TxFFE
     level changed, the per-lane levels (`adjust_equalization`). The PHY applies exactly
     what it is given; plumbob holds the state.
4. Clear `FLT_update` so the sink can post its next request.

If training has not passed within the poll limit, go to LTS:L and return
`FallbackRequired { reason: TrainingTimeout }`. A timeout does not step down: only the
sink's request does.

### LTS:P — Pass

1. Stop the training patterns (no pattern on any lane) and send gap characters only
   (`set_frl_output(GapOnly)`). Clear `FLT_update`.
2. Poll `UpdateFlags`, at most `TrainingConfig::frl_start_polls` times:
   - **`frl_start`** — clear it and return `Success`. The caller starts video
     (`set_frl_output(Active)`) on the PHY it gets back from `into_parts`, or through the
     trainer it keeps.
   - **`flt_update`** — the sink requests retraining: go back to LTS:3, at most
     `TrainingConfig::max_retrains` times per `train` call. The next request after that
     goes to LTS:L and returns `FallbackRequired { reason: RetrainsExhausted }`.
3. If neither arrives within the limit, go to LTS:L and return
   `FallbackRequired { reason: FrlStartTimeout }`.

After `Success` the sink can still request retraining during active video by setting
`FLT_update`; plumbob does not watch for it. The caller polls `Update_0` (the Xilinx
driver checks every 250 ms) and calls `train` again when it is set.

### LTS:4 — Lower rate

1. Stop the training patterns.
2. Take the next rate from the list. If there is none, go to LTS:L and return
   `FallbackRequired { reason: RatesExhausted }`.
3. Reset every lane's TxFFE level to 0, configure the PHY for the new rate, clear
   `FLT_update` and write `Config_1` with the new rate (and the FFE maximum for it).
4. Continue in LTS:3 at the new rate, with a fresh poll limit. The sink stays in FRL
   throughout; `FLT_ready` is not awaited again.

### LTS:L — Exit to TMDS

Before returning `FallbackRequired`, plumbob leaves both ends in a defined state: it stops
the training patterns, returns the PHY to TMDS (`set_frl_rate(NotSupported)`), writes
`Config_1` with `HdmiForumFrl::NotSupported` (FRL off) and clears `FLT_update` if it is
set. A failed attempt never leaves the sink configured for an FRL rate the source is not
driving. Every step is attempted even if an earlier one fails, so a PHY error cannot keep
the sink in FRL, nor an SCDC error the PHY.

LTS:L also follows an SCDC or PHY error, wherever it happens: the spec's exit from a
failed attempt applies whatever made it fail. The error is returned as
`TrainingError::Scdc` or `TrainingError::Phy`, with `exit: TmdsExit` saying what LTS:L
did — `Exited`, or `Failed` with the first error of each end whose steps failed (an end
without one is in TMDS). If LTS:L fails after a fallback, `train` returns
`TrainingError::ExitFailed` with the fallback reason and each end's error instead of
`FallbackRequired`. The trace records `ExitedToTmds` or `ExitToTmdsFailed`.

`TrainingConfig::exit_to_tmds_on_error` (default `true`) turns the exit after an error
off, for callers that want the sink and PHY as the error left them — a validation tool
reading the sink's registers, for example. The error then reports `TmdsExit::Skipped`.

---

## Key Types

### Owned protocol types

These types are defined in plumbob because they are the vocabulary of the `ScdcClient`
interface and the training state machine. SCDC implementations convert to them; the
training state machine uses them directly.

```rust
/// Link training pattern requested by the sink for one lane (4-bit field).
#[non_exhaustive]
pub enum LtpReq {
    None            = 0x0,  // lane trained
    AllOnes         = 0x1,
    AllZeros        = 0x2,
    NyquistClock    = 0x3,
    DdeCompliance   = 0x4,
    Lfsr0           = 0x5,
    Lfsr1           = 0x6,
    Lfsr2           = 0x7,
    Lfsr3           = 0x8,
    FfeChange       = 0xE,  // raise this lane's TxFFE level
    RateChange      = 0xF,  // drop the FRL rate
}

/// Requests for all four lanes; lane 3 is ignored in 3-lane FRL.
pub struct LtpRequests { pub lane0: LtpReq, pub lane1: LtpReq, pub lane2: LtpReq, pub lane3: LtpReq }

/// Highest TxFFE level index the source supports (0–7; at most 3 up to 12 Gbps).
pub struct FfeLevels(u8);

/// Written to Config_1.
pub struct FrlConfig { pub rate: HdmiForumFrl, pub ffe_levels: FfeLevels }

/// The Update_0 flags the state machine reads and clears.
pub struct UpdateFlags { pub source_test_update: bool, pub frl_start: bool, pub flt_update: bool }

/// The Source_Test_Configuration field the state machine honours.
pub struct SourceTestConfig { pub flt_no_timeout: bool }

/// Per-lane character error counts (diagnostics).
pub struct CedCounters { pub lane0: Option<CedCount>, /* … */ pub lane3: Option<CedCount> }
```

### `ScdcClient`

The typed SCDC interface required by the training state machine, one method per register
operation it performs. Defined here so the state machine has no dependency on any specific
SCDC implementation.

```rust
pub trait ScdcClient {
    type Error;

    /// Status_Flags_0 bit 6.
    fn read_flt_ready(&mut self) -> Result<bool, Self::Error>;
    /// Update_0 bits 3–5.
    fn read_update_flags(&mut self) -> Result<UpdateFlags, Self::Error>;
    /// Write-1-to-clear the given Update_0 flags.
    fn clear_update_flags(&mut self, flags: UpdateFlags) -> Result<(), Self::Error>;
    /// Status_Flags_1/2: the per-lane requests.
    fn read_ltp_requests(&mut self) -> Result<LtpRequests, Self::Error>;
    /// Source_Test_Configuration (0x35).
    fn read_source_test_config(&mut self) -> Result<SourceTestConfig, Self::Error>;
    /// Config_0: read requests disabled, FLT_no_retrain clear.
    fn write_config_0_defaults(&mut self) -> Result<(), Self::Error>;
    /// Config_1: FRL rate and FFE levels.
    fn write_frl_config(&mut self, config: FrlConfig) -> Result<(), Self::Error>;
    /// ERR_DET counters, for diagnostics.
    fn read_ced(&mut self) -> Result<CedCounters, Self::Error>;
}
```

culvert implements this for `Scdc<T>` behind its `plumbob` feature; each method maps to
one culvert method. A simulated implementation for testing needs only a register array.

### Training types

```rust
pub enum TrainingOutcome {
    /// Training passed and the sink set FRL_start. The link is ready at this rate.
    Success { achieved_rate: HdmiForumFrl },
    /// Training did not succeed at any of the rates; the sink is back in TMDS.
    FallbackRequired { reason: FallbackReason },
}

#[non_exhaustive]
pub enum FallbackReason {
    /// LTS:2: FLT_ready did not assert within the poll limit.
    FltReadyTimeout,
    /// LTS:3: the lanes did not pass within the poll limit.
    TrainingTimeout,
    /// LTS:P: FRL_start did not assert within the poll limit.
    FrlStartTimeout,
    /// LTS:4: the sink requested a lower rate than the last one in the list
    /// (or the list was empty).
    RatesExhausted,
    /// LTS:P: the sink requested retraining after max_retrains retrains.
    RetrainsExhausted,
}

#[non_exhaustive]
#[derive(Clone, Copy)]
pub struct TrainingConfig {
    /// Highest TxFFE level the source supports, written to Config_1.
    pub ffe_levels: FfeLevels,
    /// Poll limit for FLT_ready in LTS:2. Default 50 (100 ms at 2 ms per poll).
    pub flt_ready_polls: u32,
    /// Poll limit for LTS:3. Default 100 (200 ms at 2 ms per poll).
    pub ltp_polls: u32,
    /// Poll limit for FRL_start in LTS:P. Default 100 (200 ms at 2 ms per poll).
    pub frl_start_polls: u32,
    /// Hard cap on polls while the sink sets FLT_no_timeout. Default 500.
    pub no_timeout_poll_cap: u32,
    /// Returns from LTS:P to LTS:3 allowed per train call. Default 3.
    pub max_retrains: u32,
    /// Whether an SCDC or PHY error is followed by LTS:L. Default true.
    pub exit_to_tmds_on_error: bool,
}
```

```rust
impl<C: ScdcClient, P: HdmiPhy> FrlTrainer<C, P> {
    /// Train over `rates` in order, stepping down when the sink requests it.
    pub fn train(&mut self, rates: &[HdmiForumFrl], config: &TrainingConfig)
        -> Result<TrainingOutcome, TrainingError<C::Error, P::Error>>;
    /// `train(&[rate], config)`.
    pub fn train_at_rate(&mut self, rate: HdmiForumFrl, config: &TrainingConfig)
        -> Result<TrainingOutcome, TrainingError<C::Error, P::Error>>;
    /// `train` and `train_at_rate`, also returning a `TrainingTrace` (alloc).
    pub fn train_traced(&mut self, rates: &[HdmiForumFrl], config: &TrainingConfig)
        -> Result<(TrainingOutcome, TrainingTrace), TrainingError<C::Error, P::Error>>;
    pub fn train_at_rate_traced(&mut self, rate: HdmiForumFrl, config: &TrainingConfig)
        -> Result<(TrainingOutcome, TrainingTrace), TrainingError<C::Error, P::Error>>;
    // `new` and `into_parts` construct the trainer and recover the client and PHY.
}

/// A hard I/O failure, kept separate from the protocol outcome.
#[non_exhaustive]
pub enum TrainingError<ScdcErr, PhyErr> {
    Scdc { error: ScdcErr, exit: TmdsExit<ScdcErr, PhyErr> },
    Phy { error: PhyErr, exit: TmdsExit<ScdcErr, PhyErr> },
    /// The attempt fell back, and LTS:L then failed.
    ExitFailed { reason: FallbackReason, scdc: Option<ScdcErr>, phy: Option<PhyErr> },
}

/// What LTS:L did after an SCDC or PHY error.
#[non_exhaustive]
pub enum TmdsExit<ScdcErr, PhyErr> {
    Exited,
    /// `TrainingConfig::exit_to_tmds_on_error` is off.
    Skipped,
    /// Each end's first error; an end with `None` is in TMDS.
    Failed { scdc: Option<ScdcErr>, phy: Option<PhyErr> },
}
```

---

## Diagnostics

The trace is an ordered event log from LTS:2 to the terminal state, so a driver or
diagnostic tool can reconstruct the whole attempt without reading internal state.

### `TrainingEvent`

```rust
#[non_exhaustive]
pub enum TrainingEvent {
    /// The sink's Source_Test_Configuration was read (LTS:2 or LTS:3).
    SourceTestConfigRead { flt_no_timeout: bool },
    /// LTS:2: FLT_ready asserted after this many polls.
    FltReady { after_polls: u32 },
    /// LTS:2: FLT_ready did not assert.
    FltReadyTimeout { polls: u32 },
    /// LTS:2: Config_1 written.
    RateConfigured { rate: HdmiForumFrl, ffe_levels: FfeLevels },
    /// LTS:3: the sink posted new requests (one event per FLT_update, not per poll).
    LtpRequested { requests: LtpRequests },
    /// LTS:3: a lane's TxFFE level was raised in response to 0xE.
    FfeRaised { lane: u8, level: u8 },
    /// LTS:3: all active lanes reported 0x0.
    TrainingPassed { after_polls: u32 },
    /// LTS:4: the sink requested a lower rate; training continues at `to`.
    RateLowered { from: HdmiForumFrl, to: HdmiForumFrl },
    /// LTS:4: the sink requested a lower rate than the last one in the list.
    RatesExhausted,
    /// LTS:3: training did not pass within the poll limit.
    TrainingTimeout { polls: u32 },
    /// LTS:P: the sink requested retraining (FLT_update) before FRL_start.
    RetrainRequested,
    /// LTS:P: the sink requested retraining once more after max_retrains retrains.
    RetrainsExhausted { retrains: u32 },
    /// LTS:P: FRL_start asserted. Training succeeded.
    FrlStart { after_polls: u32 },
    /// LTS:P: FRL_start did not assert.
    FrlStartTimeout { polls: u32 },
    /// LTS:L: the sink was returned to TMDS.
    ExitedToTmds,
    /// LTS:L: a step failed on the SCDC side, the PHY side, or both.
    ExitToTmdsFailed { scdc: bool, phy: bool },
}
```

### `TrainingTrace`

The rates passed in, the `TrainingConfig` in force (so poll counts can be read against their
limits), and the ordered `events`. It requires the `alloc` feature.

### Interpreting the trace

```
FltReady { after_polls: 3 }
RateConfigured { rate: Rate12Gbps4Lanes, ffe_levels: 3 }
LtpRequested { requests: [Lfsr0, Lfsr1, Lfsr2, Lfsr3] }
LtpRequested { requests: [None, FfeChange, None, None] }
FfeRaised { lane: 1, level: 1 }
TrainingPassed { after_polls: 41 }
FrlStart { after_polls: 6 }
```

A sink that asks for a lower rate, trained over `[Rate12Gbps4Lanes, Rate10Gbps4Lanes]`:

```
FltReady { after_polls: 2 }
RateConfigured { rate: Rate12Gbps4Lanes, ffe_levels: 3 }
LtpRequested { requests: [RateChange, RateChange, RateChange, RateChange] }
RateLowered { from: Rate12Gbps4Lanes, to: Rate10Gbps4Lanes }
RateConfigured { rate: Rate10Gbps4Lanes, ffe_levels: 3 }
LtpRequested { requests: [Lfsr0, Lfsr0, Lfsr0, Lfsr0] }
TrainingPassed { after_polls: 12 }
FrlStart { after_polls: 4 }
```

A timeout in LTS:2 means the sink did not prepare at this rate; one in LTS:3 means lanes
did not converge (signal integrity or equalization); one in LTS:P means training passed but
the sink never released the link. `RateLowered` means the sink itself judged a rate
unattainable, and `RatesExhausted` that it judged every listed rate unattainable. Every
`FallbackRequired` trace ends with `ExitedToTmds`; a trace ending with `ExitToTmdsFailed`
belongs to a `TrainingError`.

---

## Interface Boundaries

plumbob sits between two interfaces, and defines one of them.

### Below: `ScdcClient` (defined here, implemented by SCDC crates)

**The SCDC implementation's responsibility:** typed register access. Write `Config_0` and
`Config_1`, read and clear the `Update_0` flags, read `FLT_ready`, the per-lane requests and
`Source_Test_Configuration`. It does not know what to do with those values; it only knows
how to read and write them.

**plumbob's responsibility toward the SCDC layer:** sequence the calls through LTS:2,
LTS:3 and LTS:P, clear the flags it has serviced, and return the sink to TMDS when an
attempt fails. That sequencing, the poll limits and the outcome live here.

The rule: if it touches state across multiple register accesses, timeout logic, or the
decision of what to do with a register value, it belongs in plumbob. If it reads or writes
registers and returns typed results, it belongs in the SCDC implementation.

#### Type ownership and the culvert boundary

plumbob owns the types that form the vocabulary of `ScdcClient`: `LtpReq`, `LtpRequests`,
`FfeLevels`, `FrlConfig`, `UpdateFlags`, `SourceTestConfig`, `CedCount`, `CedCounters`. These are the types the state
machine reasons about.

culvert independently defines its own register-layer types (`culvert::LtpReq`,
`culvert::FfeLevels`, etc.) for its own purposes — they are the output of SCDC register
decoding, not the input to a training state machine. The two sets of types happen to be
structurally identical today but exist at different layers and can evolve independently.

When culvert implements `ScdcClient` (via its `plumbob` cargo feature), each trait method
calls one culvert method and converts culvert's type to plumbob's (`culvert::LtpRequests` →
`plumbob::LtpRequests`, and so on). `From` impls live in the same feature-gated module;
culvert's own types are unchanged.

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
`train` and the ability to recover the SCDC client and PHY on completion.

---

## `no_std`, `alloc`, and `async`

plumbob declares `#![no_std]` and `#![forbid(unsafe_code)]`. Three capability tiers are
available depending on the target environment:

**`no_std` (no allocator)**

The full training state machine is available. `FrlTrainer<C, P>` is stack-allocated;
`TrainingConfig`, `TrainingOutcome`, `TrainingError`, and all owned protocol types
(`LtpReq`, `LtpRequests`, `FfeLevels`, `UpdateFlags`, `CedCounters`, …) are stack-allocated. No heap
use anywhere in the training loop. This tier covers bare-metal and firmware targets.
CI builds this tier and the `alloc` tier for `thumbv7em-none-eabi`, a target without
`std`.

**`no_std` + `alloc` feature**

Adds `TrainingTrace`, `train_traced` and `train_at_rate_traced`. The trace requires `Vec` to accumulate
events; everything else is unchanged. Enable with:

```toml
plumbob = { version = "0.1", features = ["alloc"] }
```

**`std` feature**

Implies `alloc`. No additional API surface beyond what `alloc` provides; `std` exists as
a convenience for targets where it is available and for host-side tooling.

**Async**

The state machine is one `async fn`, `lts::run` (see below), so async link training needs
no second implementation. `plumbob-async` defines an async `ScdcClient` and an async
`FrlTrainer` that drives `lts::run` over async I/O, and depends on `plumbob` for the state
machine and all shared types; `culvert-async` implements its `ScdcClient`. See the stack
design document's "Sync and Async Companions" section for why the core lives here.

### `plumbob::lts`

```rust
pub trait TrainingIo {
    type ScdcError;
    type PhyError;
    // The SCDC operations of `ScdcClient` that training uses (all but `read_ced`) and the
    // four `HdmiPhy` operations, as `async fn`s.
    async fn read_flt_ready(&mut self) -> Result<bool, Self::ScdcError>;
    async fn send_ltp(&mut self, patterns: LanePatterns) -> Result<(), Self::PhyError>;
    // …
}

pub async fn run<Io: TrainingIo, F: FnMut(TrainingEvent)>(
    io: &mut Io,
    rates: &[HdmiForumFrl],
    config: &TrainingConfig,
    record: &mut F,
) -> Result<TrainingOutcome, TrainingError<Io::ScdcError, Io::PhyError>>;
```

`run` is the procedure described above: the same outcomes, errors and `TrainingEvent`s,
with no I/O of its own. Drivers supply the I/O:

- **`FrlTrainer` (sync).** An adapter implements `TrainingIo` over the trainer's
  `ScdcClient` and `HdmiPhy`; each method is the sync call itself, so its future is ready
  when first polled. The trainer polls `run` once with `core::task::Waker::noop()` (stable
  since Rust 1.85, plumbob's MSRV) and it completes: no executor, no runtime, `no_std`.
  A `Pending` there would be a plumbob bug and panics.
- **`plumbob-async`.** An adapter over async `ScdcClient` and `HdmiPhy` implementations,
  `.await`ing `run`.

`TrainingIo`'s futures are not required to be `Send`, as in `hdmi-hal-async`: the targets
are single-threaded executors and the sync driver.

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
- **One implementation, sync and async.** The state machine is written once, as an
  I/O-free `async fn`; the sync and async trainers only supply the I/O. A fix to the
  procedure reaches both.
- **State machine, not scattered logic.** The link training states are an explicit
  sequence. State transitions are clear, terminal states are explicit, and every exit
  point produces a typed result and leaves the sink in a defined state: TMDS after a
  fallback or an error, or, when LTS:L itself fails, an error saying which end did not
  get there. No implicit control flow, no silent completion.
- **Policy at the right layer.** plumbob implements the spec, not strategy. Which rates to
  try and in what order, retries beyond one call, and the decision of whether to surface
  a `FallbackRequired` to the user are the caller's concerns. Stepping down on the sink's
  request is the spec's LTS:4, so it happens inside `train`.
- **Transport and PHY errors are distinct.** A caller diagnosing a training failure
  needs to know whether it came from the I²C bus, the PHY, or the protocol. `TrainingError`
  keeps them separate.
- **No unsafe code.** `#![forbid(unsafe_code)]`.
- **Stable consumer types.** `TrainingOutcome`, `TrainingConfig`, and `TrainingTrace` are
  `#[non_exhaustive]` where appropriate. Callers are insulated from internal expansions.
- **Attested releases.** Every release is published through a GitHub Actions workflow
  that signs the `.crate` package with [SLSA Build Level 2](https://slsa.dev) provenance.
  Verify with `gh attestation verify <file> --repo DracoWhitefire/plumbob`.

---

## What plumbob uses from hdmi-hal

The procedure above relies on these parts of `hdmi-hal` (mirrored in `hdmi-hal-async`,
and recorded by `hdmi-hal-i2c-dev`'s `StubPhy`):

- **`LtpPattern` with the spec values**: 1 = all ones, 2 = all zeros, 3 = Nyquist clock,
  4 = DDE compliance and 5–8 = LFSR 0–3. `LtpReq::pattern` maps a request to the pattern
  of the same value.
- **Per-lane patterns.** `HdmiPhy::send_ltp` takes the full per-lane set
  (`LanePatterns`, an `Option<LtpPattern>` per lane, `None` meaning no pattern on that
  lane). The PHY applies the set as given; plumbob tracks which pattern each lane carries.
- **Per-lane TxFFE levels.** `LaneEqParams::tx_ffe_level` (a `TxFfeLevel`, 0–7), applied
  through `HdmiPhy::adjust_equalization`.
- **FRL output mode on `HdmiPhy`.** `set_frl_output(FrlOutput)`: plumbob sets `GapOnly`
  during training and LTS:P, and the caller sets `Active` (video, data islands and
  control) after `Success`. It sits on `HdmiPhy` because both plumbob and the integration
  layer drive it, and because `HdmiPhy` already carries link-level operations
  (`set_scrambling`, `send_ltp`).
- **Block reads on `ScdcTransport`.** `read_block` lets an SCDC implementation read
  `Status_Flags_1/2` (and the CED block) in one transaction when its transport can.

## Decisions

- The sink's rate-drop request is handled inside `train` (LTS:4) over the caller's rate
  list, and every non-success is `FallbackRequired { reason }` with a non-exhaustive
  `FallbackReason`.
- Poll limits default to 50 / 100 / 100 polls (100 / 200 / 200 ms at 2 ms per poll), with
  `FLT_no_timeout` honoured up to `no_timeout_poll_cap` (500 polls). The 200 ms
  `FRL_start` wait is the AMD and Intel drivers' (the Xilinx driver has none), and the
  500-poll cap is the AMD driver's (the Xilinx driver has none, the Intel series does not
  handle `FLT_no_timeout`).
- A lane's TxFFE level is raised up to the advertised maximum and held there.
- A Nyquist clock request (0x3) without `FLT_no_timeout` leaves the lane's previous pattern
  in place; plumbob tracks and sends the full per-lane set.
- FRL output control (`set_frl_output`) is part of `HdmiPhy`.
- In LTS:P, `FRL_start` is checked before `FLT_update` when both are set.
- Retraining from LTS:P keeps each lane's pattern and TxFFE level and starts LTS:3 with a
  fresh poll limit; levels are reset only in LTS:2 and LTS:4. Retraining is bounded by
  `max_retrains` (default 3) per `train` call, so the LTS:P ↔ LTS:3 cycle always ends.
  The reference drivers differ here: Xilinx returns to LTS:3 without a limit (its state
  machine is timer-driven, not blocking), AMD reruns the whole procedure up to 3 times,
  and Intel falls back to TMDS on the first request. The default borrows AMD's count;
  `max_retrains: 0` gives Intel's behaviour.
- A 0xF on some lanes but not all keeps those lanes' state, like 0x0; only 0xF on every
  active lane lowers the rate.
- The state machine does not call `read_ced`; CED counters are diagnostics.
