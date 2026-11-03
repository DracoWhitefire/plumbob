# Roadmap

What each release contains is in the [changelog](../CHANGELOG.md); this page is about
what comes next.

## Planned

### Resuming training at LTS:3 from active FRL

When the sink sets `FLT_update` during active video, source implementations stop sending
video and resume training at LTS:3, rather than starting again at LTS:2: Xilinx's HDMI 2.1
transmitter driver (embeddedsw `v_hdmitx1/src/xv_hdmitx1_frl.c`, its LTS:P handler) and
Amlogic's (common_drivers `hdmitx21/hdmi_tx_frl.c`, its post-training state); amdgpu
handles some requests in place instead. `train` starts at LTS:2. An additive entry point will resume at LTS:3 from an active
FRL link (gap characters only, then LTS:3 → LTS:P), for the caller that polls `Update_0`
during video.

### `HdmiPhy` becomes `FrlPhy`

hdmi-hal plans to split its PHY trait by link mode: `HdmiPhy` renamed `FrlPhy`, and a
separate `TmdsPhy` for TMDS. With that, `set_frl_rate(HdmiForumFrl::NotSupported)` will mean
"FRL off" rather than "switch to TMDS", and LTS:L's promise changes from "the PHY back in
TMDS" to "FRL off"; entering TMDS becomes the job of a separate TMDS bring-up crate, also
planned. A breaking release of plumbob and plumbob-async, following hdmi-hal's; the LTS:3
entry point above ships in the same release.

### The link orchestrator's trainer interface

The planned link orchestrator will define the interface it drives link training through and
implement it for plumbob in its own crate (see "Above: the link orchestrator" in
[`architecture.md`](architecture.md)). Nothing in plumbob changes for it.
