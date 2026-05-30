//! Audio playback example.
//!
//! Opens the ALSA playback device and plays a 440Hz sine wave for 1 second.
//!
//! Run: cargo run --example audio_playback

use jetson_hal::{AudioConfig, AudioPlayback, Result};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let config = AudioConfig {
        sample_rate: 48000,
        channels: 1,
        chunk_size: 4800, // 100ms at 48kHz
        ..Default::default()
    };

    let sample_rate = config.sample_rate;
    println!("Opening playback device '{}'...", config.playback_device);
    let speaker = AudioPlayback::open(config)?;
    println!("Playback ready: {}Hz", sample_rate);

    // Generate a 440Hz sine wave, 1 second
    let sample_rate = sample_rate as f64;
    let freq = 440.0;
    let duration_secs = 1.0;
    let total_samples = (sample_rate * duration_secs) as usize;

    let samples: Vec<i16> = (0..total_samples)
        .map(|i| {
            let t = i as f64 / sample_rate;
            let value = (2.0 * std::f64::consts::PI * freq * t).sin();
            (value * 16000.0) as i16
        })
        .collect();

    println!("Playing 440Hz sine wave for {}s...", duration_secs);
    speaker.write(&samples).await?;
    speaker.drain().await?;
    println!("Done.");

    Ok(())
}
