//! BLE support for the ESP32-C6-N16 board.
//!
//! The crate combines the `esp-radio` BLE controller with `trouble-host`:
//!
//! - [`run_peripheral`] advertises a Nordic UART Service (NUS) and echoes or
//!   answers writes with TX notifications.
//! - [`init`] creates just the controller, which is useful for self-tests.
//!
//! NUS is intentionally simple: it is a practical serial-like channel for
//! phone apps and debug consoles without defining a custom protocol.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use defmt::{error, info, warn};
use embassy_futures::join::join;
use embassy_time::{Duration, Timer};
use esp_hal::{efuse, peripherals::BT};
use esp_radio::ble::{Config as ControllerConfig, controller::BleConnector};
use trouble_host::prelude::*;

pub use esp_radio::ble::controller::BleInitError;

/// BLE controller type used by this board.
pub type BleController = ExternalController<BleConnector<'static>, 1>;

/// Number of simultaneous connections supported by the example.
const CONNECTIONS_MAX: usize = 1;
/// L2CAP channels: ATT plus the signalling channel.
const L2CAP_CHANNELS_MAX: usize = 2;
/// Advertising name of the board.
const DEVICE_NAME: &str = "ESP32-C6-N16";

/// Create the BLE controller.
pub fn init(bt: BT<'static>) -> Result<BleController, BleInitError> {
    let connector = BleConnector::new(bt, ControllerConfig::default())?;
    Ok(ExternalController::new(connector))
}

/// Derive a stable static random BLE address from the chip base MAC.
#[must_use]
pub fn address() -> Address {
    let mut mac = [0u8; 6];
    mac.copy_from_slice(efuse::base_mac_address().as_bytes());
    // A static random address requires the two most significant bits of the
    // address (the last byte on the wire) to be set.
    mac[5] |= 0xC0;
    Address::random(mac)
}

// ---------------------------------------------------------------------------
// Nordic UART Service
// ---------------------------------------------------------------------------

#[gatt_server]
struct Server {
    nus: NusService,
}

#[gatt_service(uuid = "6e400001-b5a3-f393-e0a9-e50e24dcca9e")]
struct NusService {
    /// Central -> device bytes. `write_without_response` keeps phone clients
    /// responsive while `write` supports acknowledged writes too.
    #[characteristic(
        uuid = "6e400002-b5a3-f393-e0a9-e50e24dcca9e",
        write,
        write_without_response
    )]
    rx: heapless::Vec<u8, 20>,
    /// Device -> central notifications.
    #[characteristic(uuid = "6e400003-b5a3-f393-e0a9-e50e24dcca9e", notify)]
    tx: heapless::Vec<u8, 20>,
}

/// Advertise and serve NUS connections forever.
pub async fn run_peripheral(bt: BT<'static>) -> ! {
    let controller = match init(bt) {
        Ok(controller) => controller,
        Err(err) => {
            error!("BLE: controller initialization failed: {:?}", err);
            fail_forever().await
        }
    };

    let address = address();
    info!("BLE: address {}", address);

    let mut resources: HostResources<_, DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        .build();
    let mut peripheral = stack.peripheral();
    let runner = stack.runner();

    // Declared before the server so it outlives the borrowed advertising data.
    let mut adv_data = [0; 31];
    let adv_len = match AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteLocalName(DEVICE_NAME.as_bytes()),
        ],
        &mut adv_data,
    ) {
        Ok(len) => len,
        Err(_) => {
            error!("BLE: advertising data encoding failed");
            fail_forever().await
        }
    };

    let server = match Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: DEVICE_NAME,
        appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
    })) {
        Ok(server) => server,
        Err(_) => {
            error!("BLE: GATT server creation failed");
            fail_forever().await
        }
    };

    info!("BLE: advertising as {} with NUS", DEVICE_NAME);
    let _ = join(
        run_host(runner),
        advertise_forever(&mut peripheral, &server, &adv_data[..adv_len]),
    )
    .await;

    fail_forever().await
}

async fn advertise_forever<'values, C: Controller>(
    peripheral: &mut Peripheral<'values, C, DefaultPacketPool>,
    server: &Server<'values>,
    adv_data: &'values [u8],
) -> ! {
    let params = AdvertisementParameters {
        interval_min: Duration::from_millis(100),
        interval_max: Duration::from_millis(100),
        ..Default::default()
    };

    loop {
        let advertiser = match peripheral
            .advertise(
                &params,
                Advertisement::ConnectableScannableUndirected {
                    adv_data,
                    scan_data: &[],
                },
            )
            .await
        {
            Ok(advertiser) => advertiser,
            Err(_) => {
                warn!("BLE: advertise failed, retrying");
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
        };

        let accepted = match advertiser.accept().await {
            Ok(accepted) => accepted,
            Err(_) => {
                warn!("BLE: accept failed, retrying");
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
        };

        let connection = match accepted.with_attribute_server(server) {
            Ok(connection) => connection,
            Err(_) => {
                warn!("BLE: attaching GATT server failed");
                continue;
            }
        };

        info!("BLE: central connected");
        gatt_events_task(server, &connection).await;
        info!("BLE: central disconnected");
    }
}

async fn run_host<C: Controller, P: PacketPool>(mut runner: Runner<'_, C, P>) -> !
where
    C::Error: defmt::Format,
{
    loop {
        match runner.run().await {
            Ok(()) => error!("BLE: host runner returned"),
            Err(err) => {
                error!("BLE: host runner stopped: {:?}", err);
                Timer::after(Duration::from_secs(1)).await;
            }
        }
    }
}

async fn gatt_events_task<P: PacketPool>(
    server: &Server<'_>,
    connection: &GattConnection<'_, '_, P>,
) {
    loop {
        match connection.next().await {
            GattConnectionEvent::Disconnected { .. } => break,
            GattConnectionEvent::Gatt { event } => {
                let mut response = heapless::Vec::<u8, 20>::new();

                if let GattEvent::Write(write) = &event {
                    write.with_data(|_, data| {
                        if data.starts_with(b"ping") {
                            let _ = response.extend_from_slice(b"pong\n");
                        } else if data.starts_with(b"status") {
                            let _ = response.extend_from_slice(b"esp32c6-n16\n");
                        } else {
                            let _ = response.extend_from_slice(data);
                        }
                    });
                }

                match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(_) => warn!("BLE: failed to reply to GATT event"),
                }

                if !response.is_empty()
                    && server
                        .nus
                        .tx
                        .notify(connection, &response, true)
                        .await
                        .is_err()
                {
                    defmt::debug!("BLE: NUS notify failed (not subscribed?)");
                }
            }
            _ => {}
        }
    }
}

/// Wait forever after an unrecoverable initialization error.
async fn fail_forever() -> ! {
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
