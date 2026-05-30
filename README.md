# jetson-hal

[English](#english) | [中文](#中文)

---

# English

Hardware abstraction layer for Jetson-based robots.

Provides async interfaces for camera, audio, GPS, and motor control — all built on tokio for concurrent operation.

## Modules

| Module | Feature | Crate | Description |
|--------|---------|-------|-------------|
| Camera | `camera` | `v4l` | V4L2 video capture via mmap |
| Audio | `audio` | `alsa` | ALSA PCM capture & playback |
| GPS | `gps` | `tokio-serial`, `nmea` | NMEA parsing + NTRIP RTK correction |
| Motor | `motor` | `tokio-serial` | STM32 serial motor control |

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
jetson-hal = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

### Camera

```rust
use jetson_hal::{Camera, CameraConfig, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let camera = Camera::open(CameraConfig {
        width: 1280,
        height: 720,
        fps: 30,
        ..Default::default()
    }).await?;

    let frame = camera.frame().await?;
    println!("Captured {} bytes", frame.data.len());

    camera.close().await?;
    Ok(())
}
```

### Audio

```rust
use jetson_hal::{AudioCapture, AudioPlayback, AudioConfig, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let config = AudioConfig {
        sample_rate: 16000,
        channels: 1,
        chunk_size: 1600,
        ..Default::default()
    };

    // Record
    let mic = AudioCapture::open(config.clone())?;
    let samples = mic.read_chunk().await?;

    // Playback
    let speaker = AudioPlayback::open(config)?;
    speaker.write(&samples).await?;
    speaker.drain().await?;

    Ok(())
}
```

### GPS

```rust
use jetson_hal::{GpsReceiver, GpsConfig, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let (receiver, mut rx) = GpsReceiver::new(GpsConfig::default());
    receiver.start().await?;

    while let Ok(data) = rx.recv().await {
        println!("{:.6}, {:.6} sats={}", data.latitude, data.longitude, data.satellites);
    }
    Ok(())
}
```

### Motor

```rust
use jetson_hal::{MotorController, MotorConfig, Result};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let ctrl = MotorController::new(MotorConfig::default());
    ctrl.start_motors().await?;

    ctrl.set_speed(40, 0).await?;       // forward
    tokio::time::sleep(Duration::from_secs(2)).await;

    ctrl.set_speed(0, 30).await?;       // turn right
    tokio::time::sleep(Duration::from_secs(1)).await;

    ctrl.stop_motors().await?;
    Ok(())
}
```

## Feature Flags

All features are enabled by default. Use `default-features = false` to select only what you need:

```toml
# Only camera — no alsa/tokio-serial/nmea/glob pulled in
jetson-hal = { version = "0.1", default-features = false, features = ["camera"] }

# GPS + Motor only
jetson-hal = { version = "0.1", default-features = false, features = ["gps", "motor"] }

# Everything (default)
jetson-hal = "0.1"
```

| Feature | Dependencies | Description |
|---------|--------------|-------------|
| `camera` | `v4l` | V4L2 video capture |
| `audio` | `alsa` | ALSA PCM capture & playback |
| `gps` | `tokio-serial`, `nmea`, `glob` | GPS receiver with NTRIP RTK |
| `motor` | `tokio-serial` | STM32 motor control |

## Examples

```bash
cargo run --example camera_capture
cargo run --example audio_record
cargo run --example audio_playback
cargo run --example gps_tracker
cargo run --example motor_control
cargo run --example robot          # all subsystems combined
```

## Architecture

Each hardware subsystem follows the same pattern:

1. **Dedicated I/O thread** owns the hardware device (PCM, V4L2, serial port)
2. **Channel-based communication** — async methods send commands via `mpsc`, receive results via `oneshot`
3. **No `Arc<Mutex>`** — ownership lives entirely in the I/O thread

```
┌─────────────┐   mpsc/oneshot   ┌──────────────┐
│  async API  │ ──────────────── │  I/O thread  │ ── hardware
└─────────────┘                  └──────────────┘
```

## Platform

Designed for **Jetson Nano / Orin** running Linux with:

- ALSA audio devices
- V4L2 video devices (`/dev/video*`)
- Serial ports for GPS (`/dev/ttyUSB*`, `/dev/ttyACM*`) and motor (`/dev/ttyTHS*`)

The crate uses conditional compilation — it can be built on non-Linux hosts (e.g. for CI) by disabling all features:

```bash
cargo check --no-default-features
```

## License

MIT

---

# 中文

Jetson 机器人的硬件抽象层。

提供摄像头、音频、GPS 和电机控制的异步接口，基于 tokio 实现并发运行。

## 模块

| 模块 | Feature | 依赖 | 说明 |
|------|---------|------|------|
| 摄像头 | `camera` | `v4l` | V4L2 视频采集（mmap） |
| 音频 | `audio` | `alsa` | ALSA PCM 录音与播放 |
| GPS | `gps` | `tokio-serial`, `nmea` | NMEA 解析 + NTRIP RTK 差分修正 |
| 电机 | `motor` | `tokio-serial` | STM32 串口电机控制 |

## 快速开始

在 `Cargo.toml` 中添加：

```toml
[dependencies]
jetson-hal = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

### 摄像头

```rust
use jetson_hal::{Camera, CameraConfig, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let camera = Camera::open(CameraConfig {
        width: 1280,
        height: 720,
        fps: 30,
        ..Default::default()
    }).await?;

    let frame = camera.frame().await?;
    println!("采集到 {} 字节", frame.data.len());

    camera.close().await?;
    Ok(())
}
```

### 音频

```rust
use jetson_hal::{AudioCapture, AudioPlayback, AudioConfig, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let config = AudioConfig {
        sample_rate: 16000,
        channels: 1,
        chunk_size: 1600,
        ..Default::default()
    };

    // 录音
    let mic = AudioCapture::open(config.clone())?;
    let samples = mic.read_chunk().await?;

    // 播放
    let speaker = AudioPlayback::open(config)?;
    speaker.write(&samples).await?;
    speaker.drain().await?;

    Ok(())
}
```

### GPS

```rust
use jetson_hal::{GpsReceiver, GpsConfig, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let (receiver, mut rx) = GpsReceiver::new(GpsConfig::default());
    receiver.start().await?;

    while let Ok(data) = rx.recv().await {
        println!("{:.6}, {:.6} 卫星={}", data.latitude, data.longitude, data.satellites);
    }
    Ok(())
}
```

### 电机

```rust
use jetson_hal::{MotorController, MotorConfig, Result};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let ctrl = MotorController::new(MotorConfig::default());
    ctrl.start_motors().await?;

    ctrl.set_speed(40, 0).await?;       // 前进
    tokio::time::sleep(Duration::from_secs(2)).await;

    ctrl.set_speed(0, 30).await?;       // 右转
    tokio::time::sleep(Duration::from_secs(1)).await;

    ctrl.stop_motors().await?;
    Ok(())
}
```

## Feature 标志

默认启用全部 feature。使用 `default-features = false` 按需选择：

```toml
# 仅摄像头 — 不引入 alsa/tokio-serial/nmea/glob
jetson-hal = { version = "0.1", default-features = false, features = ["camera"] }

