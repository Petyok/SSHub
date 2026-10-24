//! Webcam capture (feature `camera`): grayscale frames into [`ReplyCollector`] until the phone's
//! looping reverse QR has delivered the whole reply. Frames live in memory only and are dropped
//! when the thread ends.
//!
//! Backends: Linux talks V4L2 directly through the pure-Rust `linuxvideo` crate (no libv4l, no
//! bindgen, nothing to install); macOS goes through `nokhwa` (AVFoundation). Elsewhere the
//! feature builds and reports that capture is not implemented.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::qrscan::{Gray, ReplyCollector, Scan};
use super::Emitter;

/// One luma image, row-major, tightly packed.
struct Frame {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

#[cfg(target_os = "linux")]
mod backend {
    use std::path::PathBuf;

    use linuxvideo::format::{PixFormat, PixelFormat};
    use linuxvideo::stream::ReadStream;
    use linuxvideo::{CapabilityFlags, Device};

    use super::Frame;

    fn capture_nodes() -> Vec<PathBuf> {
        let Ok(list) = linuxvideo::list() else {
            return Vec::new();
        };
        list.filter_map(Result::ok)
            .filter(|d| {
                d.capabilities()
                    .map(|c| {
                        c.device_capabilities()
                            .contains(CapabilityFlags::VIDEO_CAPTURE)
                    })
                    .unwrap_or(false)
            })
            .filter_map(|d| d.path().ok())
            .collect()
    }

    pub(super) fn present() -> bool {
        !capture_nodes().is_empty()
    }

    pub(super) fn devices() -> Vec<String> {
        capture_nodes()
            .into_iter()
            .map(|p| {
                let card = Device::open(&p)
                    .and_then(|d| d.capabilities())
                    .map(|c| c.card().to_string())
                    .unwrap_or_default();
                format!("{} {card}", p.display())
            })
            .collect()
    }

    enum Layout {
        Yuyv,
        Grey,
    }

    pub(super) struct Source {
        stream: ReadStream,
        width: usize,
        height: usize,
        stride: usize,
        layout: Layout,
        pub(super) name: String,
    }

    /// Raw formats a QR can be read from without a decoder, best first. 1280x720 first: a phone
    /// held 30 cm from a laptop camera fills about a third of the frame, and a version 11 code has
    /// 69 modules with the quiet zone.
    const WANTED: [(u32, u32, [u8; 4]); 4] = [
        (1280, 720, *b"YUYV"),
        (1280, 720, *b"GREY"),
        (640, 480, *b"YUYV"),
        (640, 480, *b"GREY"),
    ];

    pub(super) fn open() -> Result<Source, String> {
        let nodes = capture_nodes();
        if nodes.is_empty() {
            return Err("no camera found".into());
        }
        let mut last = String::from("no camera offers raw YUYV or GREY video");
        for path in nodes {
            for (w, h, fourcc) in WANTED {
                let dev = match Device::open(&path) {
                    Ok(d) => d,
                    Err(e) => {
                        last = format!("{}: {e}", path.display());
                        continue;
                    }
                };
                let name = dev
                    .capabilities()
                    .map(|c| c.card().to_string())
                    .unwrap_or_else(|_| path.display().to_string());
                let capture =
                    match dev.video_capture(PixFormat::new(w, h, PixelFormat::from_fourcc(fourcc)))
                    {
                        Ok(c) => c,
                        Err(e) => {
                            last = format!("{name}: {e}");
                            continue;
                        }
                    };
                let fmt = capture.format();
                let layout = match fmt.pixel_format().as_fourcc() {
                    f if f == *b"YUYV" => Layout::Yuyv,
                    f if f == *b"GREY" => Layout::Grey,
                    _ => continue,
                };
                let (width, height, stride) = (
                    fmt.width() as usize,
                    fmt.height() as usize,
                    fmt.bytes_per_line() as usize,
                );
                return match capture.into_stream() {
                    Ok(stream) => Ok(Source {
                        stream,
                        width,
                        height,
                        stride,
                        layout,
                        name: format!("{name} {width}x{height}"),
                    }),
                    Err(e) => Err(format!("{name}: {e}")),
                };
            }
        }
        Err(last)
    }

