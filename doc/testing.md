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
- Defaults and equality for `UpdateFlags`, `SourceTestConfig` and `LtpRequests`
- The constructors that take fields in register or lane order (`UpdateFlags::new`,
  `ClearableUpdateFlags::new`, `SourceTestConfig::new`, `LtpRequests::new`,
  `FrlConfig::new`): each field lands where its bit or lane says, since
  positional `bool` and lane arguments are easy to swap; `clearable()` drops only
  `RR_Test`; `LtpRequests::all_trained` counts lane 3.

### The simulated sink and PHY (`src/training/sim.rs`)

- `SimSink` — a scripted sink, set up with builder methods so a test reads like its
  scenario: `FLT_ready` after N polls; a queue of LTS:3 *rounds*, each raising
  `FLT_update` with a set of per-lane requests after some polls (counted from the last
  `Config_1` write with an FRL rate); `FRL_start` after N polls once the rounds are used
  up; an optional source test configuration, also changed with a round or on its own
  (`source_test_after`, setting `Source_Test_Update` without `FLT_update`); the `Update_0`
  flags training does not act on (`other_flags`). A rate drop is a round of `RateChange`, and
  a retrain during LTS:P is a round after the all-`None` one. It records every call in
  order and can fail any one operation, on every call or only on its N-th call.
- `SimPhy` — records every PHY call in order and can fail any one operation, on every
  call or only on its N-th call.
- A shared `Log` (`log_to` on both) records the sink's and the PHY's calls interleaved, in
  the order they happened, for tests of step order across the two.

The sim has its own tests, so it is fully covered before the state machine uses it.

### Warning tests (`src/warning.rs`)

Merging: repeats on a lane keep the last value and a count, lane 3 in and out of use are
kept apart, and the five distinct warnings an attempt can produce fit without `alloc`.

### State machine tests (`src/training/tests.rs`)

- The main path LTS:2 → LTS:3 → LTS:P, asserting the complete SCDC and PHY call logs
- Each poll limit: timing out after exactly N polls, and continuing on the last allowed
  poll
- LTS:3's per-lane rules: a pattern per lane, 0x0 keeping a lane's pattern, 0x3 driven
  only under `FLT_no_timeout`, 0xE raising and holding a lane's TxFFE level, a partial
  0xF, lane 3 ignored at the 3-lane rates (a pattern, 0xE or an undefined value on it
  changes nothing), both 3-lane rates driving three lanes, and undefined values
  (`Reserved`) leaving a
  lane as it was without counting as a pass or a rate change
- `FLT_no_timeout` suspending the limits (from LTS:2 or LTS:3) up to its cap, holding for
  every attempt while it stays set, and the cap ending LTS:2, LTS:3 or LTS:P with
  `NoTimeoutHold` and no LTS:L (while LTS:P without it still falls back)
- `FRL_start` and `FLT_update` set together in LTS:P: the retrain wins, within
  `max_retrains`
- Retraining from LTS:P: resuming LTS:3 with no pattern on any lane and the TxFFE levels
  kept — no reset sent, and plumbob's own model holding them, so a further 0xE after the
  retrain raises a lane from its level, not from 0 — bounded by `max_retrains` (including 0), counted across a rate drop, and still
  falling back when exhausted under `FLT_no_timeout`
- Warnings: undefined requests returned as `TrainingWarning`s, merged per lane with a
  count, on a success and on a fallback, and none for a clean attempt
- The rate list: `NotSupported` and rates that are not strictly descending rejected as
  `InvalidRates`, and an empty list as `NoRates`, with no SCDC or PHY call; every FRL
  rate, in order, walked through
- LTS:4: stepping down through the list, resetting the lanes, moving to a 3-lane rate, a
  fresh poll limit per rate, and running out of rates
- LTS:L: the exact exit sequence, clearing a pending `FLT_update`, every timeout
  ending in TMDS, and every step attempted when one of them fails, with the first error
  returned
- `TrainingError::Scdc` and `TrainingError::Phy` propagating from every SCDC and PHY
  operation the state machine uses, with LTS:L afterwards leaving both ends in TMDS — at
  every call site, not only an operation's first: three scenarios (a success with a
  source test configuration, a TxFFE raise, a rate drop and a retrain; a fallback through
  LTS:L; and a `Source_Test_Update` in LTS:3 that changes the TxFFE restrictions, then
  another change on LTS:P's first poll together with `FRL_start` and `FLT_update`) are each
  run once per call they make, failing that one call, and every failure must be reported.
  A site no scenario reaches is not checked, so a branch with I/O of its own needs a
  scenario that takes it; region coverage shows the three reach every one
