use std::sync::{Arc, mpsc};

use crate::{
	capturer::Options,
	capturer::engine::linux::LinCapError,
	frame::{BGRxFrame, Frame, RGBFrame, RGBxFrame, VideoFrame, XBGRFrame},
};

use super::{ChannelItem, build_video_frame, GpuFrame};

pub struct LinuxEngine {
	device: Arc<wgpu::Device>,
	queue: Arc<wgpu::Queue>,
	output_size: std::cell::Cell<[u32; 2]>,
	// Keep the CPU capturer alive and controllable
	inner: crate::capturer::engine::linux::LinuxCapturer,
}

#[derive(thiserror::Error, Debug)]
pub enum LinuxEngineError {
	#[error("failed to create PipeWire capturer: {0}")]
	CreateCapturer(#[source] LinCapError),
	#[error("PipeWire capturer panicked: {0}")]
	CapturerPanicked(String),
}

#[derive(thiserror::Error, Debug)]
pub enum LinuxProcessingError {
	#[error("unexpected audio frame for GPU engine")]
	UnexpectedAudio,
	#[error("unsupported pixel format for GPU upload")]
	UnsupportedFormat,
	#[error("invalid dimensions")] 
	InvalidDimensions,
}

impl LinuxEngine {
	pub fn new(
		options: &Options,
		device: Arc<wgpu::Device>,
		queue: Arc<wgpu::Queue>,
		tx: mpsc::Sender<ChannelItem>,
	) -> Result<Self, LinuxEngineError> {
		// The CPU capturer constructor may fail due to portal issues.
		let inner_result = std::panic::catch_unwind({
			let options = options.clone();
			let tx = tx.clone();
			move || crate::capturer::engine::linux::try_create_capturer(&options, tx)
		});

		let inner = match inner_result {
			Ok(Ok(inner)) => {
				inner
			}
			Ok(Err(err)) => {
				return Err(LinuxEngineError::CreateCapturer(err));
			}
			Err(panic_payload) => {
				let panic_msg = if let Some(msg) = panic_payload.downcast_ref::<&str>() {
					(*msg).to_string()
				} else if let Some(msg) = panic_payload.downcast_ref::<String>() {
					msg.clone()
				} else {
					"unknown panic".to_string()
				};
				return Err(LinuxEngineError::CapturerPanicked(panic_msg));
			}
		};

		Ok(Self {
			device,
			queue,
			output_size: std::cell::Cell::new([0, 0]),
			inner,
		})
	}

	pub fn start(&mut self) {
		self.inner.start_capture();
	}

	pub fn stop(&mut self) {
		self.inner.stop_capture();
	}

	pub fn get_output_frame_size(&self) -> [u32; 2] {
		self.output_size.get()
	}

	pub fn process_channel_item(
		&self,
		data: ChannelItem,
	) -> Result<Option<GpuFrame>, LinuxProcessingError> {
		match data {
			Frame::Audio(_) => Err(LinuxProcessingError::UnexpectedAudio),
			Frame::Video(video) => self.process_video(video),
		}
	}

	fn process_video(
		&self,
		video: VideoFrame,
	) -> Result<Option<GpuFrame>, LinuxProcessingError> {
		let (display_time, width_i32, height_i32, converted_bgra) = match video {
			VideoFrame::BGRx(BGRxFrame { display_time, width, height, data }) => {
				// Convert BGRx -> BGRA (alpha=255)
				let mut out = Vec::with_capacity((width as usize) * (height as usize) * 4);
				for px in data.chunks_exact(4) {
					out.extend_from_slice(&[px[0], px[1], px[2], 255]);
				}
				(display_time, width, height, out)
			}
			VideoFrame::RGBx(RGBxFrame { display_time, width, height, data }) => {
				// Convert RGBx -> BGRA
				let mut out = Vec::with_capacity((width as usize) * (height as usize) * 4);
				for px in data.chunks_exact(4) {
					out.extend_from_slice(&[px[2], px[1], px[0], 255]);
				}
				(display_time, width, height, out)
			}
			VideoFrame::XBGR(XBGRFrame { display_time, width, height, data }) => {
				// Convert XBGR -> BGRA (drop leading X)
				let mut out = Vec::with_capacity((width as usize) * (height as usize) * 4);
				for px in data.chunks_exact(4) {
					out.extend_from_slice(&[px[1], px[2], px[3], 255]);
				}
				(display_time, width, height, out)
			}
			VideoFrame::RGB(RGBFrame { display_time, width, height, data }) => {
				// Convert RGB -> BGRA
				let mut out = Vec::with_capacity((width as usize) * (height as usize) * 4);
				for px in data.chunks_exact(3) {
					out.extend_from_slice(&[px[2], px[1], px[0], 255]);
				}
				(display_time, width, height, out)
			}
			_ => return Err(LinuxProcessingError::UnsupportedFormat),
		};

		let width = u32::try_from(width_i32).map_err(|_| LinuxProcessingError::InvalidDimensions)?;
		let height = u32::try_from(height_i32).map_err(|_| LinuxProcessingError::InvalidDimensions)?;
		if width == 0 || height == 0 { return Ok(None); }

		self.output_size.set([width, height]);

		let texture = self.device.create_texture(&wgpu::TextureDescriptor {
			label: Some("sc-cap linux gpu frame"),
			size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Bgra8Unorm,
			usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
			view_formats: &[],
		});

		let bytes_per_row = width * 4;
		let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bpr = bytes_per_row.div_ceil(align) * align;
		if padded_bpr == bytes_per_row {
			self.queue.write_texture(
				wgpu::TexelCopyTextureInfo {
					texture: &texture,
					mip_level: 0,
					origin: wgpu::Origin3d::ZERO,
					aspect: wgpu::TextureAspect::All,
				},
				&converted_bgra,
				wgpu::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(bytes_per_row),
					rows_per_image: Some(height),
				},
				wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
			);
		} else {
			let mut padded = vec![0u8; (padded_bpr * height) as usize];
			for row in 0..height as usize {
				let src_off = row * bytes_per_row as usize;
				let dst_off = row * padded_bpr as usize;
				padded[dst_off..dst_off + bytes_per_row as usize]
					.copy_from_slice(&converted_bgra[src_off..src_off + bytes_per_row as usize]);
			}
			self.queue.write_texture(
				wgpu::TexelCopyTextureInfo {
					texture: &texture,
					mip_level: 0,
					origin: wgpu::Origin3d::ZERO,
					aspect: wgpu::TextureAspect::All,
				},
				&padded,
				wgpu::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(padded_bpr),
					rows_per_image: Some(height),
				},
				wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
			);
		}

		let video = build_video_frame(texture, wgpu::TextureFormat::Bgra8Unorm, [width, height], display_time);
		Ok(Some(GpuFrame::Video(video)))
	}
}
