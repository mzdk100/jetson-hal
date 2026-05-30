use {
    crate::{
        config::CameraConfig,
        error::{JetsonError, Result},
    },
    std::path::Path,
    tokio::{
        fs::{rename, write},
        sync::{mpsc, oneshot},
    },
    tracing::{debug, info},
    v4l::{io::traits::Stream, video::Capture},
};

/// Commands for the camera I/O thread.
enum CameraCmd {
    Frame {
        respond: oneshot::Sender<Result<Frame>>,
    },
    Close {
        respond: oneshot::Sender<Result<()>>,
    },
}

struct CameraInner {
    _device: v4l::Device,
    stream: Option<v4l::io::mmap::Stream<'static>>,
    width: u32,
    height: u32,
}

// SAFETY: CameraInner is only accessed from a single dedicated I/O thread.
// The compiler can't auto-derive Send because Stream borrows Device with a
// fake 'static lifetime. The thread guarantees exclusive access.
unsafe impl Send for CameraInner {}

/// V4L2 camera capture device.
///
/// Wraps the `v4l` crate to provide async frame capture via mmap.
/// Frames are captured in the native format (MJPEG/YUYV) and returned as raw bytes.
///
/// Internally uses a dedicated I/O thread that owns the V4L2 device.
pub struct Camera {
    cmd_tx: mpsc::Sender<CameraCmd>,
    config: CameraConfig,
}

/// A single captured video frame.
pub struct Frame {
    /// Raw pixel data (format depends on camera, typically MJPEG or RGB24).
    pub data: Vec<u8>,
    /// Frame width.
    pub width: u32,
    /// Frame height.
    pub height: u32,
    /// Pixel format FourCC.
    pub format: u32,
}

impl Camera {
    /// Open a camera device.
    ///
    /// If `config.device_index` fails, tries indices 0, 1, 2 as fallback.
    pub async fn open(config: CameraConfig) -> Result<Self> {
        let indices_to_try = {
            let mut v = vec![config.device_index];
            for i in 0..3u32 {
                if i != config.device_index {
                    v.push(i);
                }
            }
            v
        };

        for idx in &indices_to_try {
            match Self::try_open(*idx, &config) {
                Ok(inner) => {
                    info!("camera opened: /dev/video{}", idx);

                    let (cmd_tx, cmd_rx) = mpsc::channel(1);
                    std::thread::Builder::new()
                        .name("camera-capture".into())
                        .spawn(move || camera_thread(inner, cmd_rx))
                        .map_err(|e| JetsonError::Camera(format!("spawn camera thread: {}", e)))?;

                    return Ok(Self { cmd_tx, config });
                }
                Err(e) => {
                    debug!("camera /dev/video{} failed: {}", idx, e);
                }
            }
        }

        Err(JetsonError::DeviceNotFound(format!(
            "no camera found (tried indices: {:?})",
            indices_to_try
        )))
    }

    fn try_open(index: u32, config: &CameraConfig) -> Result<CameraInner> {
        let device = v4l::Device::new(index as usize)
            .map_err(|e| JetsonError::Camera(format!("open /dev/video{}: {}", index, e)))?;

        // Query and set format
        let mut fmt = device
            .format()
            .map_err(|e| JetsonError::Camera(format!("get format: {}", e)))?;
        fmt.width = config.width;
        fmt.height = config.height;
        // Prefer MJPEG for smaller data transfer; fall back to YUYV
        fmt.fourcc = v4l::FourCC::new(b"MJPG");
        device
            .set_format(&fmt)
            .map_err(|e| JetsonError::Camera(format!("set format: {}", e)))?;

        // Re-read actual format (driver may adjust)
        let actual_fmt = device
            .format()
            .map_err(|e| JetsonError::Camera(format!("get actual format: {}", e)))?;
        info!(
            "camera format: {}x{} fourcc={}",
            actual_fmt.width, actual_fmt.height, actual_fmt.fourcc
        );

        // Create mmap stream with 4 buffers
        let mut stream = v4l::io::mmap::Stream::new(&device, v4l::buffer::Type::VideoCapture)
            .map_err(|e| JetsonError::Camera(format!("create mmap stream: {}", e)))?;

        // Start streaming
        stream
            .start()
            .map_err(|e| JetsonError::Camera(format!("start stream: {}", e)))?;

        Ok(CameraInner {
            _device: device,
            stream: Some(stream),
            width: actual_fmt.width,
            height: actual_fmt.height,
        })
    }

    /// Capture a single frame.
    ///
    /// The blocking V4L2 read runs on a dedicated I/O thread.
    pub async fn frame(&self) -> Result<Frame> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(CameraCmd::Frame { respond: tx })
            .await
            .map_err(|_| JetsonError::Camera("camera device closed".into()))?;
        rx.await
            .map_err(|_| JetsonError::Camera("camera device closed".into()))?
    }

    /// Write the latest frame as a JPEG snapshot to the given path.
    ///
    /// Uses atomic write (temp file + rename) to avoid partial reads.
    pub async fn snapshot(&self, path: &Path) -> Result<()> {
        let frame = self.frame().await?;
        let tmp_path = path.with_extension("tmp");

        write(&tmp_path, &frame.data)
            .await
            .map_err(|e| JetsonError::Camera(format!("write snapshot: {}", e)))?;
        rename(&tmp_path, path)
            .await
            .map_err(|e| JetsonError::Camera(format!("rename snapshot: {}", e)))?;

        debug!("snapshot saved: {}", path.display());
        Ok(())
    }

    /// Get the configured frame width.
    pub fn width(&self) -> u32 {
        self.config.width
    }

    /// Get the configured frame height.
    pub fn height(&self) -> u32 {
        self.config.height
    }

    /// Get the configured FPS.
    pub fn fps(&self) -> u32 {
        self.config.fps
    }

    /// Stop the camera stream and release the device.
    pub async fn close(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        // Ignore send error — device may already be closed
        let _ = self.cmd_tx.send(CameraCmd::Close { respond: tx }).await;
        rx.await
            .map_err(|_| JetsonError::Camera("camera device closed".into()))?
    }
}

/// Dedicated camera I/O thread. Owns the V4L2 device and processes commands
/// until the channel is closed.
fn camera_thread(mut inner: CameraInner, mut cmd_rx: mpsc::Receiver<CameraCmd>) {
    while let Some(cmd) = cmd_rx.blocking_recv() {
        match cmd {
            CameraCmd::Frame { respond } => {
                let result = (|| -> Result<Frame> {
                    let stream = inner
                        .stream
                        .as_mut()
                        .ok_or_else(|| JetsonError::Camera("stream not started".into()))?;

                    use v4l::io::traits::CaptureStream;
                    let (buf, _meta) = stream
                        .next()
                        .map_err(|e| JetsonError::Camera(format!("capture frame: {}", e)))?;

                    Ok(Frame {
                        data: buf.to_vec(),
                        width: inner.width,
                        height: inner.height,
                        format: 0, // MJPEG
                    })
                })();
                let _ = respond.send(result);
            }
            CameraCmd::Close { respond } => {
                let result = (|| -> Result<()> {
                    if let Some(mut stream) = inner.stream.take() {
                        stream
                            .stop()
                            .map_err(|e| JetsonError::Camera(format!("stop stream: {}", e)))?;
                        info!("camera closed");
                    }
                    Ok(())
                })();
                let _ = respond.send(result);
            }
        }
    }
    // Cleanup on channel close: stop the stream if still running
    if let Some(mut stream) = inner.stream.take() {
        let _ = stream.stop();
    }
}
