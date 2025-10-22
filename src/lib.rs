//! Cross Platform, Performant and High Quality screen recordings

pub mod capturer;
pub mod frame;
pub mod gpu_capturer;
mod targets;
mod utils;

// Helper Methods
pub use targets::{Display, Target};
pub use targets::{get_all_targets, get_main_display};
pub use utils::has_permission;
pub use utils::is_supported;
pub use utils::request_permission;

#[cfg(target_os = "macos")]
pub mod engine {
    pub use crate::capturer::engine::mac;
}
