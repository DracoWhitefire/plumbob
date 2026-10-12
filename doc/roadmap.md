# Roadmap

What each release contains is in the [changelog](../CHANGELOG.md); this page is about
what comes next.

## Planned

### `LinkTrainer` trait

The integration layer above plumbob will define a `LinkTrainer` trait that plumbob
implements. This decouples the integration layer from plumbob specifically: any crate
that implements `LinkTrainer` is substitutable. The trait surface will be driven by what
the DRM/KMS integration needs to call — at minimum, `train` and recovery of the SCDC
client and PHY on completion.

