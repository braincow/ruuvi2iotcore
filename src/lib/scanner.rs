use btleplug::bluez::{adapter::ConnectedAdapter, manager::Manager};
use btleplug::api::{CentralEvent, Central, Peripheral};
use color_eyre::{eyre::eyre, SectionExt, Section, eyre::Report};
use std::sync::mpsc::Receiver;
use crossbeam::channel;
use structview::View;
use chrono;
use serde::Serialize;

use crate::lib::config::AppConfig;
use crate::lib::ruuvi::RuuviTagDataFormat5;

#[derive(Debug, Serialize)]
pub struct RuuviBluetoothBeacon {
    pub data: RuuviTagDataFormat5,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub address: String
}

pub struct BluetoothScanner {
    bt_central: ConnectedAdapter,
    bt_receiver: Receiver<CentralEvent>,
    channel_sender: channel::Sender<RuuviBluetoothBeacon>
}

impl BluetoothScanner {
    pub fn start_scanner(&self) -> Result<(), Report> {
        // use only passive scan as we are interested in beacons only
        self.bt_central.active(false);
        match self.bt_central.start_scan() {
            Ok(_) => {},
            Err(error) => return Err(
                eyre!("Unable to start Bluetooth scan")
                    .with_section(move || error.to_string().header("Reason:")) 
                )
        };

        while let Ok(event) = self.bt_receiver.recv() {
            let bd_addr = match event {
                CentralEvent::DeviceDiscovered(bd_addr) => Some(bd_addr),
                CentralEvent::DeviceUpdated(bd_addr) => Some(bd_addr),
                _ => None
            };

            // FIXME: unwrap()
            let peripheral = self.bt_central.peripheral(bd_addr.unwrap()).unwrap();
            let properties = peripheral.properties();

            if let Some(data) = properties.manufacturer_data {
                if data[0] == 153 && data[1] == 4 {
                    // these values in DEC instead of HEX to identify ruuvi tags with dataformat 5
                    // ^--- fields in index 0 and 1 indicate 99 4 as the manufacturer (ruuvi) and index 3 points data version
                    let packet = match data[2] {
                        // https://github.com/ruuvi/ruuvi-sensor-protocols/blob/master/dataformat_05.md
                        // ^--- field in index 3 points to data version and everything forward from there are data points
                        // @TODO: error handling, aka handle unwrap()
                        5 => {
                            let payload = match RuuviTagDataFormat5::view(&data[3..]) {
                                Ok(payload) => payload,
                                Err(error) => return Err(
                                    eyre!("Unable to parse Bluetooth packets peripheral properties into Ruuvitag v5 structure.")
                                        .with_section(move || error.to_string().header("Reason:")) 
                                    )
                            };
                            let beacon = RuuviBluetoothBeacon{
                                data: *payload,
                                timestamp: chrono::Utc::now(),
                                address: bd_addr.unwrap().to_string()
                            };
                            Some(beacon)
                        },
                        _ => {
                            warn!("Ruuvitag data format '{}' not implemented yet.", data[2]);
                            None
                        }
                    };
    
                    if let Some(packet) = packet {
                        self.channel_sender.send(packet).unwrap();
                    }
                }
            } else {
                debug!("No manufacturer data received in: {:?}", properties);
            }
        }

        warn!("Exiting Bluetooth discovery loop.");

        Ok(())
    }

    pub fn build(config: &AppConfig, s: &channel::Sender<RuuviBluetoothBeacon>) -> Result<BluetoothScanner, Report> {
        let manager = match Manager::new() {
            Ok(manager) => manager,
            Err(error) => return Err(
                eyre!("Unable to initialize Bluetooth manager")
                    .with_section(move || error.to_string().header("Reason:")) 
                )
        };

        let adapters = match manager.adapters() {
            Ok(adapters) => adapters,
            Err(error) => return Err(
                eyre!("Unable to list Bluetooth adapters")
                    .with_section(move || error.to_string().header("Reason:")) 
                )
        };

        let adapter = match adapters.into_iter().nth(config.bluetooth.adapter_index()) {
            Some(adapter) => adapter,
            None => return Err(
                eyre!("Configured Bluetooth adapter not found.")
                    .with_section(move || config.bluetooth.adapter_index().to_string().header("Configured adapter index:"))
                )
        };

        let central = match adapter.connect() {
            Ok(central) => central,
            Err(error) => return Err(
                eyre!("Unable to connect to Bluetooth adapter")
                    .with_section(move || config.bluetooth.adapter_index().to_string().header("Configured adapter index:"))
                    .with_section(move || error.to_string().header("Reason:")) 
                )
        };

        let receiver = match central.event_receiver() {
            Some(receiver) => receiver,
            None => return Err(
                eyre!("Unable to build Bluetooth receiver instance for configured Bluetooth adapter")
                    .with_section(move || config.bluetooth.adapter_index().to_string().header("Configured adapter index:"))
                )
        };

        Ok(BluetoothScanner {
            bt_central: central,
            bt_receiver: receiver,
            channel_sender: s.clone()
        })

    }
}

// eof
