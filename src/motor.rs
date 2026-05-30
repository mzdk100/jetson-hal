use {
    crate::{
        config::MotorConfig,
        error::{JetsonError, Result},
    },
    std::time::Duration,
    tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader, split},
        select, spawn,
        sync::mpsc,
        time::{Interval, interval, sleep},
    },
    tokio_serial::SerialPortBuilderExt,
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

impl MotorCommand {
    fn to_bytes(&self) -> Vec<u8> {
        match self {
            MotorCommand::Speed { linear, angular } => {
                let l = (*linear).clamp(-100, 100);
                let a = (*angular).clamp(-100, 100);
                format!("V{},{}\n", l, a).into_bytes()
            }
            MotorCommand::Heartbeat => b"HEARTBEAT\n".to_vec(),
            MotorCommand::Start => b"BTN_START\n".to_vec(),
            MotorCommand::Stop => b"BTN_SELECT\n".to_vec(),
            MotorCommand::ModeToggle => b"BTN_A\n".to_vec(),
            MotorCommand::EmergencyStop => b"BTN_XBOX\n".to_vec(),
        }
    }
}

/// STM32 motor controller interface.
///
/// Communicates with the STM32 microcontroller over serial using a text-based protocol.
/// Runs a 50Hz control loop to send speed commands and a receive loop to parse responses.
pub struct MotorController {
    config: MotorConfig,
    cmd_tx: mpsc::Sender<MotorCommand>,
    msg_rx: mpsc::Receiver<Stm32Message>,
}

impl MotorController {
    /// Create a new motor controller.
    pub fn new(config: MotorConfig) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (msg_tx, msg_rx) = mpsc::channel(64);

        let ctrl = MotorController {
            config,
            cmd_tx,
            msg_rx,
        };

        // Spawn the serial communication task
        let config_clone = ctrl.config.clone();
        spawn(async move {
            Self::serial_task(config_clone, cmd_rx, msg_tx).await;
        });

        ctrl
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

    /// Try to receive a message from STM32 (non-blocking).
    pub async fn recv_message(&mut self) -> Option<Stm32Message> {
        self.msg_rx.recv().await
    }

    /// Main serial task: handles both sending and receiving.
    async fn serial_task(
        config: MotorConfig,
        mut cmd_rx: mpsc::Receiver<MotorCommand>,
        msg_tx: mpsc::Sender<Stm32Message>,
    ) {
        loop {
            // Open serial port
            let serial = match tokio_serial::new(&config.port, config.baudrate)
                .timeout(Duration::from_millis(500))
                .open_native_async()
            {
                Ok(s) => {
                    info!("STM32 serial opened: {}", config.port);
                    s
                }
                Err(e) => {
                    error!("STM32 serial open failed: {}", e);
                    sleep(Duration::from_secs(3)).await;
                    continue;
                }
            };

            let (reader, mut writer) = split(serial);
            let mut buf_reader = BufReader::new(reader);

            let mut heartbeat_interval =
                interval(Duration::from_millis(config.heartbeat_interval_ms));
            let mut control_timer = interval(Duration::from_millis(config.control_interval_ms));
            let mut current_speed = (0i32, 0i32); // (linear, angular)

            // Run until error
            let result = Self::run_loop(
                &mut buf_reader,
                &mut writer,
                &mut cmd_rx,
                &msg_tx,
                &mut control_timer,
                &mut heartbeat_interval,
                &mut current_speed,
                &config,
            )
            .await;

            match result {
                Ok(()) => info!("STM32 serial task ended"),
                Err(e) => warn!("STM32 serial error: {}", e),
            }

            sleep(Duration::from_secs(1)).await;
        }
    }

    /// Main control loop combining send and receive.
    #[allow(clippy::too_many_arguments)]
    async fn run_loop(
        reader: &mut (impl AsyncBufReadExt + Unpin),
        writer: &mut (impl AsyncWriteExt + Unpin),
        cmd_rx: &mut mpsc::Receiver<MotorCommand>,
        msg_tx: &mpsc::Sender<Stm32Message>,
        control_timer: &mut Interval,
        heartbeat_timer: &mut Interval,
        current_speed: &mut (i32, i32),
        _config: &MotorConfig,
    ) -> Result<()> {
        let mut line = String::new();
        #[allow(unused_assignments)]
        let mut last_cmd = MotorCommand::Heartbeat;
        let mut cmd_active = false; // Whether a speed command is active

        loop {
            select! {
                // Read from STM32
                result = reader.read_line(&mut line) => {
                    match result {
                        Ok(0) => {
                            return Err(JetsonError::Motor("serial EOF".into()));
                        }
                        Ok(_) => {
                            let trimmed = line.trim().to_string();
                            if !trimmed.is_empty() {
                                let msg = Self::parse_stm32_message(&trimmed);
                                let _ = msg_tx.send(msg).await;
                            }
                            line.clear();
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                            line.clear();
                            continue;
                        }
                        Err(e) => {
                            return Err(JetsonError::Motor(format!("serial read: {}", e)));
                        }
                    }
                }

                // Receive commands from user
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(MotorCommand::Speed { linear, angular }) => {
                            *current_speed = (linear, angular);
                            last_cmd = MotorCommand::Speed { linear, angular };
                            cmd_active = true;
                            let data = last_cmd.to_bytes();
                            writer.write_all(&data).await
                                .map_err(|e| JetsonError::Motor(format!("serial write: {}", e)))?;
                            debug!("STM32 TX: V{},{}", linear, angular);
                        }
                        Some(cmd) => {
                            let data = cmd.to_bytes();
                            writer.write_all(&data).await
                                .map_err(|e| JetsonError::Motor(format!("serial write: {}", e)))?;
                            debug!("STM32 TX: {:?}", cmd);
                            if matches!(cmd, MotorCommand::EmergencyStop | MotorCommand::Stop) {
                                cmd_active = false;
                                *current_speed = (0, 0);
                            }
                        }
                        None => {
                            return Err(JetsonError::Motor("command channel closed".into()));
                        }
                    }
                }

                // Periodic control: re-send current speed or heartbeat
                _ = control_timer.tick() => {
                    if cmd_active {
                        let data = MotorCommand::Speed {
                            linear: current_speed.0,
                            angular: current_speed.1,
                        }.to_bytes();
                        writer.write_all(&data).await
                            .map_err(|e| JetsonError::Motor(format!("serial write: {}", e)))?;
                    }
                }

                // Heartbeat when idle
                _ = heartbeat_timer.tick() => {
                    if !cmd_active {
                        let data = MotorCommand::Heartbeat.to_bytes();
                        writer.write_all(&data).await
                            .map_err(|e| JetsonError::Motor(format!("serial write: {}", e)))?;
                    }
                }
            }
        }
    }

    /// Parse a message from STM32.
    fn parse_stm32_message(line: &str) -> Stm32Message {
        if line.contains("BALANCE_MODE") {
            Stm32Message::Mode(RobotMode::Balance)
        } else if line.contains("WALKING_MODE") {
            Stm32Message::Mode(RobotMode::Walking)
        } else if line == "PONG" {
            Stm32Message::Pong
        } else {
            Stm32Message::Unknown(line.to_string())
        }
    }
}
