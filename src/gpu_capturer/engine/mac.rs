use std::{
    ffi::c_void,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime},
};

use cidre::{arc, cm, mach, sc};
use core_foundation::base::{CFRelease, kCFAllocatorDefault};
use metal::{foreign_types::ForeignType, Device, MTLPixelFormat, MTLTextureType, MTLTextureUsage, Texture};
use wgpu::TextureDimension;
use wgpu_hal::{CopyExtent, api::Metal as HalMetal};

use super::{ChannelItem, build_video_frame};
use crate::{
    capturer::{
        Options,
        engine::mac as cpu_mac,
        engine::mac::{Capturer, ErrorHandler, get_output_frame_size as cpu_output_frame_size},
    },
    frame::{Frame, FrameType},
};

const TEXTURE_LABEL: &str = "sc-cap gpu capture frame";

pub struct MacEngine {
    capturer: (arc::R<Capturer>, arc::R<ErrorHandler>, arc::R<sc::Stream>),
    error_flag: Arc<AtomicBool>,
    texture_cache: MetalTextureCache,
    device: Arc<wgpu::Device>,
}

#[derive(thiserror::Error, Debug)]
pub enum MacEngineError {
    #[error("failed to create ScreenCaptureKit capturer: {0}")]
    CreateCapturer(String),
    #[error("Metal backend unavailable for supplied wgpu::Device")]
    HalUnavailable,
    #[error("failed to create CVMetalTextureCache (status={0})")]
    TextureCache(CVReturn),
}

#[derive(thiserror::Error, Debug)]
pub enum MacProcessingError {
    #[error("capture stream terminated")]
    StreamStopped,
    #[error("frame dropped by ScreenCaptureKit pipeline")]
    FrameDropped,
    #[error("frame metadata missing status")]
    MissingFrameStatus,
    #[error("unexpected frame status {0}")]
    UnknownFrameStatus(i32),
    #[error("sample buffer missing image data")]
    MissingImageBuffer,
    #[error("failed to create Metal texture from pixel buffer (status={0})")]
    TextureCache(CVReturn),
    #[error("CVMetalTexture returned null MTLTexture")]
    NullMetalTexture,
    #[error("captured texture dimension {0} exceeds u32::MAX")]
    DimensionsTooLarge(u64),
    #[error("Metal backend unavailable for supplied wgpu::Device")]
    HalUnavailable,
    #[error("pixel format {0:?} not supported for GPU capture")]
    UnsupportedPixelFormat(MTLPixelFormat),
    #[error("texture type {0:?} not supported for GPU capture")]
    UnsupportedTextureType(MTLTextureType),
    #[error("unexpected ScreenCaptureKit output type")]
    UnexpectedOutputType,
    #[error("audio conversion failed")]
    AudioConversion,
}

impl MacEngine {
    pub fn new(
        options: &Options,
        device: Arc<wgpu::Device>,
        tx: mpsc::Sender<ChannelItem>,
    ) -> Result<Self, MacEngineError> {
        let error_flag = Arc::new(AtomicBool::new(false));
        let capturer = cpu_mac::create_capturer(options, tx, error_flag.clone())
            .map_err(|err| MacEngineError::CreateCapturer(err.to_string()))?;

        let hal_device =
            unsafe { device.as_hal::<HalMetal>() }.ok_or(MacEngineError::HalUnavailable)?;
        let metal_device_guard = hal_device.raw_device().lock();
        let metal_device = metal_device_guard.clone();
        let texture_cache =
            MetalTextureCache::new(metal_device).map_err(MacEngineError::TextureCache)?;
        drop(metal_device_guard);

        Ok(Self {
            capturer,
            error_flag,
            texture_cache,
            device,
        })
    }

    pub fn start(&self) {
        futures::executor::block_on(self.capturer.2.start()).expect("Failed to start capture");
    }

    pub fn stop(&self) {
        futures::executor::block_on(self.capturer.2.stop()).expect("Failed to stop capture");
    }

    pub fn get_output_frame_size(&self, options: &Options) -> [u32; 2] {
        cpu_output_frame_size(options)
    }

    pub fn process_channel_item(
        &self,
        data: ChannelItem,
        options: &Options,
    ) -> Result<Option<super::GpuFrame>, MacProcessingError> {
        if self.error_flag.load(Ordering::Relaxed) {
            return Err(MacProcessingError::StreamStopped);
        }

        match data.1 {
            sc::stream::OutputType::Screen => self.process_video(data.0, options),
            sc::stream::OutputType::Audio => self.process_audio(data.0),
            _ => Err(MacProcessingError::UnexpectedOutputType),
        }
    }

