# ADR 0003: Native Rust + egui and local-first interaction

Status: Accepted

## Context

Rivet used a Tauri + React/WebView desktop architecture. Rivetr demonstrated a native Rust + egui successor and provided working calendar UI code.

Ephemeris requires low-latency interaction with large local temporal corpora and should remain useful during network failure.

## Decision

Ephemeris will be implemented as a native Rust application using `eframe`/`egui` unless a later ADR changes the UI stack.

Interactive calendar rendering, querying, filtering, inspection, and saved-view use must operate on local state without required network round trips.

Tauri and React/WebView are not part of the intended application architecture.

## Consequences

Positive:

- simpler native runtime
- lower interaction overhead
- direct reuse of selected Rivetr calendar code
- strong fit for offline/local operation

Cost:

- web UI ecosystems/components are not directly reusable
- some sophisticated UI behaviors must be implemented natively
