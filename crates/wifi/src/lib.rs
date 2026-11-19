//! Wi-Fi station support for the ESP32-C6-N16 board.
//!
//! The crate wraps `esp-radio`'s Wi-Fi controller with:
//!
//! - compile-time credentials read from `WIFI_SSID` / `WIFI_PASSWORD`,
//! - scan helpers,
//! - a reconnecting station loop,
//! - an optional `embassy-net` DHCP stack and monitor (`net` feature),
//! - optional Wi-Fi/BLE coexistence (`coex` feature).
//!
//! It does not start `esp-rtos` or initialize the heap; the main application
//! owns those.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

extern crate alloc;

use alloc::vec::Vec;
#[cfg(feature = "net")]
use core::fmt::Write as _;
use defmt::{info, warn};
#[cfg(feature = "net")]
use embassy_net::{
    Runner, Stack, StackResources,
    dns::{DnsQueryType, DnsSocket},
    tcp::TcpSocket,
};
use embassy_time::{Duration, Timer};
#[cfg(feature = "net")]
use embedded_io_async::Write as _;
use esp_hal::peripherals::WIFI;
#[cfg(feature = "net")]
use esp_hal::rng::Rng;
#[cfg(feature = "net")]
use esp_radio::wifi::Interface;
use esp_radio::wifi::{
    AuthenticationMethodConfig, Config, ControllerConfig, Ssid, WifiController, WifiError,
    ap::AccessPointInfo,
    scan::{ScanConfig, ScanTypeConfig},
    sta::StationConfig,
};
#[cfg(feature = "net")]
use static_cell::StaticCell;

/// Wi-Fi credentials compiled into the firmware.
#[derive(Clone, Copy)]
pub struct Credentials {
    ssid: &'static str,
    password: &'static str,
}

impl Credentials {
    /// Create credentials from static strings.
    #[must_use]
    pub const fn new(ssid: &'static str, password: &'static str) -> Self {
        Self { ssid, password }
    }

    /// Read credentials from the build environment.
    ///
    /// Set `WIFI_SSID` and `WIFI_PASSWORD` while compiling. `scripts/run.sh`
    /// loads them from `.env` before invoking cargo.
    #[must_use]
    pub const fn from_env() -> Option<Self> {
        match (option_env!("WIFI_SSID"), option_env!("WIFI_PASSWORD")) {
            (Some(ssid), Some(password)) if !ssid.is_empty() => Some(Self { ssid, password }),
            _ => None,
        }
    }

    /// The configured SSID.
    #[must_use]
    pub const fn ssid(&self) -> &'static str {
        self.ssid
    }

    fn station_config(&self) -> Result<StationConfig, WifiError> {
        let ssid: Ssid = self
            .ssid
            .try_into()
            .map_err(|_| WifiError::InvalidArguments)?;

        // An empty password selects an open network.
        let authentication = if self.password.is_empty() {
            AuthenticationMethodConfig::Open
        } else {
            AuthenticationMethodConfig::Wpa2Personal(
                self.password
                    .try_into()
                    .map_err(|_| WifiError::InvalidArguments)?,
            )
        };

        Ok(StationConfig::default()
            .with_ssid(ssid)
            .with_authentication(authentication))
    }
}

/// Connection state reported to the app for status LEDs and logging.
#[derive(Clone, Copy, defmt::Format)]
pub enum LinkState {
    /// A connection attempt is in progress.
    Connecting,
    /// The station connected to an access point.
    Connected {
        /// Signal strength in dBm.
        rssi: i32,
        /// Wi-Fi channel.
        channel: u8,
    },
    /// The station lost its connection.
    Disconnected,
    /// The last connection attempt failed.
    Failed,
}

/// Wi-Fi controller plus the network interface and optional credentials.
pub struct WifiService {
    controller: WifiController<'static>,
    #[cfg(feature = "net")]
    interface: Option<Interface>,
    credentials: Option<Credentials>,
}

