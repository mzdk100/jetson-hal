//! Simple motor control example without message handling.

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
        control_interval_ms: 20, // 50Hz control loop
        heartbeat_interval_ms: 100,
        ..Default::default()
    };

    println!("Connecting to STM32 on {}...", config.port);
    let ctrl = MotorController::new(config);
    println!("Motor controller ready.\n");

    // Wait for connection to stabilize
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Start motors
    println!("Starting motors...");
    ctrl.start_motors().await?;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Move forward (negative linear speed for forward)
    println!("Moving forward (linear=-40)...");
    ctrl.set_speed(-40, 0).await?;
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Stop
    println!("Stopping...");
    //    ctrl.set_speed(0, 0).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Turn right
    println!("Turning right (angular=30)...");
    ctrl.set_speed(0, 30).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Final stop
    println!("Stopping motors...");
    ctrl.stop_motors().await?;
    tokio::time::sleep(Duration::from_millis(500)).await;

    println!("Done.");
    Ok(())
}
