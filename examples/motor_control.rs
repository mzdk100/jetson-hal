//! Motor control example.
//!
//! Connects to the STM32 motor controller and runs a simple movement sequence:
//! forward → turn right → stop.
//!
//! Run: cargo run --example motor_control

use jetson_hal::{MotorConfig, MotorController, Result};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let config = MotorConfig {
        port: "/dev/ttyTHS1".into(),
        baudrate: 115200,
        normal_speed: 40,
        turn_speed: 30,
        ..Default::default()
    };

    println!("Connecting to STM32 on {}...", config.port);
    let mut ctrl = MotorController::new(config);
    println!("Motor controller ready.\n");

    // Start motors
    println!("Starting motors...");
    ctrl.start_motors().await?;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Move forward
    println!("Moving forward (speed=40)...");
    ctrl.set_speed(40, 0).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Turn right
    println!("Turning right (speed=30)...");
    ctrl.set_speed(0, 30).await?;
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Stop
    println!("Stopping...");
    ctrl.stop_motors().await?;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Read any pending messages
    println!("\nChecking for STM32 messages...");
    while let Some(msg) = ctrl.recv_message().await {
        println!("  {:?}", msg);
    }

    println!("Done.");
    Ok(())
}
