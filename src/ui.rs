use crate::audio::{
    ApplicationStreamInfo, AudioControlMsg, AudioEngineState, AudioUpdateMsg, FxMidiTarget,
    MidiControlTarget, MidiLearnTarget, MidiMessageKind, MidiTrigger, MixerStrip, PipeWireNodeInfo,
    SharedEngineBridge, StripId, StripKind,
};
use dioxus::prelude::*;
use dioxus_desktop::{
    Config, LogicalSize, WindowBuilder, launch::launch as launch_desktop, tao::window::Icon,
};
use std::{any::Any, env, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MixerDeckTab {
    Strips,
    Buses,
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
        .with_inner_size(LogicalSize::new(1520.0, 920.0))
        .with_min_inner_size(LogicalSize::new(1400.0, 860.0))
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

fn app_icon() -> Option<Icon> {
    let size = 128_u32;
    let mut rgba = vec![0_u8; (size * size * 4) as usize];
    let scale = size as f32 / 512.0;
    let inset = 64.0 * scale;

    fill_rounded_rect(
        &mut rgba,
        size,
        0.0,
        0.0,
        size as f32,
        size as f32,
        64.0 * scale,
        [15, 23, 42, 255],
    );

    for (y, color) in [
        (96.0, [51, 65, 85, 255]),
        (160.0, [56, 189, 248, 255]),
        (224.0, [74, 222, 128, 255]),
        (288.0, [51, 65, 85, 255]),
    ] {
        fill_horizontal_capsule(
            &mut rgba,
            size,
            inset + (64.0 * scale),
            inset + (320.0 * scale),
            inset + (y * scale),
            24.0 * scale,
            color,
        );
    }

    fill_convex_quad(
        &mut rgba,
        size,
        [
            (inset + (220.0 * scale), inset + (48.0 * scale)),
            (inset + (320.0 * scale), inset + (48.0 * scale)),
            (inset + (240.0 * scale), inset + (336.0 * scale)),
            (inset + (140.0 * scale), inset + (336.0 * scale)),
        ],
        [248, 250, 252, 255],
    );

    fill_circle(
        &mut rgba,
        size,
        inset + (205.0 * scale),
        inset + (160.0 * scale),
        16.0 * scale,
        [14, 165, 233, 255],
    );
    fill_circle(
        &mut rgba,
        size,
        inset + (190.0 * scale),
        inset + (224.0 * scale),
        16.0 * scale,
        [34, 197, 94, 255],
    );

    Icon::from_rgba(rgba, size, size).ok()
}

fn fill_rounded_rect(
    rgba: &mut [u8],
    size: u32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: [u8; 4],
) {
    paint_shape(rgba, size, color, |px, py| {
        let left = x;
        let right = x + width;
        let top = y;
        let bottom = y + height;

        if px < left || px > right || py < top || py > bottom {
            return false;
        }

        let inner_left = left + radius;
        let inner_right = right - radius;
        let inner_top = top + radius;
        let inner_bottom = bottom - radius;

        (px >= inner_left && px <= inner_right)
            || (py >= inner_top && py <= inner_bottom)
            || circle_contains(px, py, inner_left, inner_top, radius)
            || circle_contains(px, py, inner_right, inner_top, radius)
            || circle_contains(px, py, inner_left, inner_bottom, radius)
            || circle_contains(px, py, inner_right, inner_bottom, radius)
    });
}

fn fill_horizontal_capsule(
    rgba: &mut [u8],
    size: u32,
    x1: f32,
    x2: f32,
    y: f32,
    thickness: f32,
    color: [u8; 4],
) {
    let radius = thickness / 2.0;
    paint_shape(rgba, size, color, |px, py| {
        (px >= x1 && px <= x2 && py >= y - radius && py <= y + radius)
            || circle_contains(px, py, x1, y, radius)
            || circle_contains(px, py, x2, y, radius)
    });
}

fn fill_circle(rgba: &mut [u8], size: u32, cx: f32, cy: f32, radius: f32, color: [u8; 4]) {
    paint_shape(rgba, size, color, |px, py| {
        circle_contains(px, py, cx, cy, radius)
    });
}

fn fill_convex_quad(rgba: &mut [u8], size: u32, points: [(f32, f32); 4], color: [u8; 4]) {
    paint_shape(rgba, size, color, |px, py| {
        point_in_convex_polygon(px, py, &points)
    });
}

fn paint_shape<F>(rgba: &mut [u8], size: u32, color: [u8; 4], contains: F)
where
    F: Fn(f32, f32) -> bool,
{
    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if contains(px, py) {
                let index = ((y * size + x) * 4) as usize;
                rgba[index..index + 4].copy_from_slice(&color);
            }
        }
    }
}

fn circle_contains(px: f32, py: f32, cx: f32, cy: f32, radius: f32) -> bool {
    let dx = px - cx;
    let dy = py - cy;
    (dx * dx) + (dy * dy) <= radius * radius
}

