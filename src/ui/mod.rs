mod components;
mod helpers;
mod icon;

use crate::audio::{
    AudioControlMsg, AudioEngineState, AudioUpdateMsg, EqBand, FxMidiTarget,
    MidiControlTarget, MidiLearnTarget, MixerStrip, SharedEngineBridge, StripId,
    StripKind,
};
use dioxus::prelude::*;
use dioxus_desktop::{Config, LogicalSize, WindowBuilder, launch::launch as launch_desktop};
use std::{any::Any, collections::HashMap, env, time::Duration};

use self::components::{
    ApplicationStreamRow, BusStatusCard, CreateFxBusModal, CreateStripModal, InventoryBlock,
    MidiBindingCard, PipeWireNodeRow, RotaryKnobCard, SliderControlCard,
    SummaryCard,
};
use self::helpers::{
    compact_role_label, compact_strip_kind_label, effect_summary, format_midi_trigger,
    mono_icon, mute_icon, source_display_lines, MeterLevels, VuMeter,
};
use self::icon::app_icon;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MixerDeckTab {
    Strips,
    Buses,
    Effects,
}

pub fn launch(engine: SharedEngineBridge) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    unsafe {
        env::set_var(
            "TAO_UNIX_APPLICATION_ID",
            "io.github.onjoakimsmind.pipemeeter",
        );
    }

    let window = WindowBuilder::new()
        .with_title("Pipemeeter")
        .with_inner_size(LogicalSize::new(1520.0, 960.0))
        .with_min_inner_size(LogicalSize::new(1400.0, 960.0))
        .with_window_icon(app_icon());

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
    let mut active_mixer_tab = use_signal(|| MixerDeckTab::Strips);
    let mut show_settings = use_signal(|| false);
    let mut show_create_strip = use_signal(|| false);
    let mut show_create_fx_bus = use_signal(|| false);
    let mut route_editor_strip = use_signal(|| None::<StripId>);
    let new_virtual_cable_name = use_signal(String::new);
    let mut create_strip_name = use_signal(String::new);
    let mut create_strip_source = use_signal(|| None::<StripId>);
    let mut create_strip_buses = use_signal(Vec::<StripId>::new);
    let mut create_fx_bus_name = use_signal(String::new);
    let mut create_fx_bus_gate = use_signal(|| false);
    let mut create_fx_bus_compressor = use_signal(|| false);
    let mut create_fx_bus_eq = use_signal(|| false);
    let mut meter_signal: Signal<MeterLevels> =
        use_context_provider(|| Signal::new(HashMap::new()));

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
                                        let route_target = *route_editor_strip.read();
                                        if let Some(selected_strip) = route_target {
                                            let still_exists = next_snapshot
                                                .input_strips
                                                .iter()
                                                .chain(next_snapshot.bus_strips.iter())
                                                .any(|strip| strip.id == selected_strip);
                                            if !still_exists {
                                                route_editor_strip.set(None);
                                            }
                                        }
                                        snapshot.set(next_snapshot);
                                    }
                                    AudioUpdateMsg::MeterUpdate(levels) => {
                                        *meter_signal.write() = levels;
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            snapshot.write().last_notice = error;
                        }
                    }

                    tokio::time::sleep(Duration::from_millis(16)).await;
                }
            }
        });
    }

    let current_snapshot = snapshot.read().clone();
    let active_mixer_tab_value = *active_mixer_tab.read();
    let show_settings_value = *show_settings.read();
    let show_create_strip_value = *show_create_strip.read();
    let show_create_fx_bus_value = *show_create_fx_bus.read();
    let route_editor_value = *route_editor_strip.read();
    let refresh_engine = engine.clone();
    let add_bus_engine = engine.clone();
    let route_editor_strip_data = route_editor_value.and_then(|selected_strip| {
        current_snapshot
            .input_strips
            .iter()
            .chain(current_snapshot.bus_strips.iter())
            .find(|strip| strip.id == selected_strip)
            .cloned()
    });
    let deck_sections = match active_mixer_tab_value {
        MixerDeckTab::Strips => vec![(
            "Channel strips".to_string(),
            None::<String>,
            current_snapshot.input_strips.clone(),
        )],
        MixerDeckTab::Buses => vec![(
            "Mix buses".to_string(),
            None::<String>,
            current_snapshot
                .bus_strips
                .iter()
                .filter(|strip| strip.is_mix_bus())
                .cloned()
                .collect(),
        )],
        MixerDeckTab::Effects => vec![(
            "FX buses".to_string(),
            None::<String>,
            current_snapshot
                .bus_strips
                .iter()
                .filter(|strip| strip.is_fx_bus())
                .cloned()
                .collect(),
        )],
    };
    let deck_notice = match active_mixer_tab_value {
        MixerDeckTab::Strips => {
            "Assign one source inside each strip, send strips to buses, and keep the bus overview visible for live mix status."
        }
        MixerDeckTab::Buses => {
            "Shape each bus and map it to system or app-managed outputs without crowding the strip view."
        }
        MixerDeckTab::Effects => {
            "Build chained FX returns here, then route those FX buses into other FX buses or back into your main mix buses."
        }
    };

    rsx! {
        div { class: "h-screen overflow-hidden bg-slate-950 text-slate-100",
            main { class: "flex h-screen w-full flex-col gap-2.5 p-2.5",
                section { class: "rounded-xl border border-slate-800 bg-slate-900/80 px-3 py-2 shadow-2xl shadow-slate-950/35",
                    div { class: "flex flex-col gap-2.5 xl:flex-row xl:items-center xl:justify-between",
                        div { class: "min-w-0",
                            div { class: "flex flex-wrap items-center gap-2",
                                span { class: "rounded-md border border-cyan-500/30 bg-cyan-500/10 px-2 py-0.5 text-[10px] uppercase tracking-[0.26em] text-cyan-300", "Mixer" }
                                h1 { class: "text-xl font-semibold tracking-tight text-white", "Pipemeeter" }
                            }
                            p { class: "mt-0.5 text-xs text-slate-400", "Native Linux mixer with source-assigned strips, buses, and PipeWire output routing." }
                        }
                        div { class: "grid grid-cols-2 gap-1.5 text-sm sm:grid-cols-3 xl:min-w-[40rem] xl:grid-cols-5",
                            SummaryCard { title: "Sources".to_string(), value: current_snapshot.source_strips.len().to_string(), description: "Inputs".to_string() }
                            SummaryCard { title: "Strips".to_string(), value: current_snapshot.input_strips.len().to_string(), description: "Channels".to_string() }
                            SummaryCard { title: "Buses".to_string(), value: current_snapshot.bus_strips.len().to_string(), description: "Mixes".to_string() }
                            SummaryCard { title: "Routes".to_string(), value: current_snapshot.active_route_count().to_string(), description: "Live".to_string() }
                            SummaryCard { title: "Muted".to_string(), value: current_snapshot.muted_strip_count().to_string(), description: "Cuts".to_string() }
                        }
                    }
                    div { class: "mt-2 flex flex-col gap-2 lg:flex-row lg:items-center lg:justify-between",
                        div { class: "min-w-0 rounded-lg border border-slate-800 bg-slate-950/60 px-3 py-1.5 text-xs text-slate-300",
                            span { class: "mr-2 uppercase tracking-[0.22em] text-cyan-300", "Notice" }
                            span { class: "truncate", "{current_snapshot.last_notice}" }
                        }
                        div { class: "flex flex-wrap gap-2",
                            button {
                                class: "inline-flex items-center justify-center rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-3 py-1.5 text-sm font-medium text-cyan-100",
                                onclick: move |_| show_settings.toggle(),
                                if show_settings_value { "Close settings" } else { "Settings" }
                            }
                            button {
                                class: "inline-flex items-center justify-center rounded-lg border border-slate-700 bg-slate-950/70 px-3 py-1.5 text-sm font-medium text-slate-100",
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
                        div { class: "flex flex-col gap-2.5 xl:flex-row xl:items-center xl:justify-between",
                            div { class: "flex flex-wrap items-center gap-3",
                                h2 { class: "text-lg font-semibold text-white", "Mixer deck" }
                                span { class: "text-sm text-slate-400", "{deck_notice}" }
                            }
                            div { class: "flex flex-col gap-1.5 xl:items-end",
                                div { class: "flex flex-wrap items-center gap-2",
                                    div { class: "inline-flex rounded-lg border border-slate-800 bg-slate-950/70 p-1",
                                        button {
                                            class: if active_mixer_tab_value == MixerDeckTab::Strips {
                                                "rounded-md bg-cyan-500/20 px-3 py-1 text-sm font-medium text-cyan-100"
                                            } else {
                                                "rounded-md px-3 py-1 text-sm font-medium text-slate-300"
                                            },
                                            onclick: move |_| active_mixer_tab.set(MixerDeckTab::Strips),
                                            "Strips"
                                        }
                                        button {
                                            class: if active_mixer_tab_value == MixerDeckTab::Buses {
                                                "rounded-md bg-cyan-500/20 px-3 py-1 text-sm font-medium text-cyan-100"
                                            } else {
                                                "rounded-md px-3 py-1 text-sm font-medium text-slate-300"
                                            },
                                            onclick: move |_| active_mixer_tab.set(MixerDeckTab::Buses),
                                            "Buses"
                                        }
                                        button {
                                            class: if active_mixer_tab_value == MixerDeckTab::Effects {
                                                "rounded-md bg-cyan-500/20 px-3 py-1 text-sm font-medium text-cyan-100"
                                            } else {
                                                "rounded-md px-3 py-1 text-sm font-medium text-slate-300"
                                            },
                                            onclick: move |_| active_mixer_tab.set(MixerDeckTab::Effects),
                                            "FX"
                                        }
                                    }
                                    if active_mixer_tab_value == MixerDeckTab::Strips {
                                        button {
                                            class: "inline-flex items-center justify-center gap-2 rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-3 py-1.5 text-sm font-medium text-cyan-100",
                                            onclick: move |_| {
                                                create_strip_name.set(String::new());
                                                create_strip_source.set(None);
                                                create_strip_buses.set(Vec::new());
                                                show_create_strip.set(true);
                                            },
                                            span { class: "text-base leading-none", "+" }
                                            "Add strip"
                                        }
                                    }
                                    if active_mixer_tab_value == MixerDeckTab::Buses {
                                        button {
                                            class: "inline-flex items-center justify-center gap-2 rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-3 py-1.5 text-sm font-medium text-cyan-100",
                                            onclick: move |_| {
                                                if let Err(error) = add_bus_engine.send(AudioControlMsg::AddBus { label: String::new() }) {
                                                    snapshot.write().last_notice = error;
                                                }
                                            },
                                            span { class: "text-base leading-none", "+" }
                                            "Add bus"
                                        }
                                    }
                                    if active_mixer_tab_value == MixerDeckTab::Effects {
                                        button {
                                            class: "inline-flex items-center justify-center gap-2 rounded-lg border border-amber-400/40 bg-amber-500/10 px-3 py-1.5 text-sm font-medium text-amber-100",
                                            onclick: move |_| {
                                                create_fx_bus_name.set(String::new());
                                                create_fx_bus_gate.set(false);
                                                create_fx_bus_compressor.set(false);
                                                create_fx_bus_eq.set(false);
                                                show_create_fx_bus.set(true);
                                            },
                                            span { class: "text-base leading-none", "+" }
                                            "Add FX bus"
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "mt-3 min-h-0 flex-1 space-y-3 overflow-y-auto pr-1",
                            if active_mixer_tab_value == MixerDeckTab::Strips {
                                section { class: "rounded-xl border border-slate-800 bg-slate-950/55 p-2.5",
                                    div { class: "flex flex-wrap items-center justify-between gap-2.5",
                                        div {
                                            h3 { class: "text-sm font-semibold text-white", "Mix bus overview" }
                                        }
                                        span { class: "rounded-md border border-slate-800 bg-slate-900/80 px-2 py-1 text-[10px] uppercase tracking-[0.22em] text-slate-400",
                                            "{current_snapshot.bus_strips.iter().filter(|strip| strip.is_mix_bus()).count()} mix buses"
                                        }
                                    }
                                    div { class: "mt-2 overflow-x-auto overflow-y-hidden pb-1",
                                        if current_snapshot.bus_strips.iter().all(|strip| !strip.is_mix_bus()) {
                                            div { class: "rounded-lg border border-dashed border-slate-800 px-4 py-4 text-sm text-slate-400",
                                                "No buses yet. Add one from the Buses tab."
                                            }
                                        } else {
                                            div { class: "flex min-w-max gap-3",
                                                for bus in current_snapshot.bus_strips.iter().filter(|strip| strip.is_mix_bus()).cloned() {
                                                    BusStatusCard { bus, route_editor_signal: route_editor_strip }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            for (section_title, section_description, strips) in deck_sections.into_iter() {
                                section {
                                    key: "{section_title}",
                                    class: "rounded-xl border border-slate-800 bg-slate-950/55 p-2.5",
                                    div { class: "flex flex-wrap items-center justify-between gap-2.5",
                                        div {
                                            h3 { class: "text-sm font-semibold text-white", "{section_title}" }
                                            if let Some(description) = section_description {
                                                p { class: "mt-0.5 text-xs text-slate-400", "{description}" }
                                            }
                                        }
                                        span { class: "rounded-md border border-slate-800 bg-slate-900/80 px-2 py-1 text-[10px] uppercase tracking-[0.22em] text-slate-400",
                                            "{strips.len()} visible"
                                        }
                                    }
                                    div { class: "mt-2 overflow-x-auto overflow-y-hidden pb-1",
                                        if strips.is_empty() {
                                            div { class: "rounded-lg border border-dashed border-slate-800 px-4 py-4 text-sm text-slate-400",
                                                "Nothing to show in this section yet."
                                            }
                                        } else {
                                            div { class: "flex min-w-max gap-3",
                                                for strip in strips.into_iter() {
                                                    {
                                                        let route_targets = strip
                                                            .routes
                                                            .iter()
                                                            .map(|route| {
                                                                (
                                                                    route.output_id,
                                                                    current_snapshot
                                                                    .route_target_name(strip.id, route.output_id)
                                                                        .unwrap_or("Route target")
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
                                }
                            }
                        }
                    }
                    if let Some(selected_strip) = route_editor_strip_data {
                        RouteEditorPanel {
                            snapshot: current_snapshot.clone(),
                            strip: selected_strip,
                            route_editor_signal: route_editor_strip,
                            snapshot_signal: snapshot,
                        }
                    }
                }
                if show_settings_value {
                    SettingsModal {
                        snapshot: current_snapshot.clone(),
                        show_settings_signal: show_settings,
                        snapshot_signal: snapshot,
                        new_virtual_cable_name_signal: new_virtual_cable_name,
                    }
                }
                if show_create_strip_value {
                    CreateStripModal {
                        snapshot: current_snapshot.clone(),
                        snapshot_signal: snapshot,
                        show_signal: show_create_strip,
                        name_signal: create_strip_name,
                        source_selection_signal: create_strip_source,
                        bus_selection_signal: create_strip_buses,
                    }
                }
                if show_create_fx_bus_value {
                    CreateFxBusModal {
                        snapshot_signal: snapshot,
                        show_signal: show_create_fx_bus,
                        name_signal: create_fx_bus_name,
                        gate_signal: create_fx_bus_gate,
                        compressor_signal: create_fx_bus_compressor,
                        eq_signal: create_fx_bus_eq,
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
    let volume_display_text = strip.volume.as_percent_text();
    let volume_slider_value = format!("{:.1}", strip.volume.as_percentage());
    let has_audio_controls = matches!(
        strip.kind,
        StripKind::Strip | StripKind::Bus | StripKind::Output
    ) && !strip.is_fx_bus();
    let can_rename = strip.kind != StripKind::HardwareSource;
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
    let role_badge_class = match strip.role_label() {
        "Hardware source" => {
            "rounded-md border border-emerald-400/30 bg-emerald-500/10 px-1.5 py-1 text-[10px] font-medium text-emerald-200"
        }
        "Virtual cable" => {
            "rounded-md border border-cyan-400/30 bg-cyan-500/10 px-1.5 py-1 text-[10px] font-medium text-cyan-200"
        }
        "Channel strip" => {
            "rounded-md border border-fuchsia-400/30 bg-fuchsia-500/10 px-1.5 py-1 text-[10px] font-medium text-fuchsia-200"
        }
        "Bus" => {
            "rounded-md border border-violet-400/30 bg-violet-500/10 px-1.5 py-1 text-[10px] font-medium text-violet-200"
        }
        "FX bus" => {
            "rounded-md border border-amber-400/30 bg-amber-500/10 px-1.5 py-1 text-[10px] font-medium text-amber-100"
        }
        "Output bus" => {
            "rounded-md border border-violet-400/30 bg-violet-500/10 px-1.5 py-1 text-[10px] font-medium text-violet-200"
        }
        _ => {
            "rounded-md border border-slate-700 bg-slate-900 px-1.5 py-1 text-[10px] font-medium text-slate-300"
        }
    };
    let effect_summary = if matches!(
        strip.kind,
        StripKind::Strip | StripKind::Bus | StripKind::Output
    ) {
        effect_summary(&strip)
    } else {
        None
    };
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
    let action_grid_class = if has_audio_controls && strip.kind.supports_mono() {
        "mt-2 grid grid-cols-2 gap-1.5"
    } else if has_audio_controls {
        "mt-2 grid grid-cols-1 gap-1.5"
    } else {
        "mt-2 grid grid-cols-1 gap-1.5"
    };
    let assignment_label = if strip.kind == StripKind::Strip {
        snapshot
            .read()
            .assignment_name(strip.input_assignment.as_ref())
            .unwrap_or("Unassigned")
            .to_string()
    } else {
        String::new()
    };
    let (assignment_title, assignment_subtitle) = source_display_lines(&assignment_label);
    let meter_shell_class = if strip.meter_level.as_ratio() > 0.05 {
        "flex h-[clamp(5.5rem,15vh,10rem)] shrink-0 items-center justify-center self-center rounded-lg border border-emerald-400/30 bg-slate-900/80 px-1.5 py-1.5 shadow-[0_0_0_1px_rgba(16,185,129,0.08),0_0_18px_rgba(16,185,129,0.08)]"
    } else {
        "flex h-[clamp(5.5rem,15vh,10rem)] shrink-0 items-center justify-center self-center rounded-lg border border-slate-800 bg-slate-900/70 px-1.5 py-1.5"
    };

    rsx! {
        article {
            key: "{strip.id.as_str()}",
            class: "flex h-full min-h-0 min-w-[196px] w-[196px] flex-col overflow-hidden rounded-xl border border-slate-800 bg-slate-950/80 p-3",
            div { class: "flex items-center justify-between gap-2",
                div { class: "flex min-w-0 flex-wrap items-center gap-1.5",
                    span {
                        class: "rounded-md border border-slate-700 bg-slate-900 px-1.5 py-1 text-[10px] font-medium text-slate-300",
                        title: "{strip.kind.as_str()}",
                        "{compact_strip_kind_label(strip.kind)}"
                    }
                    span {
                        class: "{role_badge_class}",
                        title: "{strip.role_label()}",
                        "{compact_role_label(&strip)}"
                    }
                }
                if has_audio_controls {
                    span { class: "text-xs font-medium text-slate-400", "{volume_display_text}%" }
                }
            }
            input {
                class: "mt-1.5 rounded-lg border border-slate-700 bg-slate-900/90 px-2 py-1.5 text-sm font-medium text-slate-100 outline-none",
                r#type: "text",
                value: "{strip.label}",
                title: "{strip.label}",
                disabled: !can_rename,
                oninput: move |event| {
                    if let Err(error) = rename_engine.send(AudioControlMsg::RenameStrip {
                        strip: strip.id,
                        label: event.value(),
                    }) {
                        snapshot.write().last_notice = error;
                    }
                }
            }
            if strip.kind == StripKind::Strip {
                div { class: "mt-1.5 rounded-lg border border-slate-800 bg-slate-900/60 px-2.5 py-1",
                    div { class: "flex min-w-0 items-center gap-2",
                        span { class: "shrink-0 text-[10px] font-medium uppercase tracking-[0.14em] text-slate-500", "Input" }
                        div {
                            class: "min-w-0 truncate text-xs font-medium text-slate-100",
                            title: "{assignment_label}",
                            "{assignment_title}"
                        }
                    }
                    div { class: "truncate pl-[2.5rem] text-[11px] text-slate-400 min-h-[1rem]",
                        if let Some(subtitle) = assignment_subtitle {
                            "{subtitle}"
                        }
                    }
                }
            }
            div { class: "mt-1.5 flex min-w-0 items-start justify-between gap-2 text-[10px] text-slate-500",
                span { class: "font-medium uppercase tracking-[0.18em]", "{strip_mode}" }
                if let Some(summary) = effect_summary {
                    span {
                        class: "min-w-0 max-w-full truncate rounded-md border border-amber-400/20 bg-amber-500/10 px-1.5 py-1 text-[10px] tracking-[0.08em] text-amber-100",
                        title: "{summary}",
                        "{summary}"
                    }
                }
            }
            if !strip.is_fx_bus() {
            div { class: "mt-2.5 grid grid-cols-[auto_minmax(0,1fr)_auto] items-center justify-items-center gap-2.5",
                div {
                    class: "{meter_shell_class}",
                    style: "{meter_tray_style}",
                    VuMeter {
                        strip_id: strip.id,
                        fallback_channels: strip.meter_channels.len(),
                    }
                }
                if has_audio_controls {
                    div { class: "flex h-[clamp(5.5rem,15vh,10rem)] min-w-0 w-full items-center justify-center self-center",
                        input {
                            class: "h-2 w-[clamp(5.5rem,15vh,10rem)] -rotate-90 cursor-pointer appearance-none rounded-md bg-slate-700 accent-cyan-400",
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
                    div { class: "flex h-[clamp(5.5rem,15vh,10rem)] flex-col justify-between self-center text-[9px] font-medium text-slate-500",
                        span { "+12" }
                        span { "0" }
                        span { "-12" }
                        span { "-24" }
                        span { "-60" }
                    }
                }
            }
            }
            div { class: "{action_grid_class}",
                if has_audio_controls {
                    button {
                        class: "inline-flex items-center justify-center rounded-lg border px-3 py-1.5 text-center text-xs font-medium leading-none {mute_class}",
                        title: if strip.muted { "Unmute" } else { "Mute" },
                        onclick: move |_| {
                            if let Err(error) = mute_engine.send(AudioControlMsg::ToggleMute { strip: strip.id }) {
                                snapshot.write().last_notice = error;
                            }
                        },
                        span { class: "mr-1.5", {mute_icon(strip.muted)} }
                        if strip.muted { "Unmute" } else { "Mute" }
                    }
                }
                if strip.kind.supports_mono() && !strip.is_fx_bus() {
                    button {
                        class: if strip.mono {
                            "inline-flex items-center justify-center rounded-lg border border-amber-400/40 bg-amber-500/20 px-3 py-1.5 text-center text-xs font-medium leading-none text-amber-100"
                        } else {
                            "inline-flex items-center justify-center rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-1.5 text-center text-xs font-medium leading-none text-slate-300"
                        },
                        onclick: move |_| {
                            if let Err(error) = mono_engine.send(AudioControlMsg::ToggleMono { strip: strip.id }) {
                                snapshot.write().last_notice = error;
                            }
                        },
                        span { class: "mr-1.5", {mono_icon()} }
                        "Mono"
                    }
                }
            }
            if !strip.kind.supports_routes() {
                div { class: "mt-1.5 rounded-lg border border-dashed border-slate-800 px-2 py-1.5 text-center text-[10px] font-medium text-slate-500",
                    "{strip.empty_route_hint()}"
                }
            } else if strip.is_bus() && !strip.is_fx_bus() {
                // Mix buses use hardware outputs — always show settings button
                button {
                    class: "{route_button_class}",
                    onclick: move |_| {
                        if route_editor_signal.read().as_ref() == Some(&strip.id) {
                            route_editor_signal.set(None);
                        } else {
                            route_editor_signal.set(Some(strip.id));
                        }
                    },
                    span { "Settings" }
                    span { class: "rounded-md border border-cyan-400/20 bg-slate-950/70 px-1.5 py-0.5 text-[10px] font-medium text-cyan-200",
                        "{strip.hardware_outputs.len()} hw"
                    }
                }
            } else if strip.is_fx_bus() {
                // FX buses always need to be openable for EQ/gate/compressor settings.
                button {
                    class: "{route_button_class}",
                    onclick: move |_| {
                        if route_editor_signal.read().as_ref() == Some(&strip.id) {
                            route_editor_signal.set(None);
                        } else {
                            route_editor_signal.set(Some(strip.id));
                        }
                    },
                    span { "FX settings" }
                    span { class: "rounded-md border border-cyan-400/20 bg-slate-950/70 px-1.5 py-0.5 text-[10px] font-medium text-cyan-200",
                        "{enabled_route_count}/{route_targets.len()}"
                    }
                }
            } else if route_targets.is_empty() {
                div { class: "mt-1.5 rounded-lg border border-dashed border-slate-800 px-2 py-1.5 text-center text-[10px] font-medium text-slate-500",
                    "{strip.empty_route_hint()}"
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
                    span { class: "rounded-md border border-cyan-400/20 bg-slate-950/70 px-1.5 py-0.5 text-[10px] font-medium text-cyan-200",
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
    let midi_volume_engine = engine.clone();
    let clear_volume_engine = engine.clone();
    let midi_mute_engine = engine.clone();
    let clear_mute_engine = engine.clone();
    let bypass_engine = engine.clone();
    let reset_engine = engine.clone();
    let gate_toggle_engine = engine.clone();
    let gate_learn_engine = engine.clone();
    let gate_clear_engine = engine.clone();
    let gate_threshold_engine = engine.clone();
    let gate_threshold_learn_engine = engine.clone();
    let gate_threshold_clear_engine = engine.clone();
    let gate_floor_engine = engine.clone();
    let gate_floor_learn_engine = engine.clone();
    let gate_floor_clear_engine = engine.clone();
    let compressor_toggle_engine = engine.clone();
    let compressor_learn_engine = engine.clone();
    let compressor_clear_engine = engine.clone();
    let compressor_threshold_engine = engine.clone();
    let compressor_threshold_learn_engine = engine.clone();
    let compressor_threshold_clear_engine = engine.clone();
    let compressor_ratio_engine = engine.clone();
    let compressor_ratio_learn_engine = engine.clone();
    let compressor_ratio_clear_engine = engine.clone();
    let compressor_gain_engine = engine.clone();
    let compressor_gain_learn_engine = engine.clone();
    let compressor_gain_clear_engine = engine.clone();
    let eq_toggle_engine = engine.clone();
    let remove_engine = engine.clone();
    let add_hw_engine = engine.clone();
    let route_heading = format!("Route matrix -> {}", strip.route_target_label_plural());
    let route_description = strip.route_hint().to_string();
    let assignment_engine = engine.clone();
    let can_remove = strip.is_mixer_strip() || strip.is_bus();
    let delete_label = if strip.is_fx_bus() {
        "Delete FX bus"
    } else if strip.is_bus() {
        "Delete bus"
    } else {
        "Delete strip"
    };
    let assigned_source = snapshot
        .assignment_name(strip.input_assignment.as_ref())
        .unwrap_or("Unassigned")
        .to_string();
    let (assigned_source_title, assigned_source_subtitle) = source_display_lines(&assigned_source);
    let volume_binding = strip.midi.volume_binding();
    let mute_binding = strip.midi.mute_binding();
    let volume_learning = snapshot.midi_learn_target
        == Some(MidiLearnTarget::Strip {
            strip: strip.id,
            target: MidiControlTarget::Volume,
        });
    let mute_learning = snapshot.midi_learn_target
        == Some(MidiLearnTarget::Strip {
            strip: strip.id,
            target: MidiControlTarget::Mute,
        });
    rsx! {
        div { class: "fixed inset-0 z-40 flex items-start justify-center bg-slate-950/70 p-6 backdrop-blur-sm",
            section { class: "flex h-[min(92vh,980px)] w-full max-w-[88rem] flex-col rounded-2xl border border-slate-800 bg-slate-900/95 p-5 shadow-2xl shadow-black/50",
                div { class: "flex items-start justify-between gap-4 border-b border-slate-800 pb-4",
                    div {
                        p { class: "text-sm uppercase tracking-[0.3em] text-cyan-400", "Routing + effects" }
                        h2 { class: "mt-2 text-xl font-semibold text-white", "{strip.label}" }
                        p { class: "mt-2 text-sm text-slate-400", "{route_description} Keep strip or bus MIDI, route triggers, and FX controls in one settings view." }
                    }
                    button {
                        class: "rounded-lg border border-slate-700 bg-slate-950/70 px-4 py-2 text-sm font-medium text-slate-100",
                        onclick: move |_| route_editor_signal.set(None),
                        "Close"
                    }
                }
                div { class: "mt-4 min-h-0 flex-1 space-y-4 overflow-y-auto pr-1",
                    if strip.kind == StripKind::Strip {
                        article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                            h3 { class: "text-lg font-semibold text-white", "Assigned input" }
                            p { class: "mt-1 text-sm text-slate-400", "Choose the one hardware source or virtual cable that should feed this strip." }
                            div { class: "mt-3 rounded-lg border border-slate-800 bg-slate-900/70 px-3 py-2",
                                div { class: "text-[10px] font-medium uppercase tracking-[0.18em] text-slate-500", "Current input" }
                                div { class: "mt-1 text-sm font-medium text-slate-100", title: "{assigned_source}", "{assigned_source_title}" }
                                if let Some(subtitle) = assigned_source_subtitle {
                                    div { class: "mt-0.5 text-xs text-slate-400", "{subtitle}" }
                                }
                            }
                            div { class: "mt-4 space-y-2",
                                button {
                                    class: if strip.input_assignment.is_none() {
                                        "w-full rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-3 py-3 text-left text-sm font-medium text-cyan-100"
                                    } else {
                                        "w-full rounded-lg border border-slate-800 bg-slate-900/70 px-3 py-3 text-left text-sm text-slate-200"
                                    },
                                    onclick: move |_| {
                                        if let Err(error) = assignment_engine.send(AudioControlMsg::SetStripInputAssignment {
                                            strip: strip.id,
                                            source: None,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    "Clear assignment"
                                }
                                for source in snapshot.source_strips.iter().cloned() {
                                    {
                                        let checked = strip
                                            .input_assignment
                                            .as_ref()
                                            .is_some_and(|assignment| assignment.source_id == source.id);
                                        let assignment_engine = engine.clone();
                                        rsx! {
                                            button {
                                                key: "{source.id.as_str()}",
                                                class: if checked {
                                                    "w-full rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-3 py-3 text-left text-sm font-medium text-cyan-100"
                                                } else {
                                                    "w-full rounded-lg border border-slate-800 bg-slate-900/70 px-3 py-3 text-left text-sm text-slate-200"
                                                },
                                                onclick: move |_| {
                                                    if let Err(error) = assignment_engine.send(AudioControlMsg::SetStripInputAssignment {
                                                        strip: strip.id,
                                                        source: Some(source.id),
                                                    }) {
                                                        snapshot_signal.write().last_notice = error;
                                                    }
                                                },
                                                div { class: "font-medium text-white", "{source.label}" }
                                                div { class: "mt-1 text-[11px] uppercase tracking-[0.22em] text-slate-500", "{source.role_label()}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if strip.kind.supports_volume_and_mute() && !strip.is_fx_bus() {
                        article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                            h3 { class: "text-lg font-semibold text-white", "Mixer MIDI" }
                            p { class: "mt-1 text-sm text-slate-400", "Map the main fader and mute button for this strip or bus directly from its settings." }
                            div { class: "mt-4 grid gap-3 md:grid-cols-2",
                                MidiBindingCard {
                                    title: "Volume".to_string(),
                                    binding_label: format_midi_trigger(volume_binding.as_ref()),
                                    description: Some(format!("Current level {}%", strip.volume.as_percent_text())),
                                    learning: volume_learning,
                                    on_learn: move |_| {
                                        let result = if volume_learning {
                                            midi_volume_engine.send(AudioControlMsg::CancelMidiLearn)
                                        } else {
                                            midi_volume_engine.send(AudioControlMsg::StartMidiLearn {
                                                target: MidiLearnTarget::Strip {
                                                    strip: strip.id,
                                                    target: MidiControlTarget::Volume,
                                                },
                                            })
                                        };
                                        if let Err(error) = result {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    on_clear: move |_| {
                                        if let Err(error) = clear_volume_engine.send(AudioControlMsg::SetMidiBinding {
                                            strip: strip.id,
                                            target: MidiControlTarget::Volume,
                                            binding: None,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                }
                                MidiBindingCard {
                                    title: "Mute".to_string(),
                                    binding_label: format_midi_trigger(mute_binding.as_ref()),
                                    description: Some(if strip.muted { "Currently muted".to_string() } else { "Currently live".to_string() }),
                                    learning: mute_learning,
                                    on_learn: move |_| {
                                        let result = if mute_learning {
                                            midi_mute_engine.send(AudioControlMsg::CancelMidiLearn)
                                        } else {
                                            midi_mute_engine.send(AudioControlMsg::StartMidiLearn {
                                                target: MidiLearnTarget::Strip {
                                                    strip: strip.id,
                                                    target: MidiControlTarget::Mute,
                                                },
                                            })
                                        };
                                        if let Err(error) = result {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    on_clear: move |_| {
                                        if let Err(error) = clear_mute_engine.send(AudioControlMsg::SetMidiBinding {
                                            strip: strip.id,
                                            target: MidiControlTarget::Mute,
                                            binding: None,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                }
                            }
                        }
                    }
                    if strip.is_bus() && !strip.is_fx_bus() {
                        // Mix buses route directly to hardware sinks.
                        article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                            h3 { class: "text-lg font-semibold text-white", "Hardware outputs" }
                            p { class: "mt-1 text-sm text-slate-400",
                                "Route this bus to one or more hardware audio devices. Selecting a device creates a live loopback."
                            }
                            div { class: "mt-4 grid gap-3 sm:grid-cols-[minmax(0,1fr)_auto]",
                                {
                                    let add_engine = add_hw_engine.clone();
                                    rsx! {
                                        select {
                                            class: "rounded-lg border border-slate-700 bg-slate-900/90 px-3 py-3 text-sm text-slate-100 appearance-none outline-none",
                                            value: "",
                                            onchange: move |event| {
                                                let sink = event.value();
                                                if !sink.is_empty() {
                                                    if let Err(error) = add_engine.send(AudioControlMsg::AddBusHardwareOutput {
                                                        strip: strip.id,
                                                        sink_name: sink,
                                                    }) {
                                                        snapshot_signal.write().last_notice = error;
                                                    }
                                                }
                                            },
                                            option { class: "bg-slate-900 text-slate-100", value: "", "— add hardware output —" }
                                            for hw_sink in snapshot.inventory.hardware_sinks.iter() {
                                                if !strip.hardware_outputs.contains(hw_sink) {
                                                    option {
                                                        class: "bg-slate-900 text-slate-100",
                                                        value: "{hw_sink}",
                                                        "{hw_sink}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            if !strip.hardware_outputs.is_empty() {
                                div { class: "mt-3 space-y-2",
                                    for hw_sink in strip.hardware_outputs.iter().cloned() {
                                        {
                                            let remove_hw_engine = engine.clone();
                                            rsx! {
                                                div {
                                                    key: "{hw_sink}",
                                                    class: "flex items-center justify-between gap-3 rounded-lg border border-cyan-400/20 bg-cyan-500/5 px-3 py-2",
                                                    span { class: "text-sm font-medium text-cyan-100 truncate", "{hw_sink}" }
                                                    button {
                                                        class: "shrink-0 rounded-md border border-rose-400/30 bg-rose-500/10 px-2.5 py-1 text-xs font-medium text-rose-200",
                                                        onclick: move |_| {
                                                            if let Err(error) = remove_hw_engine.send(AudioControlMsg::RemoveBusHardwareOutput {
                                                                strip: strip.id,
                                                                sink_name: hw_sink.clone(),
                                                            }) {
                                                                snapshot_signal.write().last_notice = error;
                                                            }
                                                        },
                                                        "Remove"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            } else {
                                div { class: "mt-3 rounded-lg border border-dashed border-slate-700 px-4 py-4 text-sm text-slate-400",
                                    "No hardware outputs assigned. Audio from this bus is available for capture (e.g. in OBS)."
                                }
                            }
                        }
                    } else if strip.kind.supports_routes() && !strip.routes.is_empty() {
                        article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                            div { class: "flex flex-wrap items-center justify-between gap-3",
                                div {
                                    h3 { class: "text-lg font-semibold text-white", "{route_heading}" }
                                   if !strip.is_fx_bus() {
                                       p { class: "mt-1 text-sm text-slate-400", "Each send can carry its own MIDI trigger for buttons or LEDs." }
                                   }
                               }
                           }
                           div { class: "mt-4 space-y-3",
                               for route in strip.routes.iter().cloned() {
                                   {
                                       let output_label = snapshot
                                           .route_target_name(strip.id, route.output_id)
                                           .unwrap_or("Route target")
                                           .to_string();
                                       let route_binding = if !strip.is_fx_bus() { route.binding() } else { None };
                                       let route_learning = !strip.is_fx_bus() && snapshot.midi_learn_target
                                           == Some(MidiLearnTarget::Route {
                                               strip: strip.id,
                                               output: route.output_id,
                                           });
                                       let route_class = if route.enabled {
                                           "flex-1 rounded-lg border border-cyan-400/30 bg-cyan-500/10 px-4 py-3 text-left text-sm font-medium text-cyan-100"
                                       } else {
                                           "flex-1 rounded-lg border border-slate-800 bg-slate-950/70 px-4 py-3 text-left text-sm font-medium text-slate-200"
                                       };
                                       let toggle_engine = engine.clone();
                                       let binding_engine = engine.clone();
                                       let clear_binding_engine = engine.clone();

                                       rsx! {
                                           div {
                                               key: "{strip.id.as_str()}-{route.output_id.as_str()}",
                                               class: if strip.is_fx_bus() { "grid gap-3" } else { "grid gap-3 sm:grid-cols-[minmax(0,1fr)_15rem]" },
                                               button {
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
                                                        div { class: "flex min-w-0 flex-col gap-1 text-left",
                                                            span { class: "truncate", "{output_label}" }
                                                            span { class: "text-[10px] uppercase tracking-[0.22em] text-slate-500", "{strip.route_target_label()}" }
                                                        }
                                                        span { class: "text-[10px] uppercase tracking-[0.25em] text-slate-400 shrink-0",
                                                            if route.enabled { "On" } else { "Off" }
                                                        }
                                                    }
                                                }
                                                if !strip.is_fx_bus() {
                                                div { class: "space-y-2 rounded-lg border border-slate-800 bg-slate-950/70 p-3",
                                                    span { class: "block text-[10px] uppercase tracking-[0.25em] text-slate-500", "Route trigger" }
                                                    div { class: "text-sm text-slate-100", "{format_midi_trigger(route_binding.as_ref())}" }
                                                    div { class: "flex gap-2",
                                                        button {
                                                            class: if route_learning {
                                                                "flex-1 rounded-lg border border-amber-400/40 bg-amber-500/15 px-3 py-2 text-sm font-medium text-amber-100"
                                                            } else {
                                                                "flex-1 rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-3 py-2 text-sm font-medium text-cyan-100"
                                                            },
                                                            onclick: move |_| {
                                                                let result = if route_learning {
                                                                    binding_engine.send(AudioControlMsg::CancelMidiLearn)
                                                                } else {
                                                                    binding_engine.send(AudioControlMsg::StartMidiLearn {
                                                                        target: MidiLearnTarget::Route {
                                                                            strip: strip.id,
                                                                            output: route.output_id,
                                                                        },
                                                                    })
                                                                };
                                                                if let Err(error) = result {
                                                                    snapshot_signal.write().last_notice = error;
                                                                }
                                                            },
                                                            if route_learning { "Listening..." } else { "Learn" }
                                                        }
                                                        button {
                                                            class: "rounded-lg border border-slate-700 bg-slate-900/90 px-3 py-2 text-sm font-medium text-slate-200",
                                                            onclick: move |_| {
                                                                if let Err(error) = clear_binding_engine.send(AudioControlMsg::SetRouteMidiBinding {
                                                                    strip: strip.id,
                                                                    output: route.output_id,
                                                                    binding: None,
                                                                }) {
                                                                    snapshot_signal.write().last_notice = error;
                                                                }
                                                            },
                                                            "Clear"
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
                    }
                    if strip.is_fx_bus() && (strip.effects.gate.enabled || strip.effects.compressor.enabled) {
                        article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                            div { class: "flex flex-wrap items-center justify-between gap-3",
                                div {
                                    h3 { class: "text-lg font-semibold text-white", "FX engine" }
                                    p { class: "mt-1 text-sm text-slate-400", "Bypass or reset the whole chain." }
                                }
                                div { class: "flex flex-wrap gap-2",
                                    button {
                                        class: if strip.effects.bypassed {
                                            "inline-flex items-center justify-center rounded-lg border border-amber-400/40 bg-amber-500/20 px-3 py-2 text-sm font-medium text-amber-100"
                                        } else {
                                            "inline-flex items-center justify-center rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm font-medium text-slate-200"
                                        },
                                        onclick: move |_| {
                                            if let Err(error) = bypass_engine.send(AudioControlMsg::ToggleEffectsBypass { strip: strip.id }) {
                                                snapshot_signal.write().last_notice = error;
                                            }
                                        },
                                        if strip.effects.bypassed { "FX bypassed" } else { "Bypass FX" }
                                    }
                                    button {
                                        class: "inline-flex items-center justify-center rounded-lg border border-rose-400/30 bg-rose-500/10 px-3 py-2 text-sm font-medium text-rose-100",
                                        onclick: move |_| {
                                            if let Err(error) = reset_engine.send(AudioControlMsg::ResetStripEffects { strip: strip.id }) {
                                                snapshot_signal.write().last_notice = error;
                                            }
                                        },
                                        "Reset FX"
                                    }
                                }
                            }
                        }
                        if strip.effects.gate.enabled {
                        article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                            div { class: "flex flex-wrap items-center justify-between gap-3",
                                div {
                                    h3 { class: "text-lg font-semibold text-white", "Noise gate" }
                                    p { class: "mt-1 text-sm text-slate-400", "Clamp low-level signal bleed before it reaches the bus." }
                                }
                                button {
                                    class: if strip.effects.gate.enabled {
                                        "inline-flex items-center justify-center rounded-lg border border-emerald-400/40 bg-emerald-500/15 px-3 py-2 text-sm font-medium text-emerald-100"
                                    } else {
                                        "inline-flex items-center justify-center rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm font-medium text-slate-200"
                                    },
                                    onclick: move |_| {
                                        if let Err(error) = gate_toggle_engine.send(AudioControlMsg::SetNoiseGateEnabled {
                                            strip: strip.id,
                                            enabled: !strip.effects.gate.enabled,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    if strip.effects.gate.enabled { "Gate on" } else { "Gate off" }
                                }
                            }
                            div { class: "mt-4 grid gap-3 lg:grid-cols-[16rem_repeat(2,minmax(0,1fr))]",
                                MidiBindingCard {
                                    title: "Gate enable".to_string(),
                                    binding_label: format_midi_trigger(strip.fx_midi.binding(FxMidiTarget::GateEnabled).as_ref()),
                                    description: Some(if strip.effects.gate.enabled { "Gate is on".to_string() } else { "Gate is off".to_string() }),
                                    learning: snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::GateEnabled }),
                                    on_learn: move |_| {
                                        let result = if snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::GateEnabled }) {
                                            gate_learn_engine.send(AudioControlMsg::CancelMidiLearn)
                                        } else {
                                            gate_learn_engine.send(AudioControlMsg::StartMidiLearn {
                                                target: MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::GateEnabled },
                                            })
                                        };
                                        if let Err(error) = result {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    on_clear: move |_| {
                                        if let Err(error) = gate_clear_engine.send(AudioControlMsg::SetFxMidiBinding {
                                            strip: strip.id,
                                            target: FxMidiTarget::GateEnabled,
                                            binding: None,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                }
                                RotaryKnobCard {
                                    title: "Threshold".to_string(),
                                    value_text: format!("{:.0}%", strip.effects.gate.threshold_percent),
                                    min: 0.0,
                                    max: 100.0,
                                    step: 1.0,
                                    value: strip.effects.gate.threshold_percent,
                                    binding_label: format_midi_trigger(strip.fx_midi.binding(FxMidiTarget::GateThreshold).as_ref()),
                                    learning: snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::GateThreshold }),
                                    on_change: move |value| {
                                        if let Err(error) = gate_threshold_engine.send(AudioControlMsg::SetNoiseGateThreshold {
                                            strip: strip.id,
                                            threshold_percent: value,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    on_learn: move |_| {
                                        let result = if snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::GateThreshold }) {
                                            gate_threshold_learn_engine.send(AudioControlMsg::CancelMidiLearn)
                                        } else {
                                            gate_threshold_learn_engine.send(AudioControlMsg::StartMidiLearn {
                                                target: MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::GateThreshold },
                                            })
                                        };
                                        if let Err(error) = result {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    on_clear: move |_| {
                                        if let Err(error) = gate_threshold_clear_engine.send(AudioControlMsg::SetFxMidiBinding {
                                            strip: strip.id,
                                            target: FxMidiTarget::GateThreshold,
                                            binding: None,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                }
                                RotaryKnobCard {
                                    title: "Floor".to_string(),
                                    value_text: format!("{:.0}%", strip.effects.gate.floor_percent),
                                    min: 0.0,
                                    max: 100.0,
                                    step: 1.0,
                                    value: strip.effects.gate.floor_percent,
                                    binding_label: format_midi_trigger(strip.fx_midi.binding(FxMidiTarget::GateFloor).as_ref()),
                                    learning: snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::GateFloor }),
                                    on_change: move |value| {
                                        if let Err(error) = gate_floor_engine.send(AudioControlMsg::SetNoiseGateFloor {
                                            strip: strip.id,
                                            floor_percent: value,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    on_learn: move |_| {
                                        let result = if snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::GateFloor }) {
                                            gate_floor_learn_engine.send(AudioControlMsg::CancelMidiLearn)
                                        } else {
                                            gate_floor_learn_engine.send(AudioControlMsg::StartMidiLearn {
                                                target: MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::GateFloor },
                                            })
                                        };
                                        if let Err(error) = result {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    on_clear: move |_| {
                                        if let Err(error) = gate_floor_clear_engine.send(AudioControlMsg::SetFxMidiBinding {
                                            strip: strip.id,
                                            target: FxMidiTarget::GateFloor,
                                            binding: None,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                }
                            }
                        }
                        }
                        if strip.effects.compressor.enabled {
                        article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                            div { class: "flex flex-wrap items-center justify-between gap-3",
                                div {
                                    h3 { class: "text-lg font-semibold text-white", "Compressor" }
                                    p { class: "mt-1 text-sm text-slate-400", "Tame peaks and add make-up gain to keep the strip controlled." }
                                }
                                button {
                                    class: if strip.effects.compressor.enabled {
                                        "inline-flex items-center justify-center rounded-lg border border-emerald-400/40 bg-emerald-500/15 px-3 py-2 text-sm font-medium text-emerald-100"
                                    } else {
                                        "inline-flex items-center justify-center rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm font-medium text-slate-200"
                                    },
                                    onclick: move |_| {
                                        if let Err(error) = compressor_toggle_engine.send(AudioControlMsg::SetCompressorEnabled {
                                            strip: strip.id,
                                            enabled: !strip.effects.compressor.enabled,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    if strip.effects.compressor.enabled { "Comp on" } else { "Comp off" }
                                }
                            }
                            div { class: "mt-4 grid gap-3 lg:grid-cols-[16rem_repeat(3,minmax(0,1fr))]",
                                MidiBindingCard {
                                    title: "Comp enable".to_string(),
                                    binding_label: format_midi_trigger(strip.fx_midi.binding(FxMidiTarget::CompressorEnabled).as_ref()),
                                    description: Some(if strip.effects.compressor.enabled { "Compressor is on".to_string() } else { "Compressor is off".to_string() }),
                                    learning: snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::CompressorEnabled }),
                                    on_learn: move |_| {
                                        let result = if snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::CompressorEnabled }) {
                                            compressor_learn_engine.send(AudioControlMsg::CancelMidiLearn)
                                        } else {
                                            compressor_learn_engine.send(AudioControlMsg::StartMidiLearn {
                                                target: MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::CompressorEnabled },
                                            })
                                        };
                                        if let Err(error) = result {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    on_clear: move |_| {
                                        if let Err(error) = compressor_clear_engine.send(AudioControlMsg::SetFxMidiBinding {
                                            strip: strip.id,
                                            target: FxMidiTarget::CompressorEnabled,
                                            binding: None,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                }
                                RotaryKnobCard {
                                    title: "Threshold".to_string(),
                                    value_text: format!("{:.0}%", strip.effects.compressor.threshold_percent),
                                    min: 0.0,
                                    max: 100.0,
                                    step: 1.0,
                                    value: strip.effects.compressor.threshold_percent,
                                    binding_label: format_midi_trigger(strip.fx_midi.binding(FxMidiTarget::CompressorThreshold).as_ref()),
                                    learning: snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::CompressorThreshold }),
                                    on_change: move |value| {
                                        if let Err(error) = compressor_threshold_engine.send(AudioControlMsg::SetCompressorThreshold {
                                            strip: strip.id,
                                            threshold_percent: value,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    on_learn: move |_| {
                                        let result = if snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::CompressorThreshold }) {
                                            compressor_threshold_learn_engine.send(AudioControlMsg::CancelMidiLearn)
                                        } else {
                                            compressor_threshold_learn_engine.send(AudioControlMsg::StartMidiLearn {
                                                target: MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::CompressorThreshold },
                                            })
                                        };
                                        if let Err(error) = result {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    on_clear: move |_| {
                                        if let Err(error) = compressor_threshold_clear_engine.send(AudioControlMsg::SetFxMidiBinding {
                                            strip: strip.id,
                                            target: FxMidiTarget::CompressorThreshold,
                                            binding: None,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                }
                                RotaryKnobCard {
                                    title: "Ratio".to_string(),
                                    value_text: format!("{:.1}:1", strip.effects.compressor.ratio),
                                    min: 1.0,
                                    max: 20.0,
                                    step: 0.5,
                                    value: strip.effects.compressor.ratio,
                                    binding_label: format_midi_trigger(strip.fx_midi.binding(FxMidiTarget::CompressorRatio).as_ref()),
                                    learning: snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::CompressorRatio }),
                                    on_change: move |value| {
                                        if let Err(error) = compressor_ratio_engine.send(AudioControlMsg::SetCompressorRatio {
                                            strip: strip.id,
                                            ratio: value,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    on_learn: move |_| {
                                        let result = if snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::CompressorRatio }) {
                                            compressor_ratio_learn_engine.send(AudioControlMsg::CancelMidiLearn)
                                        } else {
                                            compressor_ratio_learn_engine.send(AudioControlMsg::StartMidiLearn {
                                                target: MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::CompressorRatio },
                                            })
                                        };
                                        if let Err(error) = result {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    on_clear: move |_| {
                                        if let Err(error) = compressor_ratio_clear_engine.send(AudioControlMsg::SetFxMidiBinding {
                                            strip: strip.id,
                                            target: FxMidiTarget::CompressorRatio,
                                            binding: None,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                }
                                RotaryKnobCard {
                                    title: "Make-up".to_string(),
                                    value_text: format!("{:.1} dB", strip.effects.compressor.makeup_gain_db),
                                    min: 0.0,
                                    max: 24.0,
                                    step: 0.5,
                                    value: strip.effects.compressor.makeup_gain_db,
                                    binding_label: format_midi_trigger(strip.fx_midi.binding(FxMidiTarget::CompressorMakeupGain).as_ref()),
                                    learning: snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::CompressorMakeupGain }),
                                    on_change: move |value| {
                                        if let Err(error) = compressor_gain_engine.send(AudioControlMsg::SetCompressorMakeupGain {
                                            strip: strip.id,
                                            gain_db: value,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    on_learn: move |_| {
                                        let result = if snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::CompressorMakeupGain }) {
                                            compressor_gain_learn_engine.send(AudioControlMsg::CancelMidiLearn)
                                        } else {
                                            compressor_gain_learn_engine.send(AudioControlMsg::StartMidiLearn {
                                                target: MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::CompressorMakeupGain },
                                            })
                                        };
                                        if let Err(error) = result {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    on_clear: move |_| {
                                        if let Err(error) = compressor_gain_clear_engine.send(AudioControlMsg::SetFxMidiBinding {
                                            strip: strip.id,
                                            target: FxMidiTarget::CompressorMakeupGain,
                                            binding: None,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                }
                            }
                        }
                        }
                    }
                    if strip.is_fx_bus() {
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                            div { class: "flex flex-wrap items-center justify-between gap-3",
                                div {
                                    h3 { class: "text-lg font-semibold text-white", "8-band EQ" }
                                    p { class: "mt-1 text-sm text-slate-400", "Shape the return with eight fixed bands from 63 Hz through 8 kHz." }
                                }
                                button {
                                    class: if strip.effects.eq.enabled {
                                        "inline-flex items-center justify-center rounded-lg border border-emerald-400/40 bg-emerald-500/15 px-3 py-2 text-sm font-medium text-emerald-100"
                                    } else {
                                        "inline-flex items-center justify-center rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm font-medium text-slate-200"
                                    },
                                    onclick: move |_| {
                                        if let Err(error) = eq_toggle_engine.send(AudioControlMsg::SetEqEnabled {
                                            strip: strip.id,
                                            enabled: !strip.effects.eq.enabled,
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    if strip.effects.eq.enabled { "EQ on" } else { "EQ off" }
                                }
                            }
                            div { class: "mt-4 space-y-4",
                                if strip.effects.eq.enabled {
                                div { class: "flex items-center gap-3",
                                    label { class: "text-sm text-slate-400", "Preset" }
                                    {
                                        let preset_engine = engine.clone();
                                        let eq_presets = snapshot.eq_presets.clone();
                                        rsx! {
                                            select {
                                                class: "rounded appearance-none bg-slate-800 border border-slate-600 px-2 py-1 text-sm text-slate-100",
                                                value: "",
                                                onchange: move |event| {
                                                    let name = event.value();
                                                    let preset = eq_presets.iter().find(|p| p.name == name).cloned();
                                                    if let Some(preset) = preset {
                                                        if let Err(error) = preset_engine.send(AudioControlMsg::SetEqPreset {
                                                            strip: strip.id,
                                                            preset,
                                                        }) {
                                                            snapshot_signal.write().last_notice = error;
                                                        }
                                                    }
                                                },
                                                option { class: "bg-slate-900 text-slate-100", value: "", "— select preset —" }
                                                for preset in &snapshot.eq_presets {
                                                    option {
                                                        class: "bg-slate-900 text-slate-100",
                                                        value: "{preset.name}",
                                                        { preset.name.as_str() }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                div { class: "grid gap-3 grid-cols-2 md:grid-cols-4 xl:grid-cols-8",
                                    for band in EqBand::ALL {
                                        {
                                            let value = strip.effects.eq.gain_db(band);
                                            let change_engine = engine.clone();
                                            rsx! {
                                                SliderControlCard {
                                                    key: "{strip.id.as_str()}-eq-{band.label()}",
                                                    title: band.label().to_string(),
                                                    value_text: format!("{value:+.1} dB"),
                                                    min: -12.0,
                                                    max: 12.0,
                                                    step: 0.5,
                                                    value,
                                                    on_change: move |value| {
                                                        if let Err(error) = change_engine.send(AudioControlMsg::SetEqBandGain {
                                                            strip: strip.id,
                                                            band,
                                                            gain_db: value,
                                                        }) {
                                                            snapshot_signal.write().last_notice = error;
                                                        }
                                                    },
                                                }
                                            }
                                        }
                                    }
                                }
                                }
                            }
                        }
                    }
                    if can_remove {
                        article { class: "rounded-xl border border-rose-400/20 bg-rose-500/5 p-4",
                            h3 { class: "text-lg font-semibold text-white", "Management" }
                            p { class: "mt-1 text-sm text-slate-400", "Remove this strip or bus from its settings so the main card actions stay focused on mix controls." }
                            div { class: "mt-4 flex justify-end",
                                button {
                                    class: "inline-flex items-center justify-center rounded-lg border border-rose-400/30 bg-rose-500/10 px-4 py-2 text-sm font-medium text-rose-100",
                                    onclick: move |_| {
                                        route_editor_signal.set(None);
                                        if let Err(error) = remove_engine.send(AudioControlMsg::RemoveStrip { strip: strip.id }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    "{delete_label}"
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
    show_settings_signal: Signal<bool>,
    snapshot_signal: Signal<AudioEngineState>,
    new_virtual_cable_name_signal: Signal<String>,
) -> Element {
    let engine = use_context::<SharedEngineBridge>();
    let midi_feedback_output_engine = engine.clone();
    let midi_feedback_sync_engine = engine.clone();
    let reset_mixer_engine = engine.clone();
    let add_virtual_cable_engine = engine.clone();
    let remove_cable_engine = engine.clone();
    let mut reset_confirm = use_signal(|| false);
    let midi_feedback_output = snapshot
        .midi_feedback
        .output_port_name
        .clone()
        .unwrap_or_default();
    let new_virtual_cable_name = new_virtual_cable_name_signal.read().clone();
    let virtual_cables = snapshot
        .source_strips
        .iter()
        .filter(|source| source.is_virtual_cable())
        .cloned()
        .collect::<Vec<_>>();
    let fx_buses = snapshot
        .bus_strips
        .iter()
        .filter(|bus| bus.is_fx_bus())
        .cloned()
        .collect::<Vec<_>>();
    let virtual_cable_destinations = virtual_cables
        .iter()
        .map(|source| {
            (
                source
                    .pipewire_node_name
                    .clone()
                    .unwrap_or_else(|| source.label.clone()),
                source.label.clone(),
            )
        })
        .chain(fx_buses.iter().map(|bus| {
            (
                bus.pipewire_node_name
                    .clone()
                    .unwrap_or_else(|| bus.label.clone()),
                format!("FX: {}", bus.label),
            )
        }))
        .collect::<Vec<_>>();
    rsx! {
        div { class: "fixed inset-0 z-40 flex items-start justify-center bg-slate-950/70 p-6 backdrop-blur-sm",
            section { class: "flex h-[min(92vh,980px)] w-full max-w-5xl flex-col rounded-2xl border border-slate-800 bg-slate-900/95 p-4 shadow-2xl shadow-black/50",
                div { class: "flex items-start justify-between gap-4 border-b border-slate-800 pb-4",
                    div {
                        p { class: "text-sm uppercase tracking-[0.3em] text-cyan-400", "Settings" }
                        h2 { class: "mt-2 text-xl font-semibold text-white", "Application routing, MIDI + runtime inventory" }
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
                        h3 { class: "text-lg font-semibold text-white", "Virtual cables" }
                        p { class: "mt-2 text-sm text-slate-400", "Create app-managed virtual cables here. Strips then pick one input source or cable from their create/route modal." }
                        div { class: "mt-4 grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto]",
                            input {
                                class: "w-full rounded-lg border border-slate-700 bg-slate-900/90 px-4 py-3 text-sm text-slate-100 outline-none",
                                r#type: "text",
                                value: "{new_virtual_cable_name}",
                                oninput: move |event| new_virtual_cable_name_signal.set(event.value()),
                            }
                            button {
                                class: "inline-flex items-center justify-center rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-4 py-3 text-sm font-medium text-cyan-100",
                                onclick: move |_| {
                                    let label = new_virtual_cable_name_signal.read().clone();
                                    new_virtual_cable_name_signal.set(String::new());
                                    if let Err(error) = add_virtual_cable_engine.send(AudioControlMsg::AddVirtualCable { label }) {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                                "Add virtual cable"
                            }
                        }
                        if !virtual_cables.is_empty() {
                            div { class: "mt-3 space-y-2",
                                for cable in virtual_cables.iter().cloned() {
                                    {
                                       let remove_cable_engine = remove_cable_engine.clone();
                                       rsx! {
                                           div {
                                               key: "{cable.id.as_str()}",
                                               class: "flex items-center justify-between gap-3 rounded-lg border border-slate-800 bg-slate-900/60 px-3 py-2",
                                               span { class: "text-sm font-medium text-slate-100", "{cable.label}" }
                                               button {
                                                   class: "rounded-md border border-rose-400/30 bg-rose-500/10 px-2.5 py-1 text-xs font-medium text-rose-200",
                                                   onclick: move |_| {
                                                       if let Err(error) = remove_cable_engine.send(AudioControlMsg::RemoveStrip { strip: cable.id }) {
                                                           snapshot_signal.write().last_notice = error;
                                                       }
                                                   },
                                                   "Remove"
                                               }
                                           }
                                       }
                                    }
                                }
                            }
                        }
                    }
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        h3 { class: "text-lg font-semibold text-white", "Application routing" }
                        p { class: "mt-2 text-sm text-slate-400", "Move live application playback streams into Pipemeeter virtual cables without leaving the app." }
                        div { class: "mt-3 rounded-lg border border-slate-800 bg-slate-900/80 px-4 py-3 text-sm text-slate-300",
                            "{snapshot.inventory.application_stream_status}"
                        }
                        div { class: "mt-4 space-y-3",
                            if snapshot.inventory.application_streams.is_empty() {
                                div { class: "rounded-lg border border-dashed border-slate-700 px-4 py-5 text-sm text-slate-400",
                                    "No application streams are active right now."
                                }
                            } else {
                                for stream in snapshot.inventory.application_streams.iter().cloned() {
                                    ApplicationStreamRow {
                                        stream,
                                        destinations: virtual_cable_destinations.clone(),
                                        snapshot_signal,
                                    }
                                }
                            }
                        }
                    }
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        h3 { class: "text-lg font-semibold text-white", "MIDI feedback" }
                        p { class: "mt-2 text-sm text-slate-400", "Select the controller output used for LEDs and push a full resync after binding changes." }
                        div { class: "mt-4 grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto]",
                            label { class: "space-y-1",
                                span { class: "block text-[10px] uppercase tracking-[0.25em] text-slate-500", "Feedback output" }
                                select {
                                    class: "w-full appearance-none rounded-lg border border-slate-700 bg-slate-900/90 px-3 py-3 text-sm text-slate-100 outline-none",
                                    value: "{midi_feedback_output}",
                                    oninput: move |event| {
                                        let value = event.value();
                                        if let Err(error) = midi_feedback_output_engine.send(AudioControlMsg::SetMidiFeedbackOutput {
                                            port_name: if value.trim().is_empty() {
                                                None
                                            } else {
                                                Some(value)
                                            },
                                        }) {
                                            snapshot_signal.write().last_notice = error;
                                        }
                                    },
                                    option { value: "", class: "bg-slate-900 text-slate-100", "Disabled" }
                                    for port in snapshot.inventory.midi_outputs.iter() {
                                        option { key: "{port.name}", value: "{port.name}", class: "bg-slate-900 text-slate-100", "{port.name}" }
                                    }
                                }
                            }
                            button {
                                class: "mt-[1.35rem] inline-flex items-center justify-center rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-4 py-3 text-sm font-medium text-cyan-100",
                                onclick: move |_| {
                                    if let Err(error) = midi_feedback_sync_engine.send(AudioControlMsg::SyncMidiFeedback) {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                                "Resync LEDs"
                            }
                        }
                        div { class: "mt-3 rounded-lg border border-slate-800 bg-slate-900/80 px-4 py-3 text-sm text-slate-300",
                            "{snapshot.inventory.midi_feedback_status}"
                        }
                        div { class: "mt-3 rounded-lg border border-slate-800 bg-slate-900/60 p-4",
                            div { class: "flex items-center justify-between gap-3",
                                h4 { class: "text-sm font-medium text-white", "Recent MIDI output debug" }
                                span { class: "text-[11px] uppercase tracking-[0.25em] text-slate-500",
                                    "{snapshot.midi_feedback.output_port_name.as_deref().unwrap_or(\"disabled\")}"
                                }
                            }
                            p { class: "mt-2 text-sm text-slate-400", "Most recent MIDI feedback batches sent to the selected controller output." }
                            div { class: "mt-3 space-y-2",
                                if snapshot.inventory.midi_feedback_debug.is_empty() {
                                    div { class: "rounded-lg border border-dashed border-slate-700 px-4 py-4 text-sm text-slate-400",
                                        "No MIDI feedback has been sent yet."
                                    }
                                } else {
                                    for (index, entry) in snapshot.inventory.midi_feedback_debug.iter().enumerate() {
                                        div { key: "midi-debug-{index}", class: "rounded-lg border border-slate-800 bg-slate-950/70 px-4 py-3 font-mono text-xs text-slate-200",
                                            "{entry}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        h3 { class: "text-lg font-semibold text-white", "Runtime inventory" }
                        p { class: "mt-2 text-sm text-slate-400", "PipeWire and MIDI discovery stay available here without crowding the mixer window." }
                        div { class: "mt-4 grid gap-3 sm:grid-cols-4",
                            InventoryBlock { label: "PipeWire".to_string(), message: snapshot.inventory.pipewire_status.clone() }
                            InventoryBlock { label: "Apps".to_string(), message: snapshot.inventory.application_stream_status.clone() }
                            InventoryBlock { label: "MIDI".to_string(), message: snapshot.inventory.midi_status.clone() }
                            InventoryBlock { label: "Feedback".to_string(), message: snapshot.inventory.midi_feedback_status.clone() }
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
                                for port in snapshot.inventory.midi_inputs.iter() {
                                    div { key: "{port.name}", class: "rounded-lg border border-slate-800 bg-slate-900/80 px-4 py-3 text-sm text-slate-200",
                                        "{port.name}"
                                    }
                                }
                            }
                        }
                    }
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        h3 { class: "text-lg font-semibold text-white", "Detected MIDI outputs" }
                        p { class: "mt-2 text-sm text-slate-400", "Use one of these ports for controller LED feedback." }
                        div { class: "mt-4 space-y-3",
                            if snapshot.inventory.midi_outputs.is_empty() {
                                div { class: "rounded-lg border border-dashed border-slate-700 px-4 py-5 text-sm text-slate-400",
                                    "No MIDI feedback outputs are visible yet."
                                }
                            } else {
                                for port in snapshot.inventory.midi_outputs.iter() {
                                    div { key: "{port.name}", class: "rounded-lg border border-slate-800 bg-slate-900/80 px-4 py-3 text-sm text-slate-200",
                                        "{port.name}"
                                    }
                                }
                            }
                        }
                    }
                    article { class: "rounded-xl border border-rose-900/60 bg-rose-950/30 p-4",
                        div { class: "flex items-center gap-2",
                            span { class: "text-base", "⚠️" }
                            h3 { class: "text-lg font-semibold text-rose-200", "Danger zone" }
                        }
                        p { class: "mt-2 text-sm text-rose-300/80",
                            "Resetting the mixer will permanently delete all virtual cables, channel strips, buses, and MIDI bindings you have created. Hardware audio sources will be kept. This cannot be undone."
                        }
                        div { class: "mt-3 rounded-lg border border-rose-800/50 bg-rose-900/20 px-4 py-3 text-sm text-rose-200/70",
                            "⚠️  All your strips, buses, virtual cables and routing will be lost. Your PipeWire/PulseAudio session will not be modified until after the next scan, but the configuration will be cleared immediately."
                        }
                        div { class: "mt-4",
                            if *reset_confirm.read() {
                                div { class: "flex flex-col gap-3",
                                    p { class: "text-sm font-semibold text-rose-200", "Are you absolutely sure? This will wipe your entire mixer configuration." }
                                    div { class: "flex gap-3",
                                        button {
                                            class: "inline-flex items-center justify-center rounded-lg border border-rose-400/60 bg-rose-600/30 px-5 py-2.5 text-sm font-semibold text-rose-100",
                                            onclick: move |_| {
                                                reset_confirm.set(false);
                                                if let Err(error) = reset_mixer_engine.send(AudioControlMsg::ResetMixer) {
                                                    snapshot_signal.write().last_notice = error;
                                                }
                                            },
                                            "Yes, reset everything"
                                        }
                                        button {
                                            class: "inline-flex items-center justify-center rounded-lg border border-slate-700 bg-slate-900/80 px-5 py-2.5 text-sm font-medium text-slate-200",
                                            onclick: move |_| reset_confirm.set(false),
                                            "Cancel"
                                        }
                                    }
                                }
                            } else {
                                button {
                                    class: "inline-flex items-center justify-center rounded-lg border border-rose-700/60 bg-rose-900/30 px-4 py-2.5 text-sm font-medium text-rose-300",
                                    onclick: move |_| reset_confirm.set(true),
                                    "Reset mixer…"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