impl WifiService {
    /// Create the Wi-Fi controller.
    ///
    /// Without credentials the controller is still usable for scanning.
    pub fn new(wifi: WIFI<'static>, credentials: Option<Credentials>) -> Result<Self, WifiError> {
        let config = match credentials {
            Some(credentials) => ControllerConfig::default()
                .with_initial_config(Config::Station(credentials.station_config()?)),
            None => ControllerConfig::default(),
        };

        let controller = WifiController::new(wifi, config)?;

        Ok(Self {
            controller,
            #[cfg(feature = "net")]
            interface: Some(Interface::station()),
            credentials,
        })
    }

    /// Whether credentials were compiled in.
    #[must_use]
    pub fn has_credentials(&self) -> bool {
        self.credentials.is_some()
    }

    /// The configured SSID, if any. The password is never exposed.
    #[must_use]
    pub fn ssid(&self) -> Option<&'static str> {
        self.credentials.map(|credentials| credentials.ssid())
    }

    /// Scan for access points, strongest signal first.
    pub async fn scan(&mut self) -> Result<Vec<AccessPointInfo>, WifiError> {
        let config = ScanConfig::default()
            .with_max(32)
            .with_show_hidden(true)
            .with_scan_type(ScanTypeConfig::Active {
                min: esp_hal::time::Duration::from_millis(100),
                max: esp_hal::time::Duration::from_millis(150),
            });
        let mut results = self.controller.scan_async(&config).await?;
        results.sort_by_key(|ap| core::cmp::Reverse(ap.signal_strength));
        Ok(results)
    }

    /// Repeatedly scan and log access points. Never returns.
    pub async fn scan_loop(&mut self) -> ! {
        loop {
            match self.scan().await {
                Ok(aps) => {
                    info!("scan: {} access point(s)", aps.len());
                    for ap in &aps {
                        info!(
                            "  {}  rssi={} dBm  ch={}  auth={:?}",
                            ap.ssid, ap.signal_strength, ap.channel, ap.auth_method
                        );
                    }
                }
                Err(err) => warn!("scan failed: {:?}", err),
            }

            Timer::after(Duration::from_secs(10)).await;
        }
    }

    /// Connect, then wait for a disconnect and retry forever.
    ///
    /// Falls back to scanning when no credentials were compiled in.
    pub async fn connection_loop<F>(&mut self, mut on_state: F) -> !
    where
        F: FnMut(LinkState),
    {
        let Some(credentials) = self.credentials else {
            warn!("no WIFI_SSID/WIFI_PASSWORD at build time; falling back to scan-only mode");
            self.scan_loop().await
        };

        info!("station: connecting to {}", credentials.ssid());
        loop {
            on_state(LinkState::Connecting);

            match self.controller.connect_async().await {
                Ok(connected) => {
                    let rssi = self.controller.rssi().unwrap_or(0);
                    info!(
                        "station: connected to {} (bssid={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, ch={}, rssi={} dBm)",
                        connected.ssid,
                        connected.bssid[0],
                        connected.bssid[1],
                        connected.bssid[2],
                        connected.bssid[3],
                        connected.bssid[4],
                        connected.bssid[5],
                        connected.channel,
                        rssi
                    );
                    on_state(LinkState::Connected {
                        rssi,
                        channel: connected.channel,
                    });

                    match self.controller.wait_for_disconnect_async().await {
                        Ok(info) => warn!(
                            "station: disconnected from {} (reason={:?})",
                            info.ssid, info.reason
                        ),
                        Err(err) => warn!("station: disconnect wait failed: {:?}", err),
                    }
                    on_state(LinkState::Disconnected);
                }
                Err(err) => {
                    warn!("station: connect failed: {:?}", err);
                    on_state(LinkState::Failed);
                }
            }

            Timer::after(Duration::from_secs(5)).await;
        }
    }

    /// Start the `embassy-net` stack using DHCP.
    ///
    /// Returns `None` if the interface has already been taken or if the static
    /// resources were initialized before.
    #[cfg(feature = "net")]
    pub fn start_network(&mut self) -> Option<(Stack<'static>, Runner<'static, Interface>)> {
        static RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();

        let interface = self.interface.take()?;
        let rng = Rng::new();
        let seed = (u64::from(rng.random()) << 32) | u64::from(rng.random());

        let resources = RESOURCES.try_init(StackResources::<5>::new())?;
        Some(embassy_net::new(
            interface,
            embassy_net::Config::dhcpv4(Default::default()),
            resources,
            seed,
        ))
    }
}

