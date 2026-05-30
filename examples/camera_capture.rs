//! Camera capture example.
//!
//! Opens a V4L2 camera, captures a few frames, and optionally saves a JPEG snapshot.
//!
//! Run: cargo run --example camera_capture

use jetson_hal::{Camera, CameraConfig, Result};
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let config = CameraConfig {
        width: 1280,
        height: 720,
        fps: 30,
        ..Default::default()
    };

    println!("Opening camera...");
    let camera = Camera::open(config).await?;
    println!(
        "Camera ready: {}x{} @ {}fps",
        camera.width(),
        camera.height(),
        camera.fps()
    );

    // Capture 10 frames and measure FPS
    println!("\nCapturing 10 frames...");
    let start = Instant::now();
    for i in 0..10 {
        let frame = camera.frame().await?;
        println!(
            "  Frame {}: {} bytes, {}x{}",
            i,
            frame.data.len(),
            frame.width,
            frame.height
        );
    }
    let elapsed = start.elapsed();
    println!(
        "\nAverage: {:.1} ms/frame ({:.1} fps)",
        elapsed.as_millis() as f64 / 10.0,
        10.0 / elapsed.as_secs_f64()
    );

    // Save last frame as JPEG
    let path = std::path::Path::new("snapshot.jpg");
    camera.snapshot(path).await?;
    println!("Snapshot saved to {}", path.display());

    camera.close().await?;
    Ok(())
}
