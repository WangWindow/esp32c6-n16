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
use embassy_net::{Runner, Stack, StackResources, tcp::TcpSocket};
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
        static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();

        let interface = self.interface.take()?;
        let rng = Rng::new();
        let seed = (u64::from(rng.random()) << 32) | u64::from(rng.random());

        let resources = RESOURCES.try_init(StackResources::<3>::new())?;
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
        let mut body = heapless::String::<128>::new();
        let _ = core::write!(
            body,
            "{{\"temperature_c\":{},\"rssi\":{},\"uptime_s\":{}}}",
            snapshot.temperature_c,
            snapshot.rssi,
            snapshot.uptime_s
        );

        let mut response = heapless::String::<320>::new();
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
