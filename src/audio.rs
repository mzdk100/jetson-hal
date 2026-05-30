use {
    crate::{
        config::AudioConfig,
        error::{JetsonError, Result},
    },
    alsa::{
        Direction, ValueOr,
        pcm::{Access, Format, HwParams, PCM},
    },
    tokio::sync::{mpsc, oneshot},
    tracing::info,
};

/// Commands for the capture I/O thread.
enum CaptureCmd {
    ReadChunk {
        chunk_size: usize,
        respond: oneshot::Sender<Result<Vec<i16>>>,
    },
}

/// Commands for the playback I/O thread.
enum PlaybackCmd {
    Write {
        data: Vec<i16>,
        respond: oneshot::Sender<Result<()>>,
    },
    Drain {
        respond: oneshot::Sender<Result<()>>,
    },
}

/// ALSA audio capture (microphone).
///
/// Opens an ALSA PCM device in capture mode and reads interleaved S16_LE samples.
/// Supports shared ALSA devices (e.g. "plug:dsnoop_shared") for concurrent access.
///
/// Internally uses a dedicated I/O thread that owns the PCM device.
/// All blocking ALSA operations run on that thread; the async API communicates
/// with it via channels.
pub struct AudioCapture {
    cmd_tx: mpsc::Sender<CaptureCmd>,
    config: AudioConfig,
}

impl AudioCapture {
    /// Open the capture device with the given configuration.
    pub fn open(config: AudioConfig) -> Result<Self> {
        let pcm = PCM::new(&config.capture_device, Direction::Capture, false).map_err(|e| {
            JetsonError::Audio(format!(
                "open capture device '{}': {}",
                config.capture_device, e
            ))
        })?;

        Self::configure_pcm(&pcm, &config)?;

        let actual = Self::read_actual_params(&pcm)?;
        info!(
            "audio capture opened: device='{}' rate={}Hz channels={} period={}",
            config.capture_device, actual.0, actual.1, actual.2
        );

        let (cmd_tx, cmd_rx) = mpsc::channel(1);
        std::thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || capture_thread(pcm, cmd_rx))
            .map_err(|e| JetsonError::Audio(format!("spawn capture thread: {}", e)))?;

        Ok(Self { cmd_tx, config })
    }

    fn configure_pcm(pcm: &PCM, config: &AudioConfig) -> Result<()> {
        let hwp =
            HwParams::any(pcm).map_err(|e| JetsonError::Audio(format!("hw_params any: {}", e)))?;
        hwp.set_access(Access::RWInterleaved)
            .map_err(|e| JetsonError::Audio(format!("set access: {}", e)))?;
        hwp.set_format(Format::s16())
            .map_err(|e| JetsonError::Audio(format!("set format: {}", e)))?;
        hwp.set_channels(config.channels)
            .map_err(|e| JetsonError::Audio(format!("set channels: {}", e)))?;
        hwp.set_rate_near(config.sample_rate, ValueOr::Nearest)
            .map_err(|e| JetsonError::Audio(format!("set rate: {}", e)))?;
        pcm.hw_params(&hwp)
            .map_err(|e| JetsonError::Audio(format!("apply hw_params: {}", e)))?;
        Ok(())
    }

    fn read_actual_params(pcm: &PCM) -> Result<(u32, u32, u32)> {
        let hwp = pcm
            .hw_params_current()
            .map_err(|e| JetsonError::Audio(format!("get current hw_params: {}", e)))?;
        let rate = hwp
            .get_rate()
            .map_err(|e| JetsonError::Audio(format!("get rate: {}", e)))?;
        let channels = hwp
            .get_channels()
            .map_err(|e| JetsonError::Audio(format!("get channels: {}", e)))?;
        let period = hwp
            .get_period_size()
            .map_err(|e| JetsonError::Audio(format!("get period size: {}", e)))?;
        Ok((rate, channels, period as u32))
    }

    /// Read one chunk of audio samples.
    ///
    /// Returns a Vec of i16 samples (interleaved if stereo).
    /// The blocking ALSA read runs on a dedicated I/O thread.
    pub async fn read_chunk(&self) -> Result<Vec<i16>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(CaptureCmd::ReadChunk {
                chunk_size: self.config.chunk_size,
                respond: tx,
            })
            .await
            .map_err(|_| JetsonError::Audio("capture device closed".into()))?;
        rx.await
            .map_err(|_| JetsonError::Audio("capture device closed".into()))?
    }

    /// Read one chunk of audio samples as raw bytes (S16_LE).
    pub async fn read_chunk_bytes(&self) -> Result<Vec<u8>> {
        let samples = self.read_chunk().await?;
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for s in &samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        Ok(bytes)
    }

    /// Get the configured sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    /// Get the configured number of channels.
    pub fn channels(&self) -> u32 {
        self.config.channels
    }

    /// Get the chunk size in samples.
    pub fn chunk_size(&self) -> usize {
        self.config.chunk_size
    }
}

