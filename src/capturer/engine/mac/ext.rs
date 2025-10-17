use cidre::cg;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{CGDisplayCopyDisplayMode, CGDisplayMode};

pub trait DirectDisplayIdExt {
    fn display_mode(&self) -> Option<CFRetained<CGDisplayMode>>;
}

impl DirectDisplayIdExt for cg::DirectDisplayId {
    #[inline]
    fn display_mode(&self) -> Option<CFRetained<CGDisplayMode>> {
        CGDisplayCopyDisplayMode(self.0)
    }
}
