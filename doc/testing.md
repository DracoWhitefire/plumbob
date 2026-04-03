# Testing Strategy

plumbob's test suite is built around deterministic state machine tests. All tests run
against scripted implementations of `ScdcClient` and `HdmiPhy`; no real hardware is
required at any point.

## Test structure

All tests live alongside the code they cover in `src/training.rs` and `src/types.rs`.

### Type tests (`src/types.rs`)

Tests for the owned protocol types cover:

- Constructor invariants: `CedCount::new` strips the validity bit (`bits[14:0]` preserved,
  bit 15 masked off)
- Derived trait behaviour: `PartialEq`, `Clone`, `Copy` for all types
- Enum discriminant values: all `LtpReq` and `FfeLevels` variants match the spec encoding
- `From<LtpReq> for LtpPattern` conversions for all four LFSR variants

### State machine tests (`src/training.rs`)

The sim harness consists of two types:

- `SimScdc` — a scripted `ScdcClient` backed by a `VecDeque<TrainingStatus>`. Each call
  to `read_training_status` pops the next entry, simulating a sink's phase-by-phase
  responses. Errors can be injected by inserting a scripted error value.
- `MockPhy` — a minimal `HdmiPhy` that records calls for assertion.

Every branch in `train_at_rate` has a corresponding test:

- Successful training: `flt_ready` and `frl_start` assert after N polls; `ltp_req`
  transitions through one or more patterns before reaching `None`
- Each phase timing out independently (phases 2, 3, and 4)
- Each phase succeeding on the first iteration (`after_iterations: 0`)
- `TrainingError::Scdc` propagating from each of the three `ScdcClient` methods
- `TrainingError::Phy` propagating from `set_frl_rate` and `send_ltp`
- `into_parts` recovering the SCDC client and PHY after a completed attempt

### Trace tests (requires `alloc` feature)

The `alloc`-gated tests exercise `train_at_rate_traced` and assert on:

- Event order and variant presence for all terminal states (success, three timeout variants)
- `LtpPatternRequested` emitted only on `ltp_req` transitions, not once per poll
- `after_iterations` / `iterations_elapsed` counts matching the scripted queue
- `TrainingTrace.config` matching the `TrainingConfig` passed to `train_at_rate_traced`,
  so that timeout counts in events are interpretable against the configured limits

## Coverage

CI measures line coverage with `cargo-llvm-cov` over the `std` feature set. The baseline
is stored in `.coverage-baseline`; CI fails if coverage drops more than 0.1% below it.
New logic without tests will likely trip this.

## Philosophy

The training loop runs identically against simulated and real `ScdcClient` / `HdmiPhy`
implementations. A test that cannot run with a scripted `ScdcClient` does not belong in
this repository. Hardware is never a test dependency.
