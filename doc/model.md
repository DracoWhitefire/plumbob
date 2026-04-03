# Data Model

## Owned protocol types

These types form the vocabulary of `ScdcClient` and the training state machine. They are
defined in plumbob because the state machine reasons in terms of them. SCDC implementations
convert to them at the impl boundary; the state machine uses them directly.

### `LtpReq`

The Link Training Pattern requested by the sink, decoded from `Status_Flags_1 bits[7:4]`.

```rust
pub enum LtpReq {
    None  = 0,  // all lanes satisfied; training loop exits
    Lfsr0 = 1,
    Lfsr1 = 2,
    Lfsr2 = 3,
    Lfsr3 = 4,
}
```

`LtpReq::None` is the terminal condition for phase 4 and never reaches
`HdmiPhy::send_ltp`. The training loop checks for it explicitly before converting to
`LtpPattern`. The discriminant values match the raw bit field encoding from the HDMI 2.1
specification; `From<LtpReq> for LtpPattern` uses `req as u8` directly.

### `FfeLevels`

The number of Feed-Forward Equalization levels advertised to the sink in Config_0. Values
`Ffe0`–`Ffe7` correspond to 0–7 levels. The default is `Ffe0`.

### `FrlConfig`

The configuration written to SCDC Config_0 in phase 1.

```rust
pub struct FrlConfig {
    pub rate: HdmiForumFrl,
    pub ffe_levels: FfeLevels,
    pub dsc_frl_max: bool,
}
```

`dsc_frl_max` reflects whether the negotiated configuration requires DSC transport.
plumbob passes it through from `TrainingConfig` without interpreting it.

### `TrainingStatus`

The subset of SCDC status read on each poll.

```rust
pub struct TrainingStatus {
    pub flt_ready: bool,   // sink completed internal preparation (phase 2 exit condition)
    pub frl_start: bool,   // sink ready for LTP loop (phase 3 exit condition)
    pub ltp_req:   LtpReq, // sink's current LTP request (phase 4 signal)
}
```

`ScdcClient` implementers are responsible for correctly mapping SCDC register bits to
these fields. plumbob trusts the returned values without re-validation. See the
`ScdcClient` trait documentation for the full contract.

### `CedCount` and `CedCounters`

`CedCount` is a 15-bit per-lane character error count. The high bit of the raw register
value is a validity flag; `CedCount::new` masks it off:

```rust
CedCount::new(raw) // stores raw & 0x7FFF
```

`CedCounters` holds one `Option<CedCount>` per lane. `None` means the validity bit was
not set in the hardware register. Implementers of `ScdcClient::read_ced` are responsible
for this mapping. `lane3` is always `None` in 3-lane FRL mode.

---

## Training configuration and outcome types

### `TrainingConfig`

Per-attempt configuration, constructed via `Default` and overridden as needed:

| Field | Default | Meaning |
|---|---|---|
| `ffe_levels` | `Ffe0` | FFE levels advertised in Config_0 |
| `dsc_frl_max` | `false` | Set DSC_FRL_Max flag in Config_0 |
| `flt_ready_timeout` | `1000` | Max polls in phase 2 |
| `frl_start_timeout` | `1000` | Max polls in phase 3 |
| `ltp_timeout` | `1000` | Max iterations in phase 4 |

All timeout values are exact iteration counts: a value of N means exactly N polls are
attempted before the phase gives up (not N−1). The default of 1000 is a reasonable
starting point but is not tuned for any specific hardware; callers should adjust based on
their polling cadence and inter-poll delay.

`TrainingConfig` is `#[non_exhaustive]` and derives `Clone` and `Copy`.

### `TrainingOutcome` vs. `TrainingError`

These are distinct result types representing different failure modes:

- **`TrainingOutcome::FallbackRequired`** — the link did not converge at this rate within
  the configured timeouts. This is a normal protocol outcome; the caller should retry at
  a lower rate or fall back to TMDS.
- **`TrainingError::Scdc(e)` / `TrainingError::Phy(e)`** — a hard I/O failure from the
  SCDC client or PHY. Something failed at the transport level, unrelated to whether the
  link could have trained at this rate.

This distinction matters for diagnostics: a `FallbackRequired` chain ending in TMDS is an
expected training outcome on marginal hardware; a `TrainingError` means the bus or PHY
needs attention.

---

## Type ownership and the culvert boundary

plumbob owns the types that form the vocabulary of `ScdcClient`. `culvert` independently
defines its own register-layer types (`culvert::LtpReq`, `culvert::FfeLevels`, etc.) as
the output of SCDC register decoding. The two sets are structurally identical but exist at
different layers and can evolve independently.

When `culvert` implements `plumbob::ScdcClient` (via its `plumbob` cargo feature), it
converts at the impl boundary:

```rust
fn read_training_status(&mut self) -> Result<plumbob::TrainingStatus, ...> {
    let flags = self.read_status_flags()?;
    Ok(plumbob::TrainingStatus {
        flt_ready: flags.flt_ready,
        frl_start: flags.frl_start,
        ltp_req: flags.ltp_req.into(), // culvert::LtpReq → plumbob::LtpReq
    })
}
```

The `From` impls between corresponding types live in a feature-gated module in `culvert`.
`culvert`'s own types are unchanged; the conversion is confined to the impl. This keeps
both crates independently usable without forcing plumbob into every culvert user's
dependency graph.
