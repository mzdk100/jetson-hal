//! # jetson-hal
//!
//! Hardware abstraction layer for Jetson-based robots.
//!
//! Provides async interfaces for:
//! - **Camera**: V4L2 video capture via the `v4l` crate (feature `camera`)
//! - **Audio**: ALSA PCM capture and playback via the `alsa` crate (feature `audio`)
//! - **GPS**: NMEA 0183 parsing with optional NTRIP RTK correction (feature `gps`)
//! - **Motor**: STM32 serial motor control protocol (feature `motor`)
//!
//! All subsystems use tokio for async I/O and can run concurrently.
//!
//! ## Feature Flags
//!
//! All features are enabled by default. Disable with `default-features = false`
//! and enable only what you need:
//!
//! ```toml
//! [dependencies]
//! jetson-hal = { version = "0.1", default-features = false, features = ["camera"] }
//! ```
//!
//! | Feature  | Dependencies         | Description |
//! |----------|----------------------|-------------|
//! | `camera` | `v4l`                | V4L2 video capture |
//! | `audio`  | `alsa`               | ALSA PCM capture & playback |
//! | `gps`    | `tokio-serial`, `nmea`, `glob` | GPS receiver with NTRIP RTK |
//! | `motor`  | `tokio-serial`       | STM32 motor control |
//!
//! ## Example
//!
//! ```rust,no_run
//! # #[cfg(feature = "camera")]
//! use jetson_hal::{Camera, CameraConfig};
//!
//! # #[cfg(feature = "camera")]
//! # async fn example() -> jetson_hal::Result<()> {
//! let camera = Camera::open(CameraConfig::default()).await?;
//! let frame = camera.frame().await?;
//! println!("captured {} bytes", frame.data.len());
//! # Ok(())
//! # }
//! ```

pub mod config;
pub mod error;

#[cfg(feature = "audio")]
pub mod audio;
#[cfg(feature = "camera")]
pub mod camera;
#[cfg(feature = "gps")]
pub mod gps;
#[cfg(feature = "motor")]
pub mod motor;

#[cfg(feature = "audio")]
pub use audio::{AudioCapture, AudioPlayback};
#[cfg(feature = "camera")]
pub use camera::{Camera, Frame};
#[cfg(any(
    feature = "audio",
    feature = "gps",
    feature = "camera",
    feature = "motor"
))]
pub use config::*;
pub use error::{JetsonError, Result};
#[cfg(feature = "gps")]
pub use gps::{GpsData, GpsReceiver};
#[cfg(feature = "motor")]
pub use motor::{MotorCommand, MotorController, RobotMode, Stm32Message};
