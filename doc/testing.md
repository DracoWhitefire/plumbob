# Testing Strategy

plumbob's test suite is built around deterministic state machine tests. All tests run
against scripted implementations of `ScdcClient` and `HdmiPhy`; no real hardware is
required at any point.

## Test structure

Tests live alongside the code they cover: `src/types.rs` for the protocol types, and
`src/training/tests.rs` for the state machine, using the simulated sink and PHY in
`src/training/sim.rs`.

### Type tests (`src/types.rs`)

Tests for the owned protocol types cover:

- `LtpReq` values matching the `Status_Flags` encoding, and `LtpReq::pattern` mapping
  0x1–0x8 to the `LtpPattern` of the same value (0x0, 0xE and 0xF have none)
- `FfeLevels` accepting 0–7, rejecting higher values, and `limited_to` capping it at 3 for
  every rate up to 12 Gbps
- Constructor invariants: `CedCount::new` strips the validity bit (`bits[14:0]` preserved,
  bit 15 masked off)
- Defaults and equality for `UpdateFlags`, `SourceTestConfig`, `LtpRequests` and the CED
  types

### The simulated sink and PHY (`src/training/sim.rs`)

- `SimSink` — a scripted sink, set up with builder methods so a test reads like its
  scenario: `FLT_ready` after N polls; a queue of LTS:3 *rounds*, each raising
  `FLT_update` with a set of per-lane requests after some polls (counted from the last
  `Config_1` write with an FRL rate); `FRL_start` after N polls once the rounds are used
  up; an optional source test configuration. A rate drop is a round of `RateChange`, and
  a retrain during LTS:P is a round after the all-`None` one. It records every call in
  order and can fail any one operation, on every call or only on its N-th call.
- `SimPhy` — records every PHY call in order and can fail any one operation, on every
  call or only on its N-th call.

The sim has its own tests, so it is fully covered before the state machine uses it.

### State machine tests (`src/training/tests.rs`)

- The main path LTS:2 → LTS:3 → LTS:P, asserting the complete SCDC and PHY call logs
- Each poll limit: timing out after exactly N polls, and continuing on the last allowed
  poll
- LTS:3's per-lane rules: a pattern per lane, 0x0 keeping a lane's pattern, 0x3 driven
  only under `FLT_no_timeout`, 0xE raising and holding a lane's TxFFE level, a partial
  0xF, and lane 3 ignored at the 3-lane rates
- `FLT_no_timeout` suspending the limits (from LTS:2 or LTS:3) up to its cap
- Retraining from LTS:P, bounded by `max_retrains` (including 0) and counted across a
  rate drop
- LTS:4: stepping down through the list, resetting the lanes, moving to a 3-lane rate, a
  fresh poll limit per rate, and running out of rates
- LTS:L: the exact exit sequence, clearing a pending `FLT_update`, every timeout
  ending in TMDS, and every step attempted when one of them fails, with the first error
  returned
- `TrainingError::Scdc` and `TrainingError::Phy` propagating from every SCDC and PHY
  operation the state machine uses, with LTS:L afterwards leaving both ends in TMDS
- After an error: a failed LTS:L reported per end (`TmdsExit::Failed`), the
  `ExitedToTmds` and `ExitToTmdsFailed` events, and `exit_to_tmds_on_error: false`
  leaving both ends untouched (`TmdsExit::Skipped`)
- A fallback whose LTS:L fails returning `TrainingError::ExitFailed` with each end's error

### Trace tests (requires `alloc` feature)

The `alloc`-gated tests exercise `train_traced` and `train_at_rate_traced` and assert on:

- The two example traces in [`architecture.md`](architecture.md), event for event
- Each timeout recording its limit and ending with `ExitedToTmds`, `RatesExhausted`, and
  `RetrainsExhausted`
- `SourceTestConfigRead` and `RetrainRequested` events, and no `FfeRaised` for a level
  already at the maximum
- `TrainingTrace` carrying the rates and the `TrainingConfig`, so poll counts in events
  are interpretable against the configured limits
- Traced and untraced runs agreeing on the outcome
- An error returned with its trace: the events up to the error, then LTS:L's event

### The sync driver

The state machine tests run through `FrlTrainer`, so they exercise `lts::run` driven
synchronously. One more test checks the driver's guard: a future that is still pending
after one poll — impossible over the sync adapter — panics rather than being dropped.

## Coverage

CI measures line coverage with `cargo-llvm-cov` over the `std` feature set. The baseline
is stored in `.coverage-baseline`; CI fails if coverage drops more than 0.1% below it.
New logic without tests will likely trip this.

## Philosophy

The training loop runs identically against simulated and real `ScdcClient` / `HdmiPhy`
implementations. A test that cannot run with a scripted `ScdcClient` does not belong in
this repository. Hardware is never a test dependency.
