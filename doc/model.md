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
    None,           // 0x0: lane trained
    AllOnes,        // 0x1
    AllZeros,       // 0x2
    NyquistClock,   // 0x3
    DdeCompliance,  // 0x4
    Lfsr0,          // 0x5
    Lfsr1,          // 0x6
    Lfsr2,          // 0x7
    Lfsr3,          // 0x8
    FfeChange,      // 0xE
    RateChange,     // 0xF
    Reserved(u8),   // 0x9–0xD
}

pub struct LtpRequests { pub lane0: LtpReq, pub lane1: LtpReq, pub lane2: LtpReq, pub lane3: LtpReq }
```

Values 0x1–0x8 are patterns the PHY drives on that lane; 0x0, 0xE and 0xF are signals to
the state machine and never reach the PHY. 0x3 (Nyquist clock) is only driven when the
sink sets `FLT_no_timeout`; otherwise the lane keeps its previous pattern. plumbob tracks
every lane's pattern and always sends the PHY the full per-lane set. Lane 3 is ignored in
3-lane FRL. The values the specification leaves undefined (0x9–0xD) are
`LtpReq::Reserved(value)`: an `ScdcClient` implementation passes them on rather than
failing, and plumbob leaves the lane as it was, records `UndefinedLtpRequest` and
returns a `TrainingWarning` with the outcome.
`LtpReq::value()` returns a request's 4-bit value.

### `FfeLevels`

The highest TxFFE level index the source supports, written to `Config_1` bits 7:4: 0–3 for
rates up to 12 Gbps, 0–7 above. During LTS:3 each lane's current level starts at 0, rises
by one for each 0xE request and is held at this value once reached. `FfeLevels::default()`
is 0; `TrainingConfig`'s default advertises 3.

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
| `ffe_levels` | `3` | Highest TxFFE level advertised in `Config_1` (limited per rate); 0 for a PHY without TxFFE |
| `flt_ready_polls` | `50` | Poll limit for `FLT_ready` in LTS:2 (100 ms at 2 ms/poll) |
| `ltp_polls` | `100` | Poll limit for LTS:3 (200 ms at 2 ms/poll) |
| `frl_start_polls` | `100` | Poll limit for `FRL_start` in LTS:P (200 ms at 2 ms/poll) |
| `no_timeout_poll_cap` | `500` | The LTS:2, LTS:3 and LTS:P limit while the sink sets `FLT_no_timeout` (1 s); then `NoTimeoutHold` |
| `max_retrains` | `3` | Returns from LTS:P to LTS:3 allowed per `train` call; applies under `FLT_no_timeout` too |
| `exit_to_tmds_on_error` | `true` | Whether an SCDC or PHY error is followed by LTS:L |

Poll limits are exact counts: N means exactly N polls before the state gives up. The
defaults reproduce the spec's 100 ms and 200 ms timeouts (and the AMD and Intel drivers'
200 ms `FRL_start` wait) at one poll every 2 ms; callers polling at a different cadence
should scale them. The `FLT_no_timeout` cap's value and the retrain count follow the AMD
driver; what happens when the cap runs out is plumbob's (see `NoTimeoutHold` below).

`TrainingConfig` is `#[non_exhaustive]` and derives `Clone` and `Copy`.

### `Trained` and `TrainingWarning`

`train` returns `Trained`: the `TrainingOutcome` and the `TrainingWarning`s the attempt
produced. A warning is a non-fatal anomaly that did not change the outcome:

| Warning | Meaning |
|---|---|
| `UndefinedLtpRequest { lane, value, count, in_use }` | The lane requested an undefined value (0x9–0xD) `count` times, the last being `value`; plumbob left the lane as it was. `in_use` is false for lane 3 at a 3-lane rate. |

Repeats are merged per kind and lane (and `in_use`), so an attempt has at most five
warnings. With `alloc`, `warnings` is a `Vec`; without it, a `[Option<TrainingWarning>;
MAX_WARNINGS]` with `num_warnings` in use. `iter_warnings()` reads either.
`TrainingWarning` and `Trained` are `#[non_exhaustive]`; `Trained` is `Copy` only without
`alloc`. An attempt that ends in a `TrainingError` has no `Trained`: its trace records
what happened.

### `TrainingOutcome` vs. `TrainingError`

These are distinct result types representing different failure modes:

- **`TrainingOutcome::FallbackRequired { reason }`** — training did not succeed at any of
  the rates passed in. `reason` says why: `FltReadyTimeout`, `TrainingTimeout` or
  `FrlStartTimeout` (a poll limit expired in LTS:2, LTS:3 or LTS:P), `RatesExhausted`
  (the sink kept requesting a lower rate past the end of the list), or `RetrainsExhausted`
  (the sink kept requesting retraining past `max_retrains`). `FallbackReason` is
  `#[non_exhaustive]`. A sink's request for a lower rate within the list is not an
  outcome: `train` steps down (LTS:4) and continues.
- **`TrainingOutcome::NoTimeoutHold { rate }`** — the sink set `FLT_no_timeout` (it is
  under compliance test) and `no_timeout_poll_cap` ran out in LTS:2, LTS:3 or LTS:P.
  plumbob leaves the link as it is, with no LTS:L, because the test equipment controls
  it; the caller keeps it up, trains again, or calls `exit_to_tmds`.
- **`TrainingError::Scdc { error, exit }` / `TrainingError::Phy { error, exit }`** — a
  hard I/O failure from the SCDC client or PHY. Something failed at the transport level,
  unrelated to whether the link could have trained at this rate. `exit` is a `TmdsExit`:
  LTS:L ran afterwards and both ends are in TMDS (`Exited`); it ran and a step failed
  (`Failed(ExitError)`); or it was turned off with `exit_to_tmds_on_error` (`Skipped`).
  An `ExitError` has each end's first LTS:L error, `scdc` and `phy`, with `None` for an
  end that completed its steps and is in TMDS.
- **`TrainingError::ExitFailed { reason, error }`** — the attempt fell back for `reason`,
  and LTS:L then failed with the `ExitError` `error`.
- **`TrainingError::InvalidRates { index, rate }`** — the rate list cannot be trained over:
  `rate`, at `index`, is `NotSupported` or not strictly lower than the rate before it
  (LTS:4 steps down). Nothing was done; no SCDC or PHY operation was performed.
- **`TrainingError::NoRates`** — the rate list is empty: nothing to train at, and nothing
  was done. With every list either rejected or trained, a `FallbackRequired` always
  follows training and always leaves both ends in TMDS.

`FrlTrainer::exit_to_tmds` runs LTS:L on its own and returns `Result<(), ExitError>`.

`FallbackRequired` always leaves the sink in TMDS (LTS:L). This distinction matters for
diagnostics: an outcome chain ending in TMDS is expected on marginal hardware; a
`TrainingError` means the bus or PHY needs attention. `TrainingError`, `TmdsExit` and
`ExitError` are `#[non_exhaustive]`.

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
