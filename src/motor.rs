//! Motor controller implementation for STM32 communication.

use {
    crate::{
        config::MotorConfig,
        error::{JetsonError, Result},
    },
    std::{
        io::{Read, Write},
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicI32, Ordering},
        },
        time::Duration,
    },
    tokio::sync::mpsc,
    tracing::{debug, error, info, warn},
};

/// Messages received from the STM32 controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stm32Message {
    /// Current robot mode.
    Mode(RobotMode),
    /// Heartbeat acknowledgment.
    Pong,
    /// Unknown message.
    Unknown(String),
}

/// Robot operating mode reported by STM32.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobotMode {
    /// Balance mode (self-balancing).
    Balance,
    /// Walking/guide mode.
    Walking,
}

impl RobotMode {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "BALANCE_MODE" => Some(RobotMode::Balance),
            "WALKING_MODE" => Some(RobotMode::Walking),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            RobotMode::Balance => "平衡模式",
            RobotMode::Walking => "行进模式",
        }
    }
}

/// Motor command to send to STM32.
#[derive(Debug, Clone)]
pub enum MotorCommand {
    /// Set linear and angular speed (-100 to 100).
    Speed { linear: i32, angular: i32 },
    /// Send heartbeat.
    Heartbeat,
    /// Start motor control.
    Start,
    /// Stop motor control.
    Stop,
    /// Toggle balance/walking mode.
    ModeToggle,
    /// Emergency stop.
    EmergencyStop,
}

/// STM32 motor controller interface.
///
/// Communicates with the STM32 microcontroller over serial using a text-based protocol.
/// Uses a dedicated writer thread running a 50Hz control loop (matching Python behavior).
pub struct MotorController {
    cmd_tx: mpsc::Sender<MotorCommand>,
    msg_rx: mpsc::Receiver<Stm32Message>,
    // Current robot mode (shared with async API)
    current_mode: Arc<std::sync::RwLock<RobotMode>>,
}

// Note: MotorController does NOT implement Clone because mpsc::Receiver cannot be cloned
// If you need multiple references, use Arc<MotorController>

impl MotorController {
    /// Create a new motor controller.
    pub fn new(config: MotorConfig) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (msg_tx, msg_rx) = mpsc::channel(64);

        // Spawn the serial communication thread (blocking, like Python)
        std::thread::spawn(move || {
            Self::serial_task(config, cmd_rx, msg_tx);
        });

