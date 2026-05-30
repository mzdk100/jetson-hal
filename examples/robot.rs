//! Full robot example.
//!
//! Demonstrates using all subsystems together:
//! - Camera captures frames periodically
//! - Audio records ambient sound level
//! - GPS tracks position
//! - Motor controller drives the robot
//!
//! Each subsystem runs as a concurrent tokio task.
//!
//! Run: cargo run --example robot

use jetson_hal::{
    AudioCapture, AudioConfig, Camera, CameraConfig, GpsConfig, GpsReceiver, MotorConfig,
    MotorController, Result,
};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    // ── Camera ──────────────────────────────────────────────
    let camera = Camera::open(CameraConfig {
        width: 640,
        height: 480,
        fps: 15,
        ..Default::default()
    })
    .await?;

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            match camera.frame().await {
                Ok(frame) => println!("[camera] frame: {} bytes", frame.data.len()),
                Err(e) => println!("[camera] error: {}", e),
            }
        }
    });

    // ── Audio ───────────────────────────────────────────────
    let mic = AudioCapture::open(AudioConfig {
        sample_rate: 16000,
        channels: 1,
        chunk_size: 1600,
        ..Default::default()
    })?;

    tokio::spawn(async move {
        loop {
            match mic.read_chunk().await {
                Ok(samples) => {
                    let rms: f64 = (samples.iter().map(|s| (*s as f64).powi(2)).sum::<f64>()
                        / samples.len() as f64)
                        .sqrt();
                    println!("[audio] RMS={:.0}", rms);
                }
                Err(e) => println!("[audio] error: {}", e),
            }
        }
    });

    // ── GPS ─────────────────────────────────────────────────
    let (gps, mut gps_rx) = GpsReceiver::new(GpsConfig::default());
    gps.start().await?;

    tokio::spawn(async move {
        while let Ok(data) = gps_rx.recv().await {
            if data.valid {
                println!(
                    "[gps] {:.6}, {:.6} sats={}",
                    data.latitude, data.longitude, data.satellites
                );
            }
        }
    });

    // ── Motor ───────────────────────────────────────────────
    let ctrl = MotorController::new(MotorConfig::default());
    ctrl.start_motors().await?;

    tokio::spawn(async move {
        // Simple patrol: forward 5s, turn 2s, repeat
        loop {
            println!("[motor] forward");
            let _ = ctrl.set_speed(30, 0).await;
            tokio::time::sleep(Duration::from_secs(5)).await;

            println!("[motor] turn right");
            let _ = ctrl.set_speed(0, 25).await;
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    // ── Main loop ───────────────────────────────────────────
    println!("Robot running. Press Ctrl+C to stop.\n");

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    println!("\nShutting down...");

    Ok(())
}