- The step order across sink and PHY in LTS:2 (rate, then TxFFE reset), LTS:3 (the
  requested pattern and level reach the PHY before `FLT_update` is cleared), LTS:P (the
  patterns stopped and gap characters sent before `FLT_update` is cleared) and LTS:4
  (patterns stopped, then LTS:2's end at the new rate: `FLT_update` cleared, rate, TxFFE
  reset, patterns and gap characters restored, `Config_0`, `Config_1`), from a log of both
  in order
- `FLT_no_timeout` holding across a rate drop and a retrain within one call, and a TxFFE
  raise followed under it
- After an error: a failed LTS:L reported per end (`TmdsExit::Failed`), the
  `ExitedToTmds` and `ExitToTmdsFailed` events, and `exit_to_tmds_on_error: false`
  leaving both ends untouched (`TmdsExit::Skipped`); under `FLT_no_timeout` an error
  holding the link (`HeldUnderTest`), and exiting once the tester has cleared the flag
- A fallback whose LTS:L fails returning `TrainingError::ExitFailed` with each end's error
- `train_with_events` reporting each event as it occurs, without `alloc`
- `exit_to_tmds` on demand: the LTS:L sequence, taking down a link an error left in FRL,
  each end's error on failure, and `lts::exit_to_tmds` recording its event
- `Update_0`: plumbob clears only the three flags it acts on, one at a time, and a sink's
  `Status_Update`, `CED_Update`, `RR_Test` and `RSED_Update` stay set through training
- `Source_Test_Configuration`: every bit recorded in `SourceTestConfigRead`; a
  `Source_Test_Update` handled on the poll that sees it, without `FLT_update` in LTS:3 and
  before `FRL_start` in LTS:P
- TxFFE restrictions: sent with every equalization; kept across a rate drop and a retrain;
  a change in LTS:3 sent at once with the levels held, before the round's requests are
  acted on; an unchanged re-read not sent again
- The deadline pattern in architecture.md: one failed polled read in LTS:3 stops training,
  and LTS:L, whose read is not a poll, still returns the link to TMDS; a client following
  the rule (no failed read after `write_frl_config(NotSupported)`) lets an attempt whose
  deadline lands on LTS:L's read end with its own result, and one that does not turns it
  into `ExitFailed`
- Every new test above was checked against a deliberate bug (a reset of the restrictions
  on a rate drop or a retrain, a trip on LTS:2's read, an extra cleared flag, a late
  re-send): each fails its test

### Trace tests (requires `alloc` feature)

The `alloc`-gated tests exercise `train_traced` and `train_at_rate_traced` and assert on:

- The two example traces in [`architecture.md`](architecture.md), event for event
- Each timeout recording its limit and ending with `ExitedToTmds`, `RatesExhausted`, and
  `RetrainsExhausted`
- `SourceTestConfigRead` and `RetrainRequested` events, and no `FfeRaised` for a level
  already at the maximum
- `UndefinedLtpRequest` for an undefined value on every lane, with lane 3 at a 3-lane rate
  marked as not in use
- `TrainingTrace` carrying the rates and the `TrainingConfig`, so poll counts in events
  are interpretable against the configured limits
- Traced and untraced runs agreeing on the outcome
- `train_with_events` delivering exactly the events `train_traced` collects
- An error returned with its trace: the events up to the error, then LTS:L's event

### The sync driver

The state machine tests run through `FrlTrainer`, so they exercise `lts::run` driven
synchronously. One more test checks the driver's guard by running `block_on` over a
future that waits once — impossible over the sync adapter — with the trainer's own output
type: it panics rather than dropping the attempt.

A size test keeps the training future within a budget (456 bytes without `alloc`, 432
with it, over the simulator's zero-sized error types), so a change that grows it fails until the budget is raised on purpose. CI runs
the tests with the `std` feature and again with no features, so both budgets are
checked.

## Coverage

CI measures line coverage with `cargo-llvm-cov` over the `std` feature set. The baseline
is stored in `.coverage-baseline`; CI fails if coverage drops more than 0.1% below it.
New logic without tests will likely trip this.

## Philosophy

The training loop runs identically against simulated and real `ScdcClient` / `HdmiPhy`
implementations. A test that cannot run with a scripted `ScdcClient` does not belong in
this repository. Hardware is never a test dependency.