        MotorController {
            cmd_tx,
            msg_rx,
            current_mode: Arc::new(std::sync::RwLock::new(RobotMode::Balance)),
        }
    }

    /// Send a command to the STM32.
    pub async fn send(&self, cmd: MotorCommand) -> Result<()> {
        self.cmd_tx
            .send(cmd)
            .await
            .map_err(|_e| JetsonError::Motor("send command: channel closed".to_string()))
    }

    /// Set motor speed.
    pub async fn set_speed(&self, linear: i32, angular: i32) -> Result<()> {
        self.send(MotorCommand::Speed { linear, angular }).await
    }

    /// Send heartbeat.
    pub async fn heartbeat(&self) -> Result<()> {
        self.send(MotorCommand::Heartbeat).await
    }

    /// Start motor control.
    pub async fn start_motors(&self) -> Result<()> {
        self.send(MotorCommand::Start).await
    }

    /// Stop motor control.
    pub async fn stop_motors(&self) -> Result<()> {
        self.send(MotorCommand::Stop).await
    }

    /// Toggle balance/walking mode.
    pub async fn toggle_mode(&self) -> Result<()> {
        self.send(MotorCommand::ModeToggle).await
    }

    /// Emergency stop.
    pub async fn emergency_stop(&self) -> Result<()> {
        self.send(MotorCommand::EmergencyStop).await
    }

    /// Get current robot mode.
    pub fn current_mode(&self) -> RobotMode {
        *self.current_mode.read().unwrap()
    }

    /// Try to receive a message from STM32 (non-blocking).
    pub async fn recv_message(&mut self) -> Option<Stm32Message> {
        self.msg_rx.recv().await
    }

    /// Parse STM32 message.
    fn parse_stm32_message(line: &str) -> Option<Stm32Message> {
        if let Some(mode_str) = line.strip_prefix("MODE:") {
            if let Some(mode) = RobotMode::from_str(mode_str) {
                return Some(Stm32Message::Mode(mode));
            }
        } else if line == "PONG" {
            return Some(Stm32Message::Pong);
        }

        debug!("📨 STM32: {}", line);
        Some(Stm32Message::Unknown(line.to_string()))
    }

    /// Write a command to the serial port (generic version that works with any SerialPort)
    fn write_command(port: &mut dyn serialport::SerialPort, cmd: &str) -> Result<()> {
        let full_cmd = format!("{}\n", cmd);
        port.write_all(full_cmd.as_bytes())
            .map_err(|e| JetsonError::Motor(format!("Write failed: {}", e)))?;
        port.flush()
            .map_err(|e| JetsonError::Motor(format!("Flush failed: {}", e)))?;
        Ok(())
    }

    /// Main serial task: owns the serial port, runs writer thread + command loop.
    ///
    /// This is a blocking function that runs in a dedicated thread, matching Python's
    /// `threading.Thread(target=_control_loop)` pattern.
    fn serial_task(
        config: MotorConfig,
        mut cmd_rx: mpsc::Receiver<MotorCommand>,
        msg_tx: mpsc::Sender<Stm32Message>,
    ) {
        let control_interval = Duration::from_millis(config.control_interval_ms);
        let heartbeat_interval = Duration::from_millis(config.heartbeat_interval_ms);

        loop {
            // Open serial port (blocking, matching Python's serial.Serial exactly)
            let port = match serialport::new(&config.port, config.baudrate)
                .data_bits(serialport::DataBits::Eight)
                .parity(serialport::Parity::None)
                .stop_bits(serialport::StopBits::One)
                .timeout(Duration::from_millis(100))
                .open()
            {
                Ok(p) => {
                    info!("✓ STM32 serial opened: {}", config.port);
                    p
                }
                Err(e) => {
                    error!("❌ STM32 serial open failed: {}", e);
                    std::thread::sleep(Duration::from_secs(3));
                    continue;
                }
            };

            // Clear buffers (like Python's reset_input_buffer/reset_output_buffer)
            let _ = port.clear(serialport::ClearBuffer::All);

            let current_linear = Arc::new(AtomicI32::new(0));
            let current_angular = Arc::new(AtomicI32::new(0));
            let is_started = Arc::new(AtomicBool::new(false));
            let running = Arc::new(AtomicBool::new(true));

            // Clone ports for different threads
            let mut port_writer = port
                .try_clone()
                .expect("Failed to clone serial port for writer");
            let mut port_reader = port
                .try_clone()
                .expect("Failed to clone serial port for reader");
            let mut port_commands = port; // Use original port for commands

            // Shared state for threads
            let cl = Arc::clone(&current_linear);
            let ca = Arc::clone(&current_angular);
            let started = Arc::clone(&is_started);
            let r_writer = Arc::clone(&running);
            let r_reader = Arc::clone(&running);

            // Writer thread: 50Hz control loop (matches Python _control_loop)
            let writer_handle = std::thread::spawn(move || {
                let mut last_heartbeat = std::time::Instant::now();

                while r_writer.load(Ordering::Relaxed) {
                    if started.load(Ordering::Relaxed) {
                        let linear = cl.load(Ordering::Relaxed);
                        let angular = ca.load(Ordering::Relaxed);

                        // Always send current speed (keep connection alive)
                        let cmd = format!("V{},{}\n", linear, angular);
                        if let Err(e) = port_writer.write_all(cmd.as_bytes()) {
                            debug!("Write failed: {}", e);
                            break;
                        }
                        if let Err(e) = port_writer.flush() {
                            debug!("Flush failed: {}", e);
                            break;
                        }

                        // If no movement, also send heartbeat (double guarantee)
                        if linear == 0
                            && angular == 0
                            && last_heartbeat.elapsed() >= heartbeat_interval
                        {
                            let _ = port_writer.write_all(b"HEARTBEAT\n");
                            let _ = port_writer.flush();
                            last_heartbeat = std::time::Instant::now();
                        }
                    } else {
                        // Not started: only send heartbeat
                        if last_heartbeat.elapsed() >= heartbeat_interval {
                            let _ = port_writer.write_all(b"HEARTBEAT\n");
                            let _ = port_writer.flush();
                            last_heartbeat = std::time::Instant::now();
                        }
                    }

                    std::thread::sleep(control_interval);
                }
            });

            // Reader thread: receive messages from STM32
            let msg_tx_r = msg_tx.clone();
            let reader_handle = std::thread::spawn(move || {
                let mut line_buffer = Vec::new();

                while r_reader.load(Ordering::Relaxed) {
                    let mut buf = [0u8; 256];
                    match port_reader.read(&mut buf) {
                        Ok(n) if n > 0 => {
                            // Append to line buffer
                            line_buffer.extend_from_slice(&buf[..n]);

                            // Convert to string and split by lines
                            if let Ok(text) = String::from_utf8(line_buffer.clone()) {
                                let lines: Vec<&str> = text.split('\n').collect();

                                // Keep the last incomplete line in buffer
                                if !text.ends_with('\n') && lines.len() > 1 {
                                    if let Some(last) = lines.last() {
                                        line_buffer = last.as_bytes().to_vec();
                                    } else {
                                        line_buffer.clear();
                                    }
                                } else {
                                    line_buffer.clear();
                                }

                                // Process complete lines
                                for line in lines.iter().take(lines.len() - 1) {
                                    let line = line.trim();
                                    if !line.is_empty()
                                        && let Some(msg) = Self::parse_stm32_message(line)
                                    {
                                        let _ = msg_tx_r.blocking_send(msg);
                                    }
                                }
                            } else {
                                // Invalid UTF-8, clear buffer
                                line_buffer.clear();
                            }
                        }
                        Ok(_) => {
                            // No data, just sleep
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => {
                            debug!("Read error: {}", e);
                            std::thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
            });

            // Command processing loop: receives from user, updates shared state
            let mut last_mode_toggle_time = std::time::Instant::now();
            let mode_toggle_cooldown = Duration::from_millis(100);

            loop {
                match cmd_rx.blocking_recv() {
                    Some(MotorCommand::Speed { linear, angular }) => {
                        let linear = linear.clamp(-100, 100);
                        let angular = angular.clamp(-100, 100);

                        current_linear.store(linear, Ordering::Relaxed);
                        current_angular.store(angular, Ordering::Relaxed);

                        if linear == 0 && angular == 0 {
                            debug!("🛑 停止");
                        } else {
                            debug!("🚀 V{},{}", linear, angular);
                        }
                    }
                    Some(MotorCommand::Start) => {
                        is_started.store(true, Ordering::Relaxed);
                        if let Err(e) = Self::write_command(&mut *port_commands, "BTN_START") {
                            error!("Failed to send START: {}", e);
                        }
                        debug!("底盘已启动");
                    }
                    Some(MotorCommand::Stop) => {
                        is_started.store(false, Ordering::Relaxed);
                        current_linear.store(0, Ordering::Relaxed);
                        current_angular.store(0, Ordering::Relaxed);
                        if let Err(e) = Self::write_command(&mut *port_commands, "BTN_SELECT") {
                            error!("Failed to send STOP: {}", e);
                        }
                        debug!("底盘已关闭");
                    }
                    Some(MotorCommand::EmergencyStop) => {
                        is_started.store(false, Ordering::Relaxed);
                        current_linear.store(0, Ordering::Relaxed);
                        current_angular.store(0, Ordering::Relaxed);
                        if let Err(e) = Self::write_command(&mut *port_commands, "BTN_XBOX") {
                            error!("Failed to send EMERGENCY_STOP: {}", e);
                        }
                        warn!("⚠️ 紧急停止!");
                    }
                    Some(MotorCommand::ModeToggle) => {
                        // Add cooldown to prevent rapid toggling
                        if last_mode_toggle_time.elapsed() >= mode_toggle_cooldown {
                            if let Err(e) = Self::write_command(&mut *port_commands, "BTN_A") {
                                error!("Failed to send MODE_TOGGLE: {}", e);
                            }
                            debug!("发送模式切换命令");
                            last_mode_toggle_time = std::time::Instant::now();
                        }
                    }
                    Some(MotorCommand::Heartbeat) => {
                        if let Err(e) = Self::write_command(&mut *port_commands, "HEARTBEAT") {
                            debug!("Failed to send heartbeat: {}", e);
                        }
                    }
                    None => {
                        // Channel closed, stop
                        info!("Command channel closed, stopping serial task");
                        running.store(false, Ordering::Relaxed);
                        break;
                    }
                }
            }

            // Clean shutdown
            running.store(false, Ordering::Relaxed);
            let _ = writer_handle.join();
            let _ = reader_handle.join();
            info!("STM32 serial task terminated, reconnecting in 1 second...");
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

/// Helper function to update current mode from messages
/// This should be called from the async task that receives messages
pub async fn update_mode_from_messages<F>(
    controller: &mut MotorController,
    on_mode_change: Option<F>,
) where
    F: Fn(RobotMode) + Send + 'static,
{
    while let Some(msg) = controller.recv_message().await {
        if let Stm32Message::Mode(mode) = msg {
            let current = controller.current_mode();
            if mode != current {
                info!("🔄 模式切换: {}", mode.name());
                if let Some(ref callback) = on_mode_change {
                    callback(mode);
                }
            }
        }
    }
}
