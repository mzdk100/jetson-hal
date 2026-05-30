//! Audio recording example.
//!
//! Opens the ALSA capture device, records a few chunks, and prints sample statistics.
//!
//! Run: cargo run --example audio_record

use jetson_hal::{AudioCapture, AudioConfig, Result};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let config = AudioConfig {
        sample_rate: 16000,
        channels: 1,
        chunk_size: 1600, // 100ms at 16kHz
        ..Default::default()
    };

    println!("Opening capture device '{}'...", config.capture_device);
    let mic = AudioCapture::open(config)?;
    println!(
        "Capture ready: {}Hz, {} channels, chunk={}",
        mic.sample_rate(),
        mic.channels(),
        mic.chunk_size()
    );

    // Record 5 chunks
    println!("\nRecording 5 chunks...");
    for i in 0..5 {
        let samples = mic.read_chunk().await?;
        let max = samples.iter().map(|s| s.abs()).max().unwrap_or(0);
        let rms: f64 = (samples.iter().map(|s| (*s as f64).powi(2)).sum::<f64>()
            / samples.len() as f64)
            .sqrt();
        println!(
            "  Chunk {}: {} samples, peak={}, RMS={:.0}",
            i,
            samples.len(),
            max,
            rms
        );
    }

    // Also demonstrate raw byte read
    println!("\nReading raw bytes...");
    let bytes = mic.read_chunk_bytes().await?;
    println!("  Got {} bytes ({} samples)", bytes.len(), bytes.len() / 2);

    Ok(())
}
