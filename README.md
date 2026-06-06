# Pipemeeter

A native Linux audio mixer built with Rust, [Dioxus](https://dioxuslabs.com/) and PipeWire/PulseAudio. Pipemeeter gives you a hardware-style mixing surface — virtual cables, channel strips, mix buses, FX buses with gate / compressor / EQ — all routed through PipeWire.

---

## Features

- **Virtual cables** — create null-sinks that any application can send audio to
- **Channel strips** — per-strip volume, mute, mono, and sends to mix buses
- **Mix buses** — each bus is a virtual output (capturable in OBS) and can loopback to hardware audio devices
- **FX buses** — filter-chain processing with noise gate, compressor, and 8-band EQ; selectable as a destination for app audio
- **EQ presets** — Flat, Vocal boost, Bass boost, Treble boost, Presence, Bright air
- **Application routing** — move any playing app's audio stream to a virtual cable or FX bus directly from Pipemeeter
- **MIDI control** — map CC or note messages to volume, mute, and FX knobs
- **OBS integration** — every mix bus exposes a capturable virtual source in OBS Mic/Aux
- **VU meters** — live peak + RMS metering on every strip and bus

---

## Requirements

| Dependency | Notes |
|---|---|
| PipeWire ≥ 0.3 | With `pipewire-pulse` and `pipewire-alsa` |
| `pactl` | Part of `pipewire-pulse` / `pulseaudio-utils` |
| `pw-cli` | Part of `pipewire` |
| `pw-link` | Part of `pipewire` |
| `pw-dump` | Part of `pipewire` |
| Rust ≥ 1.78 | `rustup` recommended |
| `dx` CLI | Dioxus desktop toolchain |
| GTK 3 / WebKitGTK | For the Dioxus renderer (`libwebkit2gtk-4.1`) |

### Install CLI deps (Arch)

```bash
sudo pacman -S pipewire pipewire-pulse pipewire-alsa wireplumber webkit2gtk
```

### Install CLI deps (Ubuntu / Debian)

```bash
sudo apt install pipewire pipewire-pulse pipewire-audio-client-libraries \
    libwebkit2gtk-4.1-dev libgtk-3-dev
```

### Install Rust + dx

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cargo install dioxus-cli
```

---

## Building

```bash
git clone https://github.com/onjoakimsmind/pipemeeter
cd pipemeeter
cargo build --release --features "desktop-ui system-audio"
```

The binary is at `target/release/pipemeeter`.

### Development hot-reload

```bash
dx serve --platform desktop --features "desktop-ui system-audio"
```

### Dioxus desktop bundle

```bash
dx build --platform desktop --features "desktop-ui system-audio"
```

---

## Running

```bash
./target/release/pipemeeter
```

On Wayland, set an application ID for correct window behaviour:

```bash
TAO_UNIX_APPLICATION_ID=com.pipemeeter.app ./target/release/pipemeeter
```

---

## First run

On first launch Pipemeeter creates a **PRIMARY** virtual cable automatically and shows an onboarding notice in the status bar.

1. Click **Settings** (top-right).
2. Under **Virtual cables**, add any additional cables you need (e.g. `Music`, `Chat`).
3. In the mixer **Strips** tab, create a channel strip and assign it to the virtual cable you want to capture from.
4. Route the strip to one or more mix buses using the **Routes** section in the strip settings panel.
5. Open a mix bus's **Settings** panel (click the bus card in the **Buses** tab). Under **Hardware outputs**, pick your audio device to hear the bus live; the bus is also automatically available as a capturable source in OBS.

### Routing application audio

In the **Settings → Applications** tab, every active playback stream is listed. Use the dropdown next to an app to move it to a virtual cable or FX bus in real-time — no system-wide output change required.

### Using with OBS

Each mix bus exposes a virtual source named `{bus-name}-src`. In OBS → Sources → Audio Input Capture, select the bus source you want to capture (e.g. your "Stream mix" bus).

---

## FX buses

FX buses run a PipeWire filter-chain with up to three processing stages:

| Stage | Controls |
|---|---|
| Noise gate | Open/Close threshold, floor level |
| Compressor | Threshold, ratio, makeup gain |
| EQ | 8-band parametric (63 Hz – 8 kHz), ±12 dB per band |

**Note:** changing an EQ band gain causes a brief (~100–300 ms) audio gap while the filter-chain rebuilds. The rebuild fires 800 ms after you stop adjusting sliders, so you can sweep multiple bands before the gap occurs. This is a known limitation of PipeWire filter-chain; full in-place parameter updates are planned.

FX buses can route into mix buses or chain into other FX buses. FX bus nodes are hidden from PipeWire session clients (OBS, pavucontrol) — only the mix bus output is exposed.

---

## Config file

Configuration is saved to `~/.config/pipemeeter/config.toml` automatically. The file is human-readable TOML; manual edits are supported but not required.

### Resetting

To wipe all strips, buses, virtual cables, and MIDI bindings: open **Settings**, scroll to the bottom **Danger zone** section, and click **Reset mixer…**. Hardware audio sources are preserved. This cannot be undone.

---

## MIDI

1. Open a strip or bus settings panel and scroll to the MIDI section.
2. Click **Learn** next to the control you want to map, then move a knob or press a button on your MIDI controller.
3. The mapping is saved with the config.

MIDI feedback (for motorised faders / button LEDs) can be configured in **Settings → MIDI feedback**.

---

## Packaging

### Standalone binary

```bash
make binary
# → target/release/pipemeeter
```

Copy the binary anywhere on your `$PATH`. You need these runtime dependencies installed on the host:

- `pipewire`, `pipewire-pulse` (provides `pactl`, `pw-cli`, `pw-link`, `pw-dump`)
- `libwebkit2gtk-4.1`, `libgtk-3`

### Debian / Ubuntu (.deb)

```bash
make deb
# → target/debian/pipemeeter_<version>_amd64.deb
sudo apt install ./target/debian/pipemeeter_*.deb
```

Requires [`cargo-deb`](https://github.com/kornelski/cargo-deb) (installed automatically by `make deb`).

### Arch Linux (AUR)

The `packaging/aur/` directory contains a ready-to-submit `PKGBUILD` and `.SRCINFO`. Once published to the AUR:

```bash
paru -S pipemeeter
# or
yay -S pipemeeter
```

To build locally from the PKGBUILD:

```bash
cd packaging/aur
makepkg -si
```

### GitHub Releases

Pushing a version tag triggers the release workflow and attaches a pre-built binary and `.deb` to the GitHub release automatically:

```bash
git tag v0.1.0 && git push origin v0.1.0
```

---

## Development

```bash
# Build + test
cargo build --features "desktop-ui system-audio"
cargo test --features "desktop-ui system-audio"
```

All audio engine tests run without a live PipeWire session.

