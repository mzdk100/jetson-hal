/// Camera capture configuration.
#[cfg(feature = "camera")]
#[derive(Debug, Clone)]
pub struct CameraConfig {
    /// V4L2 device index (maps to /dev/videoN).
    pub device_index: u32,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Target frames per second.
    pub fps: u32,
    /// Snapshot directory path (optional).
    pub snapshot_dir: Option<String>,
    /// Snapshot interval in seconds (0 = disabled).
    pub snapshot_interval: f64,
}

#[cfg(feature = "camera")]
impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            device_index: 0,
            width: 640,
            height: 480,
            fps: 15,
            snapshot_dir: None,
            snapshot_interval: 0.0,
        }
    }
}

/// ALSA audio configuration.
#[cfg(feature = "audio")]
#[derive(Debug, Clone)]
pub struct AudioConfig {
    /// ALSA capture device name (e.g. "plug:dsnoop_shared", "default").
    pub capture_device: String,
    /// ALSA playback device name (e.g. "plug:dmix_shared", "default").
    pub playback_device: String,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of channels (1 = mono, 2 = stereo).
    pub channels: u32,
    /// Chunk size in samples per read/write.
    pub chunk_size: usize,
    /// Playback buffer size in samples.
    pub playback_buffer_size: usize,
    /// Playback period size in samples.
    pub playback_period_size: usize,
}

#[cfg(feature = "audio")]
impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            capture_device: "plug:dsnoop_shared".into(),
            playback_device: "plug:dmix_shared".into(),
            sample_rate: 48000,
            channels: 1,
            chunk_size: 960,
            playback_buffer_size: 16384,
            playback_period_size: 2048,
        }
    }
}

/// GPS receiver configuration.
#[cfg(feature = "gps")]
#[derive(Debug, Clone)]
pub struct GpsConfig {
    /// Serial port path (e.g. "/dev/ttyUSB2"). None = auto-detect.
    pub port: Option<String>,
    /// Baud rate.
    pub baudrate: u32,
    /// NTRIP caster host (for RTK differential correction).
    pub ntrip_host: Option<String>,
    /// NTRIP caster port.
    pub ntrip_port: u16,
    /// NTRIP mountpoint.
    pub ntrip_mountpoint: Option<String>,
    /// NTRIP username.
    pub ntrip_user: Option<String>,
    /// NTRIP password.
    pub ntrip_pass: Option<String>,
    /// GGA sentence re-send interval to NTRIP caster (seconds).
    pub gga_interval: u32,
}

#[cfg(feature = "gps")]
impl Default for GpsConfig {
    fn default() -> Self {
        Self {
            port: None,
            baudrate: 460800,
            ntrip_host: None,
            ntrip_port: 8002,
            ntrip_mountpoint: None,
            ntrip_user: None,
            ntrip_pass: None,
            gga_interval: 5,
        }
    }
}

/// STM32 motor controller configuration.
#[cfg(feature = "motor")]
#[derive(Debug, Clone)]
pub struct MotorConfig {
    /// Serial port path (e.g. "/dev/ttyTHS1").
    pub port: String,
    /// Baud rate.
    pub baudrate: u32,
    /// Control loop interval in milliseconds (default 20ms = 50Hz).
    pub control_interval_ms: u64,
    /// Heartbeat interval in milliseconds when idle.
    pub heartbeat_interval_ms: u64,
    /// Normal speed (0-100).
    pub normal_speed: i32,
    /// High speed (0-100).
    pub high_speed: i32,
    /// Turn speed (0-100).
    pub turn_speed: i32,
}

#[cfg(feature = "motor")]
impl Default for MotorConfig {
    fn default() -> Self {
        Self {
            port: "/dev/ttyTHS1".into(),
            baudrate: 115200,
            control_interval_ms: 20,
            heartbeat_interval_ms: 150,
            normal_speed: 40,
            high_speed: 60,
            turn_speed: 20,
        }
    }
}
