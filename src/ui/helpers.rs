use crate::audio::{MidiMessageKind, MidiTrigger, MixerStrip, StripId, StripKind};
use dioxus::prelude::*;
use std::collections::HashMap;

pub(crate) fn midi_summary(strip: &MixerStrip) -> Option<String> {
    match (strip.midi.volume_binding(), strip.midi.mute_binding()) {
        (Some(volume), Some(mute)) => Some(format!(
            "V {} M {}",
            short_midi_trigger(&volume),
            short_midi_trigger(&mute)
        )),
        (Some(volume), None) => Some(format!("V {}", short_midi_trigger(&volume))),
        (None, Some(mute)) => Some(format!("M {}", short_midi_trigger(&mute))),
        (None, None) => None,
    }
}

pub(crate) fn full_midi_summary(strip: &MixerStrip) -> Option<String> {
    match (strip.midi.volume_binding(), strip.midi.mute_binding()) {
        (Some(volume), Some(mute)) => Some(format!(
            "Volume {} | Mute {}",
            format_midi_trigger(Some(&volume)),
            format_midi_trigger(Some(&mute))
        )),
        (Some(volume), None) => Some(format!("Volume {}", format_midi_trigger(Some(&volume)))),
        (None, Some(mute)) => Some(format!("Mute {}", format_midi_trigger(Some(&mute)))),
        (None, None) => None,
    }
}

pub(crate) fn effect_summary(strip: &MixerStrip) -> Option<String> {
    if strip.effects.bypassed {
        Some("FX Byp".to_string())
    } else {
        let active = strip.effects.active_effect_count();
        if active == 0 {
            None
        } else {
            Some(format!("FX {active}"))
        }
    }
}

pub(crate) fn format_midi_trigger(binding: Option<&MidiTrigger>) -> String {
    binding
        .map(|binding| {
            let kind = match binding.kind {
                MidiMessageKind::ControlChange => "CC",
                MidiMessageKind::Note => "Note",
            };
            let channel = binding
                .channel
                .map(|channel| format!(" on ch{}", channel + 1))
                .unwrap_or_default();
            format!("{kind} {}{channel}", binding.number)
        })
        .unwrap_or_else(|| "Not mapped".to_string())
}

pub(crate) fn short_midi_trigger(binding: &MidiTrigger) -> String {
    match binding.kind {
        MidiMessageKind::ControlChange => format!("CC{}", binding.number),
        MidiMessageKind::Note => format!("N{}", binding.number),
    }
}

pub(crate) fn compact_strip_kind_label(kind: StripKind) -> &'static str {
    match kind {
        StripKind::HardwareSource => "Source",
        StripKind::VirtualCable => "Cable",
        StripKind::Strip => "Strip",
        StripKind::Bus => "Bus",
        StripKind::Output => "Output",
    }
}

pub(crate) fn compact_role_label(strip: &MixerStrip) -> &'static str {
    match strip.kind {
        StripKind::HardwareSource => "Hardware",
        StripKind::VirtualCable => "Virtual",
        StripKind::Strip => "Channel",
        StripKind::Bus if strip.is_fx_bus() => "FX",
        StripKind::Bus => "Bus",
        StripKind::Output => "Output",
    }
}

pub(crate) fn application_badge_text(name: &str) -> String {
    name.chars()
        .find(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_uppercase().to_string())
        .unwrap_or_else(|| "A".to_string())
}

pub(crate) fn source_display_lines(label: &str) -> (String, Option<String>) {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return (String::new(), None);
    }

    let normalized = prettify_device_label(trimmed);
    for suffix in [
        " Analog Stereo",
        " Digital Stereo",
        " Mono",
        " Pro",
        " HDMI / DisplayPort",
    ] {
        if let Some(title) = normalized.strip_suffix(suffix) {
            return (title.trim().to_string(), Some(suffix.trim().to_string()));
        }
    }

    (normalized, None)
}

