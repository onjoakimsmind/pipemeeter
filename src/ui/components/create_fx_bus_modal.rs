use crate::audio::{AudioControlMsg, AudioEngineState, SharedEngineBridge};
use dioxus::prelude::*;

#[component]
pub(crate) fn CreateFxBusModal(
    snapshot_signal: Signal<AudioEngineState>,
    show_signal: Signal<bool>,
    name_signal: Signal<String>,
    gate_signal: Signal<bool>,
    compressor_signal: Signal<bool>,
    eq_signal: Signal<bool>,
) -> Element {
    let engine = use_context::<SharedEngineBridge>();
    let name = name_signal.read().clone();
    let gate = *gate_signal.read();
    let compressor = *compressor_signal.read();
    let eq = *eq_signal.read();
    let can_create = gate || compressor || eq;

    rsx! {
        div { class: "fixed inset-0 z-40 flex items-start justify-center bg-slate-950/70 p-6 backdrop-blur-sm",
            section { class: "w-full max-w-lg flex flex-col rounded-2xl border border-slate-800 bg-slate-900/95 p-4 shadow-2xl shadow-black/50",
                div { class: "flex items-start justify-between gap-4 border-b border-slate-800 pb-4",
                    div {
                        p { class: "text-sm uppercase tracking-[0.3em] text-amber-400", "Create FX bus" }
                        h2 { class: "mt-2 text-xl font-semibold text-white", "Choose effects" }
                    }
                    button {
                        class: "rounded-lg border border-slate-700 bg-slate-950/70 px-4 py-2 text-sm font-medium text-slate-100",
                        onclick: move |_| show_signal.set(false),
                        "Close"
                    }
                }
                div { class: "mt-4 space-y-4",
                    label { class: "space-y-1",
                        span { class: "block text-[10px] uppercase tracking-[0.25em] text-slate-500", "Bus name" }
                        input {
                            class: "w-full rounded-lg border border-slate-700 bg-slate-900/90 px-3 py-3 text-sm text-slate-100 outline-none",
                            r#type: "text",
                            placeholder: "Auto",
                            value: "{name}",
                            oninput: move |event| name_signal.set(event.value()),
                        }
                    }
                    div { class: "space-y-2",
                        span { class: "block text-[10px] uppercase tracking-[0.25em] text-slate-500", "Enable effects" }
                        for (label, checked, mut sig) in [
                            ("Noise Gate", gate, gate_signal),
                            ("Compressor", compressor, compressor_signal),
                            ("EQ", eq, eq_signal),
                        ] {
                            label { class: "flex cursor-pointer items-center justify-between rounded-lg border border-slate-800 bg-slate-950/70 px-4 py-3 text-sm text-slate-100",
                                span { class: "font-medium", "{label}" }
                                input {
                                    r#type: "checkbox",
                                    class: "h-4 w-4 accent-amber-400",
                                    checked: checked,
                                    onchange: move |_| {
                                    let new_val = !*sig.read();
                                    sig.set(new_val);
                                },
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
                        class: if can_create {
                            "rounded-lg border border-amber-400/40 bg-amber-500/10 px-4 py-2 text-sm font-medium text-amber-100"
                        } else {
                            "rounded-lg border border-slate-700 bg-slate-900/50 px-4 py-2 text-sm font-medium text-slate-500 cursor-not-allowed"
                        },
                        disabled: !can_create,
                        onclick: move |_| {
                            if !can_create { return; }
                            let label = name_signal.read().clone();
                            if let Err(error) = engine.send(AudioControlMsg::AddFxBus {
                                label,
                                gate: *gate_signal.read(),
                                compressor: *compressor_signal.read(),
                                eq: *eq_signal.read(),
                            }) {
                                snapshot_signal.write().last_notice = error;
                            } else {
                                name_signal.set(String::new());
                                gate_signal.set(false);
                                compressor_signal.set(false);
                                eq_signal.set(false);
                                show_signal.set(false);
                            }
                        },
                        "Create FX bus"
                    }
                }
            }
        }
    }
}
