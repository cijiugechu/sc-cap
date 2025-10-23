//! Capture the main display using [`GPUCapturer`] and save each frame as a PNG.
//! Run with `cargo run --example gpu_capture_save`.

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use futures::executor::block_on;
    use sc_cap::{
        capturer::Options,
        frame::FrameType,
        gpu_capturer::{GpuFrame, GPUCapturer},
        has_permission, is_supported, request_permission,
    };
    use std::{
        path::Path,
        sync::Arc,
        time::{Instant, SystemTime},
    };

    if !is_supported() {
        eprintln!("Platform not supported");
        return Ok(());
    }

    if !has_permission() && !request_permission() {
        eprintln!("Screen capture permission denied");
        return Ok(());
    }

    let (device, queue) = block_on(initialize_wgpu_device())?;
    let device = Arc::new(device);

    let mut capturer = GPUCapturer::build(
        Options {
            fps: 60,
            output_type: FrameType::BGRAFrame,
            ..Default::default()
        },
        device.clone(),
    )?;

    capturer.start_capture();

    let start = Instant::now();
    let mut saved = 0usize;
    while saved < 10 {
        let frame = match capturer.get_next_frame()? {
            GpuFrame::Video(video) => video,
            GpuFrame::Audio(_) => continue,
        };

        let path = format!("gpu-frame-{saved:03}.png");
        save_frame_to_png(&device, &queue, &frame, Path::new(&path))?;
        println!(
            "Saved {path} @ {:?}",
            frame.display_time().duration_since(SystemTime::UNIX_EPOCH)?
        );
        saved += 1;
    }

    capturer.stop_capture();

    println!("Captured {} frames in {:?}", saved, start.elapsed());

    Ok(())
}

#[cfg(target_os = "macos")]
async fn initialize_wgpu_device(
) -> Result<(wgpu::Device, wgpu::Queue), Box<dyn std::error::Error>> {
    use wgpu::{
        DeviceDescriptor, Features, Instance, InstanceDescriptor, RequestAdapterError,
        RequestAdapterOptions, RequestDeviceError,
    };

    let instance = Instance::new(&InstanceDescriptor::default());
    let adapter = instance
        .request_adapter(&RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        })
        .await
        .map_err(|err: RequestAdapterError| format!("failed to request adapter: {err}"))?;

    let device_descriptor = DeviceDescriptor {
        label: Some("gpu-capture-device"),
        required_features: Features::empty(),
        required_limits: adapter.limits(),
        experimental_features: Default::default(),
        memory_hints: Default::default(),
        trace: Default::default(),
    };

    let (device, queue) = adapter
        .request_device(&device_descriptor)
        .await
        .map_err(|err: RequestDeviceError| format!("request_device failed: {err}"))?;

    Ok((device, queue))
}

#[cfg(target_os = "macos")]
fn save_frame_to_png(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &sc_cap::gpu_capturer::GpuVideoFrame,
    path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use futures::{channel::oneshot, executor::block_on};
    use image::{ImageFormat, RgbaImage};

    const BPP: u32 = 4;
    let [width, height] = frame.size();
    let raw_texture = frame.texture();
    let format = frame.format();

    if format != wgpu::TextureFormat::Bgra8Unorm
        && format != wgpu::TextureFormat::Bgra8UnormSrgb
    {
        return Err(format!("unexpected texture format: {format:?}").into());
    }

    let unpadded_bytes_per_row = width * BPP;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
    let buffer_size = padded_bytes_per_row as u64 * height as u64;

    let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gpu-capture-staging"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("gpu-capture-copy-encoder"),
    });

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: raw_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(encoder.finish()));

    let buffer_slice = staging_buffer.slice(..);
    let (sender, receiver) = oneshot::channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
        sender.send(result).ok();
    });
    let _ = device.poll(wgpu::wgt::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    let mapping_result = block_on(receiver).map_err(|_| "map_async cancelled")?;
    mapping_result.map_err(|err| format!("failed to map frame buffer: {err}"))?;

    let mapped = buffer_slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((width * height * BPP) as usize);
    for chunk in mapped.chunks(padded_bytes_per_row as usize).take(height as usize) {
        pixels.extend_from_slice(&chunk[..unpadded_bytes_per_row as usize]);
    }
    drop(mapped);
    staging_buffer.unmap();

    for bgra in pixels.chunks_exact_mut(BPP as usize) {
        bgra.swap(0, 2);
    }

    let image =
        RgbaImage::from_raw(width, height, pixels).ok_or("failed to create PNG image buffer")?;
    image.save_with_format(path, ImageFormat::Png)?;

    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("GPU capture example is only available on macOS with ScreenCaptureKit.");
}
