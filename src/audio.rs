use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use std::{
    env, fmt, fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(feature = "system-audio")]
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_OUTPUTS: [&str; 2] = ["Speakers", "Stream"];
const METER_CHANNEL_COUNT: usize = 2;
const CONFIG_FILE_NAME: &str = "config.toml";
const CONFIG_VERSION: u32 = 2;
const STATE_SAVE_DEBOUNCE: Duration = Duration::from_millis(200);
const AUTO_TOPOLOGY_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const PIPEMEETER_VIRTUAL_CABLE_PREFIX: &str = "pipemeeter.";
const PIPEMEETER_STRIP_SINK_PREFIX: &str = "pipemeeter-strip.";
const PIPEMEETER_BUS_SINK_PREFIX: &str = "pipemeeter-bus.";
const PIPEMEETER_OUTPUT_SINK_PREFIX: &str = "pipemeeter-output.";
#[cfg(any(test, feature = "system-audio"))]
const MIDI_FEEDBACK_CHANNEL_STATUS: u8 = 0xB0;
#[cfg(any(test, feature = "system-audio"))]
const MIDI_NOTE_ON_CHANNEL_STATUS: u8 = 0x90;
#[cfg(any(test, feature = "system-audio"))]
const MIDI_FEEDBACK_ON_VALUE: u8 = 127;
#[cfg(any(test, feature = "system-audio"))]
const MIDI_FEEDBACK_OFF_VALUE: u8 = 0;

const fn default_channel_count() -> usize {
    METER_CHANNEL_COUNT
}