    impl Source {
        /// The newest frame, or `None` if the camera has none ready yet.
        pub(super) fn latest(&mut self) -> Result<Option<Frame>, String> {
            let mut out = None;
            // skip queued older frames: a slow decode must not read stale ones
            while !self.stream.will_block().map_err(|e| e.to_string())? {
                let (w, h, stride) = (self.width, self.height, self.stride);
                let yuyv = matches!(self.layout, Layout::Yuyv);
                out = Some(
                    self.stream
                        .dequeue(|buf| {
                            let step = if yuyv { 2 } else { 1 };
                            let mut px = Vec::with_capacity(w * h);
                            for y in 0..h {
                                let row = &buf[(y * stride).min(buf.len())..];
                                px.extend(row.iter().step_by(step).take(w));
                            }
                            Ok(Frame {
                                width: w,
                                height: h,
                                pixels: px,
                            })
                        })
                        .map_err(|e| e.to_string())?,
                );
            }
            Ok(out)
        }
    }
}

#[cfg(target_os = "macos")]
mod backend {
    use nokhwa::pixel_format::LumaFormat;
    use nokhwa::utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType};
    use nokhwa::Camera;

    use super::Frame;

    pub(super) fn present() -> bool {
        nokhwa::query(ApiBackend::Auto)
            .map(|c| !c.is_empty())
            .unwrap_or(false)
    }

    pub(super) fn devices() -> Vec<String> {
        nokhwa::query(ApiBackend::Auto)
            .map(|v| {
                v.iter()
                    .map(|c| format!("{} {}", c.index(), c.human_name()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) struct Source {
        cam: Camera,
        pub(super) name: String,
    }

    pub(super) fn open() -> Result<Source, String> {
        // the OS asks the user once; the answer arrives asynchronously
        nokhwa::nokhwa_initialize(|_| {});
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !nokhwa::nokhwa_check() {
            if std::time::Instant::now() > deadline {
                return Err("camera access was not granted".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let format =
            RequestedFormat::new::<LumaFormat>(RequestedFormatType::AbsoluteHighestResolution);
        let mut cam = Camera::new(CameraIndex::Index(0), format).map_err(|e| e.to_string())?;
        cam.open_stream().map_err(|e| e.to_string())?;
        let name = format!("{} {}", cam.info().human_name(), cam.resolution());
        Ok(Source { cam, name })
    }

    impl Source {
        pub(super) fn latest(&mut self) -> Result<Option<Frame>, String> {
            let buf = self.cam.frame().map_err(|e| e.to_string())?;
            let img = buf
                .decode_image::<LumaFormat>()
                .map_err(|e| e.to_string())?;
            Ok(Some(Frame {
                width: img.width() as usize,
                height: img.height() as usize,
                pixels: img.into_raw(),
            }))
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod backend {
    use super::Frame;

    pub(super) fn present() -> bool {
        false
    }

    pub(super) fn devices() -> Vec<String> {
        Vec::new()
    }

    pub(super) struct Source {
        pub(super) name: String,
    }

    pub(super) fn open() -> Result<Source, String> {
        Err("camera capture is not implemented on this OS".into())
    }

    impl Source {
        pub(super) fn latest(&mut self) -> Result<Option<Frame>, String> {
            Ok(None)
        }
    }
}

/// A camera exists (device enumeration only; nothing is opened for capture).
pub(super) fn present() -> bool {
    backend::present()
}

pub(super) fn devices() -> Vec<String> {
    backend::devices()
}

pub(super) fn run(stop: &AtomicBool, em: &Emitter) {
    let mut src = match backend::open() {
        Ok(s) => s,
        Err(e) => {
            em.status(format!("camera unavailable: {e}"));
            return;
        }
    };
    if !em.status(format!("camera {}: show the phone's QR to it", src.name)) {
        return;
    }
    let mut collector = ReplyCollector::new();
    while !stop.load(Ordering::Relaxed) {
        let frame = match src.latest() {
            Ok(Some(f)) => f,
            Ok(None) => {
                std::thread::sleep(Duration::from_millis(15));
                continue;
            }
            Err(e) => {
                em.status(format!("camera error: {e}"));
                return;
            }
        };
        let scan = collector.push_image(Gray {
            width: frame.width,
            height: frame.height,
            pixels: &frame.pixels,
        });
        match scan {
            Scan::Nothing => {}
            Scan::Progress(held, needed) => {
                if !em.progress(held, needed) {
                    return;
                }
            }
            Scan::Complete(doc) => {
                if em.reply(doc) {
                    return;
                }
            }
        }
    }
}
