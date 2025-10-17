# Project Overview

`sc-cap` is a cross-platform Rust crate for high-quality screen capture. The library layers a portable API over OS-specific backends for macOS (ScreenCaptureKit), Windows (Windows.Graphics.Capture via `windows-capture`), and Linux (Pipewire-based prototype). The public API is exported through `src/lib.rs`, while `src/main.rs` provides a minimal example binary.

## Coding Conventions

- Run `cargo check`, `cargo test` and `cargo clippy` automatically after making code changes.

## Module Map

- `capturer.rs`
  - Defines [`Options`](#capture-options) for configuring capture sessions.
  - Hosts the [`Capturer`](#capturer) type, a high-level controller that orchestrates the platform-specific [`engine`](#engines) implementation and receives frames through an `mpsc` channel.
  - Provides convenience helpers: `get_output_frame_size`, `try_get_next_frame`, and the macOS-only [`RawCapturer`](#rawcapturer) view for direct buffer access.

- `capturer/engine`
  - `engine.rs` selects the appropriate backend (`mac`, `win`, or `linux`) and forwards start/stop/process calls.
  - `mac/` integrates ScreenCaptureKit streams, pixel format conversion helpers, and error handling.
  - `win.rs` wraps the `windows-capture` API, applies optional cropping, converts frames to `BGRA`, and wires optional audio capture via `cpal`.
  - `linux.rs` / `linux/` host Pipewire portal bindings (work in progress).

- `frame.rs`
  - Aggregates audio/video frame representations and the `Frame` enum returned by the capturer.
  - `frame/audio.rs` defines `AudioFrame` with metadata accessors and the `AudioFormat` enum.
  - `frame/video.rs` defines the `FrameType` configuration flags plus concrete RGB/YUV frame structs and helpers for pixel conversion/cropping.

- `targets.rs`
  - Normalizes platform-specific window/display discovery. Per-platform files expose OS handles and helpers for scale factors, main display selection, and geometry queries.

- `utils.rs`
  - Surface-level platform checks for support and permission prompts (`mac` uses ScreenCapture APIs, `win` defers to `GraphicsCaptureApi`, `linux` placeholders currently return `true`).

## Core Types & Functions

### Capture Options
`capturer::Options` gathers capture preferences:
- `fps`, `show_cursor`, `show_highlight` toggles.
- `target` / `excluded_targets` for selecting windows or displays.
- `crop_area` (`Area { origin: Point, size: Size }`) for partial capture.
- `output_type` (`frame::FrameType`) and `output_resolution` (`capturer::Resolution`, or `Captured` to preserve native dimensions).
- Audio controls: `captures_audio`, `exclude_current_process_audio` (the latter used by Windows backend).

### Capturer
`capturer::Capturer` is constructed via `Capturer::build(options)` which enforces `is_supported()` and `has_permission()` before instantiating the platform engine. Key methods:
- `start_capture()` / `stop_capture()` toggle the underlying stream.
- `get_next_frame()` blocks for the next `Frame` (audio or video).
- `try_get_next_frame()` polls without blocking, filtering backend-specific control items.
- `get_output_frame_size()` reports the negotiated capture resolution.
- `raw()` (macOS) returns a [`RawCapturer`](#rawcapturer).

### RawCapturer
`RawCapturer::get_next_sample_buffer()` (macOS only) yields the retained `SampleBuf` and `OutputType` pair straight from ScreenCaptureKit, allowing consumers to bypass conversion overhead when managing pixel buffers manually.

### Frame Module
- `Frame` enum differentiates `Frame::Audio(AudioFrame)` and `Frame::Video(VideoFrame)`.
- `VideoFrame` variants (YUV, RGB, RGBx, XBGR, BGRx, BGR0, BGRA) align with `FrameType` requests.
- Utility helpers (`remove_alpha_channel`, `convert_bgra_to_rgb`, `get_cropped_data`) transform or crop pixel buffers.
- `AudioFrame` exposes metadata accessors (`format`, `planes`, `channels`, `rate`, `is_planar`, `sample_count`, `time`) and plane views for planar layouts. `AudioFormat` enumerates primitive sample encodings with `sample_size()` helpers.

### Targets Module
- `targets::get_all_targets()` returns `Vec<Target>` (display or window) populated per OS via native enumeration APIs.
- `targets::get_main_display()` selects the primary monitor (unimplemented on Linux).
- `targets::get_scale_factor()` and `targets::get_target_dimensions()` provide DPI-aware sizing.
- `Target` wraps platform-specific handles (`cidre` IDs on macOS, Win32 HWND/HMONITOR on Windows) for downstream configuration.

### Utility Helpers
- `utils::has_permission()` / `request_permission()` mediate ScreenCaptureKit prompts on macOS and act as stubs elsewhere.
- `utils::is_supported()` evaluates platform capability (macOS version check, Win32 API availability, placeholder `true` on Linux pending implementation).

## Additional Notes
- The Windows backend (`capturer/engine/win.rs`) configures `windows_capture` settings (cursor highlight, draw border, dirty-region updates) and spawns an optional `cpal` audio stream when `Options::captures_audio` is true.
- macOS stream setup (`capturer/engine/mac.rs`) builds `sc::ContentFilter` for windows or displays, applies exclusions, configures pixel format + FPS, and handles audio/video multiplexing through `StreamOutput` callbacks.
- Linux support is under active development; permission/support helpers currently stub out and target discovery returns an empty list, as window selection occurs via the portal dialog during capture creation.

