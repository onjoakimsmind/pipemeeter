use crate::audio::{AudioControlMsg, AudioEngineState, MixerStrip, SharedEngineBridge, StripId};
use crate::ui::helpers::toggle_strip_selection;
use dioxus::prelude::*;

#[component]
pub(crate) fn CreateStripModal(
    snapshot: AudioEngineState,
    snapshot_signal: Signal<AudioEngineState>,
    show_signal: Signal<bool>,
    name_signal: Signal<String>,
    source_selection_signal: Signal<Option<StripId>>,
    bus_selection_signal: Signal<Vec<StripId>>,
) -> Element {
    let engine = use_context::<SharedEngineBridge>();
    let create_engine = engine.clone();
    let sources = snapshot
        .source_strips
        .iter()
        .cloned()
        .collect::<Vec<MixerStrip>>();
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
