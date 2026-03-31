# Architecture

## Role

`plumbob` implements the Fixed Rate Link (FRL) training state machine defined in the HDMI
2.1 specification. It sits above `culvert` (typed SCDC register access) in the stack, and
alongside `hdmi-hal` (PHY configuration traits), running the state machine that determines
the maximum viable FRL tier for a given source–sink–cable combination.

Training establishes actual link capability, not just theoretical negotiation. A
`NegotiatedConfig` from concordance identifies what the hardware should support; link
training determines what it actually achieves. The caller is responsible for deciding what
to do with a `FallbackRequired` outcome — whether to retry at a lower tier, fall back to
TMDS, or surface the failure.

---

## Scope

plumbob covers:

- the FRL link training state machine: three-phase training per HDMI 2.1 §10.x,
- `FrlTrainer<T, P>`: the central type, owning an SCDC client and a PHY,
- `TrainingOutcome`: the result of a single training attempt (`Success` or `FallbackRequired`),
- `TrainingConfig`: per-attempt configuration (FFE levels, iteration limits),
- `TrainingError`: hard failures (transport or protocol errors),
- simulation support: the training procedure is fully exercisable without real hardware
  by using simulated implementations of `ScdcTransport` and `HdmiPhy`.

The following are out of scope:

- **Rate fallback policy** — `train_at_rate` returns `FallbackRequired`; the caller
  decides the retry sequence. plumbob does not maintain a rate table or retry loop.
- **SCDC register knowledge** — plumbob calls culvert's typed methods and does not
  define or decode any SCDC register fields itself.
- **PHY vendor sequences** — plumbob calls `HdmiPhy` methods; the register sequences
  for lane reconfiguration are in platform PHY backends.
- **Timing** — plumbob is synchronous and poll-based. No sleep, no timers. Timing
  between polls is implicit in the transport; the iteration limits in `TrainingConfig`
  are the only timeout mechanism.
- **TMDS link setup** — plumbob handles FRL training only. TMDS mode is the fallback
  that concordance selects if no FRL tier trains successfully; plumbob has no role in it.
- **CED-driven equalization** — CED counter feedback from culvert will eventually
  inform equalization adjustments during the LTP loop. This is blocked on `EqParams`
  being expanded; the placeholder call exists but does nothing useful yet.

---

## Dependencies

```
display-types  ─┐
hdmi-hal       ─┤
culvert        ─┴─►  plumbob
```

- `culvert` — `Scdc<T>`, `FrlConfig`, `StatusFlags`, `LtpReq`, `FfeLevels`, `ScdcError`
- `hdmi-hal` — `ScdcTransport`, `HdmiPhy`, `EqParams`
- `display-types` — `HdmiForumFrl`