/// Log the IPv4 address whenever DHCP assigns or changes it. Never returns.
#[cfg(feature = "net")]
pub async fn monitor(stack: Stack<'static>) -> ! {
    let mut last: Option<[u8; 4]> = None;

    loop {
        let current = stack
            .config_v4()
            .map(|config| config.address.address().octets());

        if current != last {
            match current {
                Some(octets) => {
                    info!(
                        "net: ipv4 {}.{}.{}.{}",
                        octets[0], octets[1], octets[2], octets[3]
                    );
                }
                None => info!("net: waiting for DHCP"),
            }
            last = current;
        }

        Timer::after(Duration::from_secs(5)).await;
    }
}

/// Snapshot returned by the example HTTP status endpoint.
#[derive(Clone, Copy)]
pub struct Status {
    /// Temperature in degrees Celsius.
    pub temperature_c: i16,
    /// Last known Wi-Fi RSSI in dBm.
    pub rssi: i32,
    /// Seconds since boot.
    pub uptime_s: u64,
    /// Last fetched weather temperature, if available.
    pub weather_temperature_c: Option<i16>,
    /// Last fetched WMO weather code, if available.
    pub weather_code: Option<u8>,
    /// Last fetched local time as `(hour, minute)`, if available.
    pub local_time: Option<(u8, u8)>,
}

/// Serve a tiny HTTP status endpoint on port 80 forever.
///
/// This is intentionally small: every request receives the same JSON object.
/// It is enough for `curl http://<dhcp-ip>/` and keeps the example practical
/// without pulling in an HTTP stack.
#[cfg(feature = "net")]
pub async fn serve_status<F>(stack: Stack<'static>, mut status: F) -> !
where
    F: FnMut() -> Status,
{
    loop {
        // Keep the TCP buffers on the heap so the async future stays small.
        let mut rx_buffer = alloc::vec![0u8; 1024];
        let mut tx_buffer = alloc::vec![0u8; 1024];
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);

        if let Err(err) = socket.accept(80).await {
            warn!("http: accept failed: {:?}", err);
            Timer::after(Duration::from_secs(1)).await;
            continue;
        }

        let mut request = [0u8; 512];
        let _ = embassy_time::with_timeout(Duration::from_secs(2), socket.read(&mut request)).await;

        let snapshot = status();
        let mut body = heapless::String::<256>::new();
        let _ = core::write!(
            body,
            "{{\"temperature_c\":{},\"rssi\":{},\"uptime_s\":{}",
            snapshot.temperature_c,
            snapshot.rssi,
            snapshot.uptime_s
        );
        if let Some(temperature) = snapshot.weather_temperature_c {
            let _ = core::write!(body, ",\"weather_temperature_c\":{}", temperature);
        }
        if let Some(code) = snapshot.weather_code {
            let _ = core::write!(body, ",\"weather_code\":{}", code);
        }
        if let Some((hour, minute)) = snapshot.local_time {
            let _ = core::write!(body, ",\"local_time\":\"{:02}:{:02}\"", hour, minute);
        }
        let _ = core::write!(body, "}}");

        let mut response = heapless::String::<384>::new();
        let _ = core::write!(
            response,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body.as_str()
        );

        if socket.write_all(response.as_bytes()).await.is_err() {
            warn!("http: response write failed");
        }
        socket.close();
        Timer::after(Duration::from_millis(10)).await;
    }
}

