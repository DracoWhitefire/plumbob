# Roadmap

## Released

### 0.1.0

- Four-phase FRL training state machine (`FrlTrainer`, `train_at_rate`)
- `ScdcClient` trait — typed SCDC interface for the training procedure
- `TrainingConfig` with configurable per-phase iteration limits
- `TrainingOutcome` and `TrainingError`
- Owned protocol types: `LtpReq`, `FfeLevels`, `FrlConfig`, `TrainingStatus`,
  `CedCount`, `CedCounters`
- `TrainingTrace` and `train_at_rate_traced` (`alloc` feature)
- `no_std` support with `alloc` and `std` feature flags
- Simulation example
- `culvert` integration: `culvert` implements `plumbob::ScdcClient` for `Scdc<T>` via its
  `plumbob` cargo feature

### `plumbob-async`

Async companion crate mirroring `ScdcClient` and `FrlTrainer` with `async fn` methods,
following the same split as `hdmi-hal` / `hdmi-hal-async`. Shares all data types with
`plumbob` rather than duplicating them.

## Planned

### Mirrored SCDC types

plumbob's `ScdcClient` types will mirror culvert's register types field for field:
- the same names: `DdeCompliance` for the 0x4 request in both crates (Xilinx calls it
  RxDDE on the receiver and TxDDE on the transmitter), `frl_rate` for `FrlConfig`'s rate
  (the register field `FRL_Rate`);
- all 7 `Update_0` flags in `UpdateFlags`; plumbob still clears only the ones it handles;
- all 6 `Source_Test_Configuration` fields in `SourceTestConfig`, each recorded in
  `SourceTestConfigRead`, so the trace shows everything the compliance tester set.

This is a breaking change for plumbob, plumbob-async, culvert and culvert-async. It also
raises a question to settle against the reference drivers: whether plumbob honours the
TxFFE constraints and `FRL_Max` / `DSC_FRL_Max`, or leaves them to the caller with a
typed warning when the tester sets them.

### `LinkTrainer` trait

The integration layer above plumbob will define a `LinkTrainer` trait that plumbob
implements. This decouples the integration layer from plumbob specifically: any crate
that implements `LinkTrainer` is substitutable. The trait surface will be driven by what
the DRM/KMS integration needs to call — at minimum, `train` and recovery of the SCDC
client and PHY on completion.

