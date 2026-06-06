use crate::audio::{
    AudioControlMsg, AudioEngineState, MidiControlTarget, MidiLearnTarget, MixerStrip,
    SharedEngineBridge,
};
use crate::ui::helpers::format_midi_trigger;
use dioxus::prelude::*;

#[component]
pub(crate) fn MidiBindingCard(
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
pub(crate) fn RotaryKnobCard(
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
    let min_text = format!("{min}");
    let max_text = format!("{max}");
    let step_text = format!("{step}");
    let value_attr = format!("{value}");
    let next_down = (value - step).max(min);
    let next_up = (value + step).min(max);

    rsx! {
        div { class: "rounded-lg border border-slate-800 bg-slate-950/70 p-3",
            div { class: "flex items-center justify-between",
                span { class: "text-[10px] uppercase tracking-[0.25em] text-slate-500", "{title}" }
                span { class: "text-sm font-semibold text-white", "{value_text}" }
            }
            div { class: "mt-2 flex items-center gap-2",
                button {
                    class: "shrink-0 rounded border border-slate-700 bg-slate-900/90 px-2 py-1 text-sm font-medium text-slate-300 leading-none",
                    onclick: move |_| on_change.call(next_down),
                    "−"
                }
                input {
                    class: "flex-1 cursor-pointer accent-cyan-400",
                    r#type: "range",
                    min: "{min_text}",
                    max: "{max_text}",
                    step: "{step_text}",
                    value: "{value_attr}",
                    oninput: move |event| {
                        if let Ok(parsed) = event.value().parse::<f32>() {
                            on_change.call(parsed);
                        }
                    },
                }
                button {
                    class: "shrink-0 rounded border border-slate-700 bg-slate-900/90 px-2 py-1 text-sm font-medium text-slate-300 leading-none",
                    onclick: move |_| on_change.call(next_up),
                    "+"
                }
            }
            div { class: "mt-3 text-xs text-slate-400", "{binding_label}" }
            div { class: "mt-2 flex gap-2",
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
pub(crate) fn SliderControlCard(
    title: String,
    value_text: String,
    min: f32,
    max: f32,
    step: f32,
    value: f32,
    on_change: EventHandler<f32>,
) -> Element {
    let mut dragging = use_signal(|| false);
    let mut drag_value = use_signal(|| value);

    let display_value = if *dragging.read() { *drag_value.read() } else { value };
    let min_text = format!("{min:.1}");
    let max_text = format!("{max:.1}");
    let step_text = format!("{step:.1}");
    let value_attr = format!("{display_value:.2}");
    let display_text = if *dragging.read() {
        format!("{display_value:+.1} dB")
    } else {
        value_text
    };

    rsx! {
        div { class: "flex w-[5.75rem] shrink-0 flex-col items-center rounded-lg border border-slate-800 bg-slate-950/70 px-2 py-3",
            div { class: "text-center text-[10px] uppercase tracking-[0.25em] text-slate-500", "{title}" }
            div { class: "mt-1 text-center text-sm font-semibold text-white", "{display_text}" }
            div { class: "mt-3 flex items-center gap-2",
                div { class: "flex h-40 flex-col items-center justify-between text-[10px] font-medium uppercase tracking-[0.18em] text-slate-500",
                    span { "{max:.0}" }
                    span { "0" }
                    span { "{min:.0}" }
                }
                input {
                    class: "h-40 w-4 cursor-pointer accent-cyan-400",
                    style: "-webkit-appearance: slider-vertical; writing-mode: vertical-lr; direction: rtl;",
                    r#type: "range",
                    min: "{min_text}",
                    max: "{max_text}",
                    step: "{step_text}",
                    value: "{value_attr}",
                    oninput: move |event| {
                        if let Ok(parsed) = event.value().parse::<f32>() {
                            dragging.set(true);
                            drag_value.set(parsed);
                            on_change.call(parsed);
                        }
                    },
                    onchange: move |_| {
                        dragging.set(false);
                    },
                }
            }
            div { class: "mt-2 text-[10px] uppercase tracking-[0.18em] text-slate-500", "dB" }
        }
    }
}

#[component]
pub(crate) fn OutputMidiBindingCard(
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
pub(crate) fn SummaryCard(title: String, value: String, description: String) -> Element {
    rsx! {
        div { class: "rounded-lg border border-slate-800 bg-slate-950/70 px-2.5 py-1.5",
            div { class: "text-[10px] uppercase tracking-[0.22em] text-slate-500", "{title}" }
            div { class: "mt-0.5 leading-tight text-lg font-semibold text-white", "{value}" }
            div { class: "text-[11px] leading-tight text-slate-400", "{description}" }
        }
    }
}

#[component]
pub(crate) fn InventoryBlock(label: String, message: String) -> Element {
    rsx! {
        div { class: "rounded-lg border border-slate-800 bg-slate-950/70 px-4 py-3",
            div { class: "text-xs uppercase tracking-[0.3em] text-slate-500", "{label}" }
            p { class: "mt-2 text-sm text-slate-300", "{message}" }
        }
    }
}