fn point_in_convex_polygon(px: f32, py: f32, points: &[(f32, f32)]) -> bool {
    let mut previous_sign = 0.0_f32;

    for index in 0..points.len() {
        let (x1, y1) = points[index];
        let (x2, y2) = points[(index + 1) % points.len()];
        let cross = (x2 - x1) * (py - y1) - (y2 - y1) * (px - x1);
        if cross.abs() < f32::EPSILON {
            continue;
        }
        if previous_sign == 0.0 {
            previous_sign = cross.signum();
            continue;
        }
        if cross.signum() != previous_sign {
            return false;
        }
    }

    true
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
    let mut route_editor_strip = use_signal(|| None::<StripId>);
    let new_virtual_cable_name = use_signal(String::new);
    let mut new_bus_name = use_signal(String::new);
    let new_output_name = use_signal(String::new);
    let mut create_strip_name = use_signal(String::new);
    let mut create_strip_source = use_signal(|| None::<StripId>);
    let mut create_strip_buses = use_signal(Vec::<StripId>::new);
    let midi_test_controller = use_signal(String::new);
    let midi_test_value = use_signal(|| "127".to_string());

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
    let route_editor_value = *route_editor_strip.read();
    let new_bus_value = new_bus_name.read().clone();
    let midi_test_controller_value = midi_test_controller.read().clone();
    let midi_test_value_value = midi_test_value.read().clone();
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
            "Each strip picks one hardware source or virtual cable in its settings, then sends onward to buses.".to_string(),
            current_snapshot.input_strips.clone(),
        )],
        MixerDeckTab::Buses => vec![(
            "Buses".to_string(),
            "Buses collect strip sends and map their mix onward to system or app-managed outputs.".to_string(),
            current_snapshot.bus_strips.clone(),
        )],
    };
    let deck_notice = match active_mixer_tab_value {
        MixerDeckTab::Strips => {
            "Assign one source inside each strip, send strips to buses, and keep the bus overview visible for live mix status."
        }
        MixerDeckTab::Buses => {
            "Shape each bus and map it to system or app-managed outputs without crowding the strip view."
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
                        div { class: "grid grid-cols-2 gap-1.5 text-sm sm:grid-cols-3 xl:min-w-[40rem] xl:grid-cols-6",
                            SummaryCard { title: "Sources".to_string(), value: current_snapshot.source_strips.len().to_string(), description: "Inputs".to_string() }
                            SummaryCard { title: "Strips".to_string(), value: current_snapshot.input_strips.len().to_string(), description: "Channels".to_string() }
                            SummaryCard { title: "Buses".to_string(), value: current_snapshot.bus_strips.len().to_string(), description: "Mixes".to_string() }
                            SummaryCard { title: "Outputs".to_string(), value: current_snapshot.output_strips.len().to_string(), description: "Destinations".to_string() }
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
                                }
                                div { class: "flex flex-col gap-2 sm:flex-row sm:items-center",
                                    if active_mixer_tab_value == MixerDeckTab::Buses {
                                        div { class: "relative w-full sm:w-64",
                                            span { class: "pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-sm text-slate-500", "+" }
                                            input {
                                                class: "w-full rounded-lg border border-slate-700 bg-slate-950/80 py-1.5 pl-8 pr-3 text-sm text-slate-100 outline-none placeholder:text-slate-500",
                                                r#type: "text",
                                                placeholder: "New bus name",
                                                value: "{new_bus_value}",
                                                oninput: move |event| new_bus_name.set(event.value()),
                                            }
                                        }
                                        button {
                                            class: "inline-flex items-center justify-center gap-2 rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-3 py-1.5 text-sm font-medium text-cyan-100",
                                            onclick: move |_| {
                                                let label = new_bus_name.read().clone();
                                                new_bus_name.set(String::new());
                                                if let Err(error) = add_bus_engine.send(AudioControlMsg::AddBus { label }) {
                                                    snapshot.write().last_notice = error;
                                                }
                                            },
                                            span { class: "text-base leading-none", "+" }
                                            "Add bus"
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
                                            h3 { class: "text-sm font-semibold text-white", "Bus overview" }
                                            p { class: "mt-0.5 text-xs text-slate-400", "Keep an eye on bus VU, volume, and mute state without leaving the strips view." }
                                        }
                                        span { class: "rounded-md border border-slate-800 bg-slate-900/80 px-2 py-1 text-[10px] uppercase tracking-[0.22em] text-slate-400",
                                            "{current_snapshot.bus_strips.len()} buses"
                                        }
                                    }
                                    div { class: "mt-2 overflow-x-auto overflow-y-hidden pb-1",
                                        if current_snapshot.bus_strips.is_empty() {
                                            div { class: "rounded-lg border border-dashed border-slate-800 px-4 py-4 text-sm text-slate-400",
                                                "No buses yet. Add one from the Buses tab."
                                            }
                                        } else {
                                            div { class: "flex min-w-max gap-3",
                                                for bus in current_snapshot.bus_strips.iter().cloned() {
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
                                            p { class: "mt-0.5 text-xs text-slate-400", "{section_description}" }
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
                                                                        .route_target_name(strip.kind, route.output_id)
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
                        midi_test_controller: midi_test_controller_value,
                        midi_test_value: midi_test_value_value,
                        show_settings_signal: show_settings,
                        snapshot_signal: snapshot,
                        midi_test_controller_signal: midi_test_controller,
                        midi_test_value_signal: midi_test_value,
                        new_virtual_cable_name_signal: new_virtual_cable_name,
                        new_output_name_signal: new_output_name,
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
            }
        }
    }
}

#[component]
fn BusStatusCard(bus: MixerStrip, route_editor_signal: Signal<Option<StripId>>) -> Element {
    let meter_tray_style = format!(
        "width: {}px;",
        16 + (bus.meter_channels.len().max(1) as i32 * 8)
            + ((bus.meter_channels.len().saturating_sub(1) as i32) * 4)
    );
    let enabled_route_count = bus.routes.iter().filter(|route| route.enabled).count();
    let mute_badge_class = if bus.muted {
        "rounded-md border border-rose-400/40 bg-rose-500/15 px-2 py-1 text-[10px] font-medium text-rose-100"
    } else {
        "rounded-md border border-emerald-400/30 bg-emerald-500/10 px-2 py-1 text-[10px] font-medium text-emerald-100"
    };

    rsx! {
        article {
            key: "{bus.id.as_str()}",
            class: "flex min-w-[176px] w-[176px] flex-col rounded-xl border border-slate-800 bg-slate-900/75 p-2.5",
            div { class: "flex items-start justify-between gap-3",
                div { class: "min-w-0",
                    div { class: "truncate text-sm font-semibold text-white", title: "{bus.label}", "{bus.label}" }
                    div { class: "mt-0.5 text-[10px] uppercase tracking-[0.14em] text-violet-200", "Bus" }
                }
                span { class: "text-xs font-medium text-slate-400", "{bus.volume.as_percent_text()}%" }
            }
            div { class: "mt-2 flex items-center gap-2.5",
                div {
                    class: "flex h-16 shrink-0 items-center justify-center rounded-lg border border-slate-800 bg-slate-950/90 px-1.5 py-1.5",
                    style: "{meter_tray_style}",
                    {vu_meter_columns(&bus)}
                }
                div { class: "min-w-0 flex-1 space-y-1.5",
                    span { class: "{mute_badge_class}",
                        if bus.muted { "Muted" } else { "Live" }
                    }
                    div { class: "text-[11px] text-slate-400", "{enabled_route_count} outputs enabled" }
                }
            }
            button {
                class: "mt-2 inline-flex items-center justify-center rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-1.5 text-xs font-medium text-slate-200",
                onclick: move |_| route_editor_signal.set(Some(bus.id)),
                "Open bus"
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
    );
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
        "Output bus" => {
            "rounded-md border border-violet-400/30 bg-violet-500/10 px-1.5 py-1 text-[10px] font-medium text-violet-200"
        }
        _ => {
            "rounded-md border border-slate-700 bg-slate-900 px-1.5 py-1 text-[10px] font-medium text-slate-300"
        }
    };
    let midi_summary = if has_audio_controls {
        midi_summary(&strip)
    } else {
        None
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
                    if let Some(subtitle) = assignment_subtitle {
                        div { class: "truncate pl-[2.5rem] text-[11px] text-slate-400", "{subtitle}" }
                    }
                }
            }
            div { class: "mt-1.5 flex min-w-0 items-start justify-between gap-2 text-[10px] text-slate-500",
                span { class: "font-medium uppercase tracking-[0.18em]", "{strip_mode}" }
                div { class: "flex min-w-0 items-center justify-end gap-1",
                    if let Some(summary) = effect_summary {
                        span {
                            class: "min-w-0 max-w-full truncate rounded-md border border-amber-400/20 bg-amber-500/10 px-1.5 py-1 text-[10px] tracking-[0.08em] text-amber-100",
                            title: "{summary}",
                            "{summary}"
                        }
                    }
                    if let Some(summary) = midi_summary {
                        span {
                            class: "min-w-0 max-w-full truncate rounded-md border border-slate-800 bg-slate-900/70 px-1.5 py-1 text-[10px] tracking-[0.04em] text-slate-300",
                            title: "{full_midi_summary(&strip).unwrap_or_default()}",
                            "{summary}"
                        }
                    }
                }
            }
            div { class: "mt-2.5 grid flex-1 grid-cols-[auto_minmax(0,1fr)_auto] items-center justify-items-center gap-2.5",
                div {
                    class: "{meter_shell_class}",
                    style: "{meter_tray_style}",
                    {vu_meter_columns(&strip)}
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
                if strip.kind.supports_mono() {
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
                    "{strip.kind.empty_route_hint()}"
                }
            } else if route_targets.is_empty() {
                div { class: "mt-1.5 rounded-lg border border-dashed border-slate-800 px-2 py-1.5 text-center text-[10px] font-medium text-slate-500",
                    "{strip.kind.empty_route_hint()}"
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
    let bypass_learn_engine = engine.clone();
    let bypass_clear_engine = engine.clone();
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
    let eq_learn_engine = engine.clone();
    let eq_clear_engine = engine.clone();
    let eq_low_engine = engine.clone();
    let eq_low_learn_engine = engine.clone();
    let eq_low_clear_engine = engine.clone();
    let eq_mid_engine = engine.clone();
    let eq_mid_learn_engine = engine.clone();
    let eq_mid_clear_engine = engine.clone();
    let eq_high_engine = engine.clone();
    let eq_high_learn_engine = engine.clone();
    let eq_high_clear_engine = engine.clone();
    let remove_engine = engine.clone();
    let route_heading = format!("Route matrix -> {}", strip.kind.route_target_label_plural());
    let route_description = strip.kind.route_hint().to_string();
    let assignment_engine = engine.clone();
    let can_remove = strip.is_mixer_strip() || strip.is_bus();
    let delete_label = if strip.is_bus() {
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
            section { class: "flex h-[min(92vh,980px)] w-full max-w-5xl flex-col rounded-2xl border border-slate-800 bg-slate-900/95 p-4 shadow-2xl shadow-black/50",
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
                    if strip.kind.supports_volume_and_mute() {
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
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        div { class: "flex flex-wrap items-center justify-between gap-3",
                            div {
                                h3 { class: "text-lg font-semibold text-white", "{route_heading}" }
                                p { class: "mt-1 text-sm text-slate-400", "Each send can carry its own MIDI trigger for buttons or LEDs." }
                            }
                        }
                        div { class: "mt-4 space-y-3",
                            for route in strip.routes.into_iter() {
                                {
                                    let output_label = snapshot
                                        .route_target_name(strip.kind, route.output_id)
                                        .unwrap_or("Route target")
                                        .to_string();
                                    let route_binding = route.binding();
                                    let route_learning = snapshot.midi_learn_target
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
                                            class: "grid gap-3 sm:grid-cols-[minmax(0,1fr)_15rem]",
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
                                                        span { class: "text-[10px] uppercase tracking-[0.22em] text-slate-500", "{strip.kind.route_target_label()}" }
                                                    }
                                                    span { class: "text-[10px] uppercase tracking-[0.25em] text-slate-400 shrink-0",
                                                        if route.enabled { "On" } else { "Off" }
                                                    }
                                                }
                                            }
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
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        div { class: "flex flex-wrap items-center justify-between gap-3",
                            div {
                                h3 { class: "text-lg font-semibold text-white", "FX engine" }
                                p { class: "mt-1 text-sm text-slate-400", "Bypass or reset the whole chain, then map bypass like any other live control." }
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
                        div { class: "mt-4 max-w-sm",
                            MidiBindingCard {
                                title: "FX bypass".to_string(),
                                binding_label: format_midi_trigger(strip.fx_midi.binding(FxMidiTarget::Bypass).as_ref()),
                                description: Some(if strip.effects.bypassed { "Currently bypassed".to_string() } else { "Currently active".to_string() }),
                                learning: snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::Bypass }),
                                on_learn: move |_| {
                                    let result = if snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::Bypass }) {
                                        bypass_learn_engine.send(AudioControlMsg::CancelMidiLearn)
                                    } else {
                                        bypass_learn_engine.send(AudioControlMsg::StartMidiLearn {
                                            target: MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::Bypass },
                                        })
                                    };
                                    if let Err(error) = result {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                                on_clear: move |_| {
                                    if let Err(error) = bypass_clear_engine.send(AudioControlMsg::SetFxMidiBinding {
                                        strip: strip.id,
                                        target: FxMidiTarget::Bypass,
                                        binding: None,
                                    }) {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                            }
                        }
                    }
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
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        div { class: "flex flex-wrap items-center justify-between gap-3",
                            div {
                                h3 { class: "text-lg font-semibold text-white", "3-band EQ" }
                                p { class: "mt-1 text-sm text-slate-400", "Shape low, mid, and high energy with quick broad-stroke gain trims." }
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
                        div { class: "mt-4 grid gap-3 lg:grid-cols-[16rem_repeat(3,minmax(0,1fr))]",
                            MidiBindingCard {
                                title: "EQ enable".to_string(),
                                binding_label: format_midi_trigger(strip.fx_midi.binding(FxMidiTarget::EqEnabled).as_ref()),
                                description: Some(if strip.effects.eq.enabled { "EQ is on".to_string() } else { "EQ is off".to_string() }),
                                learning: snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::EqEnabled }),
                                on_learn: move |_| {
                                    let result = if snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::EqEnabled }) {
                                        eq_learn_engine.send(AudioControlMsg::CancelMidiLearn)
                                    } else {
                                        eq_learn_engine.send(AudioControlMsg::StartMidiLearn {
                                            target: MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::EqEnabled },
                                        })
                                    };
                                    if let Err(error) = result {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                                on_clear: move |_| {
                                    if let Err(error) = eq_clear_engine.send(AudioControlMsg::SetFxMidiBinding {
                                        strip: strip.id,
                                        target: FxMidiTarget::EqEnabled,
                                        binding: None,
                                    }) {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                            }
                            RotaryKnobCard {
                                title: "Low".to_string(),
                                value_text: format!("{:.1} dB", strip.effects.eq.low_gain_db),
                                min: -12.0,
                                max: 12.0,
                                step: 0.5,
                                value: strip.effects.eq.low_gain_db,
                                binding_label: format_midi_trigger(strip.fx_midi.binding(FxMidiTarget::EqLowGain).as_ref()),
                                learning: snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::EqLowGain }),
                                on_change: move |value| {
                                    if let Err(error) = eq_low_engine.send(AudioControlMsg::SetEqLowGain {
                                        strip: strip.id,
                                        gain_db: value,
                                    }) {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                                on_learn: move |_| {
                                    let result = if snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::EqLowGain }) {
                                        eq_low_learn_engine.send(AudioControlMsg::CancelMidiLearn)
                                    } else {
                                        eq_low_learn_engine.send(AudioControlMsg::StartMidiLearn {
                                            target: MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::EqLowGain },
                                        })
                                    };
                                    if let Err(error) = result {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                                on_clear: move |_| {
                                    if let Err(error) = eq_low_clear_engine.send(AudioControlMsg::SetFxMidiBinding {
                                        strip: strip.id,
                                        target: FxMidiTarget::EqLowGain,
                                        binding: None,
                                    }) {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                            }
                            RotaryKnobCard {
                                title: "Mid".to_string(),
                                value_text: format!("{:.1} dB", strip.effects.eq.mid_gain_db),
                                min: -12.0,
                                max: 12.0,
                                step: 0.5,
                                value: strip.effects.eq.mid_gain_db,
                                binding_label: format_midi_trigger(strip.fx_midi.binding(FxMidiTarget::EqMidGain).as_ref()),
                                learning: snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::EqMidGain }),
                                on_change: move |value| {
                                    if let Err(error) = eq_mid_engine.send(AudioControlMsg::SetEqMidGain {
                                        strip: strip.id,
                                        gain_db: value,
                                    }) {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                                on_learn: move |_| {
                                    let result = if snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::EqMidGain }) {
                                        eq_mid_learn_engine.send(AudioControlMsg::CancelMidiLearn)
                                    } else {
                                        eq_mid_learn_engine.send(AudioControlMsg::StartMidiLearn {
                                            target: MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::EqMidGain },
                                        })
                                    };
                                    if let Err(error) = result {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                                on_clear: move |_| {
                                    if let Err(error) = eq_mid_clear_engine.send(AudioControlMsg::SetFxMidiBinding {
                                        strip: strip.id,
                                        target: FxMidiTarget::EqMidGain,
                                        binding: None,
                                    }) {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                            }
                            RotaryKnobCard {
                                title: "High".to_string(),
                                value_text: format!("{:.1} dB", strip.effects.eq.high_gain_db),
                                min: -12.0,
                                max: 12.0,
                                step: 0.5,
                                value: strip.effects.eq.high_gain_db,
                                binding_label: format_midi_trigger(strip.fx_midi.binding(FxMidiTarget::EqHighGain).as_ref()),
                                learning: snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::EqHighGain }),
                                on_change: move |value| {
                                    if let Err(error) = eq_high_engine.send(AudioControlMsg::SetEqHighGain {
                                        strip: strip.id,
                                        gain_db: value,
                                    }) {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                                on_learn: move |_| {
                                    let result = if snapshot.midi_learn_target == Some(MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::EqHighGain }) {
                                        eq_high_learn_engine.send(AudioControlMsg::CancelMidiLearn)
                                    } else {
                                        eq_high_learn_engine.send(AudioControlMsg::StartMidiLearn {
                                            target: MidiLearnTarget::Fx { strip: strip.id, target: FxMidiTarget::EqHighGain },
                                        })
                                    };
                                    if let Err(error) = result {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                                on_clear: move |_| {
                                    if let Err(error) = eq_high_clear_engine.send(AudioControlMsg::SetFxMidiBinding {
                                        strip: strip.id,
                                        target: FxMidiTarget::EqHighGain,
                                        binding: None,
                                    }) {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
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
    midi_test_controller: String,
    midi_test_value: String,
    show_settings_signal: Signal<bool>,
    snapshot_signal: Signal<AudioEngineState>,
    midi_test_controller_signal: Signal<String>,
    midi_test_value_signal: Signal<String>,
    new_virtual_cable_name_signal: Signal<String>,
    new_output_name_signal: Signal<String>,
) -> Element {
    let engine = use_context::<SharedEngineBridge>();
    let midi_test_engine = engine.clone();
    let midi_feedback_output_engine = engine.clone();
    let midi_feedback_sync_engine = engine.clone();
    let reset_mixer_engine = engine.clone();
    let add_virtual_cable_engine = engine.clone();
    let add_output_engine = engine.clone();
    let remove_output_engine = engine.clone();
    let midi_feedback_output = snapshot
        .midi_feedback
        .output_port_name
        .clone()
        .unwrap_or_default();
    let new_virtual_cable_name = new_virtual_cable_name_signal.read().clone();
    let new_output_name = new_output_name_signal.read().clone();
    let outputs = snapshot.output_strips.clone();
    let virtual_cables = snapshot
        .source_strips
        .iter()
        .filter(|source| source.is_virtual_cable())
        .map(|source| {
            (
                source
                    .pipewire_node_name
                    .clone()
                    .unwrap_or_else(|| source.label.clone()),
                source.label.clone(),
            )
        })
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
                                        destinations: virtual_cables.clone(),
                                        snapshot_signal,
                                    }
                                }
                            }
                        }
                    }
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        h3 { class: "text-lg font-semibold text-white", "Outputs" }
                        p { class: "mt-2 text-sm text-slate-400", "Create app-managed outputs here. Buses then target these outputs, alongside any system outputs discovered from PipeWire." }
                        div { class: "mt-4 grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto]",
                            input {
                                class: "w-full rounded-lg border border-slate-700 bg-slate-900/90 px-4 py-3 text-sm text-slate-100 outline-none",
                                r#type: "text",
                                placeholder: "New output name",
                                value: "{new_output_name}",
                                oninput: move |event| new_output_name_signal.set(event.value()),
                            }
                            button {
                                class: "inline-flex items-center justify-center rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-4 py-3 text-sm font-medium text-cyan-100",
                                onclick: move |_| {
                                    let label = new_output_name_signal.read().clone();
                                    new_output_name_signal.set(String::new());
                                    if let Err(error) = add_output_engine.send(AudioControlMsg::AddOutput { label }) {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                                "Add output"
                            }
                        }
                        div { class: "mt-4 space-y-3",
                            if outputs.is_empty() {
                                div { class: "rounded-lg border border-dashed border-slate-700 px-4 py-4 text-sm text-slate-400",
                                    "No outputs are available right now."
                                }
                            } else {
                                for output in outputs {
                                    {
                                        let remove_output_engine = remove_output_engine.clone();
                                        rsx! {
                                            div { key: "{output.id.as_str()}", class: "rounded-lg border border-slate-800 bg-slate-900/70 px-4 py-3",
                                                div { class: "flex flex-wrap items-center justify-between gap-3",
                                                    div {
                                                        div { class: "text-sm font-medium text-white", "{output.label}" }
                                                        div { class: "mt-1 text-[11px] uppercase tracking-[0.18em] text-slate-500", "{output.role_label()}" }
                                                    }
                                                    if output.is_managed_output() {
                                                        button {
                                                            class: "inline-flex items-center justify-center rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-2 text-sm font-medium text-slate-200",
                                                            onclick: move |_| {
                                                                if let Err(error) = remove_output_engine.send(AudioControlMsg::RemoveStrip { strip: output.id }) {
                                                                    snapshot_signal.write().last_notice = error;
                                                                }
                                                            },
                                                            "Remove"
                                                        }
                                                    } else {
                                                        span { class: "rounded-md border border-slate-800 bg-slate-950/70 px-2 py-1 text-[10px] font-medium uppercase tracking-[0.18em] text-slate-400",
                                                            "System"
                                                        }
                                                    }
                                                }
                                                div { class: "mt-3 grid gap-3 md:grid-cols-2",
                                                    OutputMidiBindingCard {
                                                        output: output.clone(),
                                                        target: MidiControlTarget::Volume,
                                                        active_learn_target: snapshot.midi_learn_target,
                                                        snapshot_signal,
                                                    }
                                                    OutputMidiBindingCard {
                                                        output,
                                                        target: MidiControlTarget::Mute,
                                                        active_learn_target: snapshot.midi_learn_target,
                                                        snapshot_signal,
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                        h3 { class: "text-lg font-semibold text-white", "MIDI feedback + reset" }
                        p { class: "mt-2 text-sm text-slate-400", "Select the controller output used for LEDs and push a full resync after binding changes. Reset clears user-created cables, strips, buses, and outputs while keeping discovered hardware sources visible." }
                        div { class: "mt-4 grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto_auto]",
                            label { class: "space-y-1",
                                span { class: "block text-[10px] uppercase tracking-[0.25em] text-slate-500", "Feedback output" }
                                select {
                                    class: "w-full rounded-lg border border-slate-700 bg-slate-900/90 px-3 py-3 text-sm text-slate-100 outline-none",
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
                                    option { value: "", "Disabled" }
                                    for port in snapshot.inventory.midi_outputs.iter() {
                                        option { key: "{port.name}", value: "{port.name}", "{port.name}" }
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
                            button {
                                class: "mt-[1.35rem] inline-flex items-center justify-center rounded-lg border border-rose-400/40 bg-rose-500/10 px-4 py-3 text-sm font-medium text-rose-100",
                                onclick: move |_| {
                                    if let Err(error) = reset_mixer_engine.send(AudioControlMsg::ResetMixer) {
                                        snapshot_signal.write().last_notice = error;
                                    }
                                },
                                "Reset mixer"
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
                                        if let Err(error) = midi_test_engine.send(AudioControlMsg::ApplyMidiEvent {
                                            event: crate::audio::MidiEvent {
                                                kind: MidiMessageKind::ControlChange,
                                                channel: 0,
                                                number: controller,
                                                value,
                                            },
                                        }) {
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
                }
            }
        }
    }
}

#[component]
fn MidiBindingCard(
    title: String,
    binding_label: String,
    description: Option<String>,
    learning: bool,
    on_learn: EventHandler<()>,
    on_clear: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "rounded-lg border border-slate-800 bg-slate-950/70 p-3",
            span { class: "block text-[10px] uppercase tracking-[0.25em] text-slate-500", "{title}" }
            div { class: "mt-2 text-sm text-slate-100", "{binding_label}" }
            if let Some(description) = description {
                div { class: "mt-1 text-xs text-slate-400", "{description}" }
            }
            div { class: "mt-3 flex gap-2",
                button {
                    class: if learning {
                        "flex-1 rounded-lg border border-amber-400/40 bg-amber-500/15 px-3 py-2 text-sm font-medium text-amber-100"
                    } else {
                        "flex-1 rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-3 py-2 text-sm font-medium text-cyan-100"
                    },
                    onclick: move |_| on_learn.call(()),
                    if learning { "Listening..." } else { "Learn" }
                }
                button {
                    class: "rounded-lg border border-slate-700 bg-slate-900/90 px-3 py-2 text-sm font-medium text-slate-200",
                    onclick: move |_| on_clear.call(()),
                    "Clear"
                }
            }
        }
    }
}

#[component]
fn RotaryKnobCard(
    title: String,
    value_text: String,
    min: f32,
    max: f32,
    step: f32,
    value: f32,
    binding_label: String,
    learning: bool,
    on_change: EventHandler<f32>,
    on_learn: EventHandler<()>,
    on_clear: EventHandler<()>,
) -> Element {
    let ratio = if max > min {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let sweep = ratio * 270.0;
    let indicator_angle = -135.0 + sweep;
    let knob_style = format!(
        "background: conic-gradient(from 225deg, rgba(34,211,238,0.88) 0deg, rgba(34,211,238,0.88) {sweep:.2}deg, rgba(51,65,85,0.92) {sweep:.2}deg, rgba(15,23,42,0.92) 360deg);"
    );
    let indicator_style = format!(
        "transform: translate(-50%, -100%) rotate({indicator_angle:.2}deg); transform-origin: center 24px;"
    );
    let next_down = (value - step).max(min);
    let next_up = (value + step).min(max);

    rsx! {
        div { class: "rounded-lg border border-slate-800 bg-slate-950/70 p-3",
            div { class: "text-[10px] uppercase tracking-[0.25em] text-slate-500", "{title}" }
            div { class: "mt-3 flex flex-col items-center",
                div { class: "relative flex h-20 w-20 items-center justify-center rounded-full border border-slate-700 shadow-[inset_0_0_0_1px_rgba(15,23,42,0.9)]",
                    style: "{knob_style}",
                    div { class: "absolute inset-[0.35rem] rounded-full bg-slate-950/95" }
                    div { class: "absolute left-1/2 top-1/2 h-5 w-1 rounded-full bg-cyan-200 shadow-[0_0_10px_rgba(34,211,238,0.55)]",
                        style: "{indicator_style}"
                    }
                    div { class: "relative z-10 text-center",
                        div { class: "text-xs font-semibold text-white", "{value_text}" }
                        div { class: "mt-0.5 text-[10px] uppercase tracking-[0.18em] text-slate-500", "Knob" }
                    }
                }
                div { class: "mt-3 flex w-full gap-2",
                    button {
                        class: "flex-1 rounded-lg border border-slate-700 bg-slate-900/90 px-2 py-1.5 text-sm font-medium text-slate-200",
                        onclick: move |_| on_change.call(next_down),
                        "-"
                    }
                    button {
                        class: "flex-1 rounded-lg border border-slate-700 bg-slate-900/90 px-2 py-1.5 text-sm font-medium text-slate-200",
                        onclick: move |_| on_change.call(next_up),
                        "+"
                    }
                }
            }
            div { class: "mt-3 text-xs text-slate-400", "{binding_label}" }
            div { class: "mt-3 flex gap-2",
                button {
                    class: if learning {
                        "flex-1 rounded-lg border border-amber-400/40 bg-amber-500/15 px-3 py-2 text-sm font-medium text-amber-100"
                    } else {
                        "flex-1 rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-3 py-2 text-sm font-medium text-cyan-100"
                    },
                    onclick: move |_| on_learn.call(()),
                    if learning { "Listening..." } else { "Learn" }
                }
                button {
                    class: "rounded-lg border border-slate-700 bg-slate-900/90 px-3 py-2 text-sm font-medium text-slate-200",
                    onclick: move |_| on_clear.call(()),
                    "Clear"
                }
            }
        }
    }
}

#[component]
fn OutputMidiBindingCard(
    output: MixerStrip,
    target: MidiControlTarget,
    active_learn_target: Option<MidiLearnTarget>,
    snapshot_signal: Signal<AudioEngineState>,
) -> Element {
    let engine = use_context::<SharedEngineBridge>();
    let learn_engine = engine.clone();
    let clear_engine = engine.clone();
    let binding = match target {
        MidiControlTarget::Volume => output.midi.volume_binding(),
        MidiControlTarget::Mute => output.midi.mute_binding(),
    };
    let learning = active_learn_target
        == Some(MidiLearnTarget::Strip {
            strip: output.id,
            target,
        });
    let title = match target {
        MidiControlTarget::Volume => "Output volume".to_string(),
        MidiControlTarget::Mute => "Output mute".to_string(),
    };
    let description = match target {
        MidiControlTarget::Volume => Some(format!(
            "Current level {}%",
            output.volume.as_percent_text()
        )),
        MidiControlTarget::Mute => Some(if output.muted {
            "Currently muted".to_string()
        } else {
            "Currently live".to_string()
        }),
    };

    rsx! {
        MidiBindingCard {
            title,
            binding_label: format_midi_trigger(binding.as_ref()),
            description,
            learning,
            on_learn: move |_| {
                let result = if learning {
                    learn_engine.send(AudioControlMsg::CancelMidiLearn)
                } else {
                    learn_engine.send(AudioControlMsg::StartMidiLearn {
                        target: MidiLearnTarget::Strip {
                            strip: output.id,
                            target,
                        },
                    })
                };
                if let Err(error) = result {
                    snapshot_signal.write().last_notice = error;
                }
            },
            on_clear: move |_| {
                if let Err(error) = clear_engine.send(AudioControlMsg::SetMidiBinding {
                    strip: output.id,
                    target,
                    binding: None,
                }) {
                    snapshot_signal.write().last_notice = error;
                }
            },
        }
    }
}

#[component]
fn CreateStripModal(
    snapshot: AudioEngineState,
    snapshot_signal: Signal<AudioEngineState>,
    show_signal: Signal<bool>,
    name_signal: Signal<String>,
    source_selection_signal: Signal<Option<StripId>>,
    bus_selection_signal: Signal<Vec<StripId>>,
) -> Element {
    let engine = use_context::<SharedEngineBridge>();
    let create_engine = engine.clone();
    let sources = snapshot.source_strips.iter().cloned().collect::<Vec<_>>();
    let buses = snapshot.bus_strips.clone();
    let selected_source = *source_selection_signal.read();
    let selected_buses = bus_selection_signal.read().clone();
    let strip_name = name_signal.read().clone();

    rsx! {
        div { class: "fixed inset-0 z-40 flex items-start justify-center bg-slate-950/70 p-6 backdrop-blur-sm",
            section { class: "flex h-[min(92vh,900px)] w-full max-w-4xl flex-col rounded-2xl border border-slate-800 bg-slate-900/95 p-4 shadow-2xl shadow-black/50",
                div { class: "flex items-start justify-between gap-4 border-b border-slate-800 pb-4",
                    div {
                        p { class: "text-sm uppercase tracking-[0.3em] text-cyan-400", "Create strip" }
                        h2 { class: "mt-2 text-xl font-semibold text-white", "Bind one input, then pick bus sends" }
                        p { class: "mt-2 text-sm text-slate-400", "Choose a single source or virtual cable for the new strip, then select which buses it should feed." }
                    }
                    button {
                        class: "rounded-lg border border-slate-700 bg-slate-950/70 px-4 py-2 text-sm font-medium text-slate-100",
                        onclick: move |_| show_signal.set(false),
                        "Close"
                    }
                }
                div { class: "mt-4 min-h-0 flex-1 space-y-4 overflow-y-auto pr-1",
                    label { class: "space-y-1",
                        span { class: "block text-[10px] uppercase tracking-[0.25em] text-slate-500", "Strip name" }
                        input {
                            class: "w-full rounded-lg border border-slate-700 bg-slate-900/90 px-3 py-3 text-sm text-slate-100 outline-none",
                            r#type: "text",
                            value: "{strip_name}",
                            oninput: move |event| name_signal.set(event.value()),
                        }
                    }
                    div { class: "grid gap-4 lg:grid-cols-2",
                        article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                            h3 { class: "text-lg font-semibold text-white", "Assign one input" }
                            p { class: "mt-1 text-sm text-slate-400", "A strip accepts exactly one real source or one virtual cable." }
                            div { class: "mt-4 space-y-2",
                                button {
                                    class: if selected_source.is_none() {
                                        "w-full rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-3 py-3 text-left text-sm font-medium text-cyan-100"
                                    } else {
                                        "w-full rounded-lg border border-slate-800 bg-slate-900/70 px-3 py-3 text-left text-sm text-slate-200"
                                    },
                                    onclick: move |_| source_selection_signal.set(None),
                                    "Leave unassigned"
                                }
                                if sources.is_empty() {
                                    div { class: "rounded-lg border border-dashed border-slate-700 px-4 py-4 text-sm text-slate-400",
                                        "No sources are available right now."
                                    }
                                } else {
                                    for source in sources {
                                        {
                                            let checked = selected_source == Some(source.id);
                                            rsx! {
                                                label { key: "{source.id.as_str()}", class: "flex cursor-pointer items-center justify-between rounded-lg border border-slate-800 bg-slate-900/70 px-3 py-3 text-sm text-slate-100",
                                                    div {
                                                        div { class: "font-medium text-white", "{source.label}" }
                                                        div { class: "mt-1 text-[11px] uppercase tracking-[0.22em] text-slate-500", "{source.role_label()}" }
                                                    }
                                                    input {
                                                        r#type: "radio",
                                                        class: "h-4 w-4 accent-cyan-400",
                                                        checked: checked,
                                                        onchange: move |_| {
                                                            source_selection_signal.set(Some(source.id));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        article { class: "rounded-xl border border-slate-800 bg-slate-950/70 p-4",
                            h3 { class: "text-lg font-semibold text-white", "Send to buses" }
                            p { class: "mt-1 text-sm text-slate-400", "These buses will receive the new strip." }
                            div { class: "mt-4 space-y-2",
                                if buses.is_empty() {
                                    div { class: "rounded-lg border border-dashed border-slate-700 px-4 py-4 text-sm text-slate-400",
                                        "No buses are available right now."
                                    }
                                } else {
                                    for bus in buses {
                                        {
                                            let checked = selected_buses.contains(&bus.id);
                                            rsx! {
                                                label { key: "{bus.id.as_str()}", class: "flex cursor-pointer items-center justify-between rounded-lg border border-slate-800 bg-slate-900/70 px-3 py-3 text-sm text-slate-100",
                                                    div {
                                                        div { class: "font-medium text-white", "{bus.label}" }
                                                        div { class: "mt-1 text-[11px] uppercase tracking-[0.22em] text-slate-500", "{bus.role_label()}" }
                                                    }
                                                    input {
                                                        r#type: "checkbox",
                                                        class: "h-4 w-4 accent-cyan-400",
                                                        checked: checked,
                                                        onchange: move |_| {
                                                            toggle_strip_selection(bus_selection_signal, bus.id);
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
                div { class: "mt-4 flex flex-wrap justify-end gap-2 border-t border-slate-800 pt-4",
                    button {
                        class: "rounded-lg border border-slate-700 bg-slate-950/70 px-4 py-2 text-sm font-medium text-slate-100",
                        onclick: move |_| show_signal.set(false),
                        "Cancel"
                    }
                    button {
                        class: "rounded-lg border border-cyan-400/40 bg-cyan-500/10 px-4 py-2 text-sm font-medium text-cyan-100",
                        onclick: move |_| {
                            let label = name_signal.read().clone();
                            let input_source = *source_selection_signal.read();
                            let buses = bus_selection_signal.read().clone();
                            if let Err(error) = create_engine.send(AudioControlMsg::CreateStrip {
                                label,
                                input_source,
                                buses,
                            }) {
                                snapshot_signal.write().last_notice = error;
                            } else {
                                name_signal.set(String::new());
                                source_selection_signal.set(None);
                                bus_selection_signal.set(Vec::new());
                                show_signal.set(false);
                            }
                        },
                        "Create strip"
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
fn ApplicationStreamRow(
    stream: ApplicationStreamInfo,
    destinations: Vec<(String, String)>,
    snapshot_signal: Signal<AudioEngineState>,
) -> Element {
    let engine = use_context::<SharedEngineBridge>();
    let route_engine = engine.clone();
    let app_name = stream.identity.application_name.clone();
    let media_name = stream.identity.media_name.clone();
    let badge_text = application_badge_text(&app_name);

    rsx! {
        div { class: "rounded-lg border border-slate-800 bg-slate-900/70 px-4 py-3",
            div { class: "flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between",
                div { class: "flex min-w-0 items-start gap-3",
                    if let Some(icon_data_url) = stream.icon_data_url.clone() {
                        img {
                            class: "h-11 w-11 shrink-0 rounded-xl border border-slate-700 bg-slate-950/80 object-contain p-2",
                            src: "{icon_data_url}",
                            alt: "{app_name} icon",
                        }
                    } else {
                        div { class: "flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-cyan-400/30 bg-cyan-500/10 text-sm font-semibold uppercase text-cyan-100",
                            "{badge_text}"
                        }
                    }
                    div { class: "min-w-0",
                        div { class: "flex flex-wrap items-center gap-2",
                            div { class: "truncate text-sm font-semibold text-white", "{app_name}" }
                            if stream.corked {
                                span { class: "rounded-md border border-amber-400/30 bg-amber-500/10 px-2 py-1 text-[10px] font-medium uppercase tracking-[0.18em] text-amber-100",
                                    "Corked"
                                }
                            }
                        }
                        div { class: "mt-1 truncate text-sm text-slate-300", "{media_name}" }
                        div { class: "mt-1 text-[11px] uppercase tracking-[0.18em] text-slate-500",
                            "Current sink: {stream.current_sink_label}"
                        }
                    }
                }
                div { class: "w-full lg:w-72",
                    if destinations.is_empty() {
                        div { class: "rounded-lg border border-dashed border-slate-700 px-4 py-3 text-sm text-slate-400",
                            "Create a virtual cable first, then route this app into it."
                        }
                    } else {
                        select {
                            class: "w-full rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-3 text-sm text-slate-100 outline-none",
                            value: "",
                            oninput: move |event| {
                                let sink_name = event.value();
                                if sink_name.trim().is_empty() {
                                    return;
                                }
                                if let Err(error) = route_engine.send(AudioControlMsg::MoveApplicationStream {
                                    stream: stream.identity.clone(),
                                    sink_name,
                                }) {
                                    snapshot_signal.write().last_notice = error;
                                }
                            },
                            option { value: "", "Move to virtual cable..." }
                            for (sink_name, sink_label) in destinations.iter() {
                                option { key: "{sink_name}", value: "{sink_name}", "{sink_label}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SummaryCard(title: String, value: String, description: String) -> Element {
    rsx! {
        div { class: "rounded-lg border border-slate-800 bg-slate-950/70 px-2.5 py-1.5",
            div { class: "text-[10px] uppercase tracking-[0.22em] text-slate-500", "{title}" }
            div { class: "mt-0.5 leading-tight text-lg font-semibold text-white", "{value}" }
            div { class: "text-[11px] leading-tight text-slate-400", "{description}" }
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

fn full_midi_summary(strip: &MixerStrip) -> Option<String> {
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

fn effect_summary(strip: &MixerStrip) -> Option<String> {
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

fn format_midi_trigger(binding: Option<&MidiTrigger>) -> String {
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

fn short_midi_trigger(binding: &MidiTrigger) -> String {
    match binding.kind {
        MidiMessageKind::ControlChange => format!("CC{}", binding.number),
        MidiMessageKind::Note => format!("N{}", binding.number),
    }
}

fn compact_strip_kind_label(kind: StripKind) -> &'static str {
    match kind {
        StripKind::HardwareSource => "Source",
        StripKind::VirtualCable => "Cable",
        StripKind::Strip => "Strip",
        StripKind::Bus => "Bus",
        StripKind::Output => "Output",
    }
}

fn compact_role_label(strip: &MixerStrip) -> &'static str {
    match strip.kind {
        StripKind::HardwareSource => "Hardware",
        StripKind::VirtualCable => "Virtual",
        StripKind::Strip => "Channel",
        StripKind::Bus => "Bus",
        StripKind::Output => "Output",
    }
}

fn application_badge_text(name: &str) -> String {
    name.chars()
        .find(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_uppercase().to_string())
        .unwrap_or_else(|| "A".to_string())
}

fn source_display_lines(label: &str) -> (String, Option<String>) {
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

fn mute_icon(muted: bool) -> Element {
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

fn mono_icon() -> Element {
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

fn toggle_strip_selection(mut selection_signal: Signal<Vec<StripId>>, strip_id: StripId) {
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

fn vu_meter_columns(strip: &MixerStrip) -> Element {
    let meter_active = strip.meter_level.as_ratio() > 0.05;
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
        div { class: "relative flex h-full w-full items-center justify-center gap-1.5",
            div { class: "pointer-events-none absolute inset-y-1 left-0 right-0 flex flex-col justify-between",
                for marker in 0..5 {
                    div { key: "meter-mark-{marker}", class: "border-t border-white/6" }
                }
            }
            for (key, fill_height, empty_height) in channel_levels {
                div {
                    key: "{key}",
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
