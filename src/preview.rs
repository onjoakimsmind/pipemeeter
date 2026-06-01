use crate::audio::{AudioEngineState, MixerStrip, PipeWireNodeInfo, StripId};
use dioxus::prelude::*;
use std::{fs, path::PathBuf};

pub fn write_preview() -> Result<(), String> {
    let output_dir = PathBuf::from("target/pipemeeter-preview");
    fs::create_dir_all(&output_dir)
        .map_err(|error| format!("failed to create preview directory: {error}"))?;

    let output_path = output_dir.join("index.html");
    let html = render_preview_page(crate::audio::load_initial_state());
    fs::write(&output_path, html).map_err(|error| {
        format!(
            "failed to write preview html to {}: {error}",
            output_path.display()
        )
    })?;

    println!("Wrote UI preview to {}", output_path.display());
    Ok(())
}

fn render_preview_page(snapshot: AudioEngineState) -> String {
    let mut snapshot = snapshot;
    snapshot.update_vu_meters(4);

    let body = dioxus_ssr::render_element(rsx! {
        PreviewApp { snapshot }
    });

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Pipemeeter UI Preview</title>
    <script src="https://cdn.tailwindcss.com"></script>
    <style>
      body {{
        background: #020617;
      }}
      input:disabled,
      button:disabled {{
        opacity: 1;
      }}
    </style>
  </head>
  <body>
    {body}
  </body>
</html>"#,
    )
}

