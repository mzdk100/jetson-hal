use thiserror::Error;

/// Unified error type for all jetson-hal operations.
#[derive(Debug, Error)]
pub enum JetsonError {
    #[cfg(feature = "camera")]
    #[error("camera error: {0}")]
    Camera(String),

    #[cfg(feature = "audio")]
    #[error("audio error: {0}")]
    Audio(String),

    #[cfg(feature = "motor")]
    #[error("serial error: {0}")]
    Serial(String),

    #[cfg(feature = "gps")]
    #[error("GPS error: {0}")]
    Gps(String),

    #[cfg(feature = "motor")]
    #[error("motor error: {0}")]
    Motor(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("device not found: {0}")]
    DeviceNotFound(String),

    #[error("timeout: {0}")]
    Timeout(String),
}

pub type Result<T> = std::result::Result<T, JetsonError>;
