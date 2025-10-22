use std::sync::{Arc, mpsc};

use crate::capturer::Options;

use super::{GpuFrame, GpuVideoFrame};

#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "macos")]
pub type ChannelItem = (
    cidre::arc::R<cidre::cm::SampleBuf>,
    cidre::sc::stream::OutputType,
);
#[cfg(not(target_os = "macos"))]
pub type ChannelItem = crate::frame::Frame;

#[derive(thiserror::Error, Debug)]
pub enum EngineError {
    #[cfg(target_os = "macos")]
    #[error(transparent)]
    Mac(#[from] mac::MacEngineError),
    #[error("GPU capture is not implemented for this platform")]
    Unsupported,
}

#[derive(thiserror::Error, Debug)]
pub enum ProcessingError {
    #[cfg(target_os = "macos")]
    #[error(transparent)]
    Mac(#[from] mac::MacProcessingError),
    #[error("GPU capture is not implemented for this platform")]
    Unsupported,
}

pub struct Engine {
    options: Options,
    #[cfg(target_os = "macos")]
    mac: mac::MacEngine,
}

impl Engine {
    pub fn new(
        options: &Options,
        device: Arc<wgpu::Device>,
        tx: mpsc::Sender<ChannelItem>,
    ) -> Result<Self, EngineError> {
        #[cfg(target_os = "macos")]
        {
            let mac = mac::MacEngine::new(options, device, tx)?;
            Ok(Self {
                options: options.clone(),
                mac,
            })
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (options, device, tx);
            Err(EngineError::Unsupported)
        }
    }

    pub fn start(&mut self) {
        #[cfg(target_os = "macos")]
        {
            self.mac.start();
        }
    }

    pub fn stop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            self.mac.stop();
        }
    }

    pub fn get_output_frame_size(&mut self) -> [u32; 2] {
        #[cfg(target_os = "macos")]
        {
            self.mac.get_output_frame_size(&self.options)
        }

        #[cfg(not(target_os = "macos"))]
        {
            [0, 0]
        }
    }

    pub fn process_channel_item(
        &self,
        data: ChannelItem,
    ) -> Result<Option<GpuFrame>, ProcessingError> {
        #[cfg(target_os = "macos")]
        {
            self
                .mac
                .process_channel_item(data, &self.options)
                .map_err(ProcessingError::from)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = data;
            Err(ProcessingError::Unsupported)
        }
    }
}

pub(crate) fn build_video_frame(
    texture: wgpu::Texture,
    format: wgpu::TextureFormat,
    size: [u32; 2],
    display_time: std::time::SystemTime,
) -> GpuVideoFrame {
    GpuVideoFrame {
        texture,
        format,
        size,
        display_time,
    }
}