plumbob does not depend on `concordance` or `piaf`. It receives a target FRL rate from the
caller (typically chosen from concordance's ranked output) and trains at that rate.

---

## Training Procedure (HDMI 2.1 §10.x)

FRL link training has three phases. `train_at_rate` runs the full sequence for a single
rate and returns when it reaches a terminal state.

### Phase 1 — Configuration

1. Write the target FRL rate and FFE level count to `Config_0` via
   `Scdc::write_frl_config`.
2. Call `HdmiPhy::set_frl_rate` to configure the physical lanes for this rate.

Configuration is a write-then-forget step. The sink detects the rate change and begins its
own internal preparation.

### Phase 2 — Initiation

Poll `Scdc::read_status_flags` until the sink asserts `frl_start`. This flag signals that
the sink has acknowledged the requested rate and is ready for the LTP training loop.

If `frl_start` does not assert within `TrainingConfig::flt_ready_timeout` iterations, the
attempt terminates with `TrainingOutcome::FallbackRequired`.

### Phase 3 — LTP Loop

On each iteration:
1. Read `StatusFlags::ltp_req`.
2. If `LtpReq::None` — all lanes are satisfied. Training succeeded.
3. Otherwise — drive the requested Link Training Pattern on the PHY lanes and iterate.

If `ltp_req` does not reach `None` within `TrainingConfig::ltp_timeout` iterations, the
attempt terminates with `TrainingOutcome::FallbackRequired`.

**Open item:** Step 3 requires a `send_ltp(req: LtpReq)` method on `HdmiPhy`. The trait
does not currently have this method. Until it is added, the LTP phase falls back to calling
`adjust_equalization` as a placeholder. See [Open Items](#open-items).

---

## Key Types

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
pub struct TrainingConfig {
    /// FFE levels advertised to the sink in Config_0.
    pub ffe_levels: FfeLevels,
    /// Maximum poll iterations waiting for frl_start (phase 2).
    pub flt_ready_timeout: u32,
    /// Maximum poll iterations in the LTP training loop (phase 3).
    pub ltp_timeout: u32,
}

/// Hard error that terminated a training attempt.
///
/// Distinct from FallbackRequired: this means something failed at the transport
/// or protocol level, not that the link simply didn't train at this rate.
pub enum TrainingError<E, F> {
    /// The SCDC transport or a protocol violation from culvert.
    Scdc(ScdcError<E>),
    /// The PHY returned an error.
    Phy(F),
}

/// The central training type. Wraps a culvert SCDC client and an HdmiPhy.
pub struct FrlTrainer<T, P> { ... }

impl<T: ScdcTransport, P: HdmiPhy> FrlTrainer<T, P> {
    pub fn new(scdc: Scdc<T>, phy: P) -> Self;
    pub fn into_parts(self) -> (Scdc<T>, P);

    /// Run the full three-phase training sequence at the given FRL rate.
    pub fn train_at_rate(
        &mut self,
        rate: HdmiForumFrl,
        config: &TrainingConfig,
    ) -> Result<TrainingOutcome, TrainingError<T::Error, P::Error>>;
}
```

`TrainingConfig` is `#[non_exhaustive]` and implements `Default`. The defaults are
reasonable for hardware use but are not tuned for any specific platform; callers should
adjust the timeout values for their polling cadence.

---

## The plumbob / culvert Boundary

Like the culvert / hdmi-hal boundary, this one is worth stating explicitly.

**culvert's responsibility:** typed register access. Given a desired FRL rate, write it
into `Config_0`. Given a status register, decode it into `StatusFlags`. culvert does not
know what to do with a `StatusFlags`; it only knows how to read one.

**plumbob's responsibility:** the state machine. Receive a target FRL rate from the caller.
Write `Config_0`. Poll for `frl_start`. Handle `ltp_req`. Declare success or fall back.
That sequencing, the iteration limits, and the fallback signal live here — not in culvert.

The rule: if it touches state across multiple register accesses, timeout logic, or the
decision of what to do with a register value, it belongs in plumbob. If it reads or writes
registers and returns typed results, it belongs in culvert.

---

## `no_std` Compatibility

plumbob requires no allocator. All types are stack-allocated. `FrlTrainer<T, P>` owns a
`Scdc<T>` and a `P`; both are caller-supplied and caller-sized. No heap use anywhere in
the training loop. The full API is available in bare `no_std` environments.

---

## Design Principles

- **Deterministic and testable.** The training procedure runs identically against a
  simulated register array and real hardware. Pre-load the simulated `ScdcTransport`
  with the register values a sink would produce at each training phase, run the state
  machine, assert on the outcome. No hardware required for any test.
- **State machine, not scattered logic.** The three phases are an explicit sequence.
  Phase transitions are clear, terminal states are explicit, and every exit point
  produces a typed result. No implicit control flow, no silent completion.
- **Policy at the right layer.** plumbob implements the spec, not strategy. Rate
  fallback order, retry counts above the per-attempt limit, and the decision of whether
  to surface a `FallbackRequired` to the user are the caller's concerns.
- **Transport and PHY errors are distinct.** A caller diagnosing a training failure
  needs to know whether it came from the I²C bus, the PHY, or the protocol. `TrainingError`
  keeps them separate.
- **No unsafe code.** `#![forbid(unsafe_code)]`.
- **Stable consumer types.** `TrainingOutcome` and `TrainingConfig` are `#[non_exhaustive]`
  where appropriate. Callers are insulated from internal expansions.

---

## Open Items

**`HdmiPhy::send_ltp`** — Phase 3 of training requires the source to drive a specific Link
Training Pattern on the physical lanes, as requested by the sink via `LTP_Req`. This is a
PHY operation. `HdmiPhy` currently has `set_frl_rate`, `adjust_equalization`, and
`set_scrambling`, but no method for driving LTP patterns. Until `send_ltp(req: LtpReq)`
(or equivalent) is added to `hdmi-hal`, the LTP loop calls `adjust_equalization` as a
placeholder. The state machine structure is complete; only this one call is a stub.

**`EqParams` expansion** — `EqParams` in hdmi-hal is currently an empty placeholder struct.
CED counters (per-lane character error counts, readable via `Scdc::read_ced`) will feed
equalization adjustments during the LTP loop once `EqParams` is expanded to carry the
relevant fields. The training loop already reads CED data; the adjustment call is the stub.