#[component]
fn PreviewApp(snapshot: AudioEngineState) -> Element {
    let midi_strips = snapshot
        .input_strips
        .iter()
        .chain(snapshot.output_strips.iter())
        .cloned()
        .collect::<Vec<_>>();

    rsx! {
        div { class: "h-screen overflow-hidden bg-slate-950 text-slate-100",
            main { class: "flex h-screen w-full flex-col gap-3 p-3",
                section { class: "rounded-xl border border-slate-800 bg-slate-900/80 px-3 py-2.5 shadow-2xl shadow-slate-950/35",
                    div { class: "flex flex-col gap-3 xl:flex-row xl:items-center xl:justify-between",
                        div { class: "min-w-0",
                            div { class: "flex flex-wrap items-center gap-2",
                                span { class: "rounded-md border border-cyan-500/30 bg-cyan-500/10 px-2 py-1 text-[10px] uppercase tracking-[0.3em] text-cyan-300", "Mixer" }
                                h1 { class: "text-2xl font-semibold tracking-tight text-white", "Pipemeeter" }
                            }
                            p { class: "mt-1 text-sm text-slate-400", "Compact desktop mixer with separate input and output decks." }
                        }
                        div { class: "grid grid-cols-2 gap-2 text-sm sm:grid-cols-4 xl:min-w-[28rem]",
                            SummaryCard { title: "Inputs", value: snapshot.input_strips.len().to_string(), description: "Sources" }
                            SummaryCard { title: "Outputs", value: snapshot.output_strips.len().to_string(), description: "Buses" }
                            SummaryCard { title: "Routes", value: snapshot.active_route_count().to_string(), description: "Live" }
                            SummaryCard { title: "Muted", value: snapshot.muted_strip_count().to_string(), description: "Cuts" }
                        }
                    }
                    div { class: "mt-3 flex flex-col gap-2 lg:flex-row lg:items-center lg:justify-between",
                        div { class: "min-w-0 rounded-lg border border-slate-800 bg-slate-950/60 px-3 py-2 text-xs text-slate-300",
                            span { class: "mr-2 uppercase tracking-[0.25em] text-cyan-300", "Notice" }
                            span { class: "truncate", "{snapshot.last_notice}" }
                        }
                        div { class: "flex flex-wrap gap-2",
                            button {
                                class: "inline-flex items-center justify-center rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-3 py-2 text-sm font-medium text-cyan-100",
                                disabled: true,
                                "Settings"
                            }
                            button {
                                class: "inline-flex items-center justify-center rounded-lg border border-slate-700 bg-slate-950/70 px-3 py-2 text-sm font-medium text-slate-100",
                                disabled: true,
                                "Refresh"
                            }
                        }
                    }
                }
                section { class: "flex min-h-0 flex-1 flex-col rounded-xl border border-slate-800 bg-slate-900/70 p-3 shadow-2xl shadow-slate-950/30",
                    div { class: "flex flex-col gap-3 xl:flex-row xl:items-center xl:justify-between",
                        div { class: "flex flex-wrap items-center gap-3",
                            h2 { class: "text-xl font-semibold text-white", "Mixer deck" }
                            span { class: "text-xs uppercase tracking-[0.28em] text-slate-500", "Sources routed into the active output buses." }
                        }
                        div { class: "flex flex-col gap-2 xl:items-end",
                            div { class: "inline-flex rounded-lg border border-slate-800 bg-slate-950/70 p-1",
                                button {
                                    class: "rounded-md bg-cyan-500/20 px-3 py-1.5 text-sm font-medium text-cyan-100",
                                    disabled: true,
                                    "Inputs"
                                }
                                button {
                                    class: "rounded-md px-3 py-1.5 text-sm font-medium text-slate-300",
                                    disabled: true,
                                    "Outputs"
                                }
                            }
                            div { class: "flex flex-col gap-2 sm:flex-row sm:items-center",
                                input {
                                    class: "w-full rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none sm:w-56",
                                    r#type: "text",
                                    value: "Podcast bus",
                                    disabled: true
                                }
                                button {
                                    class: "inline-flex items-center justify-center gap-2 rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-3 py-2 text-sm font-medium text-cyan-100",
                                    disabled: true,
                                    span { class: "text-base leading-none", "+" }
                                    "Add sink"
                                }
                            }
                        }
                    }
                    div { class: "mt-3 grid min-h-0 flex-1 auto-rows-fr gap-2 overflow-hidden [grid-template-columns:repeat(auto-fit,minmax(132px,1fr))]",
                        for strip in snapshot.input_strips.iter() {
                            {
                                let route_targets = strip
                                    .routes
                                    .iter()
                                    .map(|route| {
                                        (
                                            route.output_id,
                                            snapshot.output_name(route.output_id).unwrap_or("Output").to_string(),
                                            route.enabled,
                                        )
                                    })
                                    .collect::<Vec<_>>();

                                rsx! {
                                    PreviewStrip { strip: strip.clone(), route_targets }
                                }
                            }
                        }
                    }
                }
                PreviewSettingsDialog {
                    pipewire_status: snapshot.inventory.pipewire_status.clone(),
                    pipewire_nodes: snapshot.inventory.pipewire_nodes.clone(),
                    midi_status: snapshot.inventory.midi_status.clone(),
                    midi_inputs: snapshot
                        .inventory
                        .midi_inputs
                        .iter()
                        .map(|port| port.name.clone())
                        .collect(),
                    strips: midi_strips,
                }
            }
        }
    }
}

