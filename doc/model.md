# Data Model

## Owned protocol types

These types form the vocabulary of `ScdcClient` and the training state machine. They are
defined in plumbob because the state machine reasons in terms of them. SCDC implementations
convert to them at the impl boundary; the state machine uses them directly.

### `LtpReq` and `LtpRequests`

The link training pattern the sink requests for one lane: a 4-bit field in
`Status_Flags_1` (lanes 0–1) or `Status_Flags_2` (lanes 2–3).

```rust
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
    FfeChange       = 0xE,
    RateChange      = 0xF,
}

pub struct LtpRequests { pub lane0: LtpReq, pub lane1: LtpReq, pub lane2: LtpReq, pub lane3: LtpReq }
```

Values 0x1–0x8 are patterns the PHY drives on that lane; 0x0, 0xE and 0xF are signals to
the state machine and never reach the PHY. 0x3 (Nyquist clock) is only driven when the
sink sets `FLT_no_timeout`; otherwise the lane keeps its previous pattern. plumbob tracks
every lane's pattern and always sends the PHY the full per-lane set. Lane 3 is ignored in 3-lane FRL. Undefined
values (0x9–0xD) are rejected by the `ScdcClient` implementation as a protocol error.

### `FfeLevels`

The highest TxFFE level index the source supports, written to `Config_1` bits 7:4: 0–3 for
rates up to 12 Gbps, 0–7 above. During LTS:3 each lane's current level starts at 0, rises
by one for each 0xE request and is held at this value once reached. The default is 0.

### `FrlConfig`

Written to `Config_1` in LTS:2: the FRL rate (bits 3:0) and `FfeLevels` (bits 7:4).
`HdmiForumFrl::NotSupported` turns FRL off (LTS:L).

### `UpdateFlags` and `SourceTestConfig`

The `Update_0` flags the state machine reads and clears (`source_test_update`, `frl_start`,
`flt_update`), and the `Source_Test_Configuration` field it honours (`flt_no_timeout`).
Other `Update_0` and `Source_Test_Configuration` fields are not part of training and stay
in the SCDC implementation.

### `CedCount` and `CedCounters`

`CedCount` is a 15-bit per-lane character error count. The high bit of the raw register
value is a validity flag; `CedCount::new` masks it off:

```rust
CedCount::new(raw) // stores raw & 0x7FFF
```

`CedCounters` holds one `Option<CedCount>` per lane. `None` means the validity bit was not
set. They are diagnostics; the training procedure does not consume them. `lane3` is
`None` in 3-lane FRL mode.

---

## Training configuration and outcome types

### `TrainingConfig`

Per-attempt configuration, constructed via `Default` and overridden as needed:

| Field | Default | Meaning |
|---|---|---|
| `ffe_levels` | `0` | Highest TxFFE level advertised in `Config_1` (limited per rate) |
| `flt_ready_polls` | `50` | Poll limit for `FLT_ready` in LTS:2 (100 ms at 2 ms/poll) |
| `ltp_polls` | `100` | Poll limit for LTS:3 (200 ms at 2 ms/poll) |
| `frl_start_polls` | `125` | Poll limit for `FRL_start` in LTS:P (250 ms at 2 ms/poll) |
| `no_timeout_poll_cap` | `100_000` | Hard cap on polls while the sink sets `FLT_no_timeout` |

Poll limits are exact counts: N means exactly N polls before the state gives up. The
defaults reproduce the spec's 100 ms and 200 ms timeouts (and the Xilinx driver's 250 ms
`FRL_start` wait) at one poll every 2 ms; callers polling at a different cadence should
scale them.

`TrainingConfig` is `#[non_exhaustive]` and derives `Clone` and `Copy`.

### `TrainingOutcome` vs. `TrainingError`

These are distinct result types representing different failure modes:

- **`TrainingOutcome::FallbackRequired { reason }`** — training did not succeed at any of
  the rates passed in. `reason` says why: `FltReadyTimeout`, `TrainingTimeout` or
  `FrlStartTimeout` (a poll limit expired in LTS:2, LTS:3 or LTS:P), or `RatesExhausted`
  (the sink kept requesting a lower rate past the end of the list). `FallbackReason` is
  `#[non_exhaustive]`. A sink's request for a lower rate within the list is not an
  outcome: `train` steps down (LTS:4) and continues.
- **`TrainingError::Scdc(e)` / `TrainingError::Phy(e)`** — a hard I/O failure from the
  SCDC client or PHY. Something failed at the transport level, unrelated to whether the
  link could have trained at this rate.

`FallbackRequired` always leaves the sink in TMDS (LTS:L). This distinction matters for
diagnostics: an outcome chain ending in TMDS is expected on marginal hardware; a
`TrainingError` means the bus or PHY needs attention.

---

## Type ownership and the culvert boundary

plumbob owns the types that form the vocabulary of `ScdcClient`. `culvert` independently
defines its own register-layer types (`culvert::LtpReq`, `culvert::FfeLevels`, etc.) as
the output of SCDC register decoding. The two sets are structurally identical but exist at
different layers and can evolve independently.

When `culvert` implements `plumbob::ScdcClient` (via its `plumbob` cargo feature), each
trait method calls one culvert method and converts culvert's type to plumbob's at the impl
boundary (`culvert::LtpRequests` → `plumbob::LtpRequests`, `culvert::UpdateFlags` →
`plumbob::UpdateFlags`, and so on).

The `From` impls between corresponding types live in a feature-gated module in `culvert`.
`culvert`'s own types are unchanged; the conversion is confined to the impl. This keeps
both crates independently usable without forcing plumbob into every culvert user's
dependency graph.