/// Weather snapshot fetched from a public HTTP JSON API.
pub struct Weather {
    /// Temperature in degrees Celsius.
    pub temperature_c: f32,
    /// WMO weather interpretation code.
    pub code: u8,
    local_time: heapless::String<20>,
}

impl Weather {
    /// Local ISO-8601 time reported by the API, e.g. `2026-11-19T18:45`.
    #[must_use]
    pub fn local_time(&self) -> &str {
        self.local_time.as_str()
    }

    /// Local time as `(hour, minute)` if the API string is well-formed.
    #[must_use]
    pub fn hour_minute(&self) -> Option<(u8, u8)> {
        let bytes = self.local_time.as_bytes();
        if bytes.len() < 16 {
            return None;
        }
        let hour = core::str::from_utf8(&bytes[11..13]).ok()?.parse().ok()?;
        let minute = core::str::from_utf8(&bytes[14..16]).ok()?.parse().ok()?;
        Some((hour, minute))
    }

    /// Human-readable description of the WMO weather code.
    #[must_use]
    pub fn description(&self) -> &'static str {
        weather_description(self.code)
    }
}

/// Map a WMO weather interpretation code to a short description.
#[must_use]
pub fn weather_description(code: u8) -> &'static str {
    match code {
        0 => "clear",
        1 => "mostly clear",
        2 => "partly cloudy",
        3 => "overcast",
        45 | 48 => "fog",
        51 | 53 | 55 => "drizzle",
        56 | 57 => "freezing drizzle",
        61 | 63 | 65 => "rain",
        66 | 67 => "freezing rain",
        71 | 73 | 75 => "snow",
        77 => "snow grains",
        80..=82 => "rain showers",
        85 | 86 => "snow showers",
        95 => "thunderstorm",
        96 | 99 => "thunderstorm with hail",
        _ => "unknown",
    }
}

/// Errors returned by the minimal HTTP client / weather fetch.
#[derive(Debug, Clone, Copy, defmt::Format)]
pub enum HttpError {
    /// DNS resolution failed.
    Dns,
    /// TCP connection failed.
    Connect,
    /// Request write failed.
    Write,
    /// Response read failed or timed out.
    Read,
    /// Response status was not 200.
    BadStatus,
    /// Response headers were malformed.
    BadResponse,
    /// Chunked transfer encoding was malformed.
    Chunked,
    /// Response/body exceeded the fixed buffer.
    TooLarge,
    /// Body was not valid UTF-8.
    Utf8,
    /// Expected JSON field was missing or invalid.
    Parse,
}

/// Fetch current weather (and local time) from Open-Meteo over plain HTTP.
///
/// `latitude`/`longitude` are decimal strings, e.g. `"31.82"` and `"117.23"`.
/// The API is public and keyless; this is a small experiment, not a production
/// weather client.
#[cfg(feature = "net")]
pub async fn fetch_weather(
    stack: Stack<'static>,
    latitude: &str,
    longitude: &str,
) -> Result<Weather, HttpError> {
    let mut path = heapless::String::<192>::new();
    core::write!(
        path,
        "/v1/forecast?latitude={latitude}&longitude={longitude}&current=temperature_2m,weather_code&timezone=auto"
    )
    .map_err(|_| HttpError::TooLarge)?;

    let body = http_get::<512>(stack, "api.open-meteo.com", path.as_str()).await?;
    let text = core::str::from_utf8(&body).map_err(|_| HttpError::Utf8)?;

    // Ignore `current_units` and parse only the `current` object.
    let current = text
        .split_once("\"current\":")
        .map(|(_, rest)| rest)
        .ok_or(HttpError::Parse)?;

    let temperature_c = number_after(current, "\"temperature_2m\":")?
        .parse()
        .map_err(|_| HttpError::Parse)?;
    let code = number_after(current, "\"weather_code\":")?
        .parse()
        .map_err(|_| HttpError::Parse)?;
    let time = string_after(current, "\"time\":\"")?;
    let local_time = heapless::String::try_from(time).map_err(|_| HttpError::TooLarge)?;

    Ok(Weather {
        temperature_c,
        code,
        local_time,
    })
}

