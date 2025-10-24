pub mod engine;

use std::{
    sync::{Arc, mpsc},
    time::SystemTime,
};

use engine::{ChannelItem, Engine, EngineError, ProcessingError};

use crate::{
    capturer::Options,
    frame::{AudioFrame, FrameType},
    has_permission, is_supported,
};

/// Convenience re-exports so callers can configure the GPU capturer using the same types.
pub use crate::capturer::{Area, Point, Resolution, Size};

/// GPU-oriented frame emitted by [`GPUCapturer`].
pub enum GpuFrame {
    Video(GpuVideoFrame),
    Audio(AudioFrame),
}

/// Video frame that references a zero-copy [`wgpu::Texture`].
pub struct GpuVideoFrame {
    texture: wgpu::Texture,
    format: wgpu::TextureFormat,
    size: [u32; 2],
    display_time: SystemTime,
}

impl GpuVideoFrame {
    /// Returns the captured [`wgpu::Texture`].
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// Consumes the frame and returns the underlying [`wgpu::Texture`].
    pub fn into_texture(self) -> wgpu::Texture {
        self.texture
    }

    /// Captured texture format.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Returns the `[width, height]` of the captured frame.
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Timestamp derived from the original `CMSampleBuffer`.
    pub fn display_time(&self) -> SystemTime {
        self.display_time
    }

    /// Creates a [`wgpu::TextureView`] for the captured texture.
    pub fn create_view(&self, desc: &wgpu::TextureViewDescriptor) -> wgpu::TextureView {
        self.texture.create_view(desc)
    }

    /// Creates a default [`wgpu::TextureView`] covering the entire texture.
    pub fn create_default_view(&self) -> wgpu::TextureView {
        self.texture
            .create_view(&wgpu::TextureViewDescriptor::default())
    }
}

impl std::fmt::Debug for GpuVideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuVideoFrame")
            .field("size", &self.size)
            .field("format", &self.format)
            .field("display_time", &self.display_time)
            .finish_non_exhaustive()
    }
}

/// Errors that may occur while building a [`GPUCapturer`].
#[derive(thiserror::Error, Debug)]
pub enum GPUCapturerBuildError {
    #[error("screen capturing is not supported on this platform")]
    NotSupported,
    #[error("permission to capture the screen is not granted")]
    PermissionNotGranted,
    #[error("GPU capture currently requires BGRA output frames")]
    UnsupportedOutputType,
    #[error("GPU capture engine is unavailable: {0}")]
    Engine(&'static str),
    #[error(transparent)]
    Internal(#[from] EngineError),
}

/// Errors that may occur while retrieving frames from a [`GPUCapturer`].
#[derive(thiserror::Error, Debug)]
pub enum GPUFrameError {
    #[error(transparent)]
    Recv(#[from] mpsc::RecvError),
    #[error(transparent)]
    Processing(#[from] ProcessingError),
}

/// Non-blocking polling error wrapper for [`GPUCapturer::try_get_next_frame`].
#[derive(thiserror::Error, Debug)]
pub enum GPUFrameTryError {
    #[error(transparent)]
    Channel(#[from] mpsc::RecvError),
    #[error(transparent)]
    Processing(#[from] ProcessingError),
}

/// Screen capturer that yields zero-copy GPU textures backed by [`wgpu`].
pub struct GPUCapturer {
    engine: Engine,
    rx: mpsc::Receiver<ChannelItem>,
}

impl GPUCapturer {
    /// Builds a new [`GPUCapturer`] using the supplied options and [`wgpu::Device`].
    pub fn build(
        options: Options,
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
    ) -> Result<GPUCapturer, GPUCapturerBuildError> {
        if !is_supported() {
            return Err(GPUCapturerBuildError::NotSupported);
        }

        if !has_permission() {
            return Err(GPUCapturerBuildError::PermissionNotGranted);
        }

        if !matches!(options.output_type, FrameType::BGRAFrame) {
            return Err(GPUCapturerBuildError::UnsupportedOutputType);
        }

        let (tx, rx) = mpsc::channel();
        let engine = match Engine::new(&options, device, queue, tx) {
            Ok(engine) => engine,
            Err(EngineError::Unsupported) => {
                return Err(GPUCapturerBuildError::Engine(
                    "GPU capture is not available for this platform",
                ));
            }
            Err(other) => return Err(GPUCapturerBuildError::from(other)),
        };

        Ok(GPUCapturer { engine, rx })
    }

    /// Start capturing frames.
    pub fn start_capture(&mut self) {
        self.engine.start();
    }

    /// Stop the capture session.
    pub fn stop_capture(&mut self) {
        self.engine.stop();
    }

    /// Blocks until the next GPU frame (audio or video) is available.
    pub fn get_next_frame(&self) -> Result<GpuFrame, GPUFrameError> {
        loop {
            let item = self.rx.recv()?;
            match self.engine.process_channel_item(item) {
                Ok(Some(frame)) => return Ok(frame),
                Ok(None) => continue,
                Err(err) => return Err(err.into()),
            }
        }
    }

    /// Attempts to fetch the next available GPU frame without blocking.
    pub fn try_get_next_frame(&self) -> Result<Option<GpuFrame>, GPUFrameTryError> {
        loop {
            match self.rx.try_recv() {
                Ok(item) => match self.engine.process_channel_item(item) {
                    Ok(Some(frame)) => return Ok(Some(frame)),
                    Ok(None) => continue,
                    Err(err) => return Err(err.into()),
                },
                Err(mpsc::TryRecvError::Empty) => return Ok(None),
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(GPUFrameTryError::Channel(mpsc::RecvError));
                }
            }
        }
    }

    /// Returns the negotiated capture dimensions (`[width, height]`).
    pub fn get_output_frame_size(&mut self) -> [u32; 2] {
        self.engine.get_output_frame_size()
    }
}

impl From<GpuVideoFrame> for wgpu::Texture {
    fn from(value: GpuVideoFrame) -> Self {
        value.texture
    }
}
