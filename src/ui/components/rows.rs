use crate::audio::{
    ApplicationStreamInfo, AudioControlMsg, AudioEngineState, MixerStrip, PipeWireNodeInfo,
    SharedEngineBridge, StripId,
};
use crate::ui::helpers::{application_badge_text, VuMeter};
use dioxus::prelude::*;

#[component]
pub(crate) fn BusStatusCard(
    bus: MixerStrip,
    route_editor_signal: Signal<Option<StripId>>,
) -> Element {
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
                    VuMeter {
                        strip_id: bus.id,
                        fallback_channels: bus.meter_channels.len(),
                    }
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
pub(crate) fn PipeWireNodeRow(node: PipeWireNodeInfo) -> Element {
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
pub(crate) fn ApplicationStreamRow(
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
                            class: "w-full appearance-none rounded-lg border border-slate-700 bg-slate-950/80 px-3 py-3 text-sm text-slate-100 outline-none",
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
                            option { value: "", class: "bg-slate-900 text-slate-100", "Route to..." }
                            for (sink_name, sink_label) in destinations.iter() {
                                option { key: "{sink_name}", value: "{sink_name}", class: "bg-slate-900 text-slate-100", "{sink_label}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