fn prettify_device_label(label: &str) -> String {
    let letters = label
        .chars()
        .filter(|character| character.is_ascii_alphabetic())
        .count();
    let uppercase = label
        .chars()
        .filter(|character| character.is_ascii_alphabetic() && character.is_ascii_uppercase())
        .count();

    if letters > 0 && (uppercase as f32 / letters as f32) > 0.55 {
        title_case_ascii(label)
    } else {
        label.to_string()
    }
}

fn title_case_ascii(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut capitalize_next = true;

    for character in input.chars() {
        if character.is_ascii_alphabetic() {
            if capitalize_next {
                result.push(character.to_ascii_uppercase());
            } else {
                result.push(character.to_ascii_lowercase());
            }
            capitalize_next = false;
        } else {
            result.push(character);
            capitalize_next = matches!(character, ' ' | '/' | '-' | '_' | '(');
        }
    }

    result
}

pub(crate) fn mute_icon(muted: bool) -> Element {
    let path = if muted {
        "M3 9h3l4-4v14l-4-4H3z M14 6l6 12 M20 6l-6 12"
    } else {
        "M3 9h3l4-4v14l-4-4H3z M14 9.5a4.5 4.5 0 0 1 0 5 M16.5 7a8 8 0 0 1 0 10"
    };
    rsx! {
        svg {
            class: "h-3.5 w-3.5",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "1.8",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            path { d: "{path}" }
        }
    }
}

pub(crate) fn mono_icon() -> Element {
    rsx! {
        svg {
            class: "h-3.5 w-3.5",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "1.8",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            circle { cx: "12", cy: "12", r: "5" }
            path { d: "M4 12h3 M17 12h3" }
        }
    }
}

pub(crate) fn toggle_strip_selection(
    mut selection_signal: Signal<Vec<StripId>>,
    strip_id: StripId,
) {
    let mut selection = selection_signal.write();
    if let Some(index) = selection
        .iter()
        .position(|candidate| *candidate == strip_id)
    {
        selection.remove(index);
    } else {
        selection.push(strip_id);
    }
}

pub(crate) type MeterLevels = HashMap<StripId, Vec<f32>>;

#[component]
pub(crate) fn VuMeter(strip_id: StripId, fallback_channels: usize) -> Element {
    let meter: Signal<MeterLevels> = use_context();
    let levels = meter.read();
    let channels: Vec<f32> = levels
        .get(&strip_id)
        .cloned()
        .unwrap_or_else(|| vec![0.0_f32; fallback_channels.max(1)]);
    let meter_active = channels.iter().any(|&v| v > 0.05);

    rsx! {
        div { class: "relative flex h-full w-full items-center justify-center gap-1.5",
            div { class: "pointer-events-none absolute inset-y-1 left-0 right-0 flex flex-col justify-between",
                for marker in 0..5 {
                    div { key: "meter-mark-{marker}", class: "border-t border-white/6" }
                }
            }
            for (index, level) in channels.into_iter().enumerate() {
                {
                    let fill_height = format!("{:.1}%", level * 100.0);
                    let empty_height = format!("{:.1}%", (1.0 - level) * 100.0);
                    rsx! {
                        div {
                            key: "{strip_id.as_str()}-meter-{index}",
                            class: if meter_active {
                                "relative flex h-full w-2 overflow-hidden rounded-full border border-slate-700 bg-slate-950 shadow-[0_0_8px_rgba(16,185,129,0.18)]"
                            } else {
                                "relative flex h-full w-2 overflow-hidden rounded-full border border-slate-800 bg-slate-950/95"
                            },
                            div {
                                class: "absolute inset-0 bg-gradient-to-t from-emerald-400 via-emerald-400 via-[68%] via-yellow-400 via-[84%] to-rose-500"
                            }
                            div {
                                class: "absolute inset-x-0 top-0 bg-slate-900/95",
                                style: "height: {empty_height};"
                            }
                            div {
                                class: "absolute inset-x-0 bottom-0 border-t border-white/10",
                                style: "height: {fill_height};"
                            }
                        }
                    }
                }
            }
        }
    }
}