#[component]
fn PreviewStrip(strip: MixerStrip, route_targets: Vec<(StripId, String, bool)>) -> Element {
    let volume_display_text = strip.volume.as_percent_text();
    let volume_slider_value = format!("{:.1}", strip.volume.as_percentage());
    let mute_class = if strip.muted {
        "border-rose-400/60 bg-rose-500/20 text-rose-100"
    } else {
        "border-slate-700 bg-slate-950/80 text-slate-200"
    };

    let volume_cc = strip
        .midi
        .volume_cc
        .map(|value| value.to_string())
        .unwrap_or_default();
    let mute_cc = strip
        .midi
        .mute_cc
        .map(|value| value.to_string())
        .unwrap_or_default();
    let midi_summary = match (volume_cc.is_empty(), mute_cc.is_empty()) {
        (false, false) => Some(format!("V{volume_cc} / M{mute_cc}")),
        (false, true) => Some(format!("V{volume_cc}")),
        (true, false) => Some(format!("M{mute_cc}")),
        (true, true) => None,
    };
    let enabled_route_count = route_targets
        .iter()
        .filter(|(_, _, enabled)| *enabled)
        .count();

    rsx! {
        article {
            key: "{strip.id.as_str()}",
            class: "flex h-full min-h-0 min-w-0 w-full flex-col overflow-hidden rounded-xl border border-slate-800 bg-slate-950/80 p-2.5",
            div { class: "flex items-center justify-between gap-2",
                span { class: "max-w-full overflow-hidden rounded-md border border-slate-700 bg-slate-900 px-1.5 py-1 text-[9px] uppercase tracking-[0.24em] text-slate-400 text-ellipsis", "{strip.kind.as_str()}" }
                span { class: "text-[11px] text-slate-500", "{volume_display_text}%" }
            }
            input {
                class: "mt-2 rounded-lg border border-slate-700 bg-slate-900/90 px-2 py-1.5 text-sm font-medium text-slate-100 outline-none",
                r#type: "text",
                value: "{strip.label}",
                disabled: true
            }
            if let Some(summary) = midi_summary {
                div { class: "mt-2 flex min-w-0 items-center justify-between gap-2 text-[10px] uppercase tracking-[0.22em] text-slate-500",
                    span { "MIDI" }
                    span { class: "min-w-0 max-w-full truncate rounded-md border border-slate-800 bg-slate-900/70 px-1.5 py-1 text-[10px] tracking-[0.16em] text-slate-300", "{summary}" }
                }
            }
            div { class: "mt-2 flex flex-1 items-center justify-center gap-2",
                {preview_vu_meter_columns(&strip)}
                div { class: "flex h-[clamp(6.25rem,18vh,11.5rem)] items-center justify-center",
                    input {
                        class: "h-2 w-[clamp(6.25rem,18vh,11.5rem)] -rotate-90 cursor-pointer appearance-none rounded-md bg-slate-700 accent-cyan-400",
                        r#type: "range",
                        min: "0",
                        max: "100",
                        step: "1",
                        value: "{volume_slider_value}",
                        disabled: true
                    }
                }
            }
            div { class: "mt-2 grid grid-cols-2 gap-1.5",
                button {
                    class: "rounded-lg border px-2 py-1.5 text-xs font-medium {mute_class}",
                    disabled: true,
                    if strip.muted { "Unmute" } else { "Mute" }
                }
                button {
                    class: "rounded-lg border border-slate-700 bg-slate-950/80 px-2 py-1.5 text-xs font-medium text-slate-300",
                    disabled: true,
                    "Del"
                }
            }
            {if route_targets.is_empty() {
                rsx! {
                    div { class: "mt-2 rounded-lg border border-dashed border-slate-800 px-2 py-2 text-center text-[10px] uppercase tracking-[0.2em] text-slate-500",
                        "Direct output"
                    }
                }
            } else {
                rsx! {
                    button {
                        class: "mt-2 flex w-full items-center justify-between rounded-lg border border-cyan-400/20 bg-cyan-500/5 px-2.5 py-2 text-xs font-medium text-cyan-100",
                        disabled: true,
                        span { "Routes" }
                        span { class: "rounded-md border border-cyan-400/20 bg-slate-950/70 px-1.5 py-0.5 text-[10px] uppercase tracking-[0.16em] text-cyan-200",
                            "{enabled_route_count}/{route_targets.len()}"
                        }
                    }
                }
            }}
        }
    }
}