fn scan_midi_outputs() -> Result<Vec<MidiPortInfo>, String> {
    #[cfg(not(feature = "system-audio"))]
    {
        return Err("compiled without `system-audio`; enable it to query MIDI devices".to_string());
    }

    #[cfg(feature = "system-audio")]
    {
        let output = MidiOutput::new("pipemeeter-feedback-discovery")
            .map_err(|error| format!("could not create midi output client: {error}"))?;

        let mut ports = output
            .ports()
            .into_iter()
            .map(|port| {
                output
                    .port_name(&port)
                    .map(|name| MidiPortInfo { name })
                    .map_err(|error| format!("failed to read midi port name: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        ports.retain(|port| {
            let name = port.name.trim();
            !name.starts_with("pipemeeter-") && !name.starts_with("Midi Through")
        });

        ports.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(ports)
    }
}

#[cfg(any(test, feature = "system-audio"))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct MidiFeedbackMessage {
    kind: MidiMessageKind,
    channel: u8,
    number: u8,
    value: u8,
}

const MIDI_FEEDBACK_DEBUG_LIMIT: usize = 20;

fn format_midi_feedback_messages(messages: &[MidiFeedbackMessage]) -> String {
    messages
        .iter()
        .map(|message| {
            let kind = match message.kind {
                MidiMessageKind::ControlChange => "CC",
                MidiMessageKind::Note => "Note",
            };
            format!(
                "{kind} {} ch{} -> {}",
                message.number,
                message.channel + 1,
                message.value
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn push_midi_feedback_debug(state: &mut AudioEngineState, entry: impl Into<String>) {
    let entry = entry.into();
    if state.inventory.midi_feedback_debug.first() == Some(&entry) {
        return;
    }
    state.inventory.midi_feedback_debug.insert(0, entry);
    state
        .inventory
        .midi_feedback_debug
        .truncate(MIDI_FEEDBACK_DEBUG_LIMIT);
}

#[derive(Default)]
struct MidiFeedbackRuntime {
    #[cfg(feature = "system-audio")]
    connection: Option<MidiOutputConnection>,
    connected_port_name: Option<String>,
}

impl MidiFeedbackRuntime {
    fn sync_connection(&mut self, state: &mut AudioEngineState) {
        let Some(selected_port) = state.midi_feedback.output_port_name.clone() else {
            self.disconnect();
            state.inventory.midi_feedback_status = "MIDI feedback disabled".to_string();
            push_midi_feedback_debug(state, "Feedback disabled");
            return;
        };

        if self.connected_port_name.as_deref() == Some(selected_port.as_str()) {
            state.inventory.midi_feedback_status =
                format!("MIDI feedback ready on {}", selected_port);
            return;
        }

        #[cfg(not(feature = "system-audio"))]
        {
            self.connected_port_name = None;
            state.inventory.midi_feedback_status =
                "MIDI feedback unavailable: compiled without `system-audio`".to_string();
            return;
        }

        #[cfg(feature = "system-audio")]
        {
            self.disconnect();

            let output = match MidiOutput::new("pipemeeter-feedback") {
                Ok(output) => output,
                Err(error) => {
                    state.inventory.midi_feedback_status =
                        format!("MIDI feedback unavailable: {error}");
                    return;
                }
            };

            let port =
                match output.ports().into_iter().find(|port| {
                    output.port_name(port).ok().as_deref() == Some(selected_port.as_str())
                }) {
                    Some(port) => port,
                    None => {
                        state.inventory.midi_feedback_status =
                            format!("MIDI feedback port not found: {selected_port}");
                        return;
                    }
                };

            match output.connect(&port, "pipemeeter-feedback") {
                Ok(connection) => {
                    self.connection = Some(connection);
                    self.connected_port_name = Some(selected_port.clone());
                    state.inventory.midi_feedback_status =
                        format!("MIDI feedback ready on {selected_port}");
                    push_midi_feedback_debug(
                        state,
                        format!("Connected feedback output {selected_port}"),
                    );
                }
                Err(error) => {
                    state.inventory.midi_feedback_status =
                        format!("Failed to connect MIDI feedback output {selected_port}: {error}");
                    push_midi_feedback_debug(
                        state,
                        format!("Failed to connect {selected_port}: {error}"),
                    );
                }
            }
        }
    }

    fn send_snapshot(&mut self, state: &mut AudioEngineState) {
        if state.midi_feedback.output_port_name.is_none() {
            return;
        }

        if self.connected_port_name.is_none() {
            self.sync_connection(state);
        }

        #[cfg(not(feature = "system-audio"))]
        {
            state.inventory.midi_feedback_status =
                "MIDI feedback unavailable: compiled without `system-audio`".to_string();
        }

        #[cfg(feature = "system-audio")]
        {
            let Some(port_name) = self.connected_port_name.clone() else {
                return;
            };
            let messages = collect_midi_feedback_messages(state);
            if messages.is_empty() {
                state.inventory.midi_feedback_status =
                    format!("MIDI feedback ready on {port_name}");
                return;
            }

            {
                let Some(connection) = self.connection.as_mut() else {
                    return;
                };

                for message in &messages {
                    let status = match message.kind {
                        MidiMessageKind::ControlChange => {
                            MIDI_FEEDBACK_CHANNEL_STATUS | (message.channel & 0x0F)
                        }
                        MidiMessageKind::Note => {
                            MIDI_NOTE_ON_CHANNEL_STATUS | (message.channel & 0x0F)
                        }
                    };
                    if let Err(error) = connection.send(&[status, message.number, message.value]) {
                        self.disconnect();
                        state.inventory.midi_feedback_status =
                            format!("MIDI feedback send failed on {port_name}: {error}");
                        push_midi_feedback_debug(
                            state,
                            format!("Send failed on {port_name}: {error}"),
                        );
                        return;
                    }
                }
            }

            state.inventory.midi_feedback_status = format!("MIDI feedback synced to {port_name}");
            push_midi_feedback_debug(
                state,
                format!("{port_name}: {}", format_midi_feedback_messages(&messages)),
            );
        }
    }

    fn disconnect(&mut self) {
        #[cfg(feature = "system-audio")]
        {
            self.connection = None;
        }
        self.connected_port_name = None;
    }
}

#[derive(Default)]
struct MidiInputRuntime {
    #[cfg(feature = "system-audio")]
    connections: Vec<MidiInputConnection<()>>,
    raw_inputs: Vec<RawMidiInputProcess>,
    connected_port_names: Vec<String>,
}

struct RawMidiInputProcess {
    child: Child,
    reader: thread::JoinHandle<()>,
}

impl MidiInputRuntime {
    fn sync_connections(
        &mut self,
        state: &mut AudioEngineState,
        control_tx: &Sender<AudioControlMsg>,
    ) {
        let desired_port_names = state
            .inventory
            .midi_inputs
            .iter()
            .map(|port| port.name.clone())
            .collect::<Vec<_>>();

        if self.connected_port_names == desired_port_names {
            return;
        }

        self.disconnect_all();

        #[cfg(not(feature = "system-audio"))]
        {
            let _ = control_tx;
            state.inventory.midi_status =
                "MIDI unavailable: compiled without `system-audio`".to_string();
            return;
        }

        #[cfg(feature = "system-audio")]
        {
            let mut connected = Vec::new();
            let mut raw_inputs = Vec::new();
            let mut connected_port_names = Vec::new();
            for desired_port_name in desired_port_names {
                if let Some(device) = parse_rawmidi_port_name(&desired_port_name) {
                    let Ok(process) = spawn_rawmidi_input_process(device, control_tx.clone())
                    else {
                        continue;
                    };
                    raw_inputs.push(process);
                    connected_port_names.push(desired_port_name);
                    continue;
                }
                let Ok(input) = MidiInput::new("pipemeeter-live-input") else {
                    continue;
                };
                let Some(port) = input.ports().into_iter().find(|port| {
                    input.port_name(port).ok().as_deref() == Some(desired_port_name.as_str())
                }) else {
                    continue;
                };
                let sender = control_tx.clone();
                let Ok(connection) = input.connect(
                    &port,
                    "pipemeeter-live-input",
                    move |_timestamp, message, _| {
                        if message.len() < 2 {
                            return;
                        }
                        let status = message[0];
                        let channel = status & 0x0F;
                        let number = message[1];
                        let value = message.get(2).copied().unwrap_or(0);
                        let kind = match status & 0xF0 {
                            0x80 => Some(MidiMessageKind::Note),
                            0x90 => Some(MidiMessageKind::Note),
                            0xB0 => Some(MidiMessageKind::ControlChange),
                            _ => None,
                        };
                        let Some(kind) = kind else {
                            return;
                        };
                        let normalized_value = if status & 0xF0 == 0x80 { 0 } else { value };
                        let _ = sender.send(AudioControlMsg::ApplyMidiEvent {
                            event: MidiEvent {
                                kind,
                                channel,
                                number,
                                value: normalized_value,
                            },
                        });
                    },
                    (),
                ) else {
                    continue;
                };
                connected.push(connection);
                connected_port_names.push(desired_port_name);
            }

            self.connections = connected;
            self.raw_inputs = raw_inputs;
            self.connected_port_names = connected_port_names;
        }
    }

    fn disconnect_all(&mut self) {
        #[cfg(feature = "system-audio")]
        {
            self.connections.clear();
            for mut process in self.raw_inputs.drain(..) {
                let _ = process.child.kill();
                let _ = process.child.wait();
                let _ = process.reader.join();
            }
        }
        self.connected_port_names.clear();
    }
}

fn parse_midi_event_bytes(bytes: &[u8]) -> Option<MidiEvent> {
    if bytes.len() < 2 {
        return None;
    }
    let status = bytes[0];
    let channel = status & 0x0F;
    let number = bytes[1];
    let value = bytes.get(2).copied().unwrap_or(0);
    let kind = match status & 0xF0 {
        0x80 => Some(MidiMessageKind::Note),
        0x90 => Some(MidiMessageKind::Note),
        0xB0 => Some(MidiMessageKind::ControlChange),
        _ => None,
    }?;
    let normalized_value = if status & 0xF0 == 0x80 { 0 } else { value };
    Some(MidiEvent {
        kind,
        channel,
        number,
        value: normalized_value,
    })
}

fn rawmidi_port_name(device: &str, name: &str) -> String {
    format!("{name} [rawmidi {device}]")
}

fn parse_rawmidi_port_name(port_name: &str) -> Option<&str> {
    let suffix = port_name.strip_suffix(']')?;
    let (_, device) = suffix.rsplit_once("[rawmidi ")?;
    Some(device.trim())
}

fn spawn_rawmidi_input_process(
    device: &str,
    sender: Sender<AudioControlMsg>,
) -> Result<RawMidiInputProcess, String> {
    #[cfg(not(feature = "system-audio"))]
    {
        let _ = (device, sender);
        Err("compiled without `system-audio`; raw MIDI capture is unavailable".to_string())
    }

    #[cfg(feature = "system-audio")]
    {
        let mut child = Command::new("amidi")
            .args(["-p", device, "-d"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("failed to start raw MIDI capture on {device}: {error}"))?;
        let Some(stdout) = child.stdout.take() else {
            return Err(format!(
                "raw MIDI capture did not expose stdout for {device}"
            ));
        };
        let device_name = device.to_string();
        let reader = thread::Builder::new()
            .name(format!("pipemeeter-rawmidi-{device}"))
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut line = String::new();
                loop {
                    line.clear();
                    let Ok(read) = reader.read_line(&mut line) else {
                        break;
                    };
                    if read == 0 {
                        break;
                    }
                    let bytes = line
                        .split_whitespace()
                        .filter_map(|token| u8::from_str_radix(token, 16).ok())
                        .collect::<Vec<_>>();
                    if let Some(event) = parse_midi_event_bytes(&bytes) {
                        let _ = sender.send(AudioControlMsg::ApplyMidiEvent { event });
                    }
                }
            })
            .map_err(|error| {
                format!("failed to spawn raw MIDI reader for {device_name}: {error}")
            })?;

        Ok(RawMidiInputProcess { child, reader })
    }
}

struct MeterSnapshot {
    levels: Vec<f32>,
    last_update: Instant,
}

struct MeterTap {
    child: Child,
    reader: thread::JoinHandle<()>,
    snapshot: Arc<Mutex<MeterSnapshot>>,
}

#[derive(Default)]
struct MeterRuntime {
    taps: std::collections::HashMap<String, MeterTap>,
}

impl MeterRuntime {
    fn sync_taps(&mut self, state: &mut AudioEngineState) {
        let desired_nodes = state
            .source_strips
            .iter()
            .chain(state.input_strips.iter())
            .chain(state.bus_strips.iter())
            .chain(state.output_strips.iter())
            .filter_map(|strip| {
                strip.pipewire_node_name.clone().and_then(|node_name| {
                    strip_meter_source_name(strip).map(|source_name| (node_name, source_name))
                })
            })
            .collect::<std::collections::HashMap<_, _>>();

        let existing_nodes = self.taps.keys().cloned().collect::<Vec<_>>();
        for node_name in existing_nodes {
            if desired_nodes.contains_key(&node_name) {
                continue;
            }
            if let Some(tap) = self.taps.remove(&node_name) {
                stop_meter_tap(tap);
            }
        }

        for (node_name, source_name) in desired_nodes {
            if self.taps.contains_key(&node_name) {
                continue;
            }
            match spawn_meter_tap(&node_name, &source_name) {
                Ok(tap) => {
                    self.taps.insert(node_name, tap);
                }
                Err(error) => {
                    state.last_notice = format!("{}; meter tap failed: {error}", state.last_notice);
                }
            }
        }
    }

    fn snapshot_levels(&self) -> std::collections::HashMap<String, Vec<f32>> {
        self.taps
            .iter()
            .map(|(node_name, tap)| {
                let levels = tap
                    .snapshot
                    .lock()
                    .ok()
                    .map(|snapshot| {
                        if snapshot.last_update.elapsed() > Duration::from_millis(500) {
                            vec![0.0; METER_CHANNEL_COUNT]
                        } else {
                            snapshot.levels.clone()
                        }
                    })
                    .unwrap_or_else(|| vec![0.0; METER_CHANNEL_COUNT]);
                (node_name.clone(), levels)
            })
            .collect()
    }

    fn stop_all(&mut self) {
        for (_, tap) in self.taps.drain() {
            stop_meter_tap(tap);
        }
    }
}

fn stop_meter_tap(mut tap: MeterTap) {
    let _ = tap.child.kill();
    let _ = tap.child.wait();
    let _ = tap.reader.join();
}

fn strip_meter_source_name(strip: &MixerStrip) -> Option<String> {
    match strip.kind {
        StripKind::HardwareSource => strip.pipewire_node_name.clone(),
        StripKind::VirtualCable | StripKind::Strip | StripKind::Bus | StripKind::Output => strip
            .pipewire_node_name
            .as_ref()
            .map(|node_name| format!("{node_name}.monitor")),
    }
}

fn spawn_meter_tap(node_name: &str, source_name: &str) -> Result<MeterTap, String> {
    #[cfg(not(feature = "system-audio"))]
    {
        let _ = (node_name, source_name);
        Err("compiled without `system-audio`; enable it to read live meter data".to_string())
    }

    #[cfg(feature = "system-audio")]
    {
        let mut child = Command::new("parec")
            .args([
                "--record",
                "--raw",
                "--device",
                source_name,
                "--rate",
                "48000",
                "--channels",
                "2",
                "--format=s16le",
                "--latency-msec=20",
                "--process-time-msec=20",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("failed to start parec for {source_name}: {error}"))?;
        let Some(mut stdout) = child.stdout.take() else {
            return Err(format!("parec did not expose stdout for {source_name}"));
        };

        let snapshot = Arc::new(Mutex::new(MeterSnapshot {
            levels: vec![0.0; METER_CHANNEL_COUNT],
            last_update: Instant::now(),
        }));
        let snapshot_writer = Arc::clone(&snapshot);
        let thread_name = format!("pipemeeter-meter-{node_name}");
        let reader = thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                let mut buffer = [0_u8; 4096];
                let mut remainder = Vec::new();
                loop {
                    let read = match stdout.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => read,
                    };
                    remainder.extend_from_slice(&buffer[..read]);
                    let mut peaks = [0.0_f32; METER_CHANNEL_COUNT];
                    let frame_bytes = METER_CHANNEL_COUNT * std::mem::size_of::<i16>();
                    let usable = remainder.len() / frame_bytes * frame_bytes;
                    for frame in remainder[..usable].chunks_exact(frame_bytes) {
                        for channel in 0..METER_CHANNEL_COUNT {
                            let offset = channel * 2;
                            let sample = i16::from_le_bytes([frame[offset], frame[offset + 1]]);
                            let level = (sample as f32).abs() / i16::MAX as f32;
                            peaks[channel] = peaks[channel].max(level.clamp(0.0, 1.0));
                        }
                    }
                    remainder.drain(..usable);
                    if let Ok(mut snapshot) = snapshot_writer.lock() {
                        snapshot.levels = peaks.to_vec();
                        snapshot.last_update = Instant::now();
                    }
                }
            })
            .map_err(|error| format!("failed to spawn meter reader for {source_name}: {error}"))?;

        Ok(MeterTap {
            child,
            reader,
            snapshot,
        })
    }
}

#[cfg(any(test, feature = "system-audio"))]
fn collect_midi_feedback_messages(state: &AudioEngineState) -> Vec<MidiFeedbackMessage> {
    let mut messages = Vec::new();

    for strip in state
        .input_strips
        .iter()
        .chain(state.bus_strips.iter())
        .chain(state.output_strips.iter())
    {
        if let Some(binding) = strip.midi.volume_binding() {
            messages.push(MidiFeedbackMessage {
                kind: binding.kind,
                channel: binding.channel.unwrap_or(0),
                number: binding.number,
                value: ((strip.volume.as_ratio() * 127.0).round() as i32).clamp(0, 127) as u8,
            });
        }

        if let Some(binding) = strip.midi.mute_binding() {
            messages.push(MidiFeedbackMessage {
                kind: binding.kind,
                channel: binding.channel.unwrap_or(0),
                number: binding.number,
                value: if strip.muted {
                    MIDI_FEEDBACK_ON_VALUE
                } else {
                    MIDI_FEEDBACK_OFF_VALUE
                },
            });
        }

        for target in [
            FxMidiTarget::Bypass,
            FxMidiTarget::GateEnabled,
            FxMidiTarget::GateThreshold,
            FxMidiTarget::GateFloor,
            FxMidiTarget::CompressorEnabled,
            FxMidiTarget::CompressorThreshold,
            FxMidiTarget::CompressorRatio,
            FxMidiTarget::CompressorMakeupGain,
            FxMidiTarget::EqEnabled,
            FxMidiTarget::EqLowGain,
            FxMidiTarget::EqMidGain,
            FxMidiTarget::EqHighGain,
        ] {
            let Some(binding) = strip.fx_midi.binding(target) else {
                continue;
            };
            messages.push(MidiFeedbackMessage {
                kind: binding.kind,
                channel: binding.channel.unwrap_or(0),
                number: binding.number,
                value: strip.fx_midi_feedback_value(target),
            });
        }
    }

    for strip in state.input_strips.iter().chain(state.bus_strips.iter()) {
        for route in &strip.routes {
            if let Some(binding) = route.binding() {
                messages.push(MidiFeedbackMessage {
                    kind: binding.kind,
                    channel: binding.channel.unwrap_or(0),
                    number: binding.number,
                    value: if route.enabled {
                        MIDI_FEEDBACK_ON_VALUE
                    } else {
                        MIDI_FEEDBACK_OFF_VALUE
                    },
                });
            }
        }
    }

    messages
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
    HardwareSource,
    VirtualCable,
    Strip,
    Bus,
    Output,
}

impl StripKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HardwareSource => "Source",
            Self::VirtualCable => "Virtual cable",
            Self::Strip => "Strip",
            Self::Bus => "Bus",
            Self::Output => "Output",
        }
    }

    fn default_label_prefix(self) -> &'static str {
        match self {
            Self::HardwareSource => "Source",
            Self::VirtualCable => "Cable",
            Self::Strip => "Strip",
            Self::Bus => "Bus",
            Self::Output => "Output",
        }
    }

    pub fn route_target_label(self) -> &'static str {
        match self {
            Self::Strip => "Bus",
            Self::Bus => "Output",
            Self::HardwareSource | Self::VirtualCable => "Route target",
            Self::Output => "Route target",
        }
    }

    pub fn route_target_label_plural(self) -> &'static str {
        match self {
            Self::Strip => "buses",
            Self::Bus => "outputs",
            Self::HardwareSource | Self::VirtualCable => "route targets",
            Self::Output => "route targets",
        }
    }

    pub fn route_hint(self) -> &'static str {
        match self {
            Self::HardwareSource => "Sources are assigned to strips; they do not route directly.",
            Self::VirtualCable => "Virtual cables feed strips; they do not route directly.",
            Self::Strip => {
                "Bind exactly one source or virtual cable, then send this strip into one or more buses."
            }
            Self::Bus => "Collect strips in this bus, then map the bus to one or more outputs.",
            Self::Output => "Outputs do not route onward.",
        }
    }

    pub fn empty_route_hint(self) -> &'static str {
        match self {
            Self::HardwareSource => "Choose from a strip",
            Self::VirtualCable => "Choose from a strip",
            Self::Strip => "No bus sends",
            Self::Bus => "No output mappings",
            Self::Output => "Direct output",
        }
    }

    pub fn supports_volume_and_mute(self) -> bool {
        !matches!(self, Self::HardwareSource)
    }

    pub fn supports_mono(self) -> bool {
        matches!(self, Self::Strip | Self::Bus)
    }

    pub fn supports_routes(self) -> bool {
        matches!(self, Self::Strip | Self::Bus)
    }

    pub fn supports_input_assignment(self) -> bool {
        self == Self::Strip
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripRole {
    HardwareSource,
    VirtualCable,
    ChannelStrip,
    Bus,
    OutputBus,
    SystemOutput,
}

impl StripRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::HardwareSource => "Hardware source",
            Self::VirtualCable => "Virtual cable",
            Self::ChannelStrip => "Channel strip",
            Self::Bus => "Bus",
            Self::OutputBus => "Output bus",
            Self::SystemOutput => "System output",
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxMidiTarget {
    Bypass,
    GateEnabled,
    GateThreshold,
    GateFloor,
    CompressorEnabled,
    CompressorThreshold,
    CompressorRatio,
    CompressorMakeupGain,
    EqEnabled,
    EqLowGain,
    EqMidGain,
    EqHighGain,
}

impl FxMidiTarget {
    fn label(self) -> &'static str {
        match self {
            Self::Bypass => "FX bypass",
            Self::GateEnabled => "gate enable",
            Self::GateThreshold => "gate threshold",
            Self::GateFloor => "gate floor",
            Self::CompressorEnabled => "compressor enable",
            Self::CompressorThreshold => "compressor threshold",
            Self::CompressorRatio => "compressor ratio",
            Self::CompressorMakeupGain => "compressor makeup",
            Self::EqEnabled => "EQ enable",
            Self::EqLowGain => "low EQ",
            Self::EqMidGain => "mid EQ",
            Self::EqHighGain => "high EQ",
        }
    }

    fn requires_control_change(self) -> bool {
        matches!(
            self,
            Self::GateThreshold
                | Self::GateFloor
                | Self::CompressorThreshold
                | Self::CompressorRatio
                | Self::CompressorMakeupGain
                | Self::EqLowGain
                | Self::EqMidGain
                | Self::EqHighGain
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MidiMessageKind {
    ControlChange,
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiTrigger {
    pub kind: MidiMessageKind,
    pub number: u8,
    #[serde(default)]
    pub channel: Option<u8>,
}

impl MidiTrigger {
    fn control_change(number: u8) -> Self {
        Self {
            kind: MidiMessageKind::ControlChange,
            number,
            channel: None,
        }
    }

    fn format_midi_trigger(binding: Option<&MidiTrigger>) -> String {
        binding
            .map(|binding| {
                let kind = match binding.kind {
                    MidiMessageKind::ControlChange => "CC",
                    MidiMessageKind::Note => "Note",
                };
                let channel = binding
                    .channel
                    .map(|channel| format!(" ch{}", channel + 1))
                    .unwrap_or_default();
                format!("{kind} {}{channel}", binding.number)
            })
            .unwrap_or_else(|| "cleared".to_string())
    }

    fn matches(&self, event: &MidiEvent) -> bool {
        self.kind == event.kind
            && self.number == event.number
            && self.channel.is_none_or(|channel| channel == event.channel)
    }
}

fn midi_boolean_press(binding: &MidiTrigger, event: &MidiEvent) -> bool {
    binding.matches(event) && event.value >= 64
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiEvent {
    pub kind: MidiMessageKind,
    pub channel: u8,
    pub number: u8,
    pub value: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiLearnTarget {
    Strip {
        strip: StripId,
        target: MidiControlTarget,
    },
    Fx {
        strip: StripId,
        target: FxMidiTarget,
    },
    Route {
        strip: StripId,
        output: StripId,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiBinding {
    #[serde(default)]
    pub volume: Option<MidiTrigger>,
    #[serde(default)]
    pub mute: Option<MidiTrigger>,
    #[serde(default)]
    pub volume_cc: Option<u8>,
    #[serde(default)]
    pub mute_cc: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FxMidiBinding {
    #[serde(default)]
    pub bypass: Option<MidiTrigger>,
    #[serde(default)]
    pub gate_enabled: Option<MidiTrigger>,
    #[serde(default)]
    pub gate_threshold: Option<MidiTrigger>,
    #[serde(default)]
    pub gate_floor: Option<MidiTrigger>,
    #[serde(default)]
    pub compressor_enabled: Option<MidiTrigger>,
    #[serde(default)]
    pub compressor_threshold: Option<MidiTrigger>,
    #[serde(default)]
    pub compressor_ratio: Option<MidiTrigger>,
    #[serde(default)]
    pub compressor_makeup_gain: Option<MidiTrigger>,
    #[serde(default)]
    pub eq_enabled: Option<MidiTrigger>,
    #[serde(default)]
    pub eq_low_gain: Option<MidiTrigger>,
    #[serde(default)]
    pub eq_mid_gain: Option<MidiTrigger>,
    #[serde(default)]
    pub eq_high_gain: Option<MidiTrigger>,
}

impl FxMidiBinding {
    pub fn binding(&self, target: FxMidiTarget) -> Option<MidiTrigger> {
        match target {
            FxMidiTarget::Bypass => self.bypass.clone(),
            FxMidiTarget::GateEnabled => self.gate_enabled.clone(),
            FxMidiTarget::GateThreshold => self.gate_threshold.clone(),
            FxMidiTarget::GateFloor => self.gate_floor.clone(),
            FxMidiTarget::CompressorEnabled => self.compressor_enabled.clone(),
            FxMidiTarget::CompressorThreshold => self.compressor_threshold.clone(),
            FxMidiTarget::CompressorRatio => self.compressor_ratio.clone(),
            FxMidiTarget::CompressorMakeupGain => self.compressor_makeup_gain.clone(),
            FxMidiTarget::EqEnabled => self.eq_enabled.clone(),
            FxMidiTarget::EqLowGain => self.eq_low_gain.clone(),
            FxMidiTarget::EqMidGain => self.eq_mid_gain.clone(),
            FxMidiTarget::EqHighGain => self.eq_high_gain.clone(),
        }
    }

    fn binding_mut(&mut self, target: FxMidiTarget) -> &mut Option<MidiTrigger> {
        match target {
            FxMidiTarget::Bypass => &mut self.bypass,
            FxMidiTarget::GateEnabled => &mut self.gate_enabled,
            FxMidiTarget::GateThreshold => &mut self.gate_threshold,
            FxMidiTarget::GateFloor => &mut self.gate_floor,
            FxMidiTarget::CompressorEnabled => &mut self.compressor_enabled,
            FxMidiTarget::CompressorThreshold => &mut self.compressor_threshold,
            FxMidiTarget::CompressorRatio => &mut self.compressor_ratio,
            FxMidiTarget::CompressorMakeupGain => &mut self.compressor_makeup_gain,
            FxMidiTarget::EqEnabled => &mut self.eq_enabled,
            FxMidiTarget::EqLowGain => &mut self.eq_low_gain,
            FxMidiTarget::EqMidGain => &mut self.eq_mid_gain,
            FxMidiTarget::EqHighGain => &mut self.eq_high_gain,
        }
    }

    fn set_binding(&mut self, target: FxMidiTarget, binding: Option<MidiTrigger>) {
        *self.binding_mut(target) = binding;
    }
}

impl MidiBinding {
    pub fn volume_binding(&self) -> Option<MidiTrigger> {
        self.volume
            .clone()
            .or_else(|| self.volume_cc.map(MidiTrigger::control_change))
    }

    pub fn mute_binding(&self) -> Option<MidiTrigger> {
        self.mute
            .clone()
            .or_else(|| self.mute_cc.map(MidiTrigger::control_change))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiFeedbackConfig {
    #[serde(default)]
    pub output_port_name: Option<String>,
}

fn default_gate_threshold_percent() -> f32 {
    18.0
}

fn default_gate_floor_percent() -> f32 {
    0.0
}

fn default_route_enabled(kind: StripKind, input_index: usize, output_index: usize) -> bool {
    match kind {
        StripKind::Strip => output_index == 0 && input_index == 0,
        StripKind::Bus => output_index == 0,
        StripKind::HardwareSource | StripKind::VirtualCable => false,
        StripKind::Output => false,
    }
}

fn default_compressor_threshold_percent() -> f32 {
    78.0
}

fn default_compressor_ratio() -> f32 {
    3.0
}

fn default_eq_gain_db() -> f32 {
    0.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoiseGateSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_gate_threshold_percent")]
    pub threshold_percent: f32,
    #[serde(default = "default_gate_floor_percent")]
    pub floor_percent: f32,
}

impl Default for NoiseGateSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_percent: default_gate_threshold_percent(),
            floor_percent: default_gate_floor_percent(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompressorSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_compressor_threshold_percent")]
    pub threshold_percent: f32,
    #[serde(default = "default_compressor_ratio")]
    pub ratio: f32,
    #[serde(default)]
    pub makeup_gain_db: f32,
}

impl Default for CompressorSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_percent: default_compressor_threshold_percent(),
            ratio: default_compressor_ratio(),
            makeup_gain_db: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EqSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_eq_gain_db")]
    pub low_gain_db: f32,
    #[serde(default = "default_eq_gain_db")]
    pub mid_gain_db: f32,
    #[serde(default = "default_eq_gain_db")]
    pub high_gain_db: f32,
}

impl Default for EqSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            low_gain_db: default_eq_gain_db(),
            mid_gain_db: default_eq_gain_db(),
            high_gain_db: default_eq_gain_db(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct StripEffects {
    #[serde(default)]
    pub bypassed: bool,
    #[serde(default)]
    pub gate: NoiseGateSettings,
    #[serde(default)]
    pub compressor: CompressorSettings,
    #[serde(default)]
    pub eq: EqSettings,
}

impl StripEffects {
    pub fn active_effect_count(&self) -> usize {
        usize::from(self.gate.enabled)
            + usize::from(self.compressor.enabled)
            + usize::from(self.eq.enabled)
    }
}

fn pulse_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn is_managed_virtual_cable_name(node_name: &str) -> bool {
    node_name.starts_with(PIPEMEETER_VIRTUAL_CABLE_PREFIX)
}

fn is_managed_strip_sink_name(node_name: &str) -> bool {
    node_name.starts_with(PIPEMEETER_STRIP_SINK_PREFIX)
}

fn is_managed_bus_sink_name(node_name: &str) -> bool {
    node_name.starts_with(PIPEMEETER_BUS_SINK_PREFIX)
}

fn is_managed_output_sink_name(node_name: &str) -> bool {
    node_name.starts_with(PIPEMEETER_OUTPUT_SINK_PREFIX)
}

fn sink_name_slug(label: &str) -> String {
    let mut slug = String::new();
    let mut previous_was_separator = false;
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !previous_was_separator {
            slug.push('-');
            previous_was_separator = true;
        }
    }

    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "sink".to_string()
    } else {
        slug.to_string()
    }
}

fn scan_pulse_sink_names() -> Result<Vec<String>, String> {
    #[cfg(not(feature = "system-audio"))]
    {
        return Err("compiled without `system-audio`; enable it to query Pulse sinks".to_string());
    }

    #[cfg(feature = "system-audio")]
    {
        Ok(scan_pulse_sinks()?
            .into_iter()
            .map(|sink| sink.name)
            .collect())
    }
}

fn scan_pulse_sinks() -> Result<Vec<PulseSinkInfo>, String> {
    #[cfg(not(feature = "system-audio"))]
    {
        return Err("compiled without `system-audio`; enable it to query Pulse sinks".to_string());
    }

    #[cfg(feature = "system-audio")]
    {
        let output = Command::new("pactl")
            .args(["list", "short", "sinks"])
            .output()
            .map_err(|error| format!("failed to execute pactl list short sinks: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "pactl list short sinks failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let mut columns = line.split('\t');
                let index = columns.next()?.trim().parse::<u32>().ok()?;
                let name = columns.next()?.trim().to_string();
                if name.is_empty() {
                    return None;
                }
                Some(PulseSinkInfo { index, name })
            })
            .collect())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PulseSourceInfo {
    name: String,
    description: String,
    channel_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PulseSinkInfo {
    index: u32,
    name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationStreamIdentity {
    pub cached_index: u32,
    pub application_name: String,
    pub media_name: String,
    pub process_binary: Option<String>,
    pub process_id: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationStreamInfo {
    pub identity: ApplicationStreamIdentity,
    pub current_sink_name: String,
    pub current_sink_label: String,
    pub icon_data_url: Option<String>,
    pub corked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PulseSinkInputInfo {
    identity: ApplicationStreamIdentity,
    current_sink_name: String,
    icon_data_url: Option<String>,
    corked: bool,
}

fn parse_pulse_channel_count(specification: &str) -> usize {
    specification
        .split_whitespace()
        .find_map(|token| token.strip_suffix("ch"))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(METER_CHANNEL_COUNT)
        .max(1)
}

fn scan_pulse_sources() -> Result<Vec<PulseSourceInfo>, String> {
    #[cfg(not(feature = "system-audio"))]
    {
        return Err(
            "compiled without `system-audio`; enable it to query Pulse sources".to_string(),
        );
    }

    #[cfg(feature = "system-audio")]
    {
        let output = Command::new("pactl")
            .args(["list", "sources"])
            .output()
            .map_err(|error| format!("failed to execute pactl list sources: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "pactl list sources failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        let mut sources = Vec::new();
        let mut name = None::<String>;
        let mut description = None::<String>;
        let mut channel_count = None::<usize>;

        let push_source = |sources: &mut Vec<PulseSourceInfo>,
                           name: &mut Option<String>,
                           description: &mut Option<String>,
                           channel_count: &mut Option<usize>| {
            let Some(name_value) = name.take() else {
                *description = None;
                *channel_count = None;
                return;
            };
            if name_value.ends_with(".monitor") {
                *description = None;
                *channel_count = None;
                return;
            }
            let description_value = description.take().unwrap_or_else(|| name_value.clone());
            let channels = channel_count.take().unwrap_or(METER_CHANNEL_COUNT).max(1);
            sources.push(PulseSourceInfo {
                name: name_value,
                description: description_value,
                channel_count: channels,
            });
        };

        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Source #") {
                push_source(
                    &mut sources,
                    &mut name,
                    &mut description,
                    &mut channel_count,
                );
                continue;
            }
            if let Some(value) = trimmed.strip_prefix("Name: ") {
                name = Some(value.trim().to_string());
                continue;
            }
            if let Some(value) = trimmed.strip_prefix("Description: ") {
                description = Some(value.trim().to_string());
                continue;
            }
            if let Some(value) = trimmed.strip_prefix("Sample Specification: ") {
                channel_count = Some(parse_pulse_channel_count(value));
            }
        }

        push_source(
            &mut sources,
            &mut name,
            &mut description,
            &mut channel_count,
        );
        sources.sort_by(|left, right| left.description.cmp(&right.description));
        Ok(sources)
    }
}

fn scan_application_streams() -> Result<Vec<PulseSinkInputInfo>, String> {
    #[cfg(not(feature = "system-audio"))]
    {
        return Err(
            "compiled without `system-audio`; enable it to query application streams".to_string(),
        );
    }

    #[cfg(feature = "system-audio")]
    {
        let sinks_by_index = scan_pulse_sinks()?
            .into_iter()
            .map(|sink| (sink.index, sink.name))
            .collect::<std::collections::HashMap<_, _>>();

        let output = Command::new("pactl")
            .args(["list", "sink-inputs"])
            .output()
            .map_err(|error| format!("failed to execute pactl list sink-inputs: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "pactl list sink-inputs failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        let mut streams =
            parse_pulse_sink_inputs(&String::from_utf8_lossy(&output.stdout), &sinks_by_index);
        streams.sort_by(|left, right| {
            left.identity
                .application_name
                .cmp(&right.identity.application_name)
                .then(left.identity.media_name.cmp(&right.identity.media_name))
                .then(left.identity.cached_index.cmp(&right.identity.cached_index))
        });
        Ok(streams)
    }
}

fn parse_pulse_sink_inputs(
    dump: &str,
    sinks_by_index: &std::collections::HashMap<u32, String>,
) -> Vec<PulseSinkInputInfo> {
    #[derive(Default)]
    struct SinkInputBuilder {
        index: Option<u32>,
        sink_index: Option<u32>,
        corked: bool,
        properties: std::collections::HashMap<String, String>,
    }

    fn push_sink_input(
        streams: &mut Vec<PulseSinkInputInfo>,
        builder: &mut SinkInputBuilder,
        sinks_by_index: &std::collections::HashMap<u32, String>,
    ) {
        let Some(index) = builder.index.take() else {
            builder.sink_index = None;
            builder.corked = false;
            builder.properties.clear();
            return;
        };

        let application_name = builder
            .properties
            .get("application.name")
            .cloned()
            .or_else(|| builder.properties.get("node.name").cloned())
            .unwrap_or_default();
        if application_name.trim().is_empty() {
            builder.sink_index = None;
            builder.corked = false;
            builder.properties.clear();
            return;
        }

        let media_name = builder
            .properties
            .get("media.name")
            .cloned()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| application_name.clone());
        let process_binary = builder
            .properties
            .get("application.process.binary")
            .cloned()
            .filter(|value| !value.trim().is_empty());
        let process_id = builder
            .properties
            .get("application.process.id")
            .and_then(|value| value.parse::<u32>().ok());
        let icon_name = builder
            .properties
            .get("application.icon_name")
            .cloned()
            .or_else(|| builder.properties.get("application.icon-name").cloned())
            .filter(|value| !value.trim().is_empty());
        let sink_name = builder
            .sink_index
            .and_then(|sink_index| sinks_by_index.get(&sink_index).cloned())
            .unwrap_or_else(|| "unknown sink".to_string());

        streams.push(PulseSinkInputInfo {
            identity: ApplicationStreamIdentity {
                cached_index: index,
                application_name: application_name.trim().to_string(),
                media_name: media_name.trim().to_string(),
                process_binary,
                process_id,
            },
            current_sink_name: sink_name,
            icon_data_url: resolve_application_icon_data_url(
                icon_name.as_deref(),
                builder
                    .properties
                    .get("application.process.binary")
                    .map(String::as_str),
                Some(application_name.as_str()),
            ),
            corked: builder.corked,
        });

        builder.sink_index = None;
        builder.corked = false;
        builder.properties.clear();
    }

    let mut streams = Vec::new();
    let mut builder = SinkInputBuilder::default();
    let mut in_properties = false;

    for line in dump.lines() {
        let trimmed = line.trim();
        if let Some(index) = trimmed.strip_prefix("Sink Input #") {
            push_sink_input(&mut streams, &mut builder, sinks_by_index);
            builder.index = index.trim().parse::<u32>().ok();
            in_properties = false;
            continue;
        }

        if trimmed.is_empty() {
            in_properties = false;
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("Sink: ") {
            builder.sink_index = value.trim().parse::<u32>().ok();
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("Corked: ") {
            builder.corked = value.trim().eq_ignore_ascii_case("yes");
            continue;
        }

        if trimmed == "Properties:" {
            in_properties = true;
            continue;
        }

        if in_properties {
            if line.starts_with("\t\t") || line.starts_with("        ") {
                if let Some((key, value)) = trimmed.split_once(" = ") {
                    builder.properties.insert(
                        key.trim().to_string(),
                        value.trim().trim_matches('"').to_string(),
                    );
                }
            } else {
                in_properties = false;
            }
        }
    }

    push_sink_input(&mut streams, &mut builder, sinks_by_index);
    streams
}

fn resolve_application_icon_data_url(
    icon_name: Option<&str>,
    process_binary: Option<&str>,
    application_name: Option<&str>,
) -> Option<String> {
    let mut candidates = Vec::new();
    for candidate in [icon_name, process_binary, application_name] {
        let Some(candidate) = candidate.map(str::trim).filter(|value| !value.is_empty()) else {
            continue;
        };
        candidates.push(candidate.to_string());
        candidates.push(candidate.to_ascii_lowercase());
        candidates.push(candidate.replace(' ', "-").to_ascii_lowercase());
        candidates.push(candidate.replace(' ', "").to_ascii_lowercase());
    }

    candidates.sort();
    candidates.dedup();

    for candidate in candidates {
        if let Some(path) = resolve_icon_path(&candidate) {
            if let Some(data_url) = icon_path_to_data_url(&path) {
                return Some(data_url);
            }
        }
    }

    None
}

fn resolve_icon_path(candidate: &str) -> Option<PathBuf> {
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        return None;
    }

    let direct = PathBuf::from(trimmed);
    if direct.is_absolute() && direct.is_file() {
        return Some(direct);
    }

    let home = env::var_os("HOME").map(PathBuf::from);
    let mut roots = vec![
        PathBuf::from("/usr/share/pixmaps"),
        PathBuf::from("/usr/share/icons"),
        PathBuf::from("/usr/local/share/icons"),
        PathBuf::from("/var/lib/flatpak/exports/share/icons"),
    ];
    if let Some(home) = home {
        roots.push(home.join(".local/share/icons"));
        roots.push(home.join(".icons"));
        roots.push(home.join(".local/share/flatpak/exports/share/icons"));
    }

    for root in roots {
        if let Some(path) = resolve_icon_path_in_root(&root, trimmed) {
            return Some(path);
        }
    }

    None
}

fn resolve_icon_path_in_root(root: &Path, candidate: &str) -> Option<PathBuf> {
    if !root.exists() {
        return None;
    }

    for extension in ["png", "svg", "xpm"] {
        let direct = root.join(format!("{candidate}.{extension}"));
        if direct.is_file() {
            return Some(direct);
        }
    }

    let theme_dirs = fs::read_dir(root).ok()?;
    let mut theme_paths = theme_dirs
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.is_dir() { Some(path) } else { None }
        })
        .collect::<Vec<_>>();
    theme_paths.sort();

    let preferred = ["hicolor", "breeze", "Adwaita", "Papirus", "HighContrast"];
    theme_paths.sort_by_key(|path| {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        preferred
            .iter()
            .position(|preferred| preferred.eq_ignore_ascii_case(name))
            .unwrap_or(preferred.len())
    });

    for theme_path in theme_paths {
        for relative in [
            format!("scalable/apps/{candidate}.svg"),
            format!("symbolic/apps/{candidate}.svg"),
            format!("symbolic/apps/{candidate}-symbolic.svg"),
            format!("128x128/apps/{candidate}.png"),
            format!("96x96/apps/{candidate}.png"),
            format!("64x64/apps/{candidate}.png"),
            format!("48x48/apps/{candidate}.png"),
            format!("32x32/apps/{candidate}.png"),
            format!("24x24/apps/{candidate}.png"),
            format!("22x22/apps/{candidate}.png"),
            format!("16x16/apps/{candidate}.png"),
        ] {
            let candidate_path = theme_path.join(relative);
            if candidate_path.is_file() {
                return Some(candidate_path);
            }
        }
    }

    None
}

fn icon_path_to_data_url(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let mime = match extension.as_str() {
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "xpm" => "image/x-xpixmap",
        _ => return None,
    };
    let bytes = fs::read(path).ok()?;
    Some(format!(
        "data:{mime};base64,{}",
        BASE64_STANDARD.encode(bytes)
    ))
}

fn create_pipewire_sink(node_name: &str, label: &str) -> Result<(), String> {
    #[cfg(not(feature = "system-audio"))]
    {
        let _ = (node_name, label);
        return Err(
            "compiled without `system-audio`; enable it to create PipeWire sinks".to_string(),
        );
    }

    #[cfg(feature = "system-audio")]
    {
        if scan_pulse_sink_names()?
            .iter()
            .any(|name| name == node_name)
        {
            return Ok(());
        }

        let properties = format!(
            "sink_properties=device.description=\"{}\" node.description=\"{}\" node.nick=\"{}\"",
            pulse_escape(label),
            pulse_escape(label),
            pulse_escape(label)
        );
        let output = Command::new("pactl")
            .args([
                "load-module",
                "module-null-sink",
                &format!("sink_name={node_name}"),
                &properties,
            ])
            .output()
            .map_err(|error| format!("failed to execute pactl load-module: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "failed to create PipeWire sink {label}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        Ok(())
    }
}

fn remove_pipewire_sink(node_name: &str) -> Result<(), String> {
    #[cfg(not(feature = "system-audio"))]
    {
        let _ = node_name;
        return Err(
            "compiled without `system-audio`; enable it to remove PipeWire sinks".to_string(),
        );
    }

    #[cfg(feature = "system-audio")]
    {
        let output = Command::new("pactl")
            .args(["list", "short", "modules"])
            .output()
            .map_err(|error| format!("failed to execute pactl list short modules: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "pactl list short modules failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        let needle = format!("sink_name={node_name}");
        let module_id = String::from_utf8_lossy(&output.stdout)
            .lines()
            .find_map(|line| {
                if !line.contains("module-null-sink") || !line.contains(&needle) {
                    return None;
                }
                line.split('\t').next().map(str::trim).map(str::to_string)
            });

        let Some(module_id) = module_id else {
            return Ok(());
        };

        let unload = Command::new("pactl")
            .args(["unload-module", &module_id])
            .output()
            .map_err(|error| format!("failed to execute pactl unload-module: {error}"))?;
        if !unload.status.success() {
            return Err(format!(
                "failed to remove PipeWire sink {node_name}: {}",
                String::from_utf8_lossy(&unload.stderr).trim()
            ));
        }

        Ok(())
    }
}

fn sync_pipewire_strip_state(
    kind: StripKind,
    node_name: &str,
    volume: NormalizedVolume,
    muted: bool,
) -> Result<(), String> {
    #[cfg(not(feature = "system-audio"))]
    {
        let _ = (kind, node_name, volume, muted);
        return Err(
            "compiled without `system-audio`; enable it to control PipeWire sources and sinks"
                .to_string(),
        );
    }

    #[cfg(feature = "system-audio")]
    {
        let (volume_command, mute_command, target_kind) = match kind {
            StripKind::HardwareSource => ("set-source-volume", "set-source-mute", "source"),
            StripKind::VirtualCable | StripKind::Strip | StripKind::Bus | StripKind::Output => {
                ("set-sink-volume", "set-sink-mute", "sink")
            }
        };
        let volume_percent = format!("{:.0}%", volume.as_percentage().round());
        let volume_result = Command::new("pactl")
            .args([volume_command, node_name, &volume_percent])
            .output()
            .map_err(|error| format!("failed to execute pactl {volume_command}: {error}"))?;
        if !volume_result.status.success() {
            return Err(format!(
                "failed to set {target_kind} volume for {node_name}: {}",
                String::from_utf8_lossy(&volume_result.stderr).trim()
            ));
        }

        let mute_result = Command::new("pactl")
            .args([mute_command, node_name, if muted { "1" } else { "0" }])
            .output()
            .map_err(|error| format!("failed to execute pactl {mute_command}: {error}"))?;
        if !mute_result.status.success() {
            return Err(format!(
                "failed to set {target_kind} mute for {node_name}: {}",
                String::from_utf8_lossy(&mute_result.stderr).trim()
            ));
        }

        Ok(())
    }
}

#[cfg(feature = "system-audio")]
#[derive(Clone, Debug, PartialEq, Eq)]
struct PulseLoopbackModule {
    module_id: String,
    source: String,
    sink: String,
}

#[cfg(feature = "system-audio")]
fn pulse_module_arg_value(arguments: &str, key: &str) -> Option<String> {
    arguments
        .split_whitespace()
        .find_map(|token| token.strip_prefix(key).map(str::to_string))
}

fn list_pipewire_loopback_modules() -> Result<Vec<PulseLoopbackModule>, String> {
    #[cfg(not(feature = "system-audio"))]
    {
        return Ok(Vec::new());
    }

    #[cfg(feature = "system-audio")]
    {
        let output = Command::new("pactl")
            .args(["list", "short", "modules"])
            .output()
            .map_err(|error| format!("failed to execute pactl list short modules: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "pactl list short modules failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let mut columns = line.splitn(3, '\t');
                let module_id = columns.next()?.trim().to_string();
                let module_name = columns.next()?.trim();
                let arguments = columns.next().unwrap_or_default().trim();
                if module_name != "module-loopback" {
                    return None;
                }

                let source = pulse_module_arg_value(arguments, "source=")?;
                let sink = pulse_module_arg_value(arguments, "sink=")?;
                Some(PulseLoopbackModule {
                    module_id,
                    source,
                    sink,
                })
            })
            .collect())
    }
}

fn unload_pulse_module(module_id: &str) -> Result<(), String> {
    #[cfg(not(feature = "system-audio"))]
    {
        let _ = module_id;
        return Ok(());
    }

    #[cfg(feature = "system-audio")]
    {
        let output = Command::new("pactl")
            .args(["unload-module", module_id])
            .output()
            .map_err(|error| format!("failed to execute pactl unload-module: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "failed to unload Pulse module {module_id}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        Ok(())
    }
}

fn create_pipewire_route_loopback(source: &str, sink: &str) -> Result<(), String> {
    #[cfg(not(feature = "system-audio"))]
    {
        let _ = (source, sink);
        return Ok(());
    }

    #[cfg(feature = "system-audio")]
    {
        let output = Command::new("pactl")
            .args([
                "load-module",
                "module-loopback",
                &format!("source={source}"),
                &format!("sink={sink}"),
                "latency_msec=1",
            ])
            .output()
            .map_err(|error| {
                format!("failed to execute pactl load-module module-loopback: {error}")
            })?;
        if !output.status.success() {
            return Err(format!(
                "failed to create route from {source} to {sink}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        Ok(())
    }
}

fn desired_pipewire_route_pairs(
    state: &AudioEngineState,
) -> std::collections::HashSet<(String, String)> {
    let source_names = state
        .source_strips
        .iter()
        .filter_map(|strip| {
            strip.pipewire_node_name.as_ref().map(|node_name| {
                let source = match strip.kind {
                    StripKind::HardwareSource => node_name.clone(),
                    StripKind::VirtualCable => format!("{node_name}.monitor"),
                    StripKind::Strip | StripKind::Bus | StripKind::Output => node_name.clone(),
                };
                (strip.id, source)
            })
        })
        .collect::<std::collections::HashMap<_, _>>();
    let strip_names = state
        .input_strips
        .iter()
        .filter_map(|strip| {
            strip
                .pipewire_node_name
                .clone()
                .map(|name| (strip.id, name))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let bus_names = state
        .bus_strips
        .iter()
        .filter_map(|strip| {
            strip
                .pipewire_node_name
                .clone()
                .map(|name| (strip.id, name))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let output_names = state
        .output_strips
        .iter()
        .filter_map(|strip| {
            strip
                .pipewire_node_name
                .clone()
                .map(|name| (strip.id, name))
        })
        .collect::<std::collections::HashMap<_, _>>();

    let mut desired = std::collections::HashSet::new();

    for strip in &state.input_strips {
        let Some(strip_sink) = strip_names.get(&strip.id).cloned() else {
            continue;
        };
        let Some(assignment) = strip.input_assignment.as_ref() else {
            continue;
        };
        if let Some(source) = source_names.get(&assignment.source_id).cloned() {
            desired.insert((source, strip_sink.clone()));
        }
        let strip_monitor = format!("{strip_sink}.monitor");
        for route in strip.routes.iter().filter(|route| route.enabled) {
            if let Some(bus_sink) = bus_names.get(&route.output_id).cloned() {
                desired.insert((strip_monitor.clone(), bus_sink));
            }
        }
    }

    for bus in &state.bus_strips {
        let Some(bus_sink) = bus_names.get(&bus.id).cloned() else {
            continue;
        };
        let bus_monitor = format!("{bus_sink}.monitor");
        for route in bus.routes.iter().filter(|route| route.enabled) {
            if let Some(output_sink) = output_names.get(&route.output_id).cloned() {
                desired.insert((bus_monitor.clone(), output_sink));
            }
        }
    }

    desired
}

fn sync_pipewire_routes(state: &mut AudioEngineState) {
    let desired_routes = desired_pipewire_route_pairs(state);
    let existing_routes = match list_pipewire_loopback_modules() {
        Ok(routes) => routes,
        Err(error) => {
            state.last_notice = format!("{}; route sync failed: {error}", state.last_notice);
            return;
        }
    };

    let mut existing_by_pair = std::collections::HashMap::<(String, String), Vec<String>>::new();
    for route in existing_routes.into_iter().filter(|route| {
        route.source.starts_with(PIPEMEETER_VIRTUAL_CABLE_PREFIX)
            || route.source.starts_with(PIPEMEETER_STRIP_SINK_PREFIX)
            || route.source.starts_with(PIPEMEETER_BUS_SINK_PREFIX)
            || route.sink.starts_with(PIPEMEETER_VIRTUAL_CABLE_PREFIX)
            || route.sink.starts_with(PIPEMEETER_STRIP_SINK_PREFIX)
            || route.sink.starts_with(PIPEMEETER_BUS_SINK_PREFIX)
    }) {
        existing_by_pair
            .entry((route.source, route.sink))
            .or_default()
            .push(route.module_id);
    }

    let mut errors = Vec::new();

    for ((source, sink), module_ids) in &existing_by_pair {
        let should_keep = desired_routes.contains(&(source.clone(), sink.clone()));
        let keep_from = usize::from(should_keep);
        for module_id in module_ids.iter().skip(keep_from) {
            if let Err(error) = unload_pulse_module(module_id) {
                errors.push(error);
            }
        }
    }

    for (source, sink) in desired_routes {
        if existing_by_pair.contains_key(&(source.clone(), sink.clone())) {
            continue;
        }

        if let Err(error) = create_pipewire_route_loopback(&source, &sink) {
            errors.push(error);
        }
    }

    if let Some(error) = errors.into_iter().next() {
        state.last_notice = format!("{}; route sync failed: {error}", state.last_notice);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RouteState {
    pub output_id: StripId,
    pub enabled: bool,
    #[serde(default)]
    pub midi_binding: Option<MidiTrigger>,
    #[serde(default)]
    pub midi_cc: Option<u8>,
    #[serde(default)]
    pub output_key: Option<String>,
}

impl RouteState {
    pub fn binding(&self) -> Option<MidiTrigger> {
        self.midi_binding
            .clone()
            .or_else(|| self.midi_cc.map(MidiTrigger::control_change))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputAssignment {
    pub source_id: StripId,
    #[serde(default)]
    pub source_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MixerStrip {
    pub id: StripId,
    pub kind: StripKind,
    pub label: String,
    pub pipewire_node_name: Option<String>,
    pub volume: NormalizedVolume,
    pub meter_level: NormalizedVolume,
    pub channel_count: usize,
    pub meter_channels: Vec<NormalizedVolume>,
    pub mono: bool,
    pub muted: bool,
    pub midi: MidiBinding,
    pub fx_midi: FxMidiBinding,
    pub input_assignment: Option<InputAssignment>,
    pub routes: Vec<RouteState>,
    pub effects: StripEffects,
}

impl MixerStrip {
    fn new(id: StripId, kind: StripKind, label: impl Into<String>) -> Self {
        let channel_count = default_channel_count();
        Self {
            id,
            kind,
            label: label.into(),
            pipewire_node_name: None,
            volume: NormalizedVolume::UNITY,
            meter_level: NormalizedVolume::new(0.0).expect("zero meter level should be valid"),
            channel_count,
            meter_channels: silent_meter_channels(channel_count),
            mono: default_mono_state(),
            muted: false,
            midi: MidiBinding::default(),
            fx_midi: FxMidiBinding::default(),
            input_assignment: None,
            routes: Vec::new(),
            effects: StripEffects::default(),
        }
    }

    pub fn active_channel_count(&self) -> usize {
        if self.mono {
            1
        } else {
            self.channel_count.max(1)
        }
    }

    fn output_match_key(&self) -> String {
        self.pipewire_node_name
            .clone()
            .unwrap_or_else(|| self.label.clone())
    }

    pub fn is_managed_output(&self) -> bool {
        self.pipewire_node_name
            .as_deref()
            .is_some_and(is_managed_output_sink_name)
    }

    pub fn is_virtual_cable(&self) -> bool {
        self.kind == StripKind::VirtualCable
    }

    pub fn is_mixer_strip(&self) -> bool {
        self.kind == StripKind::Strip
    }

    pub fn is_bus(&self) -> bool {
        self.kind == StripKind::Bus
    }

    pub fn role(&self) -> StripRole {
        match self.kind {
            StripKind::HardwareSource => StripRole::HardwareSource,
            StripKind::VirtualCable => StripRole::VirtualCable,
            StripKind::Strip => StripRole::ChannelStrip,
            StripKind::Bus => StripRole::Bus,
            StripKind::Output => {
                if self.is_managed_output() {
                    StripRole::OutputBus
                } else {
                    StripRole::SystemOutput
                }
            }
        }
    }

    pub fn role_label(&self) -> &'static str {
        self.role().label()
    }

    fn fx_midi_feedback_value(&self, target: FxMidiTarget) -> u8 {
        match target {
            FxMidiTarget::Bypass => midi_bool_value(self.effects.bypassed),
            FxMidiTarget::GateEnabled => midi_bool_value(self.effects.gate.enabled),
            FxMidiTarget::GateThreshold => percent_to_midi(self.effects.gate.threshold_percent),
            FxMidiTarget::GateFloor => percent_to_midi(self.effects.gate.floor_percent),
            FxMidiTarget::CompressorEnabled => midi_bool_value(self.effects.compressor.enabled),
            FxMidiTarget::CompressorThreshold => {
                percent_to_midi(self.effects.compressor.threshold_percent)
            }
            FxMidiTarget::CompressorRatio => ratio_to_midi(self.effects.compressor.ratio),
            FxMidiTarget::CompressorMakeupGain => {
                makeup_gain_to_midi(self.effects.compressor.makeup_gain_db)
            }
            FxMidiTarget::EqEnabled => midi_bool_value(self.effects.eq.enabled),
            FxMidiTarget::EqLowGain => eq_gain_to_midi(self.effects.eq.low_gain_db),
            FxMidiTarget::EqMidGain => eq_gain_to_midi(self.effects.eq.mid_gain_db),
            FxMidiTarget::EqHighGain => eq_gain_to_midi(self.effects.eq.high_gain_db),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct PersistedState {
    version: u32,
    next_strip_id: u32,
    #[serde(default)]
    midi_feedback: MidiFeedbackConfig,
    #[serde(default)]
    source_strips: Vec<PersistedStrip>,
    #[serde(default)]
    input_strips: Vec<PersistedStrip>,
    #[serde(default)]
    bus_strips: Vec<PersistedStrip>,
    #[serde(default)]
    output_strips: Vec<PersistedStrip>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct PersistedStrip {
    id: u32,
    kind: StripKind,
    label: String,
    #[serde(default)]
    pipewire_node_name: Option<String>,
    volume: f32,
    #[serde(default = "default_channel_count")]
    channel_count: usize,
    #[serde(default = "default_mono_state")]
    mono: bool,
    muted: bool,
    midi: MidiBinding,
    #[serde(default)]
    fx_midi: FxMidiBinding,
    #[serde(default)]
    input_assignment: Option<InputAssignment>,
    #[serde(default)]
    routes: Vec<RouteState>,
    #[serde(default)]
    effects: StripEffects,
}

impl PersistedState {
    fn from_runtime(state: &AudioEngineState) -> Self {
        Self {
            version: CONFIG_VERSION,
            next_strip_id: state.next_strip_id,
            midi_feedback: state.midi_feedback.clone(),
            source_strips: state
                .source_strips
                .iter()
                .filter(|strip| strip.kind == StripKind::VirtualCable)
                .cloned()
                .map(PersistedStrip::from_runtime)
                .collect(),
            input_strips: state
                .input_strips
                .iter()
                .cloned()
                .map(PersistedStrip::from_runtime)
                .collect(),
            bus_strips: state
                .bus_strips
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

        let source_ids = self
            .source_strips
            .iter()
            .map(|strip| StripId::new(strip.id))
            .collect::<Vec<_>>();
        let bus_ids = self
            .bus_strips
            .iter()
            .map(|strip| StripId::new(strip.id))
            .collect::<Vec<_>>();
        let output_ids = self
            .output_strips
            .iter()
            .map(|strip| StripId::new(strip.id))
            .collect::<Vec<_>>();

        let source_strips = self
            .source_strips
            .into_iter()
            .map(|strip| strip.into_runtime_source())
            .collect::<Result<Vec<_>, _>>()?;

        let output_strips = self
            .output_strips
            .into_iter()
            .map(|strip| strip.into_runtime_output())
            .collect::<Result<Vec<_>, _>>()?;

        let input_strips = self
            .input_strips
            .into_iter()
            .map(|strip| strip.into_runtime_input(&source_ids, &bus_ids))
            .collect::<Result<Vec<_>, _>>()?;

        let bus_strips = self
            .bus_strips
            .into_iter()
            .map(|strip| strip.into_runtime_bus(&output_ids))
            .collect::<Result<Vec<_>, _>>()?;

        let max_strip_id = source_strips
            .iter()
            .chain(input_strips.iter())
            .chain(bus_strips.iter())
            .chain(output_strips.iter())
            .map(|strip| strip.id.as_u32())
            .max()
            .map(|value| value + 1)
            .unwrap_or(0);

        Ok(AudioEngineState {
            source_strips,
            input_strips,
            bus_strips,
            output_strips,
            inventory: BackendInventory::default(),
            live_meter_levels: std::collections::HashMap::new(),
            midi_feedback: self.midi_feedback,
            midi_learn_target: None,
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
            pipewire_node_name: strip.pipewire_node_name,
            volume: strip.volume.as_ratio(),
            channel_count: strip.channel_count,
            mono: strip.mono,
            muted: strip.muted,
            midi: strip.midi,
            fx_midi: strip.fx_midi,
            input_assignment: strip.input_assignment,
            routes: strip.routes,
            effects: strip.effects,
        }
    }

    fn into_runtime_source(self) -> Result<MixerStrip, String> {
        if self.kind != StripKind::VirtualCable {
            return Err(format!(
                "persisted source {} must use the virtual cable kind",
                self.id
            ));
        }
        if self.input_assignment.is_some() {
            return Err(format!(
                "persisted source {} cannot contain an input assignment",
                self.id
            ));
        }
        if !self.routes.is_empty() {
            return Err(format!(
                "persisted source {} cannot contain routes",
                self.id
            ));
        }

        self.into_runtime_strip()
    }

    fn into_runtime_input(
        self,
        source_ids: &[StripId],
        bus_ids: &[StripId],
    ) -> Result<MixerStrip, String> {
        if self.kind != StripKind::Strip {
            return Err(format!("input strip {} must use strip kind", self.id));
        }

        let valid_sources = source_ids
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        if self.input_assignment.as_ref().is_some_and(|assignment| {
            !valid_sources.contains(&assignment.source_id) && assignment.source_key.is_none()
        }) {
            return Err(format!(
                "input strip {} references an assigned source that does not exist",
                self.id
            ));
        }

        let valid_targets = bus_ids
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        let strip = self.into_runtime_strip()?;
        if strip
            .routes
            .iter()
            .any(|route| !valid_targets.contains(&route.output_id))
        {
            return Err(format!(
                "input strip {} references a route target that does not exist",
                strip.id.as_u32()
            ));
        }
        Ok(strip)
    }

    fn into_runtime_bus(self, output_ids: &[StripId]) -> Result<MixerStrip, String> {
        if self.kind != StripKind::Bus {
            return Err(format!("bus strip {} must use bus kind", self.id));
        }
        if self.input_assignment.is_some() {
            return Err(format!(
                "bus strip {} cannot contain an input assignment",
                self.id
            ));
        }

        let valid_targets = output_ids
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        let strip = self.into_runtime_strip()?;
        if strip
            .routes
            .iter()
            .any(|route| !valid_targets.contains(&route.output_id))
        {
            return Err(format!(
                "bus strip {} references an output target that does not exist",
                strip.id.as_u32()
            ));
        }
        Ok(strip)
    }

    fn into_runtime_output(self) -> Result<MixerStrip, String> {
        if self.kind != StripKind::Output {
            return Err(format!("output strip {} must use output kind", self.id));
        }

        if self.input_assignment.is_some() {
            return Err(format!(
                "output strip {} cannot contain an input assignment",
                self.id
            ));
        }

        if !self.routes.is_empty() {
            return Err(format!("output strip {} cannot contain routes", self.id));
        }

        self.into_runtime_strip()
    }

    fn into_runtime_strip(self) -> Result<MixerStrip, String> {
        let id = StripId::new(self.id);
        let mut strip = MixerStrip::new(id, self.kind, normalize_label(&self.label, self.kind, id));
        strip.pipewire_node_name = self
            .pipewire_node_name
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        strip.volume = NormalizedVolume::new(self.volume)
            .map_err(|error| format!("invalid saved volume for strip {}: {error}", self.id))?;
        strip.channel_count = self.channel_count.max(1);
        strip.mono = self.mono;
        strip.meter_channels = silent_meter_channels(strip.active_channel_count());
        strip.muted = self.muted;
        strip.midi = self.midi;
        strip.fx_midi = self.fx_midi;
        strip.input_assignment = self.input_assignment;
        strip.routes = self.routes;
        strip.effects = self.effects;
        Ok(strip)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PipeWireNodeInfo {
    pub id: u32,
    pub node_name: String,
    pub name: String,
    pub media_class: Option<String>,
}

impl PipeWireNodeInfo {
    fn is_audio_sink(&self) -> bool {
        self.media_class
            .as_deref()
            .is_some_and(|value| value.starts_with("Audio/Sink"))
    }

    fn is_managed_virtual_cable(&self) -> bool {
        is_managed_virtual_cable_name(&self.node_name)
    }

    fn is_managed_strip_sink(&self) -> bool {
        is_managed_strip_sink_name(&self.node_name)
    }

    fn is_managed_bus_sink(&self) -> bool {
        is_managed_bus_sink_name(&self.node_name)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MidiPortInfo {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BackendInventory {
    pub pipewire_status: String,
    pub pipewire_nodes: Vec<PipeWireNodeInfo>,
    pub application_stream_status: String,
    pub application_streams: Vec<ApplicationStreamInfo>,
    pub midi_status: String,
    pub midi_inputs: Vec<MidiPortInfo>,
    pub midi_outputs: Vec<MidiPortInfo>,
    pub midi_feedback_status: String,
    pub midi_feedback_debug: Vec<String>,
}

impl Default for BackendInventory {
    fn default() -> Self {
        Self {
            pipewire_status: "Waiting for first PipeWire scan".to_string(),
            pipewire_nodes: Vec::new(),
            application_stream_status: "Waiting for first application stream scan".to_string(),
            application_streams: Vec::new(),
            midi_status: "Waiting for first MIDI scan".to_string(),
            midi_inputs: Vec::new(),
            midi_outputs: Vec::new(),
            midi_feedback_status: "MIDI feedback disabled".to_string(),
            midi_feedback_debug: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioEngineState {
    pub source_strips: Vec<MixerStrip>,
    pub input_strips: Vec<MixerStrip>,
    pub bus_strips: Vec<MixerStrip>,
    pub output_strips: Vec<MixerStrip>,
    pub inventory: BackendInventory,
    pub live_meter_levels: std::collections::HashMap<String, Vec<f32>>,
    pub midi_feedback: MidiFeedbackConfig,
    pub midi_learn_target: Option<MidiLearnTarget>,
    pub next_strip_id: u32,
    pub last_notice: String,
}

#[derive(Debug, Default)]
struct MidiApplyResult {
    affected: usize,
    strip_ids: std::collections::HashSet<StripId>,
    routes_changed: bool,
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

        Self {
            source_strips: Vec::new(),
            input_strips: Vec::new(),
            bus_strips: Vec::new(),
            output_strips,
            inventory: BackendInventory::default(),
            live_meter_levels: std::collections::HashMap::new(),
            midi_feedback: MidiFeedbackConfig::default(),
            midi_learn_target: None,
            next_strip_id,
            last_notice: "Booting audio engine".to_string(),
        }
    }
}

impl AudioEngineState {
    pub fn total_strip_count(&self) -> usize {
        self.source_strips.len()
            + self.input_strips.len()
            + self.bus_strips.len()
            + self.output_strips.len()
    }

    pub fn active_route_count(&self) -> usize {
        self.input_strips
            .iter()
            .chain(self.bus_strips.iter())
            .flat_map(|strip| strip.routes.iter())
            .filter(|route| route.enabled)
            .count()
    }

    pub fn muted_strip_count(&self) -> usize {
        self.input_strips
            .iter()
            .chain(self.bus_strips.iter())
            .chain(self.output_strips.iter())
            .filter(|strip| strip.muted)
            .count()
    }

    pub fn active_effect_count(&self) -> usize {
        self.input_strips
            .iter()
            .chain(self.bus_strips.iter())
            .chain(self.output_strips.iter())
            .map(|strip| strip.effects.active_effect_count())
            .sum()
    }

    pub fn output_name(&self, output_id: StripId) -> Option<&str> {
        self.output_strips
            .iter()
            .find(|strip| strip.id == output_id)
            .map(|strip| strip.label.as_str())
    }

    pub fn source_name(&self, source_id: StripId) -> Option<&str> {
        self.source_strips
            .iter()
            .find(|strip| strip.id == source_id)
            .map(|strip| strip.label.as_str())
    }

    pub fn bus_name(&self, bus_id: StripId) -> Option<&str> {
        self.bus_strips
            .iter()
            .find(|strip| strip.id == bus_id)
            .map(|strip| strip.label.as_str())
    }

    pub fn route_target_name(&self, strip_kind: StripKind, target_id: StripId) -> Option<&str> {
        match strip_kind {
            StripKind::Strip => self.bus_name(target_id),
            StripKind::Bus => self.output_name(target_id),
            StripKind::HardwareSource | StripKind::VirtualCable | StripKind::Output => None,
        }
    }

    pub fn assignment_name(&self, assignment: Option<&InputAssignment>) -> Option<&str> {
        assignment.and_then(|assignment| self.source_name(assignment.source_id))
    }

    fn strip_label(&self, strip_id: StripId) -> Option<&str> {
        self.source_strips
            .iter()
            .chain(self.input_strips.iter())
            .chain(self.bus_strips.iter())
            .chain(self.output_strips.iter())
            .find(|strip| strip.id == strip_id)
            .map(|strip| strip.label.as_str())
    }

    fn strip_mut(&mut self, strip_id: StripId) -> Option<&mut MixerStrip> {
        if let Some(strip) = self
            .source_strips
            .iter_mut()
            .find(|candidate| candidate.id == strip_id)
        {
            return Some(strip);
        }
        if let Some(strip) = self
            .input_strips
            .iter_mut()
            .find(|candidate| candidate.id == strip_id)
        {
            return Some(strip);
        }
        if let Some(strip) = self
            .bus_strips
            .iter_mut()
            .find(|candidate| candidate.id == strip_id)
        {
            return Some(strip);
        }

        self.output_strips
            .iter_mut()
            .find(|candidate| candidate.id == strip_id)
    }

    fn effects_mut(&mut self, strip_id: StripId) -> Option<&mut StripEffects> {
        self.strip_mut(strip_id).and_then(|strip| {
            if strip.kind.supports_volume_and_mute() && strip.kind != StripKind::VirtualCable {
                Some(&mut strip.effects)
            } else {
                None
            }
        })
    }

    fn apply_volume(&mut self, strip_id: StripId, volume: NormalizedVolume) {
        if let Some(target) = self.strip_mut(strip_id) {
            if target.kind.supports_volume_and_mute() {
                target.volume = volume;
            } else {
                self.last_notice = format!("{} does not support volume control", target.label);
            }
        } else {
            self.last_notice = format!("Tried to update missing strip {}", strip_id.as_u32());
        }
    }

    fn rename_strip(&mut self, strip_id: StripId, label: &str) {
        if let Some(target) = self.strip_mut(strip_id) {
            if target.kind == StripKind::HardwareSource {
                self.last_notice = format!(
                    "{} is discovered from PipeWire and cannot be renamed",
                    target.label
                );
            } else {
                target.label = normalize_label(label, target.kind, target.id);
            }
        } else {
            self.last_notice = format!("Tried to rename missing strip {}", strip_id.as_u32());
        }
    }

    fn set_strip_input_assignment(&mut self, strip_id: StripId, source_id: Option<StripId>) {
        let source_key = source_id.and_then(|id| {
            self.source_strips
                .iter()
                .find(|candidate| candidate.id == id)
                .and_then(|candidate| candidate.pipewire_node_name.clone())
        });
        if let Some(target) = self
            .input_strips
            .iter_mut()
            .find(|candidate| candidate.id == strip_id)
        {
            target.input_assignment = source_id.map(|source_id| InputAssignment {
                source_id,
                source_key,
            });
        } else {
            self.last_notice = format!(
                "Tried to assign missing source to strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn sink_display_name(&self, sink_name: &str) -> String {
        if let Some(source) = self
            .source_strips
            .iter()
            .find(|strip| strip.pipewire_node_name.as_deref() == Some(sink_name))
        {
            return format!("{} ({})", source.label, source.role_label());
        }
        if let Some(output) = self
            .output_strips
            .iter()
            .find(|strip| strip.pipewire_node_name.as_deref() == Some(sink_name))
        {
            return format!("{} ({})", output.label, output.role_label());
        }
        if let Some(node) = self
            .inventory
            .pipewire_nodes
            .iter()
            .find(|node| node.node_name == sink_name)
        {
            return node.name.clone();
        }

        sink_name.to_string()
    }

    fn toggle_route(&mut self, strip_id: StripId, output_id: StripId) {
        if let Some(target) = self
            .input_strips
            .iter_mut()
            .chain(self.bus_strips.iter_mut())
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
                    "Tried to toggle missing route {} on {}",
                    output_id.as_u32(),
                    strip_id.as_u32()
                );
            }
        } else {
            self.last_notice = format!(
                "Tried to toggle route on non-routable strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn toggle_mute(&mut self, strip_id: StripId) {
        if let Some(target) = self.strip_mut(strip_id) {
            if target.kind.supports_volume_and_mute() {
                target.muted = !target.muted;
            } else {
                self.last_notice = format!("{} cannot be muted directly", target.label);
            }
        } else {
            self.last_notice = format!("Tried to mute missing strip {}", strip_id.as_u32());
        }
    }

    fn toggle_mono(&mut self, strip_id: StripId) {
        if let Some(target) = self.strip_mut(strip_id) {
            if target.kind.supports_mono() {
                target.mono = !target.mono;
                target.meter_channels = silent_meter_channels(target.active_channel_count());
            } else {
                self.last_notice = format!("{} does not expose mono mode", target.label);
            }
        } else {
            self.last_notice = format!("Tried to mono missing strip {}", strip_id.as_u32());
        }
    }

    #[cfg(test)]
    fn add_virtual_cable(&mut self, label: &str) -> MixerStrip {
        self.add_virtual_cable_with_node_name(label, None)
    }

    fn add_virtual_cable_with_node_name(
        &mut self,
        label: &str,
        pipewire_node_name: Option<String>,
    ) -> MixerStrip {
        let id = StripId::new(self.next_strip_id);
        self.next_strip_id += 1;

        let mut strip = MixerStrip::new(
            id,
            StripKind::VirtualCable,
            normalize_label(label, StripKind::VirtualCable, id),
        );
        strip.pipewire_node_name = pipewire_node_name;
        self.source_strips.push(strip.clone());
        strip
    }

    #[cfg(test)]
    fn add_mixer_strip(&mut self, label: &str) -> MixerStrip {
        self.add_mixer_strip_with_node_name(label, None)
    }

    fn add_mixer_strip_with_node_name(
        &mut self,
        label: &str,
        pipewire_node_name: Option<String>,
    ) -> MixerStrip {
        let id = StripId::new(self.next_strip_id);
        self.next_strip_id += 1;

        let mut strip = MixerStrip::new(
            id,
            StripKind::Strip,
            normalize_label(label, StripKind::Strip, id),
        );
        strip.pipewire_node_name = pipewire_node_name;
        strip.routes = self
            .bus_strips
            .iter()
            .enumerate()
            .map(|(index, bus)| RouteState {
                output_id: bus.id,
                enabled: default_route_enabled(strip.kind, self.input_strips.len(), index),
                midi_binding: None,
                midi_cc: None,
                output_key: bus.pipewire_node_name.clone(),
            })
            .collect();
        self.input_strips.push(strip.clone());
        strip
    }

    #[cfg(test)]
    fn add_bus(&mut self, label: &str) -> MixerStrip {
        self.add_bus_with_node_name(label, None)
    }

    fn add_bus_with_node_name(
        &mut self,
        label: &str,
        pipewire_node_name: Option<String>,
    ) -> MixerStrip {
        let id = StripId::new(self.next_strip_id);
        self.next_strip_id += 1;

        let mut bus = MixerStrip::new(
            id,
            StripKind::Bus,
            normalize_label(label, StripKind::Bus, id),
        );
        bus.pipewire_node_name = pipewire_node_name;
        bus.routes = self
            .output_strips
            .iter()
            .enumerate()
            .map(|(index, output)| RouteState {
                output_id: output.id,
                enabled: default_route_enabled(bus.kind, self.bus_strips.len(), index),
                midi_binding: None,
                midi_cc: None,
                output_key: output.pipewire_node_name.clone(),
            })
            .collect();
        self.bus_strips.push(bus.clone());

        for strip in &mut self.input_strips {
            strip.routes.push(RouteState {
                output_id: bus.id,
                enabled: false,
                midi_binding: None,
                midi_cc: None,
                output_key: bus.pipewire_node_name.clone(),
            });
        }

        bus
    }

    #[cfg(test)]
    fn add_output_sink(&mut self, label: &str) -> MixerStrip {
        self.add_output_sink_with_node_name(label, None)
    }

    fn add_output_sink_with_node_name(
        &mut self,
        label: &str,
        pipewire_node_name: Option<String>,
    ) -> MixerStrip {
        let id = StripId::new(self.next_strip_id);
        self.next_strip_id += 1;

        let mut output = MixerStrip::new(
            id,
            StripKind::Output,
            normalize_label(label, StripKind::Output, id),
        );
        output.pipewire_node_name = pipewire_node_name;

        for bus in &mut self.bus_strips {
            bus.routes.push(RouteState {
                output_id: output.id,
                enabled: false,
                midi_binding: None,
                midi_cc: None,
                output_key: output.pipewire_node_name.clone(),
            });
        }

        self.output_strips.push(output.clone());
        output
    }

    fn configure_strip(
        &mut self,
        strip_id: StripId,
        source_id: Option<StripId>,
        buses: &[StripId],
    ) {
        self.set_strip_input_assignment(strip_id, source_id);
        let buses = buses
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        if let Some(strip) = self
            .input_strips
            .iter_mut()
            .find(|candidate| candidate.id == strip_id)
        {
            for route in &mut strip.routes {
                route.enabled = buses.contains(&route.output_id);
            }
        }
    }

    fn remove_strip(&mut self, strip_id: StripId) -> Option<MixerStrip> {
        if let Some(index) = self
            .source_strips
            .iter()
            .position(|strip| strip.id == strip_id)
        {
            let removed = self.source_strips.remove(index);
            for strip in &mut self.input_strips {
                if strip
                    .input_assignment
                    .as_ref()
                    .is_some_and(|assignment| assignment.source_id == strip_id)
                {
                    strip.input_assignment = None;
                }
            }
            return Some(removed);
        }
        if let Some(index) = self
            .input_strips
            .iter()
            .position(|strip| strip.id == strip_id)
        {
            return Some(self.input_strips.remove(index));
        }
        if let Some(index) = self
            .bus_strips
            .iter()
            .position(|strip| strip.id == strip_id)
        {
            let removed = self.bus_strips.remove(index);
            for strip in &mut self.input_strips {
                strip.routes.retain(|route| route.output_id != strip_id);
            }
            return Some(removed);
        }
        if let Some(index) = self
            .output_strips
            .iter()
            .position(|strip| strip.id == strip_id)
        {
            let removed = self.output_strips.remove(index);
            for bus in &mut self.bus_strips {
                bus.routes.retain(|route| route.output_id != strip_id);
            }
            return Some(removed);
        }

        None
    }

    fn set_midi_binding(
        &mut self,
        strip_id: StripId,
        target: MidiControlTarget,
        binding: Option<MidiTrigger>,
    ) {
        if let Some(strip) = self.strip_mut(strip_id) {
            match target {
                MidiControlTarget::Volume => {
                    strip.midi.volume = binding;
                    strip.midi.volume_cc = None;
                }
                MidiControlTarget::Mute => {
                    strip.midi.mute = binding;
                    strip.midi.mute_cc = None;
                }
            }
        } else {
            self.last_notice = format!(
                "Tried to assign MIDI binding to missing strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn set_fx_midi_binding(
        &mut self,
        strip_id: StripId,
        target: FxMidiTarget,
        binding: Option<MidiTrigger>,
    ) {
        if let Some(strip) = self.strip_mut(strip_id) {
            strip.fx_midi.set_binding(target, binding);
        } else {
            self.last_notice = format!(
                "Tried to assign FX MIDI binding to missing strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn set_route_midi_binding(
        &mut self,
        strip_id: StripId,
        output_id: StripId,
        binding: Option<MidiTrigger>,
    ) {
        if let Some(strip) = self
            .input_strips
            .iter_mut()
            .chain(self.bus_strips.iter_mut())
            .find(|candidate| candidate.id == strip_id)
        {
            if let Some(route) = strip
                .routes
                .iter_mut()
                .find(|route| route.output_id == output_id)
            {
                route.midi_binding = binding;
                route.midi_cc = None;
            } else {
                self.last_notice = format!(
                    "Tried to assign MIDI binding to missing route {} on {}",
                    output_id.as_u32(),
                    strip_id.as_u32()
                );
            }
        } else {
            self.last_notice = format!(
                "Tried to assign MIDI binding to missing routable strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn set_midi_feedback_output(&mut self, output_port_name: Option<String>) {
        self.midi_feedback.output_port_name = output_port_name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());
    }

    fn start_midi_learn(&mut self, target: MidiLearnTarget) {
        self.midi_learn_target = Some(target);
    }

    fn clear_midi_learn(&mut self) {
        self.midi_learn_target = None;
    }

    fn reset_mixer(&mut self) {
        let hardware_sources = self
            .source_strips
            .iter()
            .filter(|strip| strip.kind == StripKind::HardwareSource)
            .cloned()
            .collect::<Vec<_>>();
        let next_preserved_id = hardware_sources
            .iter()
            .map(|strip| strip.id.as_u32())
            .max()
            .map(|value| value + 1)
            .unwrap_or(0);
        let inventory = self.inventory.clone();
        let midi_feedback = self.midi_feedback.clone();
        *self = Self::default();
        self.source_strips = hardware_sources;
        self.next_strip_id = self.next_strip_id.max(next_preserved_id);
        self.inventory = inventory;
        self.midi_feedback = midi_feedback;
    }

    fn toggle_effects_bypass(&mut self, strip_id: StripId) {
        if let Some(effects) = self.effects_mut(strip_id) {
            effects.bypassed = !effects.bypassed;
        } else {
            self.last_notice = format!(
                "Tried to change effects bypass on missing strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn reset_strip_effects(&mut self, strip_id: StripId) {
        if let Some(effects) = self.effects_mut(strip_id) {
            *effects = StripEffects::default();
        } else {
            self.last_notice = format!(
                "Tried to reset effects on missing strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn set_gate_enabled(&mut self, strip_id: StripId, enabled: bool) {
        if let Some(effects) = self.effects_mut(strip_id) {
            effects.gate.enabled = enabled;
        } else {
            self.last_notice = format!(
                "Tried to update gate on missing strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn set_gate_threshold(&mut self, strip_id: StripId, threshold_percent: f32) {
        if let Some(effects) = self.effects_mut(strip_id) {
            effects.gate.threshold_percent = clamp_percent(threshold_percent);
        } else {
            self.last_notice = format!(
                "Tried to update gate threshold on missing strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn set_gate_floor(&mut self, strip_id: StripId, floor_percent: f32) {
        if let Some(effects) = self.effects_mut(strip_id) {
            effects.gate.floor_percent = clamp_percent(floor_percent);
        } else {
            self.last_notice = format!(
                "Tried to update gate floor on missing strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn set_compressor_enabled(&mut self, strip_id: StripId, enabled: bool) {
        if let Some(effects) = self.effects_mut(strip_id) {
            effects.compressor.enabled = enabled;
        } else {
            self.last_notice = format!(
                "Tried to update compressor on missing strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn set_compressor_threshold(&mut self, strip_id: StripId, threshold_percent: f32) {
        if let Some(effects) = self.effects_mut(strip_id) {
            effects.compressor.threshold_percent = clamp_percent(threshold_percent);
        } else {
            self.last_notice = format!(
                "Tried to update compressor threshold on missing strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn set_compressor_ratio(&mut self, strip_id: StripId, ratio: f32) {
        if let Some(effects) = self.effects_mut(strip_id) {
            effects.compressor.ratio = clamp_ratio(ratio);
        } else {
            self.last_notice = format!(
                "Tried to update compressor ratio on missing strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn set_compressor_makeup_gain(&mut self, strip_id: StripId, gain_db: f32) {
        if let Some(effects) = self.effects_mut(strip_id) {
            effects.compressor.makeup_gain_db = clamp_makeup_gain_db(gain_db);
        } else {
            self.last_notice = format!(
                "Tried to update compressor gain on missing strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn set_eq_enabled(&mut self, strip_id: StripId, enabled: bool) {
        if let Some(effects) = self.effects_mut(strip_id) {
            effects.eq.enabled = enabled;
        } else {
            self.last_notice = format!("Tried to update EQ on missing strip {}", strip_id.as_u32());
        }
    }

    fn set_eq_low_gain(&mut self, strip_id: StripId, gain_db: f32) {
        if let Some(effects) = self.effects_mut(strip_id) {
            effects.eq.low_gain_db = clamp_eq_gain_db(gain_db);
        } else {
            self.last_notice = format!(
                "Tried to update low EQ on missing strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn set_eq_mid_gain(&mut self, strip_id: StripId, gain_db: f32) {
        if let Some(effects) = self.effects_mut(strip_id) {
            effects.eq.mid_gain_db = clamp_eq_gain_db(gain_db);
        } else {
            self.last_notice = format!(
                "Tried to update mid EQ on missing strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn set_eq_high_gain(&mut self, strip_id: StripId, gain_db: f32) {
        if let Some(effects) = self.effects_mut(strip_id) {
            effects.eq.high_gain_db = clamp_eq_gain_db(gain_db);
        } else {
            self.last_notice = format!(
                "Tried to update high EQ on missing strip {}",
                strip_id.as_u32()
            );
        }
    }

    fn apply_midi_event(&mut self, event: &MidiEvent) -> MidiApplyResult {
        let mut result = MidiApplyResult::default();

        for strip in self
            .input_strips
            .iter_mut()
            .chain(self.bus_strips.iter_mut())
            .chain(self.output_strips.iter_mut())
        {
            if strip
                .midi
                .volume_binding()
                .is_some_and(|binding| binding.matches(event))
            {
                strip.volume = NormalizedVolume::from_midi_value(event.value);
                result.affected += 1;
                result.strip_ids.insert(strip.id);
            }

            if strip
                .midi
                .mute_binding()
                .is_some_and(|binding| midi_boolean_press(&binding, event))
            {
                strip.muted = !strip.muted;
                result.affected += 1;
                result.strip_ids.insert(strip.id);
            }

            for target in [
                FxMidiTarget::Bypass,
                FxMidiTarget::GateEnabled,
                FxMidiTarget::GateThreshold,
                FxMidiTarget::GateFloor,
                FxMidiTarget::CompressorEnabled,
                FxMidiTarget::CompressorThreshold,
                FxMidiTarget::CompressorRatio,
                FxMidiTarget::CompressorMakeupGain,
                FxMidiTarget::EqEnabled,
                FxMidiTarget::EqLowGain,
                FxMidiTarget::EqMidGain,
                FxMidiTarget::EqHighGain,
            ] {
                let Some(binding) = strip.fx_midi.binding(target) else {
                    continue;
                };
                if target.requires_control_change() && event.kind != MidiMessageKind::ControlChange
                {
                    continue;
                }
                let matched = if target.requires_control_change() {
                    binding.matches(event)
                } else {
                    midi_boolean_press(&binding, event)
                };
                if !matched {
                    continue;
                }

                match target {
                    FxMidiTarget::Bypass => strip.effects.bypassed = !strip.effects.bypassed,
                    FxMidiTarget::GateEnabled => {
                        strip.effects.gate.enabled = !strip.effects.gate.enabled;
                    }
                    FxMidiTarget::GateThreshold => {
                        strip.effects.gate.threshold_percent =
                            clamp_percent(midi_to_percent(event.value));
                    }
                    FxMidiTarget::GateFloor => {
                        strip.effects.gate.floor_percent =
                            clamp_percent(midi_to_percent(event.value));
                    }
                    FxMidiTarget::CompressorEnabled => {
                        strip.effects.compressor.enabled = !strip.effects.compressor.enabled;
                    }
                    FxMidiTarget::CompressorThreshold => {
                        strip.effects.compressor.threshold_percent =
                            clamp_percent(midi_to_percent(event.value));
                    }
                    FxMidiTarget::CompressorRatio => {
                        strip.effects.compressor.ratio = clamp_ratio(midi_to_ratio(event.value));
                    }
                    FxMidiTarget::CompressorMakeupGain => {
                        strip.effects.compressor.makeup_gain_db =
                            clamp_makeup_gain_db(midi_to_makeup_gain(event.value));
                    }
                    FxMidiTarget::EqEnabled => strip.effects.eq.enabled = !strip.effects.eq.enabled,
                    FxMidiTarget::EqLowGain => {
                        strip.effects.eq.low_gain_db =
                            clamp_eq_gain_db(midi_to_eq_gain(event.value));
                    }
                    FxMidiTarget::EqMidGain => {
                        strip.effects.eq.mid_gain_db =
                            clamp_eq_gain_db(midi_to_eq_gain(event.value));
                    }
                    FxMidiTarget::EqHighGain => {
                        strip.effects.eq.high_gain_db =
                            clamp_eq_gain_db(midi_to_eq_gain(event.value));
                    }
                }

                result.affected += 1;
                result.strip_ids.insert(strip.id);
            }
        }

        for strip in self
            .input_strips
            .iter_mut()
            .chain(self.bus_strips.iter_mut())
        {
            for route in &mut strip.routes {
                if route
                    .binding()
                    .is_some_and(|binding| midi_boolean_press(&binding, event))
                {
                    route.enabled = !route.enabled;
                    result.affected += 1;
                    result.routes_changed = true;
                }
            }
        }

        result
    }

    pub(crate) fn update_vu_meters(&mut self, phase: u64) {
        for strip in &mut self.source_strips {
            let live_levels = strip
                .pipewire_node_name
                .as_ref()
                .and_then(|node_name| self.live_meter_levels.get(node_name))
                .map(|levels| project_channel_levels(levels, strip.channel_count.max(1)));

            let channel_levels = if let Some(levels) = live_levels {
                levels
                    .into_iter()
                    .map(|level| {
                        NormalizedVolume::new(level.clamp(0.0, 1.0))
                            .expect("live source meter level should be valid")
                    })
                    .collect::<Vec<_>>()
            } else {
                (0..strip.channel_count.max(1))
                    .map(|channel| {
                        NormalizedVolume::new(simulated_input_activity(strip.id, channel, phase))
                            .expect("simulated source meter level should be valid")
                    })
                    .collect::<Vec<_>>()
            };
            strip.meter_level = peak_meter_level(&channel_levels);
            strip.meter_channels = channel_levels;
        }

        let source_levels = self
            .source_strips
            .iter()
            .map(|strip| {
                (
                    strip.id,
                    strip
                        .meter_channels
                        .iter()
                        .map(|level| level.as_ratio())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();

        for strip in &mut self.input_strips {
            let live_levels = strip
                .pipewire_node_name
                .as_ref()
                .and_then(|node_name| self.live_meter_levels.get(node_name))
                .map(|levels| project_channel_levels(levels, strip.channel_count.max(1)));

            let channel_levels = if let Some(levels) = live_levels {
                let levels = levels
                    .into_iter()
                    .map(|level| {
                        NormalizedVolume::new(level.clamp(0.0, 1.0))
                            .expect("live input meter level should be valid")
                    })
                    .collect::<Vec<_>>();
                if strip.mono {
                    vec![average_meter_level(&levels)]
                } else {
                    levels
                }
            } else {
                let raw_channel_levels = if let Some(assignment) = strip.input_assignment.as_ref() {
                    project_channel_levels(
                        source_levels
                            .get(&assignment.source_id)
                            .map(Vec::as_slice)
                            .unwrap_or(&[]),
                        strip.channel_count.max(1),
                    )
                    .into_iter()
                    .map(|level| {
                        let level = if strip.muted {
                            0.0
                        } else {
                            (strip.volume.as_ratio() * level).clamp(0.0, 1.0)
                        };
                        NormalizedVolume::new(level)
                            .expect("assigned strip meter level should be valid")
                    })
                    .collect::<Vec<_>>()
                } else {
                    (0..strip.channel_count.max(1))
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
                        .collect::<Vec<_>>()
                };
                let processed_levels =
                    apply_strip_effects_to_levels(raw_channel_levels, &strip.effects);
                if strip.mono {
                    vec![average_meter_level(&processed_levels)]
                } else {
                    processed_levels
                }
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

        for bus in &mut self.bus_strips {
            let channel_levels = if let Some(levels) = bus
                .pipewire_node_name
                .as_ref()
                .and_then(|node_name| self.live_meter_levels.get(node_name))
            {
                let levels = project_channel_levels(levels, bus.active_channel_count())
                    .into_iter()
                    .map(|level| {
                        NormalizedVolume::new(level.clamp(0.0, 1.0))
                            .expect("live bus meter level should be valid")
                    })
                    .collect::<Vec<_>>();
                if bus.mono {
                    vec![average_meter_level(&levels)]
                } else {
                    levels
                }
            } else {
                let mut channel_levels = vec![0.0_f32; bus.active_channel_count()];
                for (_, levels) in &input_levels {
                    for (bus_id, level_pair) in levels {
                        if *bus_id != bus.id {
                            continue;
                        }

                        let projected_levels =
                            project_channel_levels(level_pair, channel_levels.len());
                        for (index, level) in projected_levels.iter().enumerate() {
                            channel_levels[index] = channel_levels[index].max(*level);
                        }
                    }
                }

                let channel_levels = channel_levels
                    .into_iter()
                    .map(|level| {
                        let level = if bus.muted {
                            0.0
                        } else {
                            (level * bus.volume.as_ratio()).clamp(0.0, 1.0)
                        };
                        NormalizedVolume::new(level)
                            .expect("simulated bus meter level should be valid")
                    })
                    .collect::<Vec<_>>();
                apply_strip_effects_to_levels(channel_levels, &bus.effects)
            };
            bus.meter_level = peak_meter_level(&channel_levels);
            bus.meter_channels = channel_levels;
        }

        let bus_levels = self
            .bus_strips
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
            let channel_levels = if let Some(levels) = output
                .pipewire_node_name
                .as_ref()
                .and_then(|node_name| self.live_meter_levels.get(node_name))
            {
                project_channel_levels(levels, output.active_channel_count())
                    .into_iter()
                    .map(|level| {
                        NormalizedVolume::new(level.clamp(0.0, 1.0))
                            .expect("live output meter level should be valid")
                    })
                    .collect::<Vec<_>>()
            } else {
                let mut channel_levels = vec![0.0_f32; output.active_channel_count()];
                for (_, levels) in &bus_levels {
                    for (output_id, level_pair) in levels {
                        if *output_id != output.id {
                            continue;
                        }

                        let projected_levels =
                            project_channel_levels(level_pair, channel_levels.len());
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
                apply_strip_effects_to_levels(channel_levels, &output.effects)
            };
            output.meter_level = peak_meter_level(&channel_levels);
            output.meter_channels = channel_levels;
        }
    }
}

fn default_virtual_cable_name(label: &str, state: &AudioEngineState) -> String {
    let existing_names = state
        .source_strips
        .iter()
        .filter_map(|strip| strip.pipewire_node_name.clone())
        .collect::<std::collections::HashSet<_>>();
    let base = format!("{PIPEMEETER_VIRTUAL_CABLE_PREFIX}{}", sink_name_slug(label));
    if !existing_names.contains(&base) {
        return base;
    }

    let mut suffix = 2_u32;
    loop {
        let candidate = format!("{base}-{suffix}");
        if !existing_names.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

fn default_strip_sink_name(label: &str, state: &AudioEngineState) -> String {
    let existing_names = state
        .input_strips
        .iter()
        .filter_map(|strip| strip.pipewire_node_name.clone())
        .collect::<std::collections::HashSet<_>>();
    let base = format!("{PIPEMEETER_STRIP_SINK_PREFIX}{}", sink_name_slug(label));
    if !existing_names.contains(&base) {
        return base;
    }

    let mut suffix = 2_u32;
    loop {
        let candidate = format!("{base}-{suffix}");
        if !existing_names.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

fn default_bus_sink_name(label: &str, state: &AudioEngineState) -> String {
    let existing_names = state
        .bus_strips
        .iter()
        .filter_map(|strip| strip.pipewire_node_name.clone())
        .collect::<std::collections::HashSet<_>>();
    let base = format!("{PIPEMEETER_BUS_SINK_PREFIX}{}", sink_name_slug(label));
    if !existing_names.contains(&base) {
        return base;
    }

    let mut suffix = 2_u32;
    loop {
        let candidate = format!("{base}-{suffix}");
        if !existing_names.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

fn default_output_sink_name(label: &str, state: &AudioEngineState) -> String {
    let existing_names = state
        .output_strips
        .iter()
        .filter_map(|strip| strip.pipewire_node_name.clone())
        .collect::<std::collections::HashSet<_>>();
    let base = format!("{PIPEMEETER_OUTPUT_SINK_PREFIX}{}", sink_name_slug(label));
    if !existing_names.contains(&base) {
        return base;
    }

    let mut suffix = 2_u32;
    loop {
        let candidate = format!("{base}-{suffix}");
        if !existing_names.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

fn apply_pipewire_state_for_strip(state: &mut AudioEngineState, strip_id: StripId) {
    let Some(strip) = state
        .source_strips
        .iter()
        .chain(state.input_strips.iter())
        .chain(state.bus_strips.iter())
        .chain(state.output_strips.iter())
        .find(|strip| strip.id == strip_id)
    else {
        return;
    };

    let Some(node_name) = strip.pipewire_node_name.clone() else {
        return;
    };

    if let Err(error) = sync_pipewire_strip_state(strip.kind, &node_name, strip.volume, strip.muted)
    {
        state.last_notice = format!("{}; PipeWire sync failed: {error}", state.last_notice);
    }
}

fn apply_pipewire_state_for_all_strips(state: &mut AudioEngineState) {
    let strip_ids = state
        .source_strips
        .iter()
        .chain(state.input_strips.iter())
        .chain(state.bus_strips.iter())
        .chain(state.output_strips.iter())
        .filter(|strip| strip.kind != StripKind::Output || strip.is_managed_output())
        .map(|strip| strip.id)
        .collect::<Vec<_>>();
    for strip_id in strip_ids {
        apply_pipewire_state_for_strip(state, strip_id);
    }
}

fn ensure_virtual_cables_exist(state: &mut AudioEngineState) {
    let strip_ids = state
        .source_strips
        .iter()
        .filter(|strip| strip.kind == StripKind::VirtualCable)
        .map(|strip| strip.id)
        .collect::<Vec<_>>();

    for strip_id in strip_ids {
        let Some(index) = state
            .source_strips
            .iter()
            .position(|strip| strip.id == strip_id)
        else {
            continue;
        };
        if state.source_strips[index].pipewire_node_name.is_none() {
            let generated = default_virtual_cable_name(&state.source_strips[index].label, state);
            state.source_strips[index].pipewire_node_name = Some(generated);
        }

        let label = state.source_strips[index].label.clone();
        let node_name = state.source_strips[index]
            .pipewire_node_name
            .clone()
            .expect("virtual cable name should be assigned");
        if let Err(error) = create_pipewire_sink(&node_name, &label) {
            state.last_notice = format!("Failed to ensure virtual cable {label}: {error}");
            continue;
        }

        let strip_id = state.source_strips[index].id;
        apply_pipewire_state_for_strip(state, strip_id);
    }
}

fn ensure_strip_sinks_exist(state: &mut AudioEngineState) {
    let strip_ids = state
        .input_strips
        .iter()
        .map(|strip| strip.id)
        .collect::<Vec<_>>();

    for strip_id in strip_ids {
        let Some(index) = state
            .input_strips
            .iter()
            .position(|strip| strip.id == strip_id)
        else {
            continue;
        };
        if state.input_strips[index].pipewire_node_name.is_none() {
            let generated = default_strip_sink_name(&state.input_strips[index].label, state);
            state.input_strips[index].pipewire_node_name = Some(generated);
        }

        let label = state.input_strips[index].label.clone();
        let node_name = state.input_strips[index]
            .pipewire_node_name
            .clone()
            .expect("strip sink name should be assigned");
        if let Err(error) = create_pipewire_sink(&node_name, &label) {
            state.last_notice = format!("Failed to ensure strip {label}: {error}");
            continue;
        }

        let strip_id = state.input_strips[index].id;
        apply_pipewire_state_for_strip(state, strip_id);
    }
}

fn ensure_bus_sinks_exist(state: &mut AudioEngineState) {
    let strip_ids = state
        .bus_strips
        .iter()
        .map(|strip| strip.id)
        .collect::<Vec<_>>();

    for strip_id in strip_ids {
        let Some(index) = state
            .bus_strips
            .iter()
            .position(|strip| strip.id == strip_id)
        else {
            continue;
        };
        if state.bus_strips[index].pipewire_node_name.is_none() {
            let generated = default_bus_sink_name(&state.bus_strips[index].label, state);
            state.bus_strips[index].pipewire_node_name = Some(generated);
        }

        let label = state.bus_strips[index].label.clone();
        let node_name = state.bus_strips[index]
            .pipewire_node_name
            .clone()
            .expect("bus sink name should be assigned");
        if let Err(error) = create_pipewire_sink(&node_name, &label) {
            state.last_notice = format!("Failed to ensure bus {label}: {error}");
            continue;
        }

        let strip_id = state.bus_strips[index].id;
        apply_pipewire_state_for_strip(state, strip_id);
    }
}

fn ensure_output_sinks_exist(state: &mut AudioEngineState) {
    let strip_ids = state
        .output_strips
        .iter()
        .filter(|strip| strip.is_managed_output())
        .map(|strip| strip.id)
        .collect::<Vec<_>>();

    for strip_id in strip_ids {
        let Some(index) = state
            .output_strips
            .iter()
            .position(|strip| strip.id == strip_id)
        else {
            continue;
        };

        let label = state.output_strips[index].label.clone();
        let Some(node_name) = state.output_strips[index].pipewire_node_name.clone() else {
            continue;
        };

        if let Err(error) = create_pipewire_sink(&node_name, &label) {
            state.last_notice = format!("Failed to ensure output {label}: {error}");
            continue;
        }

        let strip_id = state.output_strips[index].id;
        apply_pipewire_state_for_strip(state, strip_id);
    }
}

fn clamp_percent(value: f32) -> f32 {
    value.clamp(0.0, 100.0)
}

fn clamp_ratio(value: f32) -> f32 {
    value.clamp(1.0, 20.0)
}

fn clamp_makeup_gain_db(value: f32) -> f32 {
    value.clamp(0.0, 24.0)
}

fn clamp_eq_gain_db(value: f32) -> f32 {
    value.clamp(-12.0, 12.0)
}

fn midi_bool_value(value: bool) -> u8 {
    if value {
        MIDI_FEEDBACK_ON_VALUE
    } else {
        MIDI_FEEDBACK_OFF_VALUE
    }
}

fn percent_to_midi(value: f32) -> u8 {
    ((clamp_percent(value) / 100.0) * 127.0).round() as u8
}

fn midi_to_percent(value: u8) -> f32 {
    (value as f32 / 127.0) * 100.0
}

fn ratio_to_midi(value: f32) -> u8 {
    (((clamp_ratio(value) - 1.0) / 19.0) * 127.0).round() as u8
}

fn midi_to_ratio(value: u8) -> f32 {
    1.0 + ((value as f32 / 127.0) * 19.0)
}

fn makeup_gain_to_midi(value: f32) -> u8 {
    ((clamp_makeup_gain_db(value) / 24.0) * 127.0).round() as u8
}

fn midi_to_makeup_gain(value: u8) -> f32 {
    (value as f32 / 127.0) * 24.0
}

fn eq_gain_to_midi(value: f32) -> u8 {
    (((clamp_eq_gain_db(value) + 12.0) / 24.0) * 127.0).round() as u8
}

fn midi_to_eq_gain(value: u8) -> f32 {
    ((value as f32 / 127.0) * 24.0) - 12.0
}

fn db_to_gain(db: f32) -> f32 {
    10_f32.powf(db / 20.0)
}

fn apply_strip_effects_to_levels(
    levels: Vec<NormalizedVolume>,
    effects: &StripEffects,
) -> Vec<NormalizedVolume> {
    if effects.bypassed {
        return levels;
    }

    levels
        .into_iter()
        .enumerate()
        .map(|(index, level)| {
            let mut ratio = level.as_ratio();

            if effects.gate.enabled && ratio < effects.gate.threshold_percent / 100.0 {
                ratio *= effects.gate.floor_percent / 100.0;
            }

            if effects.eq.enabled {
                let band_db = match index % 3 {
                    0 => effects.eq.low_gain_db,
                    1 => effects.eq.high_gain_db,
                    _ => effects.eq.mid_gain_db,
                };
                let blended_db = (band_db + effects.eq.mid_gain_db) / 2.0;
                ratio *= db_to_gain(blended_db);
            }

            if effects.compressor.enabled {
                let threshold = effects.compressor.threshold_percent / 100.0;
                let ratio_value = clamp_ratio(effects.compressor.ratio);
                if ratio > threshold {
                    ratio = threshold + (ratio - threshold) / ratio_value;
                }
                ratio *= db_to_gain(effects.compressor.makeup_gain_db);
            }

            NormalizedVolume::new(ratio.clamp(0.0, 1.0))
                .expect("effect-processed meter level should be valid")
        })
        .collect()
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
    ToggleEffectsBypass {
        strip: StripId,
    },
    ResetStripEffects {
        strip: StripId,
    },
    SetNoiseGateEnabled {
        strip: StripId,
        enabled: bool,
    },
    SetNoiseGateThreshold {
        strip: StripId,
        threshold_percent: f32,
    },
    SetNoiseGateFloor {
        strip: StripId,
        floor_percent: f32,
    },
    SetCompressorEnabled {
        strip: StripId,
        enabled: bool,
    },
    SetCompressorThreshold {
        strip: StripId,
        threshold_percent: f32,
    },
    SetCompressorRatio {
        strip: StripId,
        ratio: f32,
    },
    SetCompressorMakeupGain {
        strip: StripId,
        gain_db: f32,
    },
    SetEqEnabled {
        strip: StripId,
        enabled: bool,
    },
    SetEqLowGain {
        strip: StripId,
        gain_db: f32,
    },
    SetEqMidGain {
        strip: StripId,
        gain_db: f32,
    },
    SetEqHighGain {
        strip: StripId,
        gain_db: f32,
    },
    RemoveStrip {
        strip: StripId,
    },
    AddVirtualCable {
        label: String,
    },
    CreateStrip {
        label: String,
        input_source: Option<StripId>,
        buses: Vec<StripId>,
    },
    AddBus {
        label: String,
    },
    AddOutput {
        label: String,
    },
    SetStripInputAssignment {
        strip: StripId,
        source: Option<StripId>,
    },
    ResetMixer,
    SetMidiBinding {
        strip: StripId,
        target: MidiControlTarget,
        binding: Option<MidiTrigger>,
    },
    SetFxMidiBinding {
        strip: StripId,
        target: FxMidiTarget,
        binding: Option<MidiTrigger>,
    },
    SetRouteMidiBinding {
        strip: StripId,
        output: StripId,
        binding: Option<MidiTrigger>,
    },
    StartMidiLearn {
        target: MidiLearnTarget,
    },
    CancelMidiLearn,
    SetMidiFeedbackOutput {
        port_name: Option<String>,
    },
    SyncMidiFeedback,
    ApplyMidiEvent {
        event: MidiEvent,
    },
    MoveApplicationStream {
        stream: ApplicationStreamIdentity,
        sink_name: String,
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
        let worker_control_tx = control_tx.clone();

        let worker = thread::Builder::new()
            .name("pipemeeter-audio-engine".to_string())
            .spawn(move || engine_loop(worker_control_tx, control_rx, updates_tx))
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

fn engine_loop(
    control_tx: Sender<AudioControlMsg>,
    control_rx: Receiver<AudioControlMsg>,
    updates_tx: Sender<AudioUpdateMsg>,
) {
    let mut state = load_initial_state();
    let mut midi_feedback = MidiFeedbackRuntime::default();
    let mut midi_input = MidiInputRuntime::default();
    let mut meter_runtime = MeterRuntime::default();
    let mut meter_phase = 0_u64;
    let mut needs_persist = false;
    let mut last_state_change_at = Instant::now();
    let mut last_topology_refresh_at = Instant::now();
    ensure_virtual_cables_exist(&mut state);
    ensure_strip_sinks_exist(&mut state);
    ensure_bus_sinks_exist(&mut state);
    ensure_output_sinks_exist(&mut state);
    refresh_inventory(&mut state, false);
    apply_pipewire_state_for_all_strips(&mut state);
    sync_pipewire_routes(&mut state);
    midi_input.sync_connections(&mut state, &control_tx);
    meter_runtime.sync_taps(&mut state);
    midi_feedback.sync_connection(&mut state);
    midi_feedback.send_snapshot(&mut state);
    state.live_meter_levels = meter_runtime.snapshot_levels();
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
                apply_pipewire_state_for_strip(&mut state, strip);
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.send_snapshot(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::RenameStrip { strip, label }) => {
                state.rename_strip(strip, &label);
                state.last_notice =
                    format!("Renamed {}", state.strip_label(strip).unwrap_or("strip"));
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.send_snapshot(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::ToggleRoute { strip, output }) => {
                let target_kind = state
                    .input_strips
                    .iter()
                    .chain(state.bus_strips.iter())
                    .find(|candidate| candidate.id == strip)
                    .map(|candidate| candidate.kind)
                    .unwrap_or(StripKind::Strip);
                let output_label = state
                    .route_target_name(target_kind, output)
                    .unwrap_or("route target")
                    .to_string();
                state.toggle_route(strip, output);
                state.last_notice = format!(
                    "Toggled {} on {}",
                    output_label,
                    state.strip_label(strip).unwrap_or("strip")
                );
                sync_pipewire_routes(&mut state);
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.send_snapshot(&mut state);
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
                apply_pipewire_state_for_strip(&mut state, strip);
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.send_snapshot(&mut state);
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
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.send_snapshot(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::ToggleEffectsBypass { strip }) => {
                state.toggle_effects_bypass(strip);
                let bypass_state = state
                    .strip_mut(strip)
                    .map(|candidate| {
                        if candidate.effects.bypassed {
                            "effects bypassed"
                        } else {
                            "effects engaged"
                        }
                    })
                    .unwrap_or("effects updated");
                state.last_notice = format!(
                    "{} {}",
                    state.strip_label(strip).unwrap_or("Strip"),
                    bypass_state
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::ResetStripEffects { strip }) => {
                state.reset_strip_effects(strip);
                state.last_notice = format!(
                    "Reset effects on {}",
                    state.strip_label(strip).unwrap_or("strip")
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetNoiseGateEnabled { strip, enabled }) => {
                state.set_gate_enabled(strip, enabled);
                state.last_notice = format!(
                    "{} gate {}",
                    state.strip_label(strip).unwrap_or("Strip"),
                    if enabled { "enabled" } else { "disabled" }
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetNoiseGateThreshold {
                strip,
                threshold_percent,
            }) => {
                state.set_gate_threshold(strip, threshold_percent);
                state.last_notice = format!(
                    "{} gate threshold {}%",
                    state.strip_label(strip).unwrap_or("Strip"),
                    threshold_percent.round()
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetNoiseGateFloor {
                strip,
                floor_percent,
            }) => {
                state.set_gate_floor(strip, floor_percent);
                state.last_notice = format!(
                    "{} gate floor {}%",
                    state.strip_label(strip).unwrap_or("Strip"),
                    floor_percent.round()
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetCompressorEnabled { strip, enabled }) => {
                state.set_compressor_enabled(strip, enabled);
                state.last_notice = format!(
                    "{} compressor {}",
                    state.strip_label(strip).unwrap_or("Strip"),
                    if enabled { "enabled" } else { "disabled" }
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetCompressorThreshold {
                strip,
                threshold_percent,
            }) => {
                state.set_compressor_threshold(strip, threshold_percent);
                state.last_notice = format!(
                    "{} compressor threshold {}%",
                    state.strip_label(strip).unwrap_or("Strip"),
                    threshold_percent.round()
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetCompressorRatio { strip, ratio }) => {
                state.set_compressor_ratio(strip, ratio);
                state.last_notice = format!(
                    "{} compressor ratio {:.1}:1",
                    state.strip_label(strip).unwrap_or("Strip"),
                    ratio
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetCompressorMakeupGain { strip, gain_db }) => {
                state.set_compressor_makeup_gain(strip, gain_db);
                state.last_notice = format!(
                    "{} compressor makeup {:.1} dB",
                    state.strip_label(strip).unwrap_or("Strip"),
                    gain_db
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetEqEnabled { strip, enabled }) => {
                state.set_eq_enabled(strip, enabled);
                state.last_notice = format!(
                    "{} EQ {}",
                    state.strip_label(strip).unwrap_or("Strip"),
                    if enabled { "enabled" } else { "disabled" }
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetEqLowGain { strip, gain_db }) => {
                state.set_eq_low_gain(strip, gain_db);
                state.last_notice = format!(
                    "{} low EQ {:.1} dB",
                    state.strip_label(strip).unwrap_or("Strip"),
                    gain_db
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetEqMidGain { strip, gain_db }) => {
                state.set_eq_mid_gain(strip, gain_db);
                state.last_notice = format!(
                    "{} mid EQ {:.1} dB",
                    state.strip_label(strip).unwrap_or("Strip"),
                    gain_db
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetEqHighGain { strip, gain_db }) => {
                state.set_eq_high_gain(strip, gain_db);
                state.last_notice = format!(
                    "{} high EQ {:.1} dB",
                    state.strip_label(strip).unwrap_or("Strip"),
                    gain_db
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::RemoveStrip { strip }) => {
                let source_sink_name = state
                    .source_strips
                    .iter()
                    .find(|candidate| {
                        candidate.id == strip && candidate.kind == StripKind::VirtualCable
                    })
                    .and_then(|candidate| candidate.pipewire_node_name.clone());
                let strip_sink_name = state
                    .input_strips
                    .iter()
                    .find(|candidate| candidate.id == strip && candidate.kind == StripKind::Strip)
                    .and_then(|candidate| candidate.pipewire_node_name.clone());
                let bus_sink_name = state
                    .bus_strips
                    .iter()
                    .find(|candidate| candidate.id == strip && candidate.kind == StripKind::Bus)
                    .and_then(|candidate| candidate.pipewire_node_name.clone());
                let output_sink_name = state
                    .output_strips
                    .iter()
                    .find(|candidate| candidate.id == strip && candidate.is_managed_output())
                    .and_then(|candidate| candidate.pipewire_node_name.clone());
                if let Some(node_name) = source_sink_name
                    .or(strip_sink_name)
                    .or(bus_sink_name)
                    .or(output_sink_name)
                {
                    if let Err(error) = remove_pipewire_sink(&node_name) {
                        state.last_notice = error;
                        push_snapshot(&updates_tx, &state);
                        continue;
                    }
                }
                match state.remove_strip(strip) {
                    Some(removed) => {
                        state.last_notice = format!("Removed {}", removed.label);
                    }
                    None => {
                        state.last_notice =
                            format!("Tried to remove missing strip {}", strip.as_u32());
                    }
                }
                refresh_inventory(&mut state, false);
                sync_pipewire_routes(&mut state);
                midi_input.sync_connections(&mut state, &control_tx);
                meter_runtime.sync_taps(&mut state);
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.send_snapshot(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::AddVirtualCable { label }) => {
                let display_label = label.trim().to_string();
                let sink_name = default_virtual_cable_name(&display_label, &state);
                if let Err(error) = create_pipewire_sink(&sink_name, &display_label) {
                    state.last_notice = error;
                    push_snapshot(&updates_tx, &state);
                    continue;
                }
                let created =
                    state.add_virtual_cable_with_node_name(&display_label, Some(sink_name));
                apply_pipewire_state_for_strip(&mut state, created.id);
                refresh_inventory(&mut state, false);
                state.last_notice = format!("Added virtual cable {}", created.label);
                sync_pipewire_routes(&mut state);
                meter_runtime.sync_taps(&mut state);
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.send_snapshot(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::CreateStrip {
                label,
                input_source,
                buses,
            }) => {
                let display_label = label.trim().to_string();
                let sink_name = default_strip_sink_name(&display_label, &state);
                if let Err(error) = create_pipewire_sink(&sink_name, &display_label) {
                    state.last_notice = error;
                    push_snapshot(&updates_tx, &state);
                    continue;
                }
                let created = state.add_mixer_strip_with_node_name(&display_label, Some(sink_name));
                state.configure_strip(created.id, input_source, &buses);
                apply_pipewire_state_for_strip(&mut state, created.id);
                refresh_inventory(&mut state, false);
                state.last_notice = format!("Created strip {}", created.label);
                sync_pipewire_routes(&mut state);
                meter_runtime.sync_taps(&mut state);
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.send_snapshot(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::AddBus { label }) => {
                let display_label = label.trim().to_string();
                let sink_name = default_bus_sink_name(&display_label, &state);
                if let Err(error) = create_pipewire_sink(&sink_name, &display_label) {
                    state.last_notice = error;
                    push_snapshot(&updates_tx, &state);
                    continue;
                }
                let created = state.add_bus_with_node_name(&display_label, Some(sink_name));
                apply_pipewire_state_for_strip(&mut state, created.id);
                refresh_inventory(&mut state, false);
                state.last_notice = format!("Added bus {}", created.label);
                sync_pipewire_routes(&mut state);
                meter_runtime.sync_taps(&mut state);
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.send_snapshot(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::AddOutput { label }) => {
                let display_label = label.trim().to_string();
                let sink_name = default_output_sink_name(&display_label, &state);
                if let Err(error) = create_pipewire_sink(&sink_name, &display_label) {
                    state.last_notice = error;
                    push_snapshot(&updates_tx, &state);
                    continue;
                }
                let created = state.add_output_sink_with_node_name(&display_label, Some(sink_name));
                apply_pipewire_state_for_strip(&mut state, created.id);
                refresh_inventory(&mut state, false);
                state.last_notice = format!("Added new output {}", created.label);
                sync_pipewire_routes(&mut state);
                meter_runtime.sync_taps(&mut state);
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.send_snapshot(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetStripInputAssignment { strip, source }) => {
                state.set_strip_input_assignment(strip, source);
                state.last_notice = format!(
                    "{} input set to {}",
                    state.strip_label(strip).unwrap_or("Strip"),
                    source
                        .and_then(|source_id| state.source_name(source_id).map(str::to_string))
                        .unwrap_or_else(|| "none".to_string())
                );
                sync_pipewire_routes(&mut state);
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.send_snapshot(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::ResetMixer) => {
                let virtual_sink_names = state
                    .source_strips
                    .iter()
                    .filter(|strip| strip.kind == StripKind::VirtualCable)
                    .filter_map(|strip| strip.pipewire_node_name.clone())
                    .chain(
                        state
                            .input_strips
                            .iter()
                            .filter(|strip| strip.kind == StripKind::Strip)
                            .filter_map(|strip| strip.pipewire_node_name.clone()),
                    )
                    .chain(
                        state
                            .bus_strips
                            .iter()
                            .filter(|strip| strip.kind == StripKind::Bus)
                            .filter_map(|strip| strip.pipewire_node_name.clone()),
                    )
                    .chain(
                        state
                            .output_strips
                            .iter()
                            .filter(|strip| strip.is_managed_output())
                            .filter_map(|strip| strip.pipewire_node_name.clone()),
                    )
                    .collect::<Vec<_>>();
                for node_name in virtual_sink_names {
                    if let Err(error) = remove_pipewire_sink(&node_name) {
                        state.last_notice = error;
                        push_snapshot(&updates_tx, &state);
                        continue;
                    }
                }
                state.reset_mixer();
                state.last_notice =
                    "Reset sources, strips, buses, and outputs to defaults".to_string();
                refresh_inventory(&mut state, false);
                apply_pipewire_state_for_all_strips(&mut state);
                sync_pipewire_routes(&mut state);
                midi_input.sync_connections(&mut state, &control_tx);
                meter_runtime.sync_taps(&mut state);
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.sync_connection(&mut state);
                midi_feedback.send_snapshot(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetMidiBinding {
                strip,
                target,
                binding,
            }) => {
                state.clear_midi_learn();
                state.set_midi_binding(strip, target, binding.clone());
                let binding_label = MidiTrigger::format_midi_trigger(binding.as_ref());
                state.last_notice = format!(
                    "{} {} MIDI binding {}",
                    state.strip_label(strip).unwrap_or("Strip"),
                    match target {
                        MidiControlTarget::Volume => "volume",
                        MidiControlTarget::Mute => "mute",
                    },
                    binding_label
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.send_snapshot(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetFxMidiBinding {
                strip,
                target,
                binding,
            }) => {
                state.clear_midi_learn();
                state.set_fx_midi_binding(strip, target, binding.clone());
                let binding_label = MidiTrigger::format_midi_trigger(binding.as_ref());
                state.last_notice = format!(
                    "{} {} MIDI binding {}",
                    state.strip_label(strip).unwrap_or("Strip"),
                    target.label(),
                    binding_label
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.send_snapshot(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetRouteMidiBinding {
                strip,
                output,
                binding,
            }) => {
                state.clear_midi_learn();
                state.set_route_midi_binding(strip, output, binding.clone());
                let binding_label = MidiTrigger::format_midi_trigger(binding.as_ref());
                let target_kind = state
                    .input_strips
                    .iter()
                    .chain(state.bus_strips.iter())
                    .find(|candidate| candidate.id == strip)
                    .map(|candidate| candidate.kind)
                    .unwrap_or(StripKind::Strip);
                let output_label = state
                    .route_target_name(target_kind, output)
                    .unwrap_or("route target")
                    .to_string();
                state.last_notice = format!(
                    "{} route to {} MIDI binding {}",
                    state.strip_label(strip).unwrap_or("Strip"),
                    output_label,
                    binding_label
                );
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.send_snapshot(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::StartMidiLearn { target }) => {
                state.start_midi_learn(target);
                state.last_notice =
                    "Move a MIDI slider or press a controller button to learn it".to_string();
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::CancelMidiLearn) => {
                state.clear_midi_learn();
                state.last_notice = "Cancelled MIDI learn".to_string();
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SetMidiFeedbackOutput { port_name }) => {
                state.set_midi_feedback_output(port_name);
                state.last_notice = state
                    .midi_feedback
                    .output_port_name
                    .as_ref()
                    .map(|name| format!("Selected MIDI feedback output {name}"))
                    .unwrap_or_else(|| "Disabled MIDI feedback output".to_string());
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.sync_connection(&mut state);
                midi_feedback.send_snapshot(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::SyncMidiFeedback) => {
                midi_feedback.sync_connection(&mut state);
                midi_feedback.send_snapshot(&mut state);
                state.last_notice = state.inventory.midi_feedback_status.clone();
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::ApplyMidiEvent { event }) => {
                if let Some(target) = state.midi_learn_target {
                    let binding = MidiTrigger {
                        kind: event.kind,
                        number: event.number,
                        channel: Some(event.channel),
                    };
                    match target {
                        MidiLearnTarget::Strip { strip, target } => {
                            state.set_midi_binding(strip, target, Some(binding.clone()));
                            state.last_notice = format!(
                                "{} {} MIDI binding {}",
                                state.strip_label(strip).unwrap_or("Strip"),
                                match target {
                                    MidiControlTarget::Volume => "volume",
                                    MidiControlTarget::Mute => "mute",
                                },
                                MidiTrigger::format_midi_trigger(Some(&binding))
                            );
                        }
                        MidiLearnTarget::Fx { strip, target } => {
                            if target.requires_control_change()
                                && event.kind != MidiMessageKind::ControlChange
                            {
                                state.last_notice = format!(
                                    "{} {} learn expects a MIDI CC/knob",
                                    state.strip_label(strip).unwrap_or("Strip"),
                                    target.label()
                                );
                                push_snapshot(&updates_tx, &state);
                                continue;
                            }
                            state.set_fx_midi_binding(strip, target, Some(binding.clone()));
                            state.last_notice = format!(
                                "{} {} MIDI binding {}",
                                state.strip_label(strip).unwrap_or("Strip"),
                                target.label(),
                                MidiTrigger::format_midi_trigger(Some(&binding))
                            );
                        }
                        MidiLearnTarget::Route { strip, output } => {
                            state.set_route_midi_binding(strip, output, Some(binding.clone()));
                            let target_label = state
                                .route_target_name(
                                    state
                                        .input_strips
                                        .iter()
                                        .chain(state.bus_strips.iter())
                                        .find(|candidate| candidate.id == strip)
                                        .map(|candidate| candidate.kind)
                                        .unwrap_or(StripKind::Strip),
                                    output,
                                )
                                .unwrap_or("route target");
                            state.last_notice = format!(
                                "{} route to {} MIDI binding {}",
                                state.strip_label(strip).unwrap_or("Strip"),
                                target_label,
                                MidiTrigger::format_midi_trigger(Some(&binding))
                            );
                        }
                    }
                    state.clear_midi_learn();
                    mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                    midi_feedback.send_snapshot(&mut state);
                    push_snapshot(&updates_tx, &state);
                    continue;
                }

                let midi_result = state.apply_midi_event(&event);
                state.last_notice = if midi_result.affected == 0 {
                    format!(
                        "Received unmapped MIDI {} {}",
                        match event.kind {
                            MidiMessageKind::ControlChange => "CC",
                            MidiMessageKind::Note => "note",
                        },
                        event.number
                    )
                } else {
                    format!(
                        "Applied MIDI {} {} to {} target(s)",
                        match event.kind {
                            MidiMessageKind::ControlChange => "CC",
                            MidiMessageKind::Note => "note",
                        },
                        event.number,
                        midi_result.affected
                    )
                };
                if midi_result.affected > 0 {
                    for strip_id in midi_result.strip_ids {
                        apply_pipewire_state_for_strip(&mut state, strip_id);
                    }
                    if midi_result.routes_changed {
                        sync_pipewire_routes(&mut state);
                    }
                    mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                    midi_feedback.send_snapshot(&mut state);
                }
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::MoveApplicationStream { stream, sink_name }) => {
                match move_application_stream_to_sink(&stream, &sink_name) {
                    Ok(()) => {
                        refresh_inventory(&mut state, false);
                        state.last_notice = format!(
                            "Moved {} to {}",
                            stream.application_name,
                            state.sink_display_name(&sink_name)
                        );
                    }
                    Err(error) => {
                        state.last_notice = error;
                    }
                }
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::RefreshTopology) => {
                refresh_inventory(&mut state, true);
                apply_pipewire_state_for_all_strips(&mut state);
                sync_pipewire_routes(&mut state);
                midi_input.sync_connections(&mut state, &control_tx);
                meter_runtime.sync_taps(&mut state);
                mark_runtime_state_dirty(&mut needs_persist, &mut last_state_change_at);
                midi_feedback.sync_connection(&mut state);
                midi_feedback.send_snapshot(&mut state);
                push_snapshot(&updates_tx, &state);
            }
            Ok(AudioControlMsg::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                flush_runtime_state_persist(&mut state, &mut needs_persist);
                meter_runtime.stop_all();
                break;
            }
            Err(RecvTimeoutError::Timeout) => {
                if needs_persist && last_state_change_at.elapsed() >= STATE_SAVE_DEBOUNCE {
                    flush_runtime_state_persist(&mut state, &mut needs_persist);
                }
                if last_topology_refresh_at.elapsed() >= AUTO_TOPOLOGY_REFRESH_INTERVAL {
                    refresh_inventory(&mut state, false);
                    sync_pipewire_routes(&mut state);
                    midi_input.sync_connections(&mut state, &control_tx);
                    meter_runtime.sync_taps(&mut state);
                    midi_feedback.sync_connection(&mut state);
                    midi_feedback.send_snapshot(&mut state);
                    last_topology_refresh_at = Instant::now();
                }
                meter_phase = meter_phase.wrapping_add(1);
                state.live_meter_levels = meter_runtime.snapshot_levels();
                state.update_vu_meters(meter_phase);
                push_snapshot(&updates_tx, &state);
            }
        }

        fn mark_runtime_state_dirty(needs_persist: &mut bool, last_state_change_at: &mut Instant) {
            *needs_persist = true;
            *last_state_change_at = Instant::now();
        }

        fn flush_runtime_state_persist(state: &mut AudioEngineState, needs_persist: &mut bool) {
            if !*needs_persist {
                return;
            }

            persist_runtime_state(state);
            *needs_persist = false;
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
    let mut sinks_synced = false;
    let mut sources_synced = false;
    let mut synced_sink_count = 0;
    match scan_pipewire_nodes() {
        Ok(nodes) => {
            synced_sink_count = nodes
                .iter()
                .filter(|node| {
                    node.is_audio_sink()
                        && !node.is_managed_virtual_cable()
                        && !node.is_managed_strip_sink()
                        && !node.is_managed_bus_sink()
                })
                .count();
            state.inventory.pipewire_status = if nodes.is_empty() {
                "PipeWire connected, but no nodes were reported".to_string()
            } else {
                format!("PipeWire connected with {} nodes", nodes.len())
            };
            state.inventory.pipewire_nodes = nodes;
            sinks_synced = sync_output_sinks_to_inventory(state);
            sources_synced = sync_real_sources_to_inventory(state);
        }
        Err(error) => {
            state.inventory.pipewire_status = format!("PipeWire unavailable: {error}");
            state.inventory.pipewire_nodes.clear();
        }
    }

    match scan_midi_inputs() {
        Ok(inputs) => {
            state.inventory.midi_inputs = inputs;
        }
        Err(error) => {
            state.inventory.midi_status = format!("MIDI unavailable: {error}");
            state.inventory.midi_inputs.clear();
        }
    }

    match scan_midi_outputs() {
        Ok(outputs) => {
            state.inventory.midi_outputs = outputs;
            if !state.inventory.midi_status.starts_with("MIDI unavailable:") {
                state.inventory.midi_status = match (
                    state.inventory.midi_inputs.len(),
                    state.inventory.midi_outputs.len(),
                ) {
                    (0, 0) => "MIDI subsystem ready, but no input or output devices were found"
                        .to_string(),
                    (inputs, outputs) => {
                        format!(
                            "MIDI subsystem ready with {inputs} input(s) and {outputs} output(s)"
                        )
                    }
                };
            }
        }
        Err(error) => {
            state.inventory.midi_outputs.clear();
            if state.inventory.midi_status.starts_with("MIDI unavailable:") {
                state.inventory.midi_status = format!("MIDI unavailable: {error}");
            } else {
                state.inventory.midi_status = format!("MIDI output scan failed: {error}");
            }
        }
    }

    match scan_application_streams() {
        Ok(streams) => {
            state.inventory.application_stream_status = match streams.len() {
                0 => "No application playback streams are active right now".to_string(),
                count => format!("{count} application playback stream(s) are available"),
            };
            state.inventory.application_streams = streams
                .into_iter()
                .map(|stream| ApplicationStreamInfo {
                    current_sink_label: state.sink_display_name(&stream.current_sink_name),
                    identity: stream.identity,
                    current_sink_name: stream.current_sink_name,
                    icon_data_url: stream.icon_data_url,
                    corked: stream.corked,
                })
                .collect();
        }
        Err(error) => {
            state.inventory.application_stream_status =
                format!("Application routing unavailable: {error}");
            state.inventory.application_streams.clear();
        }
    }

    if update_notice {
        state.last_notice = if sinks_synced || sources_synced {
            format!(
                "Topology refreshed from {synced_sink_count} PipeWire sink(s) and {} real source(s)",
                state
                    .source_strips
                    .iter()
                    .filter(|strip| strip.kind == StripKind::HardwareSource)
                    .count()
            )
        } else {
            "Topology refreshed".to_string()
        };
    }
}

fn sync_real_sources_to_inventory(state: &mut AudioEngineState) -> bool {
    let previous_source_snapshot = state.source_strips.clone();
    let sources = match scan_pulse_sources() {
        Ok(sources) => sources,
        Err(error) => {
            state.last_notice = format!("{}; source sync failed: {error}", state.last_notice);
            return false;
        }
    };

    let previous_sources = state.source_strips.clone();
    let mut previous_hardware_sources = std::mem::take(&mut state.source_strips)
        .into_iter()
        .filter(|strip| strip.kind == StripKind::HardwareSource)
        .filter_map(|strip| strip.pipewire_node_name.clone().map(|name| (name, strip)))
        .collect::<std::collections::HashMap<_, _>>();
    let virtual_cables = previous_sources
        .into_iter()
        .filter(|strip| strip.kind == StripKind::VirtualCable)
        .collect::<Vec<_>>();

    let mut next_sources = Vec::with_capacity(sources.len() + virtual_cables.len());
    for source in sources {
        let mut strip = previous_hardware_sources
            .remove(&source.name)
            .unwrap_or_else(|| {
                let id = StripId::new(state.next_strip_id);
                state.next_strip_id += 1;
                MixerStrip::new(id, StripKind::HardwareSource, &source.description)
            });
        strip.kind = StripKind::HardwareSource;
        strip.label = source.description.clone();
        strip.pipewire_node_name = Some(source.name);
        strip.channel_count = source.channel_count.max(1);
        strip.meter_channels = silent_meter_channels(strip.active_channel_count());
        strip.input_assignment = None;
        strip.routes.clear();
        next_sources.push(strip);
    }
    next_sources.extend(virtual_cables);
    state.source_strips = next_sources;

    let source_lookup = state
        .source_strips
        .iter()
        .filter_map(|strip| {
            strip
                .pipewire_node_name
                .clone()
                .map(|name| (name, strip.id))
        })
        .collect::<std::collections::HashMap<_, _>>();
    for strip in &mut state.input_strips {
        if let Some(assignment) = strip.input_assignment.as_mut() {
            if let Some(source_key) = assignment.source_key.clone() {
                if let Some(source_id) = source_lookup.get(&source_key).copied() {
                    assignment.source_id = source_id;
                }
            }
        }
    }

    previous_source_snapshot != state.source_strips
}

fn sync_output_sinks_to_inventory(state: &mut AudioEngineState) -> bool {
    let sink_nodes = state
        .inventory
        .pipewire_nodes
        .iter()
        .filter(|node| {
            node.is_audio_sink()
                && !node.is_managed_virtual_cable()
                && !node.is_managed_strip_sink()
                && !node.is_managed_bus_sink()
        })
        .cloned()
        .collect::<Vec<_>>();
    if sink_nodes.is_empty() {
        return false;
    }

    let previous_outputs = state.output_strips.clone();
    let previous_routes = state
        .bus_strips
        .iter()
        .map(|strip| (strip.id, strip.routes.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    let previous_output_keys = state
        .output_strips
        .iter()
        .map(|strip| (strip.id, strip.output_match_key()))
        .collect::<std::collections::HashMap<_, _>>();
    let mut previous_outputs_by_key = std::mem::take(&mut state.output_strips)
        .into_iter()
        .map(|strip| (strip.output_match_key(), strip))
        .collect::<std::collections::HashMap<_, _>>();

    let mut next_outputs = Vec::with_capacity(sink_nodes.len());
    for node in sink_nodes {
        let mut output = previous_outputs_by_key
            .remove(&node.node_name)
            .unwrap_or_else(|| {
                let id = StripId::new(state.next_strip_id);
                state.next_strip_id += 1;
                MixerStrip::new(id, StripKind::Output, &node.name)
            });
        output.kind = StripKind::Output;
        output.label = node.name;
        output.pipewire_node_name = Some(node.node_name);
        output.routes.clear();
        next_outputs.push(output);
    }

    for (input_index, input) in state.bus_strips.iter_mut().enumerate() {
        if input.kind != StripKind::Bus {
            continue;
        }
        let previous_input_routes = input.routes.clone();
        let mut preserved_routes = previous_input_routes
            .into_iter()
            .filter_map(|route| {
                route
                    .output_key
                    .clone()
                    .or_else(|| {
                        previous_output_keys
                            .get(&route.output_id)
                            .cloned()
                            .filter(|value| !value.trim().is_empty())
                    })
                    .map(|key| (key, route))
            })
            .collect::<std::collections::HashMap<_, _>>();
        let had_preserved_routes = !preserved_routes.is_empty();

        input.routes = next_outputs
            .iter()
            .enumerate()
            .map(|(output_index, output)| {
                let output_key = output
                    .pipewire_node_name
                    .clone()
                    .expect("synced PipeWire outputs should have a node name");
                if let Some(mut route) = preserved_routes.remove(&output_key) {
                    route.output_id = output.id;
                    route.output_key = Some(output_key);
                    route
                } else {
                    RouteState {
                        output_id: output.id,
                        enabled: if had_preserved_routes {
                            false
                        } else {
                            default_route_enabled(input.kind, input_index, output_index)
                        },
                        midi_binding: None,
                        midi_cc: None,
                        output_key: Some(output_key),
                    }
                }
            })
            .collect();
    }

    state.output_strips = next_outputs;
    previous_outputs != state.output_strips
        || state
            .bus_strips
            .iter()
            .any(|strip| previous_routes.get(&strip.id) != Some(&strip.routes))
}

fn push_snapshot(updates_tx: &Sender<AudioUpdateMsg>, state: &AudioEngineState) {
    let _ = updates_tx.send(AudioUpdateMsg::Snapshot(state.clone()));
}

fn application_stream_matches_identity(
    stream: &PulseSinkInputInfo,
    identity: &ApplicationStreamIdentity,
) -> bool {
    stream.identity.application_name == identity.application_name
        && stream.identity.media_name == identity.media_name
        && stream.identity.process_binary == identity.process_binary
        && stream.identity.process_id == identity.process_id
}

fn resolve_application_stream_index(
    streams: &[PulseSinkInputInfo],
    identity: &ApplicationStreamIdentity,
) -> Result<u32, String> {
    if let Some(stream) = streams.iter().find(|stream| {
        stream.identity.cached_index == identity.cached_index
            && application_stream_matches_identity(stream, identity)
    }) {
        return Ok(stream.identity.cached_index);
    }

    let matches = streams
        .iter()
        .filter(|stream| application_stream_matches_identity(stream, identity))
        .map(|stream| stream.identity.cached_index)
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [index] => Ok(*index),
        [] => Err(format!(
            "{} is no longer active; refresh and try again",
            identity.application_name
        )),
        _ => Err(format!(
            "{} now has multiple matching streams; refresh and choose the specific stream again",
            identity.application_name
        )),
    }
}

fn move_application_stream_to_sink(
    identity: &ApplicationStreamIdentity,
    sink_name: &str,
) -> Result<(), String> {
    #[cfg(not(feature = "system-audio"))]
    {
        let _ = (identity, sink_name);
        return Err(
            "compiled without `system-audio`; enable it to route application streams".to_string(),
        );
    }

    #[cfg(feature = "system-audio")]
    {
        let streams = scan_application_streams()?;
        let index = resolve_application_stream_index(&streams, identity)?;
        let output = Command::new("pactl")
            .args(["move-sink-input", &index.to_string(), sink_name])
            .output()
            .map_err(|error| format!("failed to execute pactl move-sink-input: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "failed to move {} to {sink_name}: {}",
                identity.application_name,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }
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

        ports.retain(|port| {
            let name = port.name.trim();
            !name.starts_with("pipemeeter-") && !name.starts_with("Midi Through")
        });

        ports.extend(
            scan_rawmidi_ports()?
                .into_iter()
                .filter(|port| port.input)
                .map(|port| MidiPortInfo {
                    name: rawmidi_port_name(&port.device, &port.name),
                }),
        );

        ports.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(ports)
    }
}

struct RawMidiPortInfo {
    device: String,
    name: String,
    input: bool,
}

fn scan_rawmidi_ports() -> Result<Vec<RawMidiPortInfo>, String> {
    let output = Command::new("amidi")
        .arg("-l")
        .output()
        .map_err(|error| format!("failed to execute amidi -l: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "amidi -l failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .skip_while(|line| !line.starts_with("Dir "))
        .skip(1)
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let dir = parts.next()?.trim().to_string();
            let device = parts.next()?.trim().to_string();
            let name = parts.collect::<Vec<_>>().join(" ");
            if device.is_empty() || name.is_empty() {
                return None;
            }
            Some(RawMidiPortInfo {
                input: dir.contains('I'),
                device,
                name,
            })
        })
        .collect())
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
        let output = Command::new("pw-dump")
            .output()
            .map_err(|error| format!("failed to execute pw-dump: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "pw-dump failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        parse_pw_dump_nodes(&String::from_utf8_lossy(&output.stdout))
    }
}

fn parse_pw_dump_nodes(dump: &str) -> Result<Vec<PipeWireNodeInfo>, String> {
    let items = serde_json::from_str::<Vec<Value>>(dump)
        .map_err(|error| format!("failed to parse pw-dump JSON: {error}"))?;

    let mut nodes = items
        .iter()
        .filter_map(parse_pw_dump_node)
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    Ok(nodes)
}

fn parse_pw_dump_node(item: &Value) -> Option<PipeWireNodeInfo> {
    if item.get("type")?.as_str()? != "PipeWire:Interface:Node" {
        return None;
    }

    let id = item.get("id")?.as_u64()?.try_into().ok()?;
    let props = item
        .get("info")
        .and_then(|info| info.get("props"))
        .and_then(Value::as_object);
    let info = item.get("info");
    let prop = |key: &str| props.and_then(|props| props.get(key).and_then(pw_dump_prop_text));

    let name = prop("node.description")
        .or_else(|| prop("node.nick"))
        .or_else(|| prop("node.name"))
        .or_else(|| prop("application.name"))
        .or_else(|| {
            info.and_then(|info| info.get("name"))
                .and_then(pw_dump_prop_text)
        })
        .unwrap_or_else(|| "Unnamed PipeWire node".to_string());
    let node_name = prop("node.name")
        .or_else(|| prop("node.description"))
        .or_else(|| prop("application.name"))
        .unwrap_or_else(|| format!("pipewire-node-{id}"));
    let media_class = prop("media.class");

    Some(PipeWireNodeInfo {
        id,
        node_name,
        name,
        media_class,
    })
}

fn pw_dump_prop_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
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

    fn state_with_input() -> AudioEngineState {
        let mut state = AudioEngineState::default();
        let source = state.add_virtual_cable("Test Cable");
        let bus = state.add_bus("Main Bus");
        let strip = state.add_mixer_strip("Test Strip");
        state.configure_strip(strip.id, Some(source.id), &[bus.id]);
        state
    }

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
    fn adding_output_appends_new_route_targets_to_buses() {
        let mut state = AudioEngineState::default();
        state.add_bus("Main Bus");
        let route_counts = state
            .bus_strips
            .iter()
            .map(|strip| strip.routes.len())
            .collect::<Vec<_>>();

        let output = state.add_output_sink("Headphones");

        assert_eq!(output.kind, StripKind::Output);
        assert!(
            state
                .bus_strips
                .iter()
                .zip(route_counts)
                .all(|(strip, count)| strip.routes.len() == count + 1)
        );
    }

    #[test]
    fn adding_bus_uses_output_list_for_routes() {
        let mut state = AudioEngineState::default();

        let created = state.add_bus("Podcast Bus");

        assert_eq!(created.kind, StripKind::Bus);
        assert_eq!(created.label, "Podcast Bus");
        assert_eq!(created.routes.len(), state.output_strips.len());
    }

    #[test]
    fn configuring_strip_applies_selected_input_and_buses() {
        let mut state = AudioEngineState::default();
        let mic = {
            let id = StripId::new(state.next_strip_id);
            state.next_strip_id += 1;
            let mut strip = MixerStrip::new(id, StripKind::HardwareSource, "Mic");
            strip.pipewire_node_name = Some("alsa_input.mic".to_string());
            state.source_strips.push(strip.clone());
            strip
        };
        let chat = {
            let id = StripId::new(state.next_strip_id);
            state.next_strip_id += 1;
            let mut strip = MixerStrip::new(id, StripKind::HardwareSource, "Chat");
            strip.pipewire_node_name = Some("alsa_input.chat".to_string());
            state.source_strips.push(strip.clone());
            strip
        };
        let headphones = state.add_bus("Headphones");
        let created = state.add_mixer_strip("Podcast");

        state.configure_strip(created.id, Some(mic.id), &[headphones.id]);

        let created_strip = state
            .input_strips
            .iter()
            .find(|strip| strip.id == created.id)
            .expect("created strip should exist");
        let output_route = created_strip
            .routes
            .iter()
            .find(|route| route.output_id == headphones.id)
            .expect("bus route should exist");

        assert_eq!(
            created_strip
                .input_assignment
                .as_ref()
                .map(|assignment| assignment.source_id),
            Some(mic.id)
        );
        assert_ne!(
            created_strip
                .input_assignment
                .as_ref()
                .map(|assignment| assignment.source_id),
            Some(chat.id)
        );
        assert!(output_route.enabled);
    }

    #[test]
    fn toggling_route_updates_matrix_state() {
        let mut state = state_with_input();
        let strip = state.input_strips[0].id;
        let output = state.bus_strips[0].id;
        let before = state.input_strips[0].routes[0].enabled;

        state.toggle_route(strip, output);

        assert_ne!(before, state.input_strips[0].routes[0].enabled);
    }

    #[test]
    fn midi_cc_updates_volume_and_mute() {
        let mut state = state_with_input();
        let strip = state.input_strips[0].id;
        state.set_midi_binding(
            strip,
            MidiControlTarget::Volume,
            Some(MidiTrigger::control_change(12)),
        );
        state.set_midi_binding(
            strip,
            MidiControlTarget::Mute,
            Some(MidiTrigger::control_change(13)),
        );

        let affected_volume = state.apply_midi_event(&MidiEvent {
            kind: MidiMessageKind::ControlChange,
            channel: 0,
            number: 12,
            value: 64,
        });
        let affected_mute = state.apply_midi_event(&MidiEvent {
            kind: MidiMessageKind::ControlChange,
            channel: 0,
            number: 13,
            value: 127,
        });

        assert_eq!(affected_volume.affected, 1);
        assert_eq!(affected_mute.affected, 1);
        assert!(!affected_volume.routes_changed);
        assert!(!affected_mute.routes_changed);
        assert!((state.input_strips[0].volume.as_percentage() - 50.3937).abs() < 0.01);
        assert!(state.input_strips[0].muted);

        let release = state.apply_midi_event(&MidiEvent {
            kind: MidiMessageKind::ControlChange,
            channel: 0,
            number: 13,
            value: 0,
        });
        assert_eq!(release.affected, 0);
        assert!(state.input_strips[0].muted);

        let second_press = state.apply_midi_event(&MidiEvent {
            kind: MidiMessageKind::ControlChange,
            channel: 0,
            number: 13,
            value: 127,
        });
        assert_eq!(second_press.affected, 1);
        assert!(!state.input_strips[0].muted);
    }

    #[test]
    fn fx_midi_cc_updates_eq_gain_with_center_zero_mapping() {
        let mut state = state_with_input();
        let strip = state.input_strips[0].id;
        state.set_fx_midi_binding(
            strip,
            FxMidiTarget::EqLowGain,
            Some(MidiTrigger::control_change(30)),
        );

        let low = state.apply_midi_event(&MidiEvent {
            kind: MidiMessageKind::ControlChange,
            channel: 0,
            number: 30,
            value: 0,
        });
        assert_eq!(low.affected, 1);
        assert_eq!(state.input_strips[0].effects.eq.low_gain_db, -12.0);

        let center = state.apply_midi_event(&MidiEvent {
            kind: MidiMessageKind::ControlChange,
            channel: 0,
            number: 30,
            value: 64,
        });
        assert_eq!(center.affected, 1);
        assert!(state.input_strips[0].effects.eq.low_gain_db.abs() < 0.2);

        let high = state.apply_midi_event(&MidiEvent {
            kind: MidiMessageKind::ControlChange,
            channel: 0,
            number: 30,
            value: 127,
        });
        assert_eq!(high.affected, 1);
        assert_eq!(state.input_strips[0].effects.eq.low_gain_db, 12.0);
    }

    #[test]
    fn reset_fx_keeps_fx_midi_bindings() {
        let mut state = state_with_input();
        let strip = state.input_strips[0].id;
        state.set_fx_midi_binding(
            strip,
            FxMidiTarget::GateThreshold,
            Some(MidiTrigger::control_change(31)),
        );
        state.set_gate_enabled(strip, true);
        state.set_gate_threshold(strip, 37.0);

        state.reset_strip_effects(strip);

        assert_eq!(
            state.input_strips[0]
                .fx_midi
                .binding(FxMidiTarget::GateThreshold),
            Some(MidiTrigger::control_change(31))
        );
        assert!(!state.input_strips[0].effects.gate.enabled);
        assert_eq!(
            state.input_strips[0].effects.gate.threshold_percent,
            default_gate_threshold_percent()
        );
    }

    #[test]
    fn midi_feedback_messages_cover_strip_and_route_bindings() {
        let mut state = state_with_input();
        let input_id = state.input_strips[0].id;
        let output_id = state.bus_strips[0].id;

        state.apply_volume(input_id, NormalizedVolume::from_percent(25.0).unwrap());
        state.toggle_mute(input_id);
        state.set_midi_binding(
            input_id,
            MidiControlTarget::Volume,
            Some(MidiTrigger::control_change(14)),
        );
        state.set_midi_binding(
            input_id,
            MidiControlTarget::Mute,
            Some(MidiTrigger::control_change(15)),
        );
        state.set_route_midi_binding(input_id, output_id, Some(MidiTrigger::control_change(16)));

        let messages = collect_midi_feedback_messages(&state);

        assert!(messages.contains(&MidiFeedbackMessage {
            kind: MidiMessageKind::ControlChange,
            channel: 0,
            number: 14,
            value: 32,
        }));
        assert!(messages.contains(&MidiFeedbackMessage {
            kind: MidiMessageKind::ControlChange,
            channel: 0,
            number: 15,
            value: MIDI_FEEDBACK_ON_VALUE,
        }));
        assert!(messages.contains(&MidiFeedbackMessage {
            kind: MidiMessageKind::ControlChange,
            channel: 0,
            number: 16,
            value: MIDI_FEEDBACK_ON_VALUE,
        }));
        assert!(
            messages
                .iter()
                .all(|message| message.number != MIDI_FEEDBACK_CHANNEL_STATUS)
        );
    }

    #[test]
    fn midi_feedback_supports_note_bound_buttons() {
        let mut state = state_with_input();
        let input_id = state.input_strips[0].id;
        let output_id = state.bus_strips[0].id;

        state.toggle_mute(input_id);
        state.set_midi_binding(
            input_id,
            MidiControlTarget::Mute,
            Some(MidiTrigger {
                kind: MidiMessageKind::Note,
                number: 36,
                channel: Some(1),
            }),
        );
        state.set_route_midi_binding(
            input_id,
            output_id,
            Some(MidiTrigger {
                kind: MidiMessageKind::Note,
                number: 37,
                channel: Some(1),
            }),
        );

        let messages = collect_midi_feedback_messages(&state);

        assert!(messages.contains(&MidiFeedbackMessage {
            kind: MidiMessageKind::Note,
            channel: 1,
            number: 36,
            value: MIDI_FEEDBACK_ON_VALUE,
        }));
        assert!(messages.contains(&MidiFeedbackMessage {
            kind: MidiMessageKind::Note,
            channel: 1,
            number: 37,
            value: MIDI_FEEDBACK_ON_VALUE,
        }));
    }

    #[test]
    fn effects_shape_input_meter_levels() {
        let mut state = state_with_input();
        let strip_id = state.input_strips[0].id;
        state.input_strips[0].volume = NormalizedVolume::from_percent(20.0).unwrap();
        state.update_vu_meters(3);
        let baseline = state.input_strips[0].meter_level;

        state.set_gate_enabled(strip_id, true);
        state.set_gate_threshold(strip_id, 25.0);
        state.set_gate_floor(strip_id, 0.0);
        state.update_vu_meters(3);
        let gated = state.input_strips[0].meter_level;

        state.set_gate_enabled(strip_id, false);
        state.set_compressor_enabled(strip_id, true);
        state.set_compressor_threshold(strip_id, 5.0);
        state.set_compressor_ratio(strip_id, 10.0);
        state.set_compressor_makeup_gain(strip_id, 12.0);
        state.update_vu_meters(3);
        let compressed = state.input_strips[0].meter_level;

        assert!(gated.as_ratio() < baseline.as_ratio());
        assert!(compressed.as_ratio() > baseline.as_ratio());
    }

    #[test]
    fn vu_meters_follow_inputs_and_routes() {
        let mut state = state_with_input();
        let output_id = state.bus_strips[0].id;

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
        assert!(state.bus_strips[0].meter_level.as_ratio() > 0.0);
        assert_eq!(state.bus_strips[0].meter_channels.len(), 2);
        assert!(
            state.bus_strips[0]
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
        let mut state = state_with_input();
        let strip_id = state.input_strips[0].id;

        state.toggle_mono(strip_id);
        state.update_vu_meters(7);

        assert!(state.input_strips[0].mono);
        assert_eq!(state.input_strips[0].meter_channels.len(), 1);
        assert!(state.input_strips[0].meter_channels[0].as_ratio() >= 0.0);
        assert_eq!(state.bus_strips[0].meter_channels.len(), 2);
        assert_eq!(state.output_strips[0].meter_channels.len(), 2);
    }

    #[test]
    fn removing_output_prunes_routes_from_buses() {
        let mut state = AudioEngineState::default();
        state.add_bus("Main Bus");
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
        assert!(state.bus_strips.iter().all(|strip| {
            strip
                .routes
                .iter()
                .all(|route| route.output_id != removed_output)
        }));
    }

    #[test]
    fn removing_virtual_cable_clears_strip_assignment() {
        let mut state = state_with_input();
        let removed_input = state.source_strips[0].id;
        let original_output_count = state.output_strips.len();

        let removed = state
            .remove_strip(removed_input)
            .expect("input should exist");

        assert_eq!(removed.id, removed_input);
        assert!(
            state
                .source_strips
                .iter()
                .all(|strip| strip.id != removed_input)
        );
        assert!(state.input_strips[0].input_assignment.is_none());
        assert_eq!(state.output_strips.len(), original_output_count);
    }

    #[test]
    fn reset_mixer_restores_default_layout() {
        let mut state = AudioEngineState::default();
        let hardware_source = {
            let id = StripId::new(state.next_strip_id);
            state.next_strip_id += 1;
            let mut strip = MixerStrip::new(id, StripKind::HardwareSource, "Mic");
            strip.pipewire_node_name = Some("alsa_input.mic".to_string());
            state.source_strips.push(strip.clone());
            strip
        };
        state.set_midi_feedback_output(Some("MIDI Mix OUT".to_string()));
        state.add_virtual_cable("Podcast");
        state.add_bus("Main Bus");
        state.add_mixer_strip("Voice");
        state.add_output_sink("Headphones");
        state.toggle_mute(state.input_strips[0].id);
        state.set_eq_enabled(state.input_strips[0].id, true);

        state.reset_mixer();

        assert_eq!(state.source_strips.len(), 1);
        assert_eq!(state.source_strips[0].id, hardware_source.id);
        assert_eq!(state.source_strips[0].kind, StripKind::HardwareSource);
        assert!(state.input_strips.is_empty());
        assert!(state.bus_strips.is_empty());
        assert_eq!(state.output_strips.len(), DEFAULT_OUTPUTS.len());
        assert_eq!(
            state.midi_feedback.output_port_name.as_deref(),
            Some("MIDI Mix OUT")
        );
        assert!(
            state
                .source_strips
                .iter()
                .chain(state.input_strips.iter())
                .chain(state.bus_strips.iter())
                .chain(state.output_strips.iter())
                .all(|strip| !strip.muted)
        );
        assert!(
            state
                .input_strips
                .iter()
                .chain(state.bus_strips.iter())
                .chain(state.output_strips.iter())
                .all(|strip| strip.effects.active_effect_count() == 0 && !strip.effects.bypassed)
        );
    }

    #[test]
    fn persisted_state_round_trips_custom_mixer_config() {
        let mut state = state_with_input();
        let source_id = state.source_strips[0].id;
        let input_id = state.input_strips[0].id;
        let bus_id = state.bus_strips[0].id;
        let output_id = state.output_strips[0].id;

        state.rename_strip(input_id, "Game");
        state.apply_volume(input_id, NormalizedVolume::from_percent(63.0).unwrap());
        state.toggle_mute(output_id);
        state.set_midi_binding(
            input_id,
            MidiControlTarget::Volume,
            Some(MidiTrigger::control_change(21)),
        );
        state.set_midi_binding(
            input_id,
            MidiControlTarget::Mute,
            Some(MidiTrigger::control_change(22)),
        );
        state.toggle_route(input_id, bus_id);
        state.set_route_midi_binding(input_id, bus_id, Some(MidiTrigger::control_change(23)));
        state.toggle_mono(input_id);
        state.set_gate_enabled(input_id, true);
        state.set_gate_threshold(input_id, 27.0);
        state.set_gate_floor(input_id, 10.0);
        state.set_compressor_enabled(input_id, true);
        state.set_compressor_threshold(input_id, 66.0);
        state.set_compressor_ratio(input_id, 4.5);
        state.set_compressor_makeup_gain(input_id, 6.0);
        state.set_eq_enabled(input_id, true);
        state.set_eq_low_gain(input_id, -2.5);
        state.set_eq_mid_gain(input_id, 1.0);
        state.set_eq_high_gain(input_id, 3.5);
        state.set_fx_midi_binding(
            input_id,
            FxMidiTarget::EqLowGain,
            Some(MidiTrigger::control_change(24)),
        );
        state.set_fx_midi_binding(
            input_id,
            FxMidiTarget::CompressorEnabled,
            Some(MidiTrigger::control_change(25)),
        );
        state.set_midi_feedback_output(Some("MIDI Mix OUT".to_string()));
        let created_input = state.add_mixer_strip("Podcast");
        state.set_strip_input_assignment(created_input.id, Some(source_id));
        let created_bus = state.add_bus("Headphones Bus");
        let created_output = state.add_output_sink("Headphones");
        state.toggle_route(created_input.id, created_bus.id);
        state.toggle_route(created_bus.id, created_output.id);

        let config_path = temp_config_path("round-trip");
        save_state_to_path(&state, &config_path).expect("config should save");
        let restored = load_state_from_path(&config_path)
            .expect("config should load")
            .expect("config should exist");

        assert_eq!(restored.source_strips.len(), state.source_strips.len());
        assert_eq!(restored.input_strips.len(), state.input_strips.len());
        assert_eq!(restored.bus_strips.len(), state.bus_strips.len());
        assert_eq!(restored.output_strips.len(), state.output_strips.len());
        assert_eq!(restored.next_strip_id, state.next_strip_id);
        assert_eq!(restored.input_strips[0].label, "Game");
        assert_eq!(
            restored.input_strips[0].volume,
            NormalizedVolume::from_percent(63.0).unwrap()
        );
        assert!(restored.input_strips[0].mono);
        assert_eq!(restored.input_strips[0].channel_count, 2);
        assert_eq!(
            restored.input_strips[0].midi.volume_binding(),
            Some(MidiTrigger::control_change(21))
        );
        assert_eq!(
            restored.input_strips[0].midi.mute_binding(),
            Some(MidiTrigger::control_change(22))
        );
        assert_eq!(
            restored.input_strips[0].routes[0].binding(),
            Some(MidiTrigger::control_change(23))
        );
        assert_eq!(
            restored.input_strips[0]
                .input_assignment
                .as_ref()
                .map(|assignment| assignment.source_id),
            Some(source_id)
        );
        assert!(restored.input_strips[0].effects.gate.enabled);
        assert_eq!(
            restored.input_strips[0].effects.gate.threshold_percent,
            27.0
        );
        assert_eq!(restored.input_strips[0].effects.gate.floor_percent, 10.0);
        assert!(restored.input_strips[0].effects.compressor.enabled);
        assert_eq!(
            restored.input_strips[0]
                .effects
                .compressor
                .threshold_percent,
            66.0
        );
        assert_eq!(restored.input_strips[0].effects.compressor.ratio, 4.5);
        assert_eq!(
            restored.input_strips[0].effects.compressor.makeup_gain_db,
            6.0
        );
        assert!(restored.input_strips[0].effects.eq.enabled);
        assert_eq!(restored.input_strips[0].effects.eq.low_gain_db, -2.5);
        assert_eq!(restored.input_strips[0].effects.eq.mid_gain_db, 1.0);
        assert_eq!(restored.input_strips[0].effects.eq.high_gain_db, 3.5);
        assert_eq!(
            restored.input_strips[0]
                .fx_midi
                .binding(FxMidiTarget::EqLowGain),
            Some(MidiTrigger::control_change(24))
        );
        assert_eq!(
            restored.input_strips[0]
                .fx_midi
                .binding(FxMidiTarget::CompressorEnabled),
            Some(MidiTrigger::control_change(25))
        );
        assert_eq!(
            restored.midi_feedback.output_port_name.as_deref(),
            Some("MIDI Mix OUT")
        );
        assert!(restored.output_strips[0].muted);
        assert!(
            restored
                .input_strips
                .iter()
                .find(|strip| strip.id == created_input.id)
                .expect("saved input should exist")
                .routes
                .iter()
                .any(|route| route.output_id == created_bus.id && route.enabled)
        );
        assert!(
            restored
                .bus_strips
                .iter()
                .find(|strip| strip.id == created_bus.id)
                .expect("saved bus should exist")
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

    #[test]
    fn parsing_sink_inputs_keeps_external_app_streams_only() {
        let sinks = std::collections::HashMap::from([
            (27996, "alsa_output.usb-headset".to_string()),
            (29830, "pipemeeter.game".to_string()),
        ]);
        let dump = r#"
Sink Input #29523
	Driver: PipeWire
	Owner Module: n/a
	Client: 2045
	Sink: 27996
	Corked: yes
	Properties:
		client.api = "pipewire-pulse"
		application.name = "Firefox"
		application.process.id = "85850"
		application.process.binary = "firefox"
		media.name = "A Video"

Sink Input #29912
	Driver: PipeWire
	Owner Module: 536870923
	Client: n/a
	Sink: 29830
	Corked: no
	Properties:
		target.object = "pipemeeter-bus.a1"
		media.name = "loopback-1322-16 output"
"#;

        let streams = parse_pulse_sink_inputs(dump, &sinks);

        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].identity.cached_index, 29523);
        assert_eq!(streams[0].identity.application_name, "Firefox");
        assert_eq!(streams[0].identity.media_name, "A Video");
        assert_eq!(streams[0].current_sink_name, "alsa_output.usb-headset");
        assert!(streams[0].corked);
    }

    #[test]
    fn resolving_application_stream_index_rejects_ambiguous_matches() {
        let identity = ApplicationStreamIdentity {
            cached_index: 1,
            application_name: "Firefox".to_string(),
            media_name: "YouTube".to_string(),
            process_binary: Some("firefox".to_string()),
            process_id: Some(100),
        };
        let streams = vec![
            PulseSinkInputInfo {
                identity: ApplicationStreamIdentity {
                    cached_index: 2,
                    application_name: "Firefox".to_string(),
                    media_name: "YouTube".to_string(),
                    process_binary: Some("firefox".to_string()),
                    process_id: Some(100),
                },
                current_sink_name: "pipemeeter.game".to_string(),
                icon_data_url: None,
                corked: false,
            },
            PulseSinkInputInfo {
                identity: ApplicationStreamIdentity {
                    cached_index: 3,
                    application_name: "Firefox".to_string(),
                    media_name: "YouTube".to_string(),
                    process_binary: Some("firefox".to_string()),
                    process_id: Some(100),
                },
                current_sink_name: "pipemeeter.chat".to_string(),
                icon_data_url: None,
                corked: false,
            },
        ];

        let error = resolve_application_stream_index(&streams, &identity)
            .expect_err("ambiguous matches should fail");
        assert!(error.contains("multiple matching streams"));
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
