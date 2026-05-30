//! GPS tracker example.
//!
//! Opens a GPS receiver, subscribes to position updates, and prints them.
//! Supports optional NTRIP RTK correction.
//!
//! Run: cargo run --example gps_tracker

use jetson_hal::{GpsConfig, GpsReceiver, Result};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let config = GpsConfig {
        // port: Some("/dev/ttyUSB0".into()),  // Uncomment to specify port
        baudrate: 460800,
        // NTRIP RTK correction (optional)
        // ntrip_host: Some("rtk.example.com".into()),
        // ntrip_port: 8002,
        // ntrip_mountpoint: Some("RTCM3".into()),
        // ntrip_user: Some("user".into()),
        // ntrip_pass: Some("pass".into()),
        ..Default::default()
    };

    println!("Starting GPS receiver...");
    let (receiver, mut rx) = GpsReceiver::new(config);
    receiver.start().await?;
    println!("GPS started. Waiting for fixes...\n");

    // Print position updates
    let mut count = 0u32;
    while let Ok(data) = rx.recv().await {
        count += 1;
        println!(
            "[#{:04}] {:.6}, {:.6} (WGS84: {:.6}, {:.6}) | \
             alt={:.1}m spd={:.1}m/s hdop={:.1} sats={} fix={}",
            count,
            data.latitude,
            data.longitude,
            data.latitude_wgs84,
            data.longitude_wgs84,
            data.altitude,
            data.speed,
            data.hdop,
            data.satellites,
            match data.fix_type {
                0 => "none",
                1 => "GPS",
                2 => "DGPS",
                4 => "RTK-fixed",
                5 => "RTK-float",
                _ => "unknown",
            }
        );

        if count >= 100 {
            println!("\nReceived 100 fixes, exiting.");
            break;
        }
    }

    Ok(())
}