    fn process_audio(
        &self,
        sample: arc::R<cm::SampleBuf>,
    ) -> Result<Option<super::GpuFrame>, MacProcessingError> {
        let frame = cpu_mac::process_sample_buffer(
            sample,
            sc::stream::OutputType::Audio,
            FrameType::BGRAFrame,
        )
        .ok_or(MacProcessingError::AudioConversion)?;

        match frame {
            Frame::Audio(audio) => Ok(Some(super::GpuFrame::Audio(audio))),
            _ => Err(MacProcessingError::AudioConversion),
        }
    }

    fn process_video(
        &self,
        sample: arc::R<cm::SampleBuf>,
        _options: &Options,
    ) -> Result<Option<super::GpuFrame>, MacProcessingError> {
        let status = sample
            .attaches(false)
            .and_then(|a| a.iter().next())
            .and_then(|dict| dict.get(sc::FrameInfo::status().as_cf()))
            .and_then(|value| value.as_number().to_i32())
            .ok_or(MacProcessingError::MissingFrameStatus)?;

        match status {
            0 => {}
            1 => return Err(MacProcessingError::FrameDropped),
            other => return Err(MacProcessingError::UnknownFrameStatus(other)),
        }

        let display_time = compute_display_time(sample.as_ref());

        let image_buffer = sample
            .image_buf()
            .ok_or(MacProcessingError::MissingImageBuffer)?;

        let raw_image =
            unsafe { image_buffer.as_ref().as_type_ref().as_type_ptr() } as CVImageBufferRef;

        let size = image_buffer.encoded_size();
        let width = size.width.round() as usize;
        let height = size.height.round() as usize;

        if width == 0 || height == 0 {
            return Ok(None);
        }

        let pixel_format = MTLPixelFormat::BGRA8Unorm;

        let cache_texture = self
            .texture_cache
            .create_texture(raw_image, pixel_format, width, height)
            .map_err(MacProcessingError::TextureCache)?;

        let metal_texture = cache_texture.into_metal_texture()?;

        let _hal_device_guard = unsafe { self.device.as_hal::<HalMetal>() }
            .ok_or(MacProcessingError::HalUnavailable)?;

        let width = u32::try_from(metal_texture.width())
            .map_err(|_| MacProcessingError::DimensionsTooLarge(metal_texture.width()))?;
        let height = u32::try_from(metal_texture.height())
            .map_err(|_| MacProcessingError::DimensionsTooLarge(metal_texture.height()))?;
        let depth = u32::try_from(std::cmp::max(1, metal_texture.depth()))
            .map_err(|_| MacProcessingError::DimensionsTooLarge(metal_texture.depth()))?;

        let texture_type = metal_texture.texture_type();
        let format = map_pixel_format(metal_texture.pixel_format())?;
        let dimension = map_texture_dimension(texture_type)?;
        let usage = map_texture_usage(metal_texture.usage()) | wgpu::TextureUsages::COPY_SRC;
        let mip_levels = metal_texture.mipmap_level_count() as u32;
        let array_layers = std::cmp::max(1, metal_texture.array_length() as u32);
        let sample_count = std::cmp::max(1, metal_texture.sample_count() as u32);

        let hal_texture = unsafe {
            wgpu_hal::metal::Device::texture_from_raw(
                metal_texture,
                format,
                texture_type,
                array_layers,
                mip_levels,
                CopyExtent {
                    width,
                    height,
                    depth,
                },
            )
        };

        let texture = unsafe {
            self.device.create_texture_from_hal::<HalMetal>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some(TEXTURE_LABEL),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: if dimension == TextureDimension::D3 {
                            depth
                        } else {
                            array_layers
                        },
                    },
                    mip_level_count: mip_levels,
                    sample_count,
                    dimension,
                    format,
                    usage,
                    view_formats: &[],
                },
            )
        };

        let video = build_video_frame(texture, format, [width, height], display_time);
        Ok(Some(super::GpuFrame::Video(video)))
    }
}

fn compute_display_time(sample: &cm::SampleBuf) -> SystemTime {
    let system_time = SystemTime::now();
    let system_mach_time = mach::abs_time();

    let frame_cm_time = sample.pts();
    let frame_mach_time = cm::Clock::convert_host_time_to_sys_units(frame_cm_time);

    let mach_time_diff = if frame_mach_time > system_mach_time {
        (frame_mach_time - system_mach_time) as i64
    } else {
        -((system_mach_time - frame_mach_time) as i64)
    };

    let mach_timebase = mach::TimeBaseInfo::new();
    let nanos_diff = (mach_time_diff * mach_timebase.numer as i64) / mach_timebase.denom as i64;

    if nanos_diff >= 0 {
        system_time + Duration::from_nanos(nanos_diff as u64)
    } else {
        system_time - Duration::from_nanos((-nanos_diff) as u64)
    }
}

fn map_pixel_format(format: MTLPixelFormat) -> Result<wgpu::TextureFormat, MacProcessingError> {
    match format {
        MTLPixelFormat::BGRA8Unorm => Ok(wgpu::TextureFormat::Bgra8Unorm),
        MTLPixelFormat::BGRA8Unorm_sRGB => Ok(wgpu::TextureFormat::Bgra8UnormSrgb),
        other => Err(MacProcessingError::UnsupportedPixelFormat(other)),
    }
}