#[cfg(feature = "net")]
async fn http_get<const N: usize>(
    stack: Stack<'static>,
    host: &str,
    path: &str,
) -> Result<heapless::Vec<u8, N>, HttpError> {
    let dns = DnsSocket::new(stack);
    let addresses = dns
        .query(host, DnsQueryType::A)
        .await
        .map_err(|_| HttpError::Dns)?;
    let address = *addresses.first().ok_or(HttpError::Dns)?;

    let mut rx_buffer = alloc::vec![0u8; 512];
    let mut tx_buffer = alloc::vec![0u8; 512];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket
        .connect((address, 80))
        .await
        .map_err(|_| HttpError::Connect)?;

    let mut request = heapless::String::<384>::new();
    core::write!(
        request,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nUser-Agent: esp32c6-n16\r\n\r\n"
    )
    .map_err(|_| HttpError::TooLarge)?;
    socket
        .write_all(request.as_bytes())
        .await
        .map_err(|_| HttpError::Write)?;

    let mut raw = alloc::vec![0u8; 1024];
    let mut total = 0;
    loop {
        let result =
            embassy_time::with_timeout(Duration::from_secs(10), socket.read(&mut raw[total..]))
                .await;
        match result {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                total += n;
                if total == raw.len() {
                    break;
                }
            }
            _ => return Err(HttpError::Read),
        }
    }
    socket.close();

    let header_end = raw[..total]
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(HttpError::BadResponse)?
        + 4;
    let header = &raw[..header_end];
    if !status_ok(header) {
        return Err(HttpError::BadStatus);
    }

    let body = &raw[header_end..total];
    let mut output = heapless::Vec::<u8, N>::new();
    if contains_ignore_ascii_case(header, b"transfer-encoding: chunked") {
        decode_chunked(body, &mut output)?;
    } else {
        output
            .extend_from_slice(body)
            .map_err(|_| HttpError::TooLarge)?;
    }
    Ok(output)
}

#[cfg(feature = "net")]
fn status_ok(header: &[u8]) -> bool {
    header.starts_with(b"HTTP/") && header.get(9..12) == Some(b"200")
}

#[cfg(feature = "net")]
fn contains_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

#[cfg(feature = "net")]
fn decode_chunked<const N: usize>(
    input: &[u8],
    output: &mut heapless::Vec<u8, N>,
) -> Result<(), HttpError> {
    let mut offset = 0;
    loop {
        let line_end = input[offset..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or(HttpError::Chunked)?
            + offset;
        let size_text = core::str::from_utf8(&input[offset..line_end])
            .map_err(|_| HttpError::Chunked)?
            .trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| HttpError::Chunked)?;
        offset = line_end + 2;

        if size == 0 {
            return Ok(());
        }

        let data_end = offset.checked_add(size).ok_or(HttpError::TooLarge)?;
        let data = input.get(offset..data_end).ok_or(HttpError::Chunked)?;
        output
            .extend_from_slice(data)
            .map_err(|_| HttpError::TooLarge)?;
        offset = data_end;

        if input.get(offset..offset + 2) != Some(b"\r\n") {
            return Err(HttpError::Chunked);
        }
        offset += 2;
    }
}

#[cfg(feature = "net")]
fn number_after<'a>(text: &'a str, key: &str) -> Result<&'a str, HttpError> {
    let start = text.find(key).ok_or(HttpError::Parse)? + key.len();
    let rest = &text[start..];
    let end = rest.find([',', '}']).ok_or(HttpError::Parse)?;
    Ok(rest[..end].trim())
}

#[cfg(feature = "net")]
fn string_after<'a>(text: &'a str, key: &str) -> Result<&'a str, HttpError> {
    let start = text.find(key).ok_or(HttpError::Parse)? + key.len();
    let rest = &text[start..];
    let end = rest.find('"').ok_or(HttpError::Parse)?;
    Ok(&rest[..end])
}
