use crate::audio::{
    AudioControlMsg, AudioEngineState, AudioUpdateMsg, MidiControlTarget, MixerStrip,
    PipeWireNodeInfo, SharedEngineBridge, StripId, StripKind,
};
use dioxus::prelude::*;
use dioxus_desktop::{Config, LogicalSize, WindowBuilder, launch::launch as launch_desktop};
use std::{any::Any, collections::HashMap, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MixerDeck {
    Inputs,
    Outputs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum MidiField {
    Volume,
    Mute,
}

pub fn launch(engine: SharedEngineBridge) -> Result<(), String> {
    let window = WindowBuilder::new()
        .with_title("Pipemeeter")
        .with_inner_size(LogicalSize::new(1400.0, 860.0))
        .with_min_inner_size(LogicalSize::new(1320.0, 820.0));

    let config = Config::new()
        .with_window(window)
        .with_menu(None::<dioxus_desktop::muda::Menu>)
        .with_disable_context_menu(true)
        .with_background_color((2, 6, 23, 255))
        .with_custom_head(desktop_head());

    let shared_engine = engine.clone();
    let contexts: Vec<Box<dyn Fn() -> Box<dyn Any>>> = vec![Box::new(move || {
        Box::new(shared_engine.clone()) as Box<dyn Any>
    })];

    launch_desktop(app, contexts, config);
    Ok(())
}

fn desktop_head() -> String {
    r#"
    <script src="https://cdn.tailwindcss.com"></script>
    <style>
      html, body {
        background: #020617;
      }

      input[type="range"]::-webkit-slider-thumb {
        -webkit-appearance: none;
        appearance: none;
        width: 14px;
        height: 14px;
        border-radius: 9999px;
        background: #22d3ee;
        border: 0;
        box-shadow: 0 0 0 2px rgba(8, 47, 73, 0.9);
      }

      input[type="range"]::-moz-range-thumb {
        width: 14px;
        height: 14px;
        border: 0;
        border-radius: 9999px;
        background: #22d3ee;
        box-shadow: 0 0 0 2px rgba(8, 47, 73, 0.9);
      }
    </style>
    "#
    .to_string()
}

fn app() -> Element {
    let engine = use_context::<SharedEngineBridge>();
    let mut snapshot = use_signal(crate::audio::load_initial_state);
    let mut active_deck = use_signal(|| MixerDeck::Inputs);
    let mut show_settings = use_signal(|| false);
    let mut route_editor_strip = use_signal(|| None::<StripId>);
    let mut new_sink_name = use_signal(String::new);
    let mut new_output_name = use_signal(String::new);
    let midi_test_controller = use_signal(String::new);
    let midi_test_value = use_signal(|| "127".to_string());
    let mut midi_inputs = use_signal(HashMap::<(StripId, MidiField), String>::new);

    {
        let engine = engine.clone();
        use_future(move || {
            let engine = engine.clone();
            async move {
                loop {
                    match engine.drain_updates() {
                        Ok(updates) => {
                            for update in updates {
                                match update {
                                    AudioUpdateMsg::Snapshot(next_snapshot) => {
                                        sync_midi_inputs_from_snapshot(
                                            &mut midi_inputs.write(),
                                            &next_snapshot,
                                        );
                                        let route_target = *route_editor_strip.read();
                                        if let Some(selected_strip) = route_target {
                                            let still_exists = next_snapshot
                                                .input_strips
                                                .iter()
                                                .any(|strip| strip.id == selected_strip);
                                            if !still_exists {
                                                route_editor_strip.set(None);
                                            }
                                        }
                                        snapshot.set(next_snapshot);
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            snapshot.write().last_notice = error;
                        }
                    }

                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        });
    }

    let current_snapshot = snapshot.read().clone();
    let active_deck_value = *active_deck.read();
    let show_settings_value = *show_settings.read();
    let route_editor_value = *route_editor_strip.read();
    let new_sink_value = new_sink_name.read().clone();
    let new_output_value = new_output_name.read().clone();
    let midi_test_controller_value = midi_test_controller.read().clone();
    let midi_test_value_value = midi_test_value.read().clone();
    let refresh_engine = engine.clone();
    let add_sink_engine = engine.clone();
    let add_output_engine = engine.clone();
    let route_editor_strip_data = route_editor_value.and_then(|selected_strip| {
        current_snapshot
            .input_strips
            .iter()
            .find(|strip| strip.id == selected_strip)
            .cloned()
    });
    let midi_strips = current_snapshot
        .input_strips
        .iter()
        .chain(current_snapshot.output_strips.iter())
        .cloned()
        .collect::<Vec<_>>();

    let deck_strips = match active_deck_value {
        MixerDeck::Inputs => current_snapshot.input_strips.clone(),
        MixerDeck::Outputs => current_snapshot.output_strips.clone(),
    };

    let deck_notice = match active_deck_value {
        MixerDeck::Inputs => "Sources routed into the active output buses.",
        MixerDeck::Outputs => "Destination buses kept on their own compact deck.",
    };

    let deck_add_controls = if active_deck_value == MixerDeck::Inputs {
        rsx! {
            input {
                class: "w-full rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none sm:w-56",
                r#type: "text",
                value: "{new_sink_value}",
                oninput: move |event| new_sink_name.set(event.value()),
            }
            button {
                class: "inline-flex items-center justify-center gap-2 rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-3 py-2 text-sm font-medium text-cyan-100",
                onclick: move |_| {
                    let label = new_sink_name.read().clone();
                    new_sink_name.set(String::new());
                    if let Err(error) = add_sink_engine.send(AudioControlMsg::AddSink { label }) {
                        snapshot.write().last_notice = error;
                    }
                },
                span { class: "text-base leading-none", "+" }
                "Add sink"
            }
        }
    } else {
        rsx! {
            input {
                class: "w-full rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none sm:w-56",
                r#type: "text",
                value: "{new_output_value}",
                oninput: move |event| new_output_name.set(event.value()),
            }
            button {
                class: "inline-flex items-center justify-center gap-2 rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-3 py-2 text-sm font-medium text-cyan-100",
                onclick: move |_| {
                    let label = new_output_name.read().clone();
                    new_output_name.set(String::new());
                    if let Err(error) = add_output_engine.send(AudioControlMsg::AddOutput { label }) {
                        snapshot.write().last_notice = error;
                    }
                },
                span { class: "text-base leading-none", "+" }
                "Add output"
            }
        }
    };

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
                            SummaryCard { title: "Inputs".to_string(), value: current_snapshot.input_strips.len().to_string(), description: "Sources".to_string() }
                            SummaryCard { title: "Outputs".to_string(), value: current_snapshot.output_strips.len().to_string(), description: "Buses".to_string() }
                            SummaryCard { title: "Routes".to_string(), value: current_snapshot.active_route_count().to_string(), description: "Live".to_string() }
                            SummaryCard { title: "Muted".to_string(), value: current_snapshot.muted_strip_count().to_string(), description: "Cuts".to_string() }
                        }
                    }
                    div { class: "mt-3 flex flex-col gap-2 lg:flex-row lg:items-center lg:justify-between",
                        div { class: "min-w-0 rounded-lg border border-slate-800 bg-slate-950/60 px-3 py-2 text-xs text-slate-300",
                            span { class: "mr-2 uppercase tracking-[0.25em] text-cyan-300", "Notice" }
                            span { class: "truncate", "{current_snapshot.last_notice}" }
                        }
                        div { class: "flex flex-wrap gap-2",
                            button {
                                class: "inline-flex items-center justify-center rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-3 py-2 text-sm font-medium text-cyan-100",
                                onclick: move |_| show_settings.toggle(),
                                if show_settings_value { "Close settings" } else { "Settings" }
                            }
                            button {
                                class: "inline-flex items-center justify-center rounded-lg border border-slate-700 bg-slate-950/70 px-3 py-2 text-sm font-medium text-slate-100",
                                onclick: move |_| {
                                    if let Err(error) = refresh_engine.send(AudioControlMsg::RefreshTopology) {
                                        snapshot.write().last_notice = error;
                                    }
                                },
                                "Refresh"
                            }
                        }
                    }
                }
                div { class: "flex min-h-0 flex-1 gap-3",
                    section { class: "flex min-h-0 flex-1 flex-col rounded-xl border border-slate-800 bg-slate-900/70 p-3 shadow-2xl shadow-slate-950/30",
                        div { class: "flex flex-col gap-3 xl:flex-row xl:items-center xl:justify-between",
                            div { class: "flex flex-wrap items-center gap-3",
                                h2 { class: "text-xl font-semibold text-white", "Mixer deck" }
                                span { class: "text-xs uppercase tracking-[0.28em] text-slate-500", "{deck_notice}" }
                            }
                            div { class: "flex flex-col gap-2 xl:items-end",
                                div { class: "inline-flex rounded-lg border border-slate-800 bg-slate-950/70 p-1",
                                    button {
                                        class: if active_deck_value == MixerDeck::Inputs {
                                            "rounded-md bg-cyan-500/20 px-3 py-1.5 text-sm font-medium text-cyan-100"
                                        } else {
                                            "rounded-md px-3 py-1.5 text-sm font-medium text-slate-300"
                                        },
                                        onclick: move |_| active_deck.set(MixerDeck::Inputs),
                                        "Inputs"
                                    }
                                    button {
                                        class: if active_deck_value == MixerDeck::Outputs {
                                            "rounded-md bg-cyan-500/20 px-3 py-1.5 text-sm font-medium text-cyan-100"
                                        } else {
                                            "rounded-md px-3 py-1.5 text-sm font-medium text-slate-300"
                                        },
                                        onclick: move |_| active_deck.set(MixerDeck::Outputs),
                                        "Outputs"
                                    }
                                }
                                div { class: "flex flex-col gap-2 sm:flex-row sm:items-center",
                                    {deck_add_controls}
                                }
                            }
                        }
                        div { class: "mt-3 min-h-0 flex-1 overflow-x-auto overflow-y-hidden pb-2",
                            div { class: "flex h-full min-w-max gap-2",
                                for strip in deck_strips.into_iter() {
                                    {
                                        let route_targets = strip
                                            .routes
                                            .iter()
                                            .map(|route| {
                                                (
                                                    route.output_id,
                                                    current_snapshot
                                                        .output_name(route.output_id)
                                                        .unwrap_or("Output")
                                                        .to_string(),
                                                    route.enabled,
                                                )
                                            })
                                            .collect::<Vec<_>>();

                                        rsx! {
                                            DesktopStrip {
                                                strip,
                                                route_targets,
                                                route_editor_strip: route_editor_value,
                                                snapshot,
                                                route_editor_signal: route_editor_strip,
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if route_editor_strip_data.is_some() {
                        aside { class: "flex min-h-0 w-[26rem] shrink-0 flex-col gap-3",
                            if let Some(selected_strip) = route_editor_strip_data {
                                RouteEditorPanel {
                                    snapshot: current_snapshot.clone(),
                                    strip: selected_strip,
                                    route_editor_signal: route_editor_strip,
                                    snapshot_signal: snapshot,
                                }
                            }
                        }
                    }
                }
                if show_settings_value {
                    SettingsModal {
                        snapshot: current_snapshot.clone(),
                        strips: midi_strips,
                        midi_inputs: midi_inputs.read().clone(),
                        midi_test_controller: midi_test_controller_value,
                        midi_test_value: midi_test_value_value,
                        show_settings_signal: show_settings,
                        snapshot_signal: snapshot,
                        midi_inputs_signal: midi_inputs,
                        midi_test_controller_signal: midi_test_controller,
                        midi_test_value_signal: midi_test_value,
                    }
                }
            }
        }
    }
}

#[component]
fn DesktopStrip(
    strip: MixerStrip,
    route_targets: Vec<(StripId, String, bool)>,
    route_editor_strip: Option<StripId>,
    snapshot: Signal<AudioEngineState>,
    route_editor_signal: Signal<Option<StripId>>,
) -> Element {
    let engine = use_context::<SharedEngineBridge>();
    let rename_engine = engine.clone();
    let volume_engine = engine.clone();
    let mute_engine = engine.clone();
    let mono_engine = engine.clone();
    let remove_engine = engine.clone();
    let volume_display_text = strip.volume.as_percent_text();
    let volume_slider_value = format!("{:.1}", strip.volume.as_percentage());
    let is_output = matches!(strip.kind, StripKind::Output);
    let meter_tray_style = format!(
        "width: {}px;",
        18 + (strip.meter_channels.len().max(1) as i32 * 10)
            + ((strip.meter_channels.len().saturating_sub(1) as i32) * 6)
    );
    let mute_class = if strip.muted {
        "border-rose-400/60 bg-rose-500/20 text-rose-100"
    } else {
        "border-slate-700 bg-slate-950/80 text-slate-200"
    };
    let midi_summary = midi_summary(&strip);
    let enabled_route_count = route_targets
        .iter()
        .filter(|(_, _, enabled)| *enabled)
        .count();
    let route_button_class = if route_editor_strip == Some(strip.id) {
        "mt-2 flex w-full items-center justify-between rounded-lg border border-cyan-400/40 bg-cyan-500/15 px-2.5 py-2 text-xs font-medium text-cyan-100"
    } else {
        "mt-2 flex w-full items-center justify-between rounded-lg border border-cyan-400/20 bg-cyan-500/5 px-2.5 py-2 text-xs font-medium text-cyan-100"
    };
    let strip_mode = if strip.mono {
        "Mono".to_string()
    } else {
        match strip.channel_count {
            1 => "1 ch".to_string(),
            count => format!("{count} ch"),
        }
    };
    let action_grid_class = if is_output {
        "mt-2 grid grid-cols-2 gap-1.5"
    } else {
        "mt-2 grid grid-cols-3 gap-1.5"
    };

    rsx! {
        article {
            key: "{strip.id.as_str()}",
            class: "flex h-full min-h-0 min-w-[156px] w-[156px] flex-col overflow-hidden rounded-xl border border-slate-800 bg-slate-950/80 p-2.5",
            div { class: "flex items-center justify-between gap-2",
                span { class: "max-w-full overflow-hidden rounded-md border border-slate-700 bg-slate-900 px-1.5 py-1 text-[9px] uppercase tracking-[0.24em] text-slate-400 text-ellipsis", "{strip.kind.as_str()}" }
                span { class: "text-[11px] text-slate-500", "{volume_display_text}%" }
            }
            input {
                class: "mt-2 rounded-lg border border-slate-700 bg-slate-900/90 px-2 py-1.5 text-sm font-medium text-slate-100 outline-none",
                r#type: "text",
                value: "{strip.label}",
                oninput: move |event| {
                    if let Err(error) = rename_engine.send(AudioControlMsg::RenameStrip {
                        strip: strip.id,
                        label: event.value(),
                    }) {
                        snapshot.write().last_notice = error;
                    }
                }
            }
            div { class: "mt-2 flex min-w-0 items-center justify-between gap-2 text-[10px] uppercase tracking-[0.22em] text-slate-500",
                span { "{strip_mode}" }
                if let Some(summary) = midi_summary {
                    span { class: "min-w-0 max-w-full truncate rounded-md border border-slate-800 bg-slate-900/70 px-1.5 py-1 text-[10px] tracking-[0.16em] text-slate-300", "{summary}" }
                }
            }
            div { class: "mt-2 grid flex-1 grid-cols-[auto_minmax(0,1fr)] items-end justify-items-center gap-3",
                div {
                    class: "flex h-[clamp(6.25rem,18vh,11.5rem)] shrink-0 items-end justify-center self-stretch rounded-lg border border-slate-800 bg-slate-900/70 px-1.5 py-2",
                    style: "{meter_tray_style}",
                    {vu_meter_columns(&strip)}
                }
                div { class: "flex h-[clamp(6.25rem,18vh,11.5rem)] min-w-0 w-full items-center justify-center",
                    input {
                        class: "h-2 w-[clamp(6.25rem,18vh,11.5rem)] -rotate-90 cursor-pointer appearance-none rounded-md bg-slate-700 accent-cyan-400",
                        r#type: "range",
                        min: "0",
                        max: "100",
                        step: "1",
                        value: "{volume_slider_value}",
                        oninput: move |event| {
                            if let Ok(percent) = event.value().parse::<f32>() {
                                if let Ok(volume) = crate::audio::NormalizedVolume::from_percent(percent.round()) {
                                    if let Err(error) = volume_engine.send(AudioControlMsg::SetStripVolume {
                                        strip: strip.id,
                                        volume,
                                    }) {
                                        snapshot.write().last_notice = error;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "{action_grid_class}",
                button {
                    class: "rounded-lg border px-2 py-1.5 text-xs font-medium {mute_class}",
                    onclick: move |_| {
                        if let Err(error) = mute_engine.send(AudioControlMsg::ToggleMute { strip: strip.id }) {
                            snapshot.write().last_notice = error;
                        }
                    },
                    if strip.muted { "Unmute" } else { "Mute" }
                }
                if !is_output {
                    button {
                        class: if strip.mono {
                            "rounded-lg border border-amber-400/40 bg-amber-500/20 px-2 py-1.5 text-xs font-medium text-amber-100"
                        } else {
                            "rounded-lg border border-slate-700 bg-slate-950/80 px-2 py-1.5 text-xs font-medium text-slate-300"
                        },
                        onclick: move |_| {
                            if let Err(error) = mono_engine.send(AudioControlMsg::ToggleMono { strip: strip.id }) {
                                snapshot.write().last_notice = error;
                            }
                        },
                        "Mono"
                    }
                }
                button {
                    class: "rounded-lg border border-slate-700 bg-slate-950/80 px-2 py-1.5 text-xs font-medium text-slate-300",
                    onclick: move |_| {
                        if route_editor_signal.read().as_ref() == Some(&strip.id) {
                            route_editor_signal.set(None);
                        }
                        if let Err(error) = remove_engine.send(AudioControlMsg::RemoveStrip { strip: strip.id }) {
                            snapshot.write().last_notice = error;
                        }
                    },
                    "Del"
                }
            }
            if route_targets.is_empty() {
                div { class: "mt-2 rounded-lg border border-dashed border-slate-800 px-2 py-2 text-center text-[10px] uppercase tracking-[0.2em] text-slate-500",
                    "Direct output"
                }
            } else {
                button {
                    class: "{route_button_class}",
                    onclick: move |_| {
                        if route_editor_signal.read().as_ref() == Some(&strip.id) {
                            route_editor_signal.set(None);
                        } else {
                            route_editor_signal.set(Some(strip.id));
                        }
                    },
                    span { "Routes" }
                    span { class: "rounded-md border border-cyan-400/20 bg-slate-950/70 px-1.5 py-0.5 text-[10px] uppercase tracking-[0.16em] text-cyan-200",
                        "{enabled_route_count}/{route_targets.len()}"
                    }
                }
            }
        }
    }
}

#[component]
fn RouteEditorPanel(
    snapshot: AudioEngineState,
    strip: MixerStrip,
    route_editor_signal: Signal<Option<StripId>>,
    snapshot_signal: Signal<AudioEngineState>,
) -> Element {
    let engine = use_context::<SharedEngineBridge>();
    rsx! {
        section { class: "rounded-xl border border-slate-800 bg-slate-900/80 p-4 shadow-2xl shadow-slate-950/30",
            div { class: "flex items-start justify-between gap-4 border-b border-slate-800 pb-4",
                div {
                    p { class: "text-sm uppercase tracking-[0.3em] text-cyan-400", "Routing" }
                    h2 { class: "mt-2 text-xl font-semibold text-white", "{strip.label}" }
                    p { class: "mt-2 text-sm text-slate-400", "Toggle which outputs receive this strip." }
                }
                button {
                    class: "rounded-lg border border-slate-700 bg-slate-950/70 px-4 py-2 text-sm font-medium text-slate-100",
                    onclick: move |_| route_editor_signal.set(None),
                    "Close"
                }
            }
            div { class: "mt-4 space-y-3",
                for route in strip.routes.into_iter() {
                    {
                        let output_label = snapshot
                            .output_name(route.output_id)
                            .unwrap_or("Output")
                            .to_string();
                        let route_class = if route.enabled {
                            "w-full rounded-lg border border-cyan-400/30 bg-cyan-500/10 px-4 py-3 text-left text-sm font-medium text-cyan-100"
                        } else {
                            "w-full rounded-lg border border-slate-800 bg-slate-950/70 px-4 py-3 text-left text-sm font-medium text-slate-200"
                        };
                        let toggle_engine = engine.clone();

                        rsx! {
                            button {
                                key: "{strip.id.as_str()}-{route.output_id.as_str()}",
                                class: "{route_class}",
                                onclick: move |_| {
                                    if let Err(error) = toggle_engine.send(AudioControlMsg::ToggleRoute {
                                        strip: strip.id,
                                        output: route.output_id,
                                    }) {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                                div { class: "flex items-center justify-between gap-3",
                                    span { "{output_label}" }
                                    span { class: "text-[10px] uppercase tracking-[0.25em] text-slate-400",
                                        if route.enabled { "On" } else { "Off" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SettingsModal(
    snapshot: AudioEngineState,
    strips: Vec<MixerStrip>,
    midi_inputs: HashMap<(StripId, MidiField), String>,
    midi_test_controller: String,
    midi_test_value: String,
    show_settings_signal: Signal<bool>,
    snapshot_signal: Signal<AudioEngineState>,
    midi_inputs_signal: Signal<HashMap<(StripId, MidiField), String>>,
    midi_test_controller_signal: Signal<String>,
    midi_test_value_signal: Signal<String>,
) -> Element {
    let engine = use_context::<SharedEngineBridge>();
    let midi_test_engine = engine.clone();
    rsx! {
        div { class: "fixed inset-0 z-40 flex items-start justify-center bg-slate-950/70 p-6 backdrop-blur-sm",
            section { class: "flex h-[min(92vh,980px)] w-full max-w-5xl flex-col rounded-2xl border border-slate-800 bg-slate-900/95 p-4 shadow-2xl shadow-black/50",
                div { class: "flex items-start justify-between gap-4 border-b border-slate-800 pb-4",
                    div {
                        p { class: "text-sm uppercase tracking-[0.3em] text-cyan-400", "Settings" }
                        h2 { class: "mt-2 text-xl font-semibold text-white", "MIDI + runtime inventory" }
                        p { class: "mt-2 text-sm text-slate-400", "Moved into a dedicated modal so the mixer surface stays focused." }
                    }
                    button {
                        class: "rounded-lg border border-slate-700 bg-slate-950/70 px-4 py-2 text-sm font-medium text-slate-100",
                        onclick: move |_| show_settings_signal.set(false),
                        "Close"
                    }
                }
                div { class: "mt-4 min-h-0 flex-1 space-y-4 overflow-y-auto pr-1",
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        h3 { class: "text-lg font-semibold text-white", "MIDI test injector" }
                        p { class: "mt-2 text-sm text-slate-400", "Send CC messages without a hardware controller to validate mappings." }
                        div { class: "mt-4 grid gap-3 sm:grid-cols-2",
                            input {
                                class: "rounded-lg border border-slate-700 bg-slate-900/90 px-4 py-3 text-sm text-slate-100 outline-none",
                                r#type: "number",
                                value: "{midi_test_controller}",
                                oninput: move |event| midi_test_controller_signal.set(event.value()),
                            }
                            input {
                                class: "rounded-lg border border-slate-700 bg-slate-900/90 px-4 py-3 text-sm text-slate-100 outline-none",
                                r#type: "number",
                                value: "{midi_test_value}",
                                oninput: move |event| midi_test_value_signal.set(event.value()),
                            }
                        }
                        button {
                            class: "mt-4 w-full rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-4 py-3 text-sm font-medium text-cyan-100",
                            onclick: move |_| {
                                match (
                                    midi_test_controller_signal.read().parse::<u8>(),
                                    midi_test_value_signal.read().parse::<u8>(),
                                ) {
                                    (Ok(controller), Ok(value)) => {
                                        if let Err(error) = midi_test_engine.send(AudioControlMsg::ApplyMidiCc { controller, value }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    }
                                    _ => {
                                        snapshot_signal.write().last_notice =
                                            "MIDI test needs numeric controller and value fields".to_string();
                                    }
                                }
                            },
                            "Send MIDI CC"
                        }
                    }
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        h3 { class: "text-lg font-semibold text-white", "Runtime inventory" }
                        p { class: "mt-2 text-sm text-slate-400", "PipeWire and MIDI discovery stay available here without crowding the mixer window." }
                        div { class: "mt-4 grid gap-3 sm:grid-cols-2",
                            InventoryBlock { label: "PipeWire".to_string(), message: snapshot.inventory.pipewire_status.clone() }
                            InventoryBlock { label: "MIDI".to_string(), message: snapshot.inventory.midi_status.clone() }
                        }
                    }
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        h3 { class: "text-lg font-semibold text-white", "PipeWire nodes" }
                        div { class: "mt-4 space-y-3",
                            if snapshot.inventory.pipewire_nodes.is_empty() {
                                div { class: "rounded-lg border border-dashed border-slate-700 px-4 py-5 text-sm text-slate-400",
                                    "No nodes available yet."
                                }
                            } else {
                                for node in snapshot.inventory.pipewire_nodes.into_iter() {
                                    PipeWireNodeRow { node }
                                }
                            }
                        }
                    }
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        h3 { class: "text-lg font-semibold text-white", "Detected MIDI inputs" }
                        p { class: "mt-2 text-sm text-slate-400", "{snapshot.inventory.midi_status}" }
                        div { class: "mt-4 space-y-3",
                            if snapshot.inventory.midi_inputs.is_empty() {
                                div { class: "rounded-lg border border-dashed border-slate-700 px-4 py-5 text-sm text-slate-400",
                                    "No MIDI controllers are visible yet."
                                }
                            } else {
                                for port in snapshot.inventory.midi_inputs.into_iter() {
                                    div { key: "{port.name}", class: "rounded-lg border border-slate-800 bg-slate-900/80 px-4 py-3 text-sm text-slate-200",
                                        "{port.name}"
                                    }
                                }
                            }
                        }
                    }
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        h3 { class: "text-lg font-semibold text-white", "Per-strip MIDI bindings" }
                        p { class: "mt-2 text-sm text-slate-400", "Bindings remain available for both input and output strips, just outside the mixer surface." }
                        div { class: "mt-4 space-y-3",
                            for strip in strips.into_iter() {
                                MidiBindingRow {
                                    strip,
                                    midi_inputs: midi_inputs.clone(),
                                    snapshot_signal,
                                    midi_inputs_signal,
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn MidiBindingRow(
    strip: MixerStrip,
    midi_inputs: HashMap<(StripId, MidiField), String>,
    snapshot_signal: Signal<AudioEngineState>,
    midi_inputs_signal: Signal<HashMap<(StripId, MidiField), String>>,
) -> Element {
    let engine = use_context::<SharedEngineBridge>();
    let volume_engine = engine.clone();
    let mute_engine = engine.clone();
    let volume_cc = midi_inputs
        .get(&(strip.id, MidiField::Volume))
        .cloned()
        .unwrap_or_default();
    let mute_cc = midi_inputs
        .get(&(strip.id, MidiField::Mute))
        .cloned()
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
                        oninput: move |event| {
                            let value = event.value();
                            midi_inputs_signal.write().insert((strip.id, MidiField::Volume), value.clone());
                            apply_midi_binding(
                                &volume_engine,
                                snapshot_signal,
                                strip.id,
                                MidiField::Volume,
                                value,
                            );
                        }
                    }
                }
                label { class: "space-y-1",
                    span { class: "block text-[10px] uppercase tracking-[0.25em] text-slate-500", "Mute CC" }
                    input {
                        class: "w-full rounded-lg border border-slate-700 bg-slate-900/90 px-3 py-2 text-sm text-slate-100 outline-none",
                        r#type: "number",
                        value: "{mute_cc}",
                        oninput: move |event| {
                            let value = event.value();
                            midi_inputs_signal.write().insert((strip.id, MidiField::Mute), value.clone());
                            apply_midi_binding(
                                &mute_engine,
                                snapshot_signal,
                                strip.id,
                                MidiField::Mute,
                                value,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn PipeWireNodeRow(node: PipeWireNodeInfo) -> Element {
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
fn InventoryBlock(label: String, message: String) -> Element {
    rsx! {
        div { class: "rounded-lg border border-slate-800 bg-slate-950/70 px-4 py-3",
            div { class: "text-xs uppercase tracking-[0.3em] text-slate-500", "{label}" }
            p { class: "mt-2 text-sm text-slate-300", "{message}" }
        }
    }
}

fn midi_summary(strip: &MixerStrip) -> Option<String> {
    match (strip.midi.volume_cc, strip.midi.mute_cc) {
        (Some(volume), Some(mute)) => Some(format!("V{volume} / M{mute}")),
        (Some(volume), None) => Some(format!("V{volume}")),
        (None, Some(mute)) => Some(format!("M{mute}")),
        (None, None) => None,
    }
}

fn apply_midi_binding(
    engine: &SharedEngineBridge,
    mut snapshot_signal: Signal<AudioEngineState>,
    strip: StripId,
    field: MidiField,
    value: String,
) {
    let controller = match value.trim() {
        "" => Some(None),
        raw => raw.parse::<u8>().ok().map(Some),
    };

    if let Some(controller) = controller {
        let target = match field {
            MidiField::Volume => MidiControlTarget::Volume,
            MidiField::Mute => MidiControlTarget::Mute,
        };
        if let Err(error) = engine.send(AudioControlMsg::SetMidiBinding {
            strip,
            target,
            controller,
        }) {
            snapshot_signal.write().last_notice = error;
        }
    }
}

fn sync_midi_inputs_from_snapshot(
    midi_inputs: &mut HashMap<(StripId, MidiField), String>,
    snapshot: &AudioEngineState,
) {
    let mut active_keys = Vec::new();

    for strip in snapshot
        .input_strips
        .iter()
        .chain(snapshot.output_strips.iter())
    {
        let volume_key = (strip.id, MidiField::Volume);
        let mute_key = (strip.id, MidiField::Mute);

        active_keys.push(volume_key);
        active_keys.push(mute_key);

        midi_inputs.entry(volume_key).or_insert_with(|| {
            strip
                .midi
                .volume_cc
                .map(|value| value.to_string())
                .unwrap_or_default()
        });
        midi_inputs.entry(mute_key).or_insert_with(|| {
            strip
                .midi
                .mute_cc
                .map(|value| value.to_string())
                .unwrap_or_default()
        });
    }

    midi_inputs.retain(|key, _| active_keys.contains(key));
}

fn vu_meter_columns(strip: &MixerStrip) -> Element {
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
        div { class: "flex h-full w-full items-end justify-center gap-1.5",
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
