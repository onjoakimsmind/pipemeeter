use std::{
    env, fmt, fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
    },
    thread,
    time::Duration,
};

#[cfg(feature = "system-audio")]
use midir::MidiInput;
#[cfg(feature = "system-audio")]
use pipewire as pw;
use serde::{Deserialize, Serialize};
#[cfg(feature = "system-audio")]
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

const DEFAULT_INPUTS: [&str; 3] = ["Desktop", "Voice Chat", "Browser"];
const DEFAULT_OUTPUTS: [&str; 2] = ["Speakers", "Stream"];
const METER_CHANNEL_COUNT: usize = 2;
const CONFIG_FILE_NAME: &str = "config.toml";
const CONFIG_VERSION: u32 = 1;

const fn default_channel_count() -> usize {
    METER_CHANNEL_COUNT
}

const fn default_mono_state() -> bool {
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StripId(u32);

impl StripId {
    pub fn new(value: u32) -> Self {
        Self(value)
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }

    pub fn as_str(self) -> String {
        format!("strip-{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StripKind {
    Input,
    VirtualSink,
    Output,
}

impl StripKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Input => "Input",
            Self::VirtualSink => "Virtual Sink",
            Self::Output => "Output",
        }
    }

    fn default_label_prefix(self) -> &'static str {
        match self {
            Self::Input | Self::VirtualSink => "Sink",
            Self::Output => "Output",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormalizedVolume(f32);

impl NormalizedVolume {
    pub const UNITY: Self = Self(1.0);

    pub fn new(value: f32) -> Result<Self, VolumeError> {
        if !(0.0..=1.0).contains(&value) {
            return Err(VolumeError::OutOfRange(value));
        }

        Ok(Self(value))
    }

    pub fn from_percent(value: f32) -> Result<Self, VolumeError> {
        if !(0.0..=100.0).contains(&value) {
            return Err(VolumeError::PercentOutOfRange(value));
        }

        Self::new(value / 100.0)
    }

    pub fn from_midi_value(value: u8) -> Self {
        Self(value as f32 / 127.0)
    }

    pub fn as_percentage(self) -> f32 {
        self.0 * 100.0
    }

    pub fn as_ratio(self) -> f32 {
        self.0
    }

    pub fn as_percent_text(self) -> String {
        let percentage = self.as_percentage();
        if (percentage.fract()).abs() < 0.05 {
            format!("{percentage:.0}")
        } else {
            format!("{percentage:.1}")
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiControlTarget {
    Volume,
    Mute,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiBinding {
    pub volume_cc: Option<u8>,
    pub mute_cc: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RouteState {
    pub output_id: StripId,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MixerStrip {
    pub id: StripId,
    pub kind: StripKind,
    pub label: String,
    pub volume: NormalizedVolume,
    pub meter_level: NormalizedVolume,
    pub channel_count: usize,
    pub meter_channels: Vec<NormalizedVolume>,
    pub mono: bool,
    pub muted: bool,
    pub midi: MidiBinding,
    pub routes: Vec<RouteState>,
}

impl MixerStrip {
    fn new(id: StripId, kind: StripKind, label: impl Into<String>) -> Self {
        let channel_count = default_channel_count();
        Self {
            id,
            kind,
            label: label.into(),
            volume: NormalizedVolume::UNITY,
            meter_level: NormalizedVolume::new(0.0).expect("zero meter level should be valid"),
            channel_count,
            meter_channels: silent_meter_channels(channel_count),
            mono: default_mono_state(),
            muted: false,
            midi: MidiBinding::default(),
            routes: Vec::new(),
        }
    }

    pub fn active_channel_count(&self) -> usize {
        if self.mono {
            1
        } else {
            self.channel_count.max(1)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct PersistedState {
    version: u32,
    next_strip_id: u32,
    input_strips: Vec<PersistedStrip>,
    output_strips: Vec<PersistedStrip>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct PersistedStrip {
    id: u32,
    kind: StripKind,
    label: String,
    volume: f32,
    #[serde(default = "default_channel_count")]
    channel_count: usize,
    #[serde(default = "default_mono_state")]
    mono: bool,
    muted: bool,
    midi: MidiBinding,
    routes: Vec<RouteState>,
}

impl PersistedState {
    fn from_runtime(state: &AudioEngineState) -> Self {
        Self {
            version: CONFIG_VERSION,
            next_strip_id: state.next_strip_id,
            input_strips: state
                .input_strips
                .iter()
                .cloned()
                .map(PersistedStrip::from_runtime)
                .collect(),
            output_strips: state
                .output_strips
                .iter()
                .cloned()
                .map(PersistedStrip::from_runtime)
                .collect(),
        }
    }

    fn into_runtime(self) -> Result<AudioEngineState, String> {
        if self.version != CONFIG_VERSION {
            return Err(format!(
                "unsupported config version {}; expected {}",
                self.version, CONFIG_VERSION
            ));
        }

        let output_ids = self
            .output_strips
            .iter()
            .map(|strip| StripId::new(strip.id))
            .collect::<Vec<_>>();

        let output_strips = self
            .output_strips
            .into_iter()
            .map(|strip| strip.into_runtime_output())
            .collect::<Result<Vec<_>, _>>()?;

        let input_strips = self
            .input_strips
            .into_iter()
            .map(|strip| strip.into_runtime_input(&output_ids))
            .collect::<Result<Vec<_>, _>>()?;

        let max_strip_id = input_strips
            .iter()
            .chain(output_strips.iter())
            .map(|strip| strip.id.as_u32())
            .max()
            .map(|value| value + 1)
            .unwrap_or(0);

        Ok(AudioEngineState {
            input_strips,
            output_strips,
            inventory: BackendInventory::default(),
            next_strip_id: self.next_strip_id.max(max_strip_id),
            last_notice: "Loaded config".to_string(),
        })
    }
}

impl PersistedStrip {
    fn from_runtime(strip: MixerStrip) -> Self {
        Self {
            id: strip.id.as_u32(),
            kind: strip.kind,
            label: strip.label,
            volume: strip.volume.as_ratio(),
            channel_count: strip.channel_count,
            mono: strip.mono,
            muted: strip.muted,
            midi: strip.midi,
            routes: strip.routes,
        }
    }

    fn into_runtime_input(self, output_ids: &[StripId]) -> Result<MixerStrip, String> {
        if self.kind == StripKind::Output {
            return Err(format!("input strip {} cannot use output kind", self.id));
        }

        let valid_outputs = output_ids
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        let strip = self.into_runtime_strip()?;
        if strip
            .routes
            .iter()
            .any(|route| !valid_outputs.contains(&route.output_id))
        {
            return Err(format!(
                "input strip {} references an output that does not exist",
                strip.id.as_u32()
            ));
        }
        Ok(strip)
    }

    fn into_runtime_output(self) -> Result<MixerStrip, String> {
        if self.kind != StripKind::Output {
            return Err(format!("output strip {} must use output kind", self.id));
        }

        if !self.routes.is_empty() {
            return Err(format!("output strip {} cannot contain routes", self.id));
        }

        self.into_runtime_strip()
    }

    fn into_runtime_strip(self) -> Result<MixerStrip, String> {
        let id = StripId::new(self.id);
        let mut strip = MixerStrip::new(id, self.kind, normalize_label(&self.label, self.kind, id));
        strip.volume = NormalizedVolume::new(self.volume)
            .map_err(|error| format!("invalid saved volume for strip {}: {error}", self.id))?;
        strip.channel_count = self.channel_count.max(1);
        strip.mono = self.mono;
        strip.meter_channels = silent_meter_channels(strip.active_channel_count());
        strip.muted = self.muted;
        strip.midi = self.midi;
        strip.routes = self.routes;
        Ok(strip)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PipeWireNodeInfo {
    pub id: u32,
    pub name: String,
    pub media_class: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MidiPortInfo {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BackendInventory {
    pub pipewire_status: String,
    pub pipewire_nodes: Vec<PipeWireNodeInfo>,
    pub midi_status: String,
    pub midi_inputs: Vec<MidiPortInfo>,
}

impl Default for BackendInventory {
    fn default() -> Self {
        Self {
            pipewire_status: "Waiting for first PipeWire scan".to_string(),
            pipewire_nodes: Vec::new(),
            midi_status: "Waiting for first MIDI scan".to_string(),
            midi_inputs: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioEngineState {
    pub input_strips: Vec<MixerStrip>,
    pub output_strips: Vec<MixerStrip>,
    pub inventory: BackendInventory,
    pub next_strip_id: u32,
    pub last_notice: String,
}

impl Default for AudioEngineState {
    fn default() -> Self {
        let mut next_strip_id = 0;

        let mut output_strips = Vec::new();
        for label in DEFAULT_OUTPUTS {
            output_strips.push(MixerStrip::new(
                StripId::new(next_strip_id),
                StripKind::Output,
                label,
            ));
            next_strip_id += 1;
        }

        let output_ids = output_strips
            .iter()
            .map(|strip| strip.id)
            .collect::<Vec<_>>();
        let mut input_strips = Vec::new();
        for (index, label) in DEFAULT_INPUTS.into_iter().enumerate() {
            let mut strip = MixerStrip::new(StripId::new(next_strip_id), StripKind::Input, label);
            strip.routes = output_ids
                .iter()
                .enumerate()
                .map(|(output_index, output_id)| RouteState {
                    output_id: *output_id,
                    enabled: index == output_index || (index == 0 && output_index == 1),
                })
                .collect();
            input_strips.push(strip);
            next_strip_id += 1;
        }

        Self {
            input_strips,
            output_strips,
            inventory: BackendInventory::default(),
            next_strip_id,
            last_notice: "Booting audio engine".to_string(),
        }
    }
}

impl AudioEngineState {
    pub fn total_strip_count(&self) -> usize {
        self.input_strips.len() + self.output_strips.len()
    }

    pub fn active_route_count(&self) -> usize {
        self.input_strips
            .iter()
            .flat_map(|strip| strip.routes.iter())
            .filter(|route| route.enabled)
            .count()
    }

    pub fn muted_strip_count(&self) -> usize {
        self.input_strips
            .iter()
            .chain(self.output_strips.iter())
            .filter(|strip| strip.muted)
            .count()
    }

    pub fn output_name(&self, output_id: StripId) -> Option<&str> {
        self.output_strips
            .iter()
            .find(|strip| strip.id == output_id)
            .map(|strip| strip.label.as_str())
    }

    fn strip_label(&self, strip_id: StripId) -> Option<&str> {
        self.input_strips
            .iter()
            .chain(self.output_strips.iter())
            .find(|strip| strip.id == strip_id)
            .map(|strip| strip.label.as_str())
    }

    fn strip_mut(&mut self, strip_id: StripId) -> Option<&mut MixerStrip> {
        if let Some(strip) = self
            .input_strips
            .iter_mut()
            .find(|candidate| candidate.id == strip_id)
        {
            return Some(strip);
        }

        self.output_strips
            .iter_mut()
            .find(|candidate| candidate.id == strip_id)
    }

    fn apply_volume(&mut self, strip_id: StripId, volume: NormalizedVolume) {
        if let Some(target) = self.strip_mut(strip_id) {
            target.volume = volume;
        } else {
            self.last_notice = format!("Tried to update missing strip {}", strip_id.as_u32());
        }
    }

    fn rename_strip(&mut self, strip_id: StripId, label: &str) {
        if let Some(target) = self.strip_mut(strip_id) {
            target.label = normalize_label(label, target.kind, target.id);
        } else {
            self.last_notice = format!("Tried to rename missing strip {}", strip_id.as_u32());
        }
    }

    fn toggle_route(&mut self, strip_id: StripId, output_id: StripId) {
        if let Some(target) = self
            .input_strips
            .iter_mut()
            .find(|candidate| candidate.id == strip_id)
        {
            if let Some(route) = target
                .routes
                .iter_mut()
                .find(|route| route.output_id == output_id)
            {
                route.enabled = !route.enabled;
            } else {
                self.last_notice = format!(
                    "Tried to toggle missing output route {} on {}",
                    output_id.as_u32(),
                    strip_id.as_u32()
                );
            }
        } else {
            self.last_notice = format!(
                "Tried to toggle route on non-input strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn toggle_mute(&mut self, strip_id: StripId) {
        if let Some(target) = self.strip_mut(strip_id) {
            target.muted = !target.muted;
        } else {
            self.last_notice = format!("Tried to mute missing strip {}", strip_id.as_u32());
        }
    }

    fn toggle_mono(&mut self, strip_id: StripId) {
        if let Some(target) = self.strip_mut(strip_id) {
            target.mono = !target.mono;
            target.meter_channels = silent_meter_channels(target.active_channel_count());
        } else {
            self.last_notice = format!("Tried to mono missing strip {}", strip_id.as_u32());
        }
    }

    fn add_input_sink(&mut self, label: &str) -> MixerStrip {
        let id = StripId::new(self.next_strip_id);
        self.next_strip_id += 1;

        let mut strip = MixerStrip::new(
            id,
            StripKind::VirtualSink,
            normalize_label(label, StripKind::VirtualSink, id),
        );
        strip.routes = self
            .output_strips
            .iter()
            .enumerate()
            .map(|(index, output)| RouteState {
                output_id: output.id,
                enabled: index == 0,
            })
            .collect();

        self.input_strips.push(strip.clone());
        strip
    }

    fn add_output_sink(&mut self, label: &str) -> MixerStrip {
        let id = StripId::new(self.next_strip_id);
        self.next_strip_id += 1;

        let output = MixerStrip::new(
            id,
            StripKind::Output,
            normalize_label(label, StripKind::Output, id),
        );

        for strip in &mut self.input_strips {
            strip.routes.push(RouteState {
                output_id: output.id,
                enabled: false,
            });
        }

        self.output_strips.push(output.clone());
        output
    }

    fn remove_strip(&mut self, strip_id: StripId) -> Option<MixerStrip> {
        if let Some(index) = self
            .input_strips
            .iter()
            .position(|strip| strip.id == strip_id)
        {
            return Some(self.input_strips.remove(index));
        }

        if let Some(index) = self
            .output_strips
            .iter()
            .position(|strip| strip.id == strip_id)
        {
            let removed = self.output_strips.remove(index);
            for input in &mut self.input_strips {
                input.routes.retain(|route| route.output_id != strip_id);
            }
            return Some(removed);
        }

        None
    }

    fn set_midi_binding(
        &mut self,
        strip_id: StripId,
        target: MidiControlTarget,
        controller: Option<u8>,
    ) {
        if let Some(strip) = self.strip_mut(strip_id) {
            match target {
                MidiControlTarget::Volume => strip.midi.volume_cc = controller,
                MidiControlTarget::Mute => strip.midi.mute_cc = controller,
            }
        } else {
            self.last_notice = format!(
                "Tried to assign MIDI binding to missing strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn apply_midi_cc(&mut self, controller: u8, value: u8) -> usize {
        let mut affected = 0;

        for strip in self
            .input_strips
            .iter_mut()
            .chain(self.output_strips.iter_mut())
        {
            if strip.midi.volume_cc == Some(controller) {
                strip.volume = NormalizedVolume::from_midi_value(value);
                affected += 1;
            }

            if strip.midi.mute_cc == Some(controller) {
                strip.muted = value >= 64;
                affected += 1;
            }
        }

        affected
    }

    pub(crate) fn update_vu_meters(&mut self, phase: u64) {
        for strip in &mut self.input_strips {
            let raw_channel_levels = (0..strip.channel_count.max(1))
                .map(|channel| {
                    let activity = simulated_input_activity(strip.id, channel, phase);
                    let level = if strip.muted {
                        0.0
                    } else {
                        (strip.volume.as_ratio() * activity).clamp(0.0, 1.0)
                    };
                    NormalizedVolume::new(level)
                        .expect("simulated input meter level should be valid")
                })
                .collect::<Vec<_>>();
            let channel_levels = if strip.mono {
                vec![average_meter_level(&raw_channel_levels)]
            } else {
                raw_channel_levels
            };
            strip.meter_level = peak_meter_level(&channel_levels);
            strip.meter_channels = channel_levels;
        }

        let input_levels = self
            .input_strips
            .iter()
            .map(|strip| {
                let level = strip
                    .routes
                    .iter()
                    .filter(|route| route.enabled)
                    .map(|route| {
                        (
                            route.output_id,
                            strip
                                .meter_channels
                                .iter()
                                .map(|level| level.as_ratio())
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>();
                (strip.id, level)
            })
            .collect::<Vec<_>>();

        for output in &mut self.output_strips {
            let mut channel_levels = vec![0.0_f32; output.active_channel_count()];
            for (_, levels) in &input_levels {
                for (output_id, level_pair) in levels {
                    if *output_id != output.id {
                        continue;
                    }

                    let projected_levels = project_channel_levels(level_pair, channel_levels.len());
                    for (index, level) in projected_levels.iter().enumerate() {
                        channel_levels[index] = channel_levels[index].max(*level);
                    }
                }
            }

            let channel_levels = channel_levels
                .into_iter()
                .map(|level| {
                    let level = if output.muted {
                        0.0
                    } else {
                        (level * output.volume.as_ratio()).clamp(0.0, 1.0)
                    };
                    NormalizedVolume::new(level)
                        .expect("simulated output meter level should be valid")
                })
                .collect::<Vec<_>>();
            output.meter_level = peak_meter_level(&channel_levels);
            output.meter_channels = channel_levels;
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AudioControlMsg {
    SetStripVolume {
        strip: StripId,
        volume: NormalizedVolume,
    },
    RenameStrip {
        strip: StripId,
        label: String,
    },
    ToggleRoute {
        strip: StripId,
        output: StripId,
    },
    ToggleMute {
        strip: StripId,
    },
    ToggleMono {
        strip: StripId,
    },
    RemoveStrip {
        strip: StripId,
    },
    AddSink {
        label: String,
    },
    AddOutput {
        label: String,
    },
    SetMidiBinding {
        strip: StripId,
        target: MidiControlTarget,
        controller: Option<u8>,
    },
    ApplyMidiCc {
        controller: u8,
        value: u8,
    },
    RefreshTopology,
    Shutdown,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AudioUpdateMsg {
    Snapshot(AudioEngineState),
}

pub struct EngineBridge {
    control_tx: Sender<AudioControlMsg>,
    updates_rx: Mutex<Receiver<AudioUpdateMsg>>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
}

impl EngineBridge {
    pub fn spawn() -> Result<Self, String> {
        let (control_tx, control_rx) = mpsc::channel();
        let (updates_tx, updates_rx) = mpsc::channel();

        let worker = thread::Builder::new()
            .name("pipemeeter-audio-engine".to_string())
            .spawn(move || engine_loop(control_rx, updates_tx))
            .map_err(|error| format!("failed to spawn audio engine thread: {error}"))?;

        Ok(Self {
            control_tx,
            updates_rx: Mutex::new(updates_rx),
            worker: Mutex::new(Some(worker)),
        })
    }

    pub fn send(&self, message: AudioControlMsg) -> Result<(), String> {
        self.control_tx
            .send(message)
            .map_err(|error| format!("failed to send control message to audio engine: {error}"))
    }

    pub fn drain_updates(&self) -> Result<Vec<AudioUpdateMsg>, String> {
        let receiver = self
            .updates_rx
            .lock()
            .map_err(|_| "audio update receiver lock was poisoned".to_string())?;

        let mut updates = Vec::new();
        loop {
            match receiver.try_recv() {
                Ok(update) => updates.push(update),
                Err(TryRecvError::Empty) => return Ok(updates),
                Err(TryRecvError::Disconnected) => {
                    return Err("audio engine update channel disconnected".to_string());
                }
            }
        }
    }
}

impl Drop for EngineBridge {
    fn drop(&mut self) {
        let _ = self.control_tx.send(AudioControlMsg::Shutdown);

        if let Ok(mut worker) = self.worker.lock() {
            if let Some(handle) = worker.take() {
                let _ = handle.join();
            }
        }
    }
}

fn engine_loop(control_rx: Receiver<AudioControlMsg>, updates_tx: Sender<AudioUpdateMsg>) {
    let mut state = load_initial_state();
    let mut meter_phase = 0_u64;
    refresh_inventory(&mut state, false);
    state.update_vu_meters(meter_phase);
    push_snapshot(&updates_tx, &state);

    loop {
        match control_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(AudioControlMsg::SetStripVolume { strip, volume }) => {
                state.apply_volume(strip, volume);
                state.last_notice = format!(
                    "Updated {} to {}%",
                    state.strip_label(strip).unwrap_or("strip"),
                    volume.as_percent_text()
                );
                persist_runtime_state(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::RenameStrip { strip, label }) => {
                state.rename_strip(strip, &label);
                state.last_notice =
                    format!("Renamed {}", state.strip_label(strip).unwrap_or("strip"));
                persist_runtime_state(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::ToggleRoute { strip, output }) => {
                let output_label = state.output_name(output).unwrap_or("output").to_string();
                state.toggle_route(strip, output);
                state.last_notice = format!(
                    "Toggled {} on {}",
                    output_label,
                    state.strip_label(strip).unwrap_or("strip")
                );
                persist_runtime_state(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::ToggleMute { strip }) => {
                state.toggle_mute(strip);
                let mute_state = state
                    .strip_mut(strip)
                    .map(|candidate| if candidate.muted { "muted" } else { "unmuted" })
                    .unwrap_or("updated");
                state.last_notice = format!(
                    "{} {}",
                    state.strip_label(strip).unwrap_or("Strip"),
                    mute_state
                );
                persist_runtime_state(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::ToggleMono { strip }) => {
                state.toggle_mono(strip);
                let mono_state = state
                    .strip_mut(strip)
                    .map(|candidate| if candidate.mono { "mono" } else { "stereo" })
                    .unwrap_or("updated");
                state.last_notice = format!(
                    "{} set to {}",
                    state.strip_label(strip).unwrap_or("Strip"),
                    mono_state
                );
                persist_runtime_state(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::RemoveStrip { strip }) => {
                match state.remove_strip(strip) {
                    Some(removed) => {
                        state.last_notice = format!("Removed {}", removed.label);
                    }
                    None => {
                        state.last_notice =
                            format!("Tried to remove missing strip {}", strip.as_u32());
                    }
                }
                persist_runtime_state(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::AddSink { label }) => {
                let created = state.add_input_sink(&label);
                state.last_notice = format!("Added new sink {}", created.label);
                persist_runtime_state(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::AddOutput { label }) => {
                let created = state.add_output_sink(&label);
                state.last_notice = format!("Added new output {}", created.label);
                persist_runtime_state(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetMidiBinding {
                strip,
                target,
                controller,
            }) => {
                state.set_midi_binding(strip, target, controller);
                let binding_label = controller
                    .map(|value| format!("CC {value}"))
                    .unwrap_or_else(|| "cleared".to_string());
                state.last_notice = format!(
                    "{} {} MIDI binding {}",
                    state.strip_label(strip).unwrap_or("Strip"),
                    match target {
                        MidiControlTarget::Volume => "volume",
                        MidiControlTarget::Mute => "mute",
                    },
                    binding_label
                );
                persist_runtime_state(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::ApplyMidiCc { controller, value }) => {
                let affected = state.apply_midi_cc(controller, value);
                state.last_notice = if affected == 0 {
                    format!("Received unmapped MIDI CC {controller}")
                } else {
                    format!("Applied MIDI CC {controller} to {affected} target(s)")
                };
                if affected > 0 {
                    persist_runtime_state(&mut state);
                }
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::RefreshTopology) => {
                refresh_inventory(&mut state, true);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                meter_phase = meter_phase.wrapping_add(1);
                state.update_vu_meters(meter_phase);
                push_snapshot(&updates_tx, &state);
            }
        }
    }
}

fn simulated_input_activity(strip_id: StripId, channel: usize, phase: u64) -> f32 {
    let phase = phase as f32 * 0.18;
    let seed = strip_id.as_u32() as f32 * 0.73 + channel as f32 * 0.41;
    let lfo = ((phase + seed).sin() * 0.5) + 0.5;
    let accent = (((phase * 0.53) + (seed * 1.7)).cos() * 0.5) + 0.5;
    (0.18 + (lfo * 0.52) + (accent * 0.3)).clamp(0.0, 1.0)
}

fn peak_meter_level(levels: &[NormalizedVolume]) -> NormalizedVolume {
    let peak = levels
        .iter()
        .map(|level| level.as_ratio())
        .fold(0.0_f32, f32::max);
    NormalizedVolume::new(peak).expect("peak meter level should be valid")
}

fn average_meter_level(levels: &[NormalizedVolume]) -> NormalizedVolume {
    let average = if levels.is_empty() {
        0.0
    } else {
        levels.iter().map(|level| level.as_ratio()).sum::<f32>() / levels.len() as f32
    };
    NormalizedVolume::new(average.clamp(0.0, 1.0)).expect("average meter level should be valid")
}

fn project_channel_levels(levels: &[f32], target_channels: usize) -> Vec<f32> {
    if target_channels == 0 {
        return Vec::new();
    }

    if levels.is_empty() {
        return vec![0.0; target_channels];
    }

    if levels.len() == 1 {
        return vec![levels[0]; target_channels];
    }

    (0..target_channels)
        .map(|index| {
            levels
                .get(index)
                .copied()
                .unwrap_or(*levels.last().unwrap_or(&0.0))
        })
        .collect()
}

fn silent_meter_channels(channel_count: usize) -> Vec<NormalizedVolume> {
    vec![
        NormalizedVolume::new(0.0).expect("zero meter level should be valid");
        channel_count.max(1)
    ]
}

fn normalize_label(label: &str, kind: StripKind, id: StripId) -> String {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        format!("{} {}", kind.default_label_prefix(), id.as_u32() + 1)
    } else {
        trimmed.to_string()
    }
}

fn refresh_inventory(state: &mut AudioEngineState, update_notice: bool) {
    match scan_pipewire_nodes() {
        Ok(nodes) => {
            state.inventory.pipewire_status = if nodes.is_empty() {
                "PipeWire connected, but no nodes were reported".to_string()
            } else {
                format!("PipeWire connected with {} nodes", nodes.len())
            };
            state.inventory.pipewire_nodes = nodes;
        }
        Err(error) => {
            state.inventory.pipewire_status = format!("PipeWire unavailable: {error}");
            state.inventory.pipewire_nodes.clear();
        }
    }

    match scan_midi_inputs() {
        Ok(inputs) => {
            state.inventory.midi_status = if inputs.is_empty() {
                "MIDI subsystem ready, but no input devices were found".to_string()
            } else {
                format!("MIDI subsystem ready with {} inputs", inputs.len())
            };
            state.inventory.midi_inputs = inputs;
        }
        Err(error) => {
            state.inventory.midi_status = format!("MIDI unavailable: {error}");
            state.inventory.midi_inputs.clear();
        }
    }

    if update_notice {
        state.last_notice = "Topology refreshed".to_string();
    }
}

fn push_snapshot(updates_tx: &Sender<AudioUpdateMsg>, state: &AudioEngineState) {
    let _ = updates_tx.send(AudioUpdateMsg::Snapshot(state.clone()));
}

pub fn load_initial_state() -> AudioEngineState {
    let config_path = match config_path() {
        Ok(path) => path,
        Err(error) => {
            let mut state = AudioEngineState::default();
            state.last_notice = format!("Config unavailable: {error}; using defaults");
            return state;
        }
    };

    match load_state_from_path(&config_path) {
        Ok(Some(mut state)) => {
            state.last_notice = format!("Loaded config from {}", config_path.display());
            state
        }
        Ok(None) => {
            let mut state = AudioEngineState::default();
            state.last_notice = format!(
                "No config found at {}; using defaults",
                config_path.display()
            );
            state
        }
        Err(error) => {
            let mut state = AudioEngineState::default();
            state.last_notice = format!("Failed to load config: {error}; using defaults");
            state
        }
    }
}

fn persist_runtime_state(state: &mut AudioEngineState) {
    if let Err(error) = save_state_to_default_path(state) {
        state.last_notice = format!("{}; config save failed: {error}", state.last_notice);
    }
}

fn config_path() -> Result<PathBuf, String> {
    let home = env::var_os("HOME")
        .ok_or_else(|| "HOME is not set, so the config directory cannot be resolved".to_string())?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("pipemeeter")
        .join(CONFIG_FILE_NAME))
}

fn save_state_to_default_path(state: &AudioEngineState) -> Result<(), String> {
    let path = config_path()?;
    save_state_to_path(state, &path)
}

fn save_state_to_path(state: &AudioEngineState, path: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| {
        format!(
            "config path {} does not have a parent directory",
            path.display()
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "failed to create config directory {}: {error}",
            parent.display()
        )
    })?;

    let serialized = toml::to_string_pretty(&PersistedState::from_runtime(state))
        .map_err(|error| format!("failed to serialize config: {error}"))?;

    fs::write(path, serialized)
        .map_err(|error| format!("failed to write config file {}: {error}", path.display()))
}

fn load_state_from_path(path: &Path) -> Result<Option<AudioEngineState>, String> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)
        .map_err(|error| format!("failed to read config file {}: {error}", path.display()))?;
    let persisted = toml::from_str::<PersistedState>(&raw)
        .map_err(|error| format!("failed to parse config file {}: {error}", path.display()))?;
    persisted.into_runtime().map(Some)
}

fn scan_midi_inputs() -> Result<Vec<MidiPortInfo>, String> {
    #[cfg(not(feature = "system-audio"))]
    {
        return Err("compiled without `system-audio`; enable it to query MIDI devices".to_string());
    }

    #[cfg(feature = "system-audio")]
    {
        let input = MidiInput::new("pipemeeter-discovery")
            .map_err(|error| format!("could not create midi input client: {error}"))?;

        let mut ports = input
            .ports()
            .into_iter()
            .map(|port| {
                input
                    .port_name(&port)
                    .map(|name| MidiPortInfo { name })
                    .map_err(|error| format!("failed to read midi port name: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        ports.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(ports)
    }
}

fn scan_pipewire_nodes() -> Result<Vec<PipeWireNodeInfo>, String> {
    #[cfg(not(feature = "system-audio"))]
    {
        return Err(
            "compiled without `system-audio`; enable it to query PipeWire nodes".to_string(),
        );
    }

    #[cfg(feature = "system-audio")]
    {
        pw::init();

        let result = (|| {
            let mainloop = pw::main_loop::MainLoopRc::new(None)
                .map_err(|error| format!("could not create PipeWire main loop: {error}"))?;
            let context = pw::context::ContextRc::new(&mainloop, None)
                .map_err(|error| format!("could not create PipeWire context: {error}"))?;
            let core = context
                .connect_rc(None)
                .map_err(|error| format!("could not connect to PipeWire core: {error}"))?;
            let registry = core
                .get_registry()
                .map_err(|error| format!("could not access PipeWire registry: {error}"))?;

            let done = Rc::new(Cell::new(false));
            let nodes = Rc::new(RefCell::new(Vec::new()));

            let pending = core
                .sync(0)
                .map_err(|error| format!("could not sync PipeWire registry: {error}"))?;

            let done_for_core = Rc::clone(&done);
            let loop_for_core = mainloop.clone();
            let _listener_core = core
                .add_listener_local()
                .done(move |id, seq| {
                    if id == pw::core::PW_ID_CORE && seq == pending {
                        done_for_core.set(true);
                        loop_for_core.quit();
                    }
                })
                .register();

            let nodes_for_registry = Rc::clone(&nodes);
            let _listener_registry = registry
                .add_listener_local()
                .global(move |global| {
                    if global.type_ != pw::types::ObjectType::Node {
                        return;
                    }

                    let props = global.props.as_ref();
                    let name = props
                        .and_then(|props| props.get(*pw::keys::NODE_DESCRIPTION))
                        .or_else(|| props.and_then(|props| props.get(*pw::keys::NODE_NAME)))
                        .or_else(|| props.and_then(|props| props.get(*pw::keys::APP_NAME)))
                        .unwrap_or("Unnamed PipeWire node")
                        .to_string();
                    let media_class = props
                        .and_then(|props| props.get(*pw::keys::MEDIA_CLASS))
                        .map(ToOwned::to_owned);

                    nodes_for_registry.borrow_mut().push(PipeWireNodeInfo {
                        id: global.id,
                        name,
                        media_class,
                    });
                })
                .register();

            while !done.get() {
                mainloop.run();
            }

            let mut nodes = nodes.borrow().clone();
            nodes.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
            Ok(nodes)
        })();

        unsafe { pw::deinit() };
        result
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum VolumeError {
    OutOfRange(f32),
    PercentOutOfRange(f32),
}

impl fmt::Display for VolumeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange(value) => {
                write!(formatter, "volume {value} must be between 0.0 and 1.0")
            }
            Self::PercentOutOfRange(value) => {
                write!(
                    formatter,
                    "volume percent {value} must be between 0 and 100"
                )
            }
        }
    }
}

impl std::error::Error for VolumeError {}

pub type SharedEngineBridge = Arc<EngineBridge>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn normalized_volume_rejects_invalid_values() {
        assert_eq!(
            NormalizedVolume::new(-0.1),
            Err(VolumeError::OutOfRange(-0.1))
        );
        assert_eq!(
            NormalizedVolume::from_percent(101.0),
            Err(VolumeError::PercentOutOfRange(101.0))
        );
    }

    #[test]
    fn adding_output_appends_new_route_targets() {
        let mut state = AudioEngineState::default();
        let route_counts = state
            .input_strips
            .iter()
            .map(|strip| strip.routes.len())
            .collect::<Vec<_>>();

        let output = state.add_output_sink("Headphones");

        assert_eq!(output.kind, StripKind::Output);
        assert!(
            state
                .input_strips
                .iter()
                .zip(route_counts)
                .all(|(strip, count)| strip.routes.len() == count + 1)
        );
    }

    #[test]
    fn adding_sink_uses_output_list_for_routes() {
        let mut state = AudioEngineState::default();

        let created = state.add_input_sink("Podcast");

        assert_eq!(created.kind, StripKind::VirtualSink);
        assert_eq!(created.label, "Podcast");
        assert_eq!(created.routes.len(), state.output_strips.len());
    }

    #[test]
    fn toggling_route_updates_matrix_state() {
        let mut state = AudioEngineState::default();
        let strip = state.input_strips[0].id;
        let output = state.output_strips[0].id;
        let before = state.input_strips[0].routes[0].enabled;

        state.toggle_route(strip, output);

        assert_ne!(before, state.input_strips[0].routes[0].enabled);
    }

    #[test]
    fn midi_cc_updates_volume_and_mute() {
        let mut state = AudioEngineState::default();
        let strip = state.input_strips[0].id;
        state.set_midi_binding(strip, MidiControlTarget::Volume, Some(12));
        state.set_midi_binding(strip, MidiControlTarget::Mute, Some(13));

        let affected_volume = state.apply_midi_cc(12, 64);
        let affected_mute = state.apply_midi_cc(13, 127);

        assert_eq!(affected_volume, 1);
        assert_eq!(affected_mute, 1);
        assert!((state.input_strips[0].volume.as_percentage() - 50.3937).abs() < 0.01);
        assert!(state.input_strips[0].muted);
    }

    #[test]
    fn vu_meters_follow_inputs_and_routes() {
        let mut state = AudioEngineState::default();
        let output_id = state.output_strips[0].id;

        for strip in state.input_strips.iter_mut().skip(1) {
            for route in &mut strip.routes {
                route.enabled = false;
            }
        }

        let routed_input = &mut state.input_strips[0];
        routed_input.volume = NormalizedVolume::from_percent(75.0).unwrap();
        routed_input.muted = false;
        for route in &mut routed_input.routes {
            route.enabled = route.output_id == output_id;
        }

        state.update_vu_meters(4);

        assert!(state.input_strips[0].meter_level.as_ratio() > 0.0);
        assert_eq!(state.input_strips[0].meter_channels.len(), 2);
        assert!(
            state.input_strips[0]
                .meter_channels
                .iter()
                .any(|level| level.as_ratio() > 0.0)
        );
        assert!(state.output_strips[0].meter_level.as_ratio() > 0.0);
        assert_eq!(state.output_strips[0].meter_channels.len(), 2);
        assert!(
            state.output_strips[0]
                .meter_channels
                .iter()
                .any(|level| level.as_ratio() > 0.0)
        );
    }

    #[test]
    fn toggling_mono_collapses_input_meter_to_single_channel() {
        let mut state = AudioEngineState::default();
        let strip_id = state.input_strips[0].id;

        state.toggle_mono(strip_id);
        state.update_vu_meters(7);

        assert!(state.input_strips[0].mono);
        assert_eq!(state.input_strips[0].meter_channels.len(), 1);
        assert!(state.input_strips[0].meter_channels[0].as_ratio() >= 0.0);
        assert_eq!(state.output_strips[0].meter_channels.len(), 2);
    }

    #[test]
    fn removing_output_prunes_routes_from_inputs() {
        let mut state = AudioEngineState::default();
        let removed_output = state.output_strips[0].id;

        let removed = state
            .remove_strip(removed_output)
            .expect("output should exist");

        assert_eq!(removed.id, removed_output);
        assert!(
            state
                .output_strips
                .iter()
                .all(|strip| strip.id != removed_output)
        );
        assert!(state.input_strips.iter().all(|strip| {
            strip
                .routes
                .iter()
                .all(|route| route.output_id != removed_output)
        }));
    }

    #[test]
    fn removing_input_sink_only_removes_target_strip() {
        let mut state = AudioEngineState::default();
        let removed_input = state.input_strips[0].id;
        let original_output_count = state.output_strips.len();

        let removed = state
            .remove_strip(removed_input)
            .expect("input should exist");

        assert_eq!(removed.id, removed_input);
        assert!(
            state
                .input_strips
                .iter()
                .all(|strip| strip.id != removed_input)
        );
        assert_eq!(state.output_strips.len(), original_output_count);
    }

    #[test]
    fn persisted_state_round_trips_custom_mixer_config() {
        let mut state = AudioEngineState::default();
        let input_id = state.input_strips[0].id;
        let output_id = state.output_strips[0].id;

        state.rename_strip(input_id, "Game");
        state.apply_volume(input_id, NormalizedVolume::from_percent(63.0).unwrap());
        state.toggle_mute(output_id);
        state.set_midi_binding(input_id, MidiControlTarget::Volume, Some(21));
        state.set_midi_binding(input_id, MidiControlTarget::Mute, Some(22));
        state.toggle_route(input_id, output_id);
        state.toggle_mono(input_id);
        let created_input = state.add_input_sink("Podcast");
        let created_output = state.add_output_sink("Headphones");
        state.toggle_route(created_input.id, created_output.id);

        let config_path = temp_config_path("round-trip");
        save_state_to_path(&state, &config_path).expect("config should save");
        let restored = load_state_from_path(&config_path)
            .expect("config should load")
            .expect("config should exist");

        assert_eq!(restored.input_strips.len(), state.input_strips.len());
        assert_eq!(restored.output_strips.len(), state.output_strips.len());
        assert_eq!(restored.next_strip_id, state.next_strip_id);
        assert_eq!(restored.input_strips[0].label, "Game");
        assert_eq!(
            restored.input_strips[0].volume,
            NormalizedVolume::from_percent(63.0).unwrap()
        );
        assert!(restored.input_strips[0].mono);
        assert_eq!(restored.input_strips[0].channel_count, 2);
        assert_eq!(restored.input_strips[0].midi.volume_cc, Some(21));
        assert_eq!(restored.input_strips[0].midi.mute_cc, Some(22));
        assert!(restored.output_strips[0].muted);
        assert!(
            restored
                .input_strips
                .iter()
                .find(|strip| strip.id == created_input.id)
                .expect("saved input should exist")
                .routes
                .iter()
                .any(|route| route.output_id == created_output.id && route.enabled)
        );

        let _ =
            std::fs::remove_dir_all(config_path.parent().expect("temp path should have parent"));
    }

    #[test]
    fn loading_missing_config_returns_none() {
        let config_path = temp_config_path("missing");

        let restored = load_state_from_path(&config_path).expect("missing config should not error");

        assert!(restored.is_none());
    }

    fn temp_config_path(label: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough for tests")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("pipemeeter-{label}-{unique}"))
            .join("config.toml")
    }
}