/// Dedicated capture I/O thread. Owns the PCM device and processes commands
/// until the channel is closed.
fn capture_thread(pcm: PCM, mut cmd_rx: mpsc::Receiver<CaptureCmd>) {
    while let Some(cmd) = cmd_rx.blocking_recv() {
        match cmd {
            CaptureCmd::ReadChunk {
                chunk_size,
                respond,
            } => {
                let result = (|| -> Result<Vec<i16>> {
                    let io = pcm
                        .io_i16()
                        .map_err(|e| JetsonError::Audio(format!("io_i16: {}", e)))?;
                    let mut buf = vec![0i16; chunk_size];
                    let read = io
                        .readi(&mut buf)
                        .map_err(|e| JetsonError::Audio(format!("readi: {}", e)))?;
                    buf.truncate(read);
                    Ok(buf)
                })();
                let _ = respond.send(result);
            }
        }
    }
    // pcm is dropped here when the channel closes
}

/// ALSA audio playback (speaker).
///
/// Opens an ALSA PCM device in playback mode and writes interleaved S16_LE samples.
///
/// Internally uses a dedicated I/O thread that owns the PCM device.
pub struct AudioPlayback {
    cmd_tx: mpsc::Sender<PlaybackCmd>,
    _config: AudioConfig,
}

impl AudioPlayback {
    /// Open the playback device with the given configuration.
    pub fn open(config: AudioConfig) -> Result<Self> {
        let pcm = PCM::new(&config.playback_device, Direction::Playback, false).map_err(|e| {
            JetsonError::Audio(format!(
                "open playback device '{}': {}",
                config.playback_device, e
            ))
        })?;

        Self::configure_pcm(&pcm, &config)?;

        info!(
            "audio playback opened: device='{}' rate={}Hz channels={}",
            config.playback_device, config.sample_rate, config.channels
        );

        let (cmd_tx, cmd_rx) = mpsc::channel(1);
        std::thread::Builder::new()
            .name("audio-playback".into())
            .spawn(move || playback_thread(pcm, cmd_rx))
            .map_err(|e| JetsonError::Audio(format!("spawn playback thread: {}", e)))?;

        Ok(Self {
            cmd_tx,
            _config: config,
        })
    }

    fn configure_pcm(pcm: &PCM, config: &AudioConfig) -> Result<()> {
        let hwp =
            HwParams::any(pcm).map_err(|e| JetsonError::Audio(format!("hw_params any: {}", e)))?;
        hwp.set_access(Access::RWInterleaved)
            .map_err(|e| JetsonError::Audio(format!("set access: {}", e)))?;
        hwp.set_format(Format::s16())
            .map_err(|e| JetsonError::Audio(format!("set format: {}", e)))?;
        hwp.set_channels(config.channels)
            .map_err(|e| JetsonError::Audio(format!("set channels: {}", e)))?;
        hwp.set_rate_near(config.sample_rate, ValueOr::Nearest)
            .map_err(|e| JetsonError::Audio(format!("set rate: {}", e)))?;

        // Set buffer and period size to match Python's aplay configuration
        let _ = hwp.set_buffer_size_near(config.playback_buffer_size as i64);
        let _ = hwp.set_period_size_near(config.playback_period_size as i64, ValueOr::Nearest);

        pcm.hw_params(&hwp)
            .map_err(|e| JetsonError::Audio(format!("apply hw_params: {}", e)))?;
        Ok(())
    }

    /// Write audio samples to the playback device.
    ///
    /// `samples` should be interleaved i16 PCM data.
    /// The blocking ALSA write runs on a dedicated I/O thread.
    pub async fn write(&self, samples: &[i16]) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(PlaybackCmd::Write {
                data: samples.to_vec(),
                respond: tx,
            })
            .await
            .map_err(|_| JetsonError::Audio("playback device closed".into()))?;
        rx.await
            .map_err(|_| JetsonError::Audio("playback device closed".into()))?
    }

    /// Write raw S16_LE bytes to the playback device.
    pub async fn write_bytes(&self, data: &[u8]) -> Result<()> {
        // Convert bytes to i16 samples
        let samples: Vec<i16> = data
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        self.write(&samples).await
    }

    /// Drain the playback buffer (block until all queued samples are played).
    pub async fn drain(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(PlaybackCmd::Drain { respond: tx })
            .await
            .map_err(|_| JetsonError::Audio("playback device closed".into()))?;
        rx.await
            .map_err(|_| JetsonError::Audio("playback device closed".into()))?
    }
}

/// Dedicated playback I/O thread. Owns the PCM device and processes commands
/// until the channel is closed.
fn playback_thread(pcm: PCM, mut cmd_rx: mpsc::Receiver<PlaybackCmd>) {
    while let Some(cmd) = cmd_rx.blocking_recv() {
        match cmd {
            PlaybackCmd::Write { data, respond } => {
                let result = (|| -> Result<()> {
                    let io = pcm
                        .io_i16()
                        .map_err(|e| JetsonError::Audio(format!("io_i16: {}", e)))?;
                    io.writei(&data)
                        .map_err(|e| JetsonError::Audio(format!("writei: {}", e)))?;
                    Ok(())
                })();
                let _ = respond.send(result);
            }
            PlaybackCmd::Drain { respond } => {
                let result = pcm
                    .drain()
                    .map_err(|e| JetsonError::Audio(format!("drain: {}", e)));
                let _ = respond.send(result);
            }
        }
    }
    // pcm is dropped here when the channel closes
}