fn preview_vu_meter_columns(strip: &MixerStrip) -> Element {
    let channel_levels = strip
        .meter_channels
        .iter()
        .enumerate()
        .map(|(index, level)| {
            (
                format!("{}-meter-{index}", strip.id.as_str()),
                format!("{:.1}%", level.as_percentage()),
                format!("{:.1}%", 100.0 - level.as_percentage()),
            )
        })
        .collect::<Vec<_>>();

    rsx! {
        div { class: "flex h-[clamp(6.25rem,18vh,11.5rem)] items-end gap-1",
            for (key, fill_height, empty_height) in channel_levels {
                div {
                    key: "{key}",
                    class: "relative flex h-full w-2 overflow-hidden rounded-full border border-slate-800 bg-slate-900/90",
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

#[component]
fn PreviewSettingsDialog(
    pipewire_status: String,
    pipewire_nodes: Vec<PipeWireNodeInfo>,
    midi_status: String,
    midi_inputs: Vec<String>,
    strips: Vec<MixerStrip>,
) -> Element {
    rsx! {
        section { class: "rounded-xl border border-slate-800 bg-slate-900/80 p-5 shadow-2xl shadow-slate-950/30",
            div { class: "flex flex-wrap items-start justify-between gap-4 border-b border-slate-800 pb-4",
                div {
                    p { class: "text-sm uppercase tracking-[0.3em] text-cyan-400", "Dialog preview" }
                    h2 { class: "mt-2 text-2xl font-semibold text-white", "Settings" }
                    p { class: "mt-2 max-w-3xl text-sm text-slate-400", "The live UI keeps MIDI mappings and runtime discovery here so the main deck stays compact." }
                }
                button {
                    class: "rounded-lg border border-slate-700 bg-slate-950/70 px-4 py-2 text-sm font-medium text-slate-100",
                    disabled: true,
                    "Close"
                }
            }
            div { class: "mt-5 grid gap-5 xl:grid-cols-[1.15fr_1fr]",
                div { class: "grid gap-5",
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        h3 { class: "text-lg font-semibold text-white", "MIDI test injector" }
                        p { class: "mt-2 text-sm text-slate-400", "Send CC messages without a hardware controller to validate mappings." }
                        div { class: "mt-4 grid gap-3 sm:grid-cols-2",
                            input {
                                class: "rounded-lg border border-slate-700 bg-slate-900/90 px-4 py-3 text-sm text-slate-100 outline-none",
                                r#type: "number",
                                value: "12",
                                disabled: true
                            }
                            input {
                                class: "rounded-lg border border-slate-700 bg-slate-900/90 px-4 py-3 text-sm text-slate-100 outline-none",
                                r#type: "number",
                                value: "127",
                                disabled: true
                            }
                        }
                        button {
                            class: "mt-4 w-full rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-4 py-3 text-sm font-medium text-cyan-100",
                            disabled: true,
                            "Send MIDI CC"
                        }
                    }
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        h3 { class: "text-lg font-semibold text-white", "Runtime inventory" }
                        p { class: "mt-2 text-sm text-slate-400", "PipeWire and MIDI discovery stay available here without crowding the mixer window." }
                        div { class: "mt-4 grid gap-3 sm:grid-cols-2",
                            PreviewInventoryBlock { label: "PipeWire".to_string(), message: pipewire_status }
                            PreviewInventoryBlock { label: "MIDI".to_string(), message: midi_status.clone() }
                        }
                    }
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        h3 { class: "text-lg font-semibold text-white", "PipeWire nodes" }
                        div { class: "mt-4 space-y-3",
                            {if pipewire_nodes.is_empty() {
                                rsx! {
                                    div { class: "rounded-lg border border-dashed border-slate-700 px-4 py-5 text-sm text-slate-400",
                                        "No nodes available yet."
                                    }
                                }
                            } else {
                                rsx! {
                                    for node in pipewire_nodes.into_iter() {
                                        PreviewPipeWireNodeRow { node }
                                    }
                                }
                            }}
                        }
                    }
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        h3 { class: "text-lg font-semibold text-white", "Detected MIDI inputs" }
                        p { class: "mt-2 text-sm text-slate-400", "{midi_status}" }
                        div { class: "mt-4 space-y-3",
                            {if midi_inputs.is_empty() {
                                rsx! {
                                    div { class: "rounded-lg border border-dashed border-slate-700 px-4 py-5 text-sm text-slate-400",
                                        "No MIDI controllers are visible yet."
                                    }
                                }
                            } else {
                                rsx! {
                                    for name in midi_inputs.into_iter() {
                                        div { key: "{name}", class: "rounded-lg border border-slate-800 bg-slate-900/80 px-4 py-3 text-sm text-slate-200",
                                            "{name}"
                                        }
                                    }
                                }
                            }}
                        }
                    }
                }
                article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                    h3 { class: "text-lg font-semibold text-white", "Per-strip MIDI bindings" }
                    p { class: "mt-2 text-sm text-slate-400", "Bindings remain available for both input and output strips, just outside the mixer surface." }
                    div { class: "mt-4 space-y-3",
                        for strip in strips.into_iter() {
                            PreviewMidiBindingRow { strip }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn PreviewMidiBindingRow(strip: MixerStrip) -> Element {
    let volume_cc = strip
        .midi
        .volume_cc
        .map(|value| value.to_string())
        .unwrap_or_default();
    let mute_cc = strip
        .midi
        .mute_cc
        .map(|value| value.to_string())
        .unwrap_or_default();

    rsx! {
        div { key: "{strip.id.as_str()}", class: "rounded-lg border border-slate-800 bg-slate-900/80 p-4",
            div { class: "flex flex-wrap items-center justify-between gap-3",
                div {
                    div { class: "text-sm font-medium text-white", "{strip.label}" }
                    div { class: "mt-1 text-[11px] uppercase tracking-[0.25em] text-slate-500", "{strip.kind.as_str()}" }
                }
                div { class: "text-xs text-slate-500", "{strip.volume.as_percent_text()}%" }
            }
            div { class: "mt-4 grid gap-3 sm:grid-cols-2",
                label { class: "space-y-1",
                    span { class: "block text-[10px] uppercase tracking-[0.25em] text-slate-500", "Vol CC" }
                    input {
                        class: "w-full rounded-lg border border-slate-700 bg-slate-900/90 px-3 py-2 text-sm text-slate-100 outline-none",
                        r#type: "number",
                        value: "{volume_cc}",
                        disabled: true
                    }
                }
                label { class: "space-y-1",
                    span { class: "block text-[10px] uppercase tracking-[0.25em] text-slate-500", "Mute CC" }
                    input {
                        class: "w-full rounded-lg border border-slate-700 bg-slate-900/90 px-3 py-2 text-sm text-slate-100 outline-none",
                        r#type: "number",
                        value: "{mute_cc}",
                        disabled: true
                    }
                }
            }
        }
    }
}

#[component]
fn PreviewPipeWireNodeRow(node: PipeWireNodeInfo) -> Element {
    rsx! {
        div {
            key: "{node.id}",
            class: "rounded-lg border border-slate-800 bg-slate-950/70 px-4 py-3",
            div { class: "flex items-center justify-between gap-4",
                span { class: "font-medium text-slate-100", "{node.name}" }
                span { class: "text-xs uppercase tracking-[0.3em] text-slate-500", "#{node.id}" }
            }
            if let Some(media_class) = node.media_class {
                p { class: "mt-2 text-sm text-slate-400", "{media_class}" }
            }
        }
    }
}

#[component]
fn SummaryCard(title: String, value: String, description: String) -> Element {
    rsx! {
        div { class: "rounded-lg border border-slate-800 bg-slate-950/70 px-3 py-2",
            div { class: "text-[10px] uppercase tracking-[0.28em] text-slate-500", "{title}" }
            div { class: "mt-0.5 text-xl font-semibold text-white", "{value}" }
            div { class: "text-[11px] text-slate-400", "{description}" }
        }
    }
}

#[component]
fn PreviewInventoryBlock(label: String, message: String) -> Element {
    rsx! {
        div { class: "rounded-lg border border-slate-800 bg-slate-950/70 px-4 py-3",
            div { class: "text-xs uppercase tracking-[0.3em] text-slate-500", "{label}" }
            p { class: "mt-2 text-sm text-slate-300", "{message}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_html_contains_output_and_settings_controls() {
        let html = render_preview_page(AudioEngineState::default());

        assert!(html.contains("Outputs"));
        assert!(html.contains("Settings"));
        assert!(html.contains("Send MIDI CC"));
        assert!(html.contains("PipeWire nodes"));
    }
}
