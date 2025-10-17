#![allow(unexpected_cfgs)]

use cidre::{cg, sc};
use futures::executor::block_on;
use objc2::MainThreadMarker;
use objc2_app_kit::NSApp;
use objc2_core_graphics::CGDisplayMode;
use objc2_foundation::{NSInteger, NSRect};

use crate::engine::mac::ext::DirectDisplayIdExt;

use super::{Display, Target};

#[inline]
fn main_thread_marker() -> MainThreadMarker {
    MainThreadMarker::new().expect("macOS target APIs must be called on the main thread")
}

fn get_display_name(display_id: cg::DirectDisplayId) -> String {
    // On newer macOS versions the -[NSScreen CGDirectDisplayID] selector is not available.
    // Avoid calling into AppKit for this mapping and use a stable fallback name instead.
    format!("Display {}", display_id.0)
}

pub fn get_all_targets() -> Vec<Target> {
    let mut targets: Vec<Target> = Vec::new();

    let content = block_on(sc::ShareableContent::current()).unwrap();

    // Add displays to targets
    for display in content.displays().iter() {
        let id = display.display_id();

        let title = get_display_name(id);

        let target = Target::Display(super::Display {
            id: id.0,
            title,
            raw_handle: id,
        });

        targets.push(target);
    }

    // Add windows to targets
    for window in content.windows().iter() {
        let id = window.id();
        let title = window
            .title()
            // on intel chips we can have Some but also a null pointer for some reason
            .filter(|v| !unsafe { v.utf8_chars_ar().is_null() });

        let target = Target::Window(super::Window {
            id,
            title: title.map(|v| v.to_string()).unwrap_or_default(),
            raw_handle: id,
        });
        targets.push(target);
    }

    targets
}

pub fn get_main_display() -> Display {
    let id = cg::direct_display::Id::main();
    let title = get_display_name(id);

    Display {
        id: id.0,
        title,
        raw_handle: id,
    }
}

pub fn get_scale_factor(target: &Target) -> f64 {
    match target {
        Target::Window(window) => {
            let mtm = main_thread_marker();
            let app = NSApp(mtm);
            let window_id = window.raw_handle as NSInteger;

            app.windowWithWindowNumber(window_id)
                .map(|ns_window| ns_window.backingScaleFactor())
                .unwrap_or(1.0)
        }
        Target::Display(display) => {
            let mode = display.raw_handle.display_mode().unwrap();
            let pixel_width = CGDisplayMode::pixel_width(Some(&mode)) as f64;
            let width = CGDisplayMode::width(Some(&mode)) as f64;
            pixel_width / width
        }
    }
}

pub fn get_target_dimensions(target: &Target) -> (u64, u64) {
    match target {
        Target::Window(window) => {
            let mtm = main_thread_marker();
            let app = NSApp(mtm);
            let window_id = window.raw_handle as NSInteger;

            if let Some(ns_window) = app.windowWithWindowNumber(window_id) {
                let frame: NSRect = ns_window.frame();
                (frame.size.width as u64, frame.size.height as u64)
            } else {
                (0, 0)
            }
        }
        Target::Display(display) => {
            let mode = display.raw_handle.display_mode().unwrap();
            let width = CGDisplayMode::width(Some(&mode)) as u64;
            let height = CGDisplayMode::height(Some(&mode)) as u64;
            (width, height)
        }
    }
}