# 仅 GPS + 电机
jetson-hal = { version = "0.1", default-features = false, features = ["gps", "motor"] }

# 全部启用（默认）
jetson-hal = "0.1"
```

| Feature | 依赖 | 说明 |
|---------|------|------|
| `camera` | `v4l` | V4L2 视频采集 |
| `audio` | `alsa` | ALSA PCM 录音与播放 |
| `gps` | `tokio-serial`, `nmea`, `glob` | GPS 接收器 + NTRIP RTK |
| `motor` | `tokio-serial` | STM32 电机控制 |

## 示例

```bash
cargo run --example camera_capture    # 摄像头采集
cargo run --example audio_record      # 录音
cargo run --example audio_playback    # 播放正弦波
cargo run --example gps_tracker       # GPS 定位
cargo run --example motor_control     # 电机控制
cargo run --example robot             # 全模块联调
```

## 架构

每个硬件子系统遵循相同的模式：

1. **专用 I/O 线程** 拥有硬件设备（PCM、V4L2、串口）
2. **Channel 通信** — 异步方法通过 `mpsc` 发送命令，通过 `oneshot` 接收结果
3. **无 `Arc<Mutex>`** — 所有权完全在 I/O 线程中

```
┌─────────────┐   mpsc/oneshot   ┌──────────────┐
│  异步 API   │ ──────────────── │  I/O 线程    │ ── 硬件
└─────────────┘                  └──────────────┘
```

## 平台

面向运行 Linux 的 **Jetson Nano / Orin**，需要：

- ALSA 音频设备
- V4L2 视频设备（`/dev/video*`）
- GPS 串口（`/dev/ttyUSB*`、`/dev/ttyACM*`）和电机串口（`/dev/ttyTHS*`）

使用条件编译 — 可在非 Linux 主机（如 CI 环境）上通过禁用全部 feature 进行构建：

```bash
cargo check --no-default-features
```

## 许可证

MIT
