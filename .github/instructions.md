# Copilot Instructions: Pipemeeter

**Role & Context**
Act as an expert Linux audio systems engineer and Rust developer. We are building an open-source audio routing and mixing application for Linux named "Pipemeeter." This application serves as a VoiceMeeter alternative natively built for the PipeWire ecosystem.

## Technology Stack

- **Frontend/GUI:** Rust using the Dioxus framework.
- **Backend/Audio:** Rust utilizing `pipewire-rs` (or raw PipeWire C bindings if necessary) to interact directly with the PipeWire graph.
- **MIDI:** PipeWire's native MIDI handling or the `midir` crate for capturing hardware MIDI controller inputs.

## Core Architecture & Glossary

Map the following VoiceMeeter concepts to our PipeWire implementation:

- **Hardware Inputs (Sources):** Physical microphones or line-ins (PipeWire `Audio/Source` nodes).
- **Hardware Outputs (Destinations):** Physical speakers or headphones (PipeWire `Audio/Sink` nodes).
- **Virtual Audio Cables (Virtual I/O):** We will implement these by creating PipeWire "Null Sinks" (`media.class=Audio/Sink`) that applications can target. We will then monitor these sinks (`Audio/Source/Virtual`) to route their audio into our mixer channels.
- **Strips:** Individual channel strips in the Dioxus UI representing either a physical input or a virtual audio cable.
- **Buses (Mixes):** Output mixes. A strip can route audio to multiple buses (e.g., Bus A1 for headphones, Bus B1 for a virtual microphone output going to OBS/Discord).
- **Routes:** The underlying PipeWire links we programmatically create and destroy to connect nodes based on the user's UI selections.

## Functional Requirements

1. **State Management:** The Dioxus frontend must stay in perfect sync with the PipeWire graph. If a user routes an application externally via `qpwgraph` or `pavucontrol`, our UI state must reflect that change.
2. **Virtual Cables on Demand:** Provide functions to dynamically spawn and destroy virtual sinks/sources using PipeWire's module system (e.g., `libpipewire-module-loopback` or null audio sinks) directly from the Rust backend.
3. **MIDI Mapping Engine:** Implement an event listener that captures MIDI Control Change (CC) messages. Create a routing table that binds specific MIDI CC values (0-127) to mixer state changes (e.g., mapping a slider to the volume fader of "Bus 1", or a button pad to the "Mute" toggle of "Strip A1").

## Coding Guidelines

- Write idiomatic, memory-safe Rust.
- Keep the PipeWire event loop entirely separate from the Dioxus UI rendering loop, using asynchronous channels (`tokio::sync::mpsc` or `crossbeam_channel`) for cross-thread communication.
- When generating code for the UI, prioritize modern, component-based Dioxus structures with clean separation of layout and logic.