fn map_texture_dimension(ty: MTLTextureType) -> Result<TextureDimension, MacProcessingError> {
    match ty {
        MTLTextureType::D1 | MTLTextureType::D1Array => Ok(TextureDimension::D1),
        MTLTextureType::D2
        | MTLTextureType::D2Array
        | MTLTextureType::D2Multisample
        | MTLTextureType::Cube
        | MTLTextureType::CubeArray => Ok(TextureDimension::D2),
        MTLTextureType::D3 => Ok(TextureDimension::D3),
        other => Err(MacProcessingError::UnsupportedTextureType(other)),
    }
}

fn map_texture_usage(usage: MTLTextureUsage) -> wgpu::TextureUsages {
    let mut flags = wgpu::TextureUsages::empty();

    if usage.contains(MTLTextureUsage::ShaderRead) {
        flags |= wgpu::TextureUsages::TEXTURE_BINDING;
    }
    if usage.contains(MTLTextureUsage::ShaderWrite) {
        flags |= wgpu::TextureUsages::STORAGE_BINDING;
    }
    if usage.contains(MTLTextureUsage::RenderTarget) {
        flags |= wgpu::TextureUsages::RENDER_ATTACHMENT;
    }

    flags
}

struct MetalTextureCache {
    raw: CVMetalTextureCacheRef,
}

unsafe impl Send for MetalTextureCache {}
unsafe impl Sync for MetalTextureCache {}

impl MetalTextureCache {
    fn new(device: Device) -> Result<Self, CVReturn> {
        let mut cache = std::ptr::null_mut();
        let status = unsafe {
            CVMetalTextureCacheCreate(
                kCFAllocatorDefault,
                std::ptr::null(),
                device.as_ptr() as *mut objc::runtime::Object,
                std::ptr::null(),
                &mut cache,
            )
        };

        if status == K_CV_RETURN_SUCCESS {
            Ok(Self { raw: cache })
        } else {
            Err(status)
        }
    }

    fn create_texture(
        &self,
        image: CVImageBufferRef,
        pixel_format: MTLPixelFormat,
        width: usize,
        height: usize,
    ) -> Result<MetalTexture, CVReturn> {
        let mut texture = std::ptr::null_mut();
        let status = unsafe {
            CVMetalTextureCacheCreateTextureFromImage(
                kCFAllocatorDefault,
                self.raw,
                image,
                std::ptr::null(),
                pixel_format as u64,
                width,
                height,
                0,
                &mut texture,
            )
        };

        if status == K_CV_RETURN_SUCCESS {
            Ok(MetalTexture { raw: texture })
        } else {
            Err(status)
        }
    }
}

impl Drop for MetalTextureCache {
    fn drop(&mut self) {
        unsafe {
            CVMetalTextureCacheFlush(self.raw, 0);
            CFRelease(self.raw as _);
        }
    }
}

struct MetalTexture {
    raw: CVMetalTextureRef,
}

impl MetalTexture {
    fn into_metal_texture(self) -> Result<Texture, MacProcessingError> {
        let raw = self.raw;
        std::mem::forget(self);

        let texture_ptr = unsafe { CVMetalTextureGetTexture(raw) };
        if texture_ptr.is_null() {
            unsafe { CFRelease(raw as _) };
            return Err(MacProcessingError::NullMetalTexture);
        }

        unsafe {
            objc::runtime::objc_retain(texture_ptr);
            CFRelease(raw as _);
        }

        Ok(unsafe { Texture::from_ptr(texture_ptr.cast()) })
    }
}

impl Drop for MetalTexture {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.raw as _);
        }
    }
}

type CVMetalTextureCacheRef = *mut c_void;
type CVMetalTextureRef = *mut c_void;
type CVImageBufferRef = *mut c_void;
type CVReturn = i32;

const K_CV_RETURN_SUCCESS: CVReturn = 0;

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVMetalTextureCacheCreate(
        allocator: *const c_void,
        cache_attrs: *const c_void,
        metal_device: *mut objc::runtime::Object,
        texture_attrs: *const c_void,
        cache_out: *mut CVMetalTextureCacheRef,
    ) -> CVReturn;

    fn CVMetalTextureCacheCreateTextureFromImage(
        allocator: *const c_void,
        texture_cache: CVMetalTextureCacheRef,
        source_image: CVImageBufferRef,
        texture_attrs: *const c_void,
        pixel_format: u64,
        width: usize,
        height: usize,
        plane_index: usize,
        texture_out: *mut CVMetalTextureRef,
    ) -> CVReturn;

    fn CVMetalTextureCacheFlush(texture_cache: CVMetalTextureCacheRef, options: u64);

    fn CVMetalTextureGetTexture(image: CVMetalTextureRef) -> *mut objc::runtime::Object;
}
