use {
    crate::{
        config::GpsConfig,
        error::{JetsonError, Result},
    },
    std::time::Duration,
    tokio::{
        io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
        net::TcpStream,
        sync::broadcast,
        time::{interval, sleep},
    },
    tokio_serial::{SerialPortBuilderExt, SerialStream},
    tracing::{debug, info, warn},
};

/// Parsed GPS data from NMEA sentences.
#[derive(Debug, Clone, Default)]
pub struct GpsData {
    /// Latitude in decimal degrees (GCJ-02).
    pub latitude: f64,
    /// Longitude in decimal degrees (GCJ-02).
    pub longitude: f64,
    /// Latitude in WGS-84.
    pub latitude_wgs84: f64,
    /// Longitude in WGS-84.
    pub longitude_wgs84: f64,
    /// Speed in meters per second.
    pub speed: f64,
    /// Course over ground in degrees.
    pub course: f64,
    /// Altitude in meters.
    pub altitude: f64,
    /// Number of satellites in use.
    pub satellites: u32,
    /// Horizontal dilution of precision.
    pub hdop: f64,
    /// Fix type (0=none, 1=GPS, 2=DGPS, 4=RTK fixed, 5=RTK float).
    pub fix_type: u8,
    /// Whether the fix is valid.
    pub valid: bool,
}

/// GPS receiver with NMEA parsing and optional NTRIP differential correction.
///
/// Reads NMEA sentences from a serial port, parses them using the `nmea` crate,
/// and broadcasts `GpsData` to all subscribers. Optionally injects RTCM3 correction
/// data from an NTRIP caster for RTK precision.
pub struct GpsReceiver {
    config: GpsConfig,
    data_tx: broadcast::Sender<GpsData>,
}

/// NTRIP client for RTK differential correction.
#[allow(dead_code)]
struct NtripClient {
    stream: Option<TcpStream>,
    config: GpsConfig,
    last_gga: Option<String>,
}

impl GpsReceiver {
    /// Create a new GPS receiver.
    ///
    /// Returns the receiver and a broadcast channel for GPS data updates.
    pub fn new(config: GpsConfig) -> (Self, broadcast::Receiver<GpsData>) {
        let (tx, rx) = broadcast::channel(16);
        (
            Self {
                config,
                data_tx: tx,
            },
            rx,
        )
    }

    /// Start the GPS receiver tasks.
    ///
    /// Spawns two tokio tasks:
    /// - Serial reader: reads NMEA sentences and parses them
    /// - NTRIP client: connects to caster and injects RTCM3 correction data
    pub async fn start(&self) -> Result<()> {
        let port_path = self.detect_port().await?;
        info!("GPS using port: {}", port_path);

        let baudrate = self.config.baudrate;
        let serial = tokio_serial::new(&port_path, baudrate)
            .timeout(Duration::from_millis(100))
            .open_native_async()
            .map_err(|e| JetsonError::Gps(format!("open serial '{}': {}", port_path, e)))?;

        let config = self.config.clone();
        let data_tx = self.data_tx.clone();

        // Spawn serial reader task
        let read_config = config.clone();
        let read_tx = data_tx.clone();
        tokio::spawn(async move {
            Self::serial_read_loop(serial, read_config, read_tx).await;
        });

        // Spawn NTRIP task if configured
        if config.ntrip_host.is_some() {
            let ntrip_config = config.clone();
            // NTRIP needs its own serial connection to inject RTCM data
            let ntrip_serial = tokio_serial::new(&port_path, baudrate)
                .timeout(Duration::from_millis(100))
                .open_native_async()
                .map_err(|e| JetsonError::Gps(format!("open serial for NTRIP: {}", e)));

            match ntrip_serial {
                Ok(ntrip_ser) => {
                    tokio::spawn(async move {
                        Self::ntrip_loop(ntrip_ser, ntrip_config).await;
                    });
                }
                Err(e) => {
                    warn!("NTRIP disabled: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Auto-detect GPS serial port.
    async fn detect_port(&self) -> Result<String> {
        if let Some(ref port) = self.config.port {
            return Ok(port.clone());
        }

        let patterns = [
            "/dev/ttyACM*",
            "/dev/ttyUSB*",
            "/dev/ttyAMA*",
            "/dev/serial*",
        ];

        for pattern in &patterns {
            if let Ok(paths) = glob::glob(pattern) {
                for path in paths.flatten() {
                    let path_str = path.to_string_lossy().to_string();
                    info!("GPS auto-detect: trying {}", path_str);
                    // Try to open at each baud rate
                    for &baud in &[460800u32, 115200, 57600, 38400, 9600] {
                        if let Ok(ser) = tokio_serial::new(&path_str, baud)
                            .timeout(Duration::from_millis(100))
                            .open_native_async()
                        {
                            // Try to read a line and check for NMEA
                            let mut reader = BufReader::new(ser);
                            let mut line = String::new();
                            if tokio::time::timeout(
                                Duration::from_secs(1),
                                reader.read_line(&mut line),
                            )
                            .await
                            .is_ok()
                                && line.starts_with('$')
                            {
                                info!("GPS found: {} at {} baud", path_str, baud);
                                return Ok(path_str);
                            }
                        }
                    }
                }
            }
        }

        Err(JetsonError::DeviceNotFound("no GPS device found".into()))
    }

    /// Main serial read loop: read NMEA lines and parse them.
    async fn serial_read_loop(
        serial: SerialStream,
        _config: GpsConfig,
        data_tx: broadcast::Sender<GpsData>,
    ) {
        let mut reader = BufReader::new(serial);
        let mut parser = nmea::Nmea::default();
        let mut current = GpsData::default();
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    warn!("GPS serial EOF");
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if !trimmed.starts_with('$') {
                        continue;
                    }

                    match parser.parse(trimmed) {
                        Ok(_) => {
                            Self::update_data(&parser, &mut current);
                            if current.valid {
                                // Apply WGS84 -> GCJ-02 conversion
                                let (gcj_lat, gcj_lon) =
                                    wgs84_to_gcj02(current.latitude_wgs84, current.longitude_wgs84);
                                current.latitude = gcj_lat;
                                current.longitude = gcj_lon;

                                let _ = data_tx.send(current.clone());
                            }
                        }
                        Err(e) => {
                            debug!("NMEA parse error: {} (line: {})", e, trimmed);
                        }
                    }
                }
                Err(e) => {
                    warn!("GPS serial read error: {}", e);
                    sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Update GpsData from parsed NMEA data.
    fn update_data(parser: &nmea::Nmea, data: &mut GpsData) {
        if let Some(lat) = parser.latitude() {
            data.latitude_wgs84 = lat;
        }
        if let Some(lon) = parser.longitude() {
            data.longitude_wgs84 = lon;
        }
        if let Some(alt) = parser.altitude() {
            data.altitude = alt as f64;
        }
        if let Some(fix) = parser.fix_type() {
            data.fix_type = fix as u8;
            data.valid = fix.is_valid();
        }
        if let Some(sats) = parser.fix_satellites() {
            data.satellites = sats;
        }
        if let Some(hdop) = parser.hdop() {
            data.hdop = hdop as f64;
        }
    }

    /// NTRIP correction loop: connect to caster, inject RTCM3 data into GPS serial.
    async fn ntrip_loop(mut serial: SerialStream, config: GpsConfig) {
        let host = config.ntrip_host.as_ref().unwrap();
        let port = config.ntrip_port;
        let mountpoint = config.ntrip_mountpoint.as_deref().unwrap_or("auto");
        let user = config.ntrip_user.as_deref().unwrap_or("");
        let pass = config.ntrip_pass.as_deref().unwrap_or("");
        let gga_interval_secs = config.gga_interval;

        loop {
            match Self::ntrip_connect(host, port, mountpoint, user, pass).await {
                Ok(mut stream) => {
                    info!("NTRIP connected to {}:{}", host, port);
                    let mut buf = [0u8; 4096];
                    let mut gga_timer = interval(Duration::from_secs(gga_interval_secs as u64));

                    loop {
                        tokio::select! {
                            result = stream.read(&mut buf) => {
                                match result {
                                    Ok(0) => {
                                        warn!("NTRIP connection closed");
                                        break;
                                    }
                                    Ok(n) => {
                                        // Forward RTCM3 data to GPS serial port
                                        if let Err(e) = serial.write_all(&buf[..n]).await {
                                            warn!("NTRIP write to serial failed: {}", e);
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        warn!("NTRIP read error: {}", e);
                                        break;
                                    }
                                }
                            }
                            _ = gga_timer.tick() => {
                                // Re-send last GGA to keep correction stream alive
                                // In production, this would use the latest GGA from the GPS
                                let gga = b"$GPGGA,000000.000,0000.0000,N,00000.0000,E,1,08,1.0,0.0,M,0.0,M,,*47\r\n";
                                let _ = stream.write_all(gga).await;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("NTRIP connect failed: {}", e);
                }
            }

            sleep(Duration::from_secs(3)).await;
        }
    }

    /// Connect to an NTRIP caster with HTTP Basic auth.
    async fn ntrip_connect(
        host: &str,
        port: u16,
        mountpoint: &str,
        user: &str,
        pass: &str,
    ) -> Result<TcpStream> {
        let addr = format!("{}:{}", host, port);
        let mut stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| JetsonError::Gps(format!("NTRIP TCP connect: {}", e)))?;

        // Send HTTP Basic auth request
        let auth = base64_encode(&format!("{}:{}", user, pass));
        let request = format!(
            "GET /{} HTTP/1.1\r\n\
             Host: {}:{}\r\n\
             Authorization: Basic {}\r\n\
             Ntrip-Version: Ntrip/2.0\r\n\
             User-Agent: jetson-hal/0.1\r\n\
             \r\n",
            mountpoint, host, port, auth
        );

        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| JetsonError::Gps(format!("NTRIP send request: {}", e)))?;

        // Read HTTP response headers byte-by-byte to find \r\n\r\n
        // This avoids creating a BufReader that borrows the stream
        let mut header_buf = Vec::new();
        let mut found_end = false;
        loop {
            let mut byte = [0u8; 1];
            let n = stream
                .read(&mut byte)
                .await
                .map_err(|e| JetsonError::Gps(format!("NTRIP read response: {}", e)))?;
            if n == 0 {
                break;
            }
            header_buf.push(byte[0]);
            // Check for \r\n\r\n (end of headers)
            if header_buf.len() >= 4 {
                let tail = &header_buf[header_buf.len() - 4..];
                if tail == b"\r\n\r\n" {
                    found_end = true;
                    break;
                }
            }
        }

        if !found_end {
            return Err(JetsonError::Gps("NTRIP: incomplete HTTP response".into()));
        }

        let headers = String::from_utf8_lossy(&header_buf);
        if !headers.contains("200") {
            return Err(JetsonError::Gps(format!(
                "NTRIP server returned: {}",
                headers.lines().next().unwrap_or("unknown").trim()
            )));
        }

        Ok(stream)
    }
}

/// Simple base64 encoder for NTRIP auth (avoids pulling in a full base64 crate).
fn base64_encode(input: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

/// WGS-84 to GCJ-02 coordinate conversion.
///
/// Implements the standard China national coordinate offset algorithm.
/// Input: (latitude, longitude) in WGS-84 decimal degrees.
/// Output: (latitude, longitude) in GCJ-02.
pub fn wgs84_to_gcj02(lat: f64, lon: f64) -> (f64, f64) {
    // Check if point is in China; if not, return as-is
    if !is_in_china(lat, lon) {
        return (lat, lon);
    }

    let a = 6378245.0; // Semi-major axis
    let ee = 0.00669342162296594; // Eccentricity squared

    let dlat = transform_lat(lon - 105.0, lat - 35.0);
    let dlon = transform_lon(lon - 105.0, lat - 35.0);

    let rad_lat = lat * std::f64::consts::PI / 180.0;
    let magic = rad_lat.sin();
    let magic_sq = 1.0 - ee * magic * magic;
    let sqrt_magic = magic_sq.sqrt();

    let gcj_lat =
        lat + (dlat * 180.0) / ((a * (1.0 - ee)) / (magic_sq * sqrt_magic) * std::f64::consts::PI);
    let gcj_lon = lon + (dlon * 180.0) / (a / sqrt_magic * rad_lat.cos() * std::f64::consts::PI);

    (gcj_lat, gcj_lon)
}

fn is_in_china(lat: f64, lon: f64) -> bool {
    (72.004..=137.8347).contains(&lon) && (0.8293..=55.8271).contains(&lat)
}

fn transform_lat(x: f64, y: f64) -> f64 {
    let mut ret = -100.0 + 2.0 * x + 3.0 * y + 0.2 * y * y + 0.1 * x * y + 0.2 * x.abs().sqrt();
    ret += (2.0 * (20.0 * x * std::f64::consts::PI).sin() * 2.0 * (x * std::f64::consts::PI).sin())
        / 3.0;
    ret += (2.0
        * (20.0 * x / 3.0 * std::f64::consts::PI).sin()
        * 2.0
        * (x / 3.0 * std::f64::consts::PI).sin())
        * 2.0
        / 3.0;
    ret += (2.0
        * (20.0 * x / 12.0 * std::f64::consts::PI).sin()
        * 2.0
        * (x / 12.0 * std::f64::consts::PI).sin())
        * 2.0
        / 3.0;
    ret
}

fn transform_lon(x: f64, y: f64) -> f64 {
    let mut ret = 300.0 + x + 2.0 * y + 0.1 * x * x + 0.1 * x * y + 0.1 * x.abs().sqrt();
    ret += (2.0 * (20.0 * x * std::f64::consts::PI).sin() * 2.0 * (x * std::f64::consts::PI).sin())
        * 2.0
        / 3.0;
    ret += (2.0
        * (20.0 * x / 3.0 * std::f64::consts::PI).sin()
        * 2.0
        * (x / 3.0 * std::f64::consts::PI).sin())
        * 2.0
        / 3.0;
    ret += (2.0
        * (20.0 * x / 12.0 * std::f64::consts::PI).sin()
        * 2.0
        * (x / 12.0 * std::f64::consts::PI).sin())
        * 2.0
        / 3.0;
    ret
}
