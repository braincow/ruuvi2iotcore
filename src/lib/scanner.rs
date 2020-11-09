use btleplug::bluez::{adapter::ConnectedAdapter, manager::Manager};
use btleplug::api::{CentralEvent, Central, Peripheral};
use color_eyre::{eyre::eyre, SectionExt, Section, eyre::Report};
use std::sync::mpsc::Receiver;
use crossbeam::channel;
use structview::View;
use chrono;
use serde::Serialize;
use std::{time, thread};
use rand::Rng;

use crate::lib::config::AppConfig;
use crate::lib::ruuvi::RuuviTagDataFormat5;
use crate::lib::iotcore::{CNCCommand, CNCCommandMessage};

#[derive(Debug, Serialize)]
pub struct RuuviBluetoothBeacon {
    pub data: RuuviTagDataFormat5,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub address: String
}

pub struct BluetoothScanner {
    bt_central: ConnectedAdapter,
    bt_receiver: Receiver<CentralEvent>,
    channel_sender: channel::Sender<RuuviBluetoothBeacon>,
    cnc_receiver: channel::Receiver<CNCCommandMessage>
}

impl BluetoothScanner {
    pub fn start_scanner(&self) -> Result<(), Report> {
        let mut rng = rand::thread_rng(); // initialize random number generator for this start_scanner() run
        let restart_number = rng.gen_range(1,100000); // used to determine random number that determines if we should try to restart the bluetooth scan
        let mut initially_started = false;

        loop {
            if rng.gen_range(1, 100000) == restart_number || !initially_started {
                match self.bt_central.stop_scan() {
                    Ok(_) => {
                        if initially_started {
                            warn!("Shutting down Bluetooth scan due to hacky fix to random deaths.")
                        }
                    },
                    Err(error) => return Err(
                        eyre!("Unable to stop Bluetooth scan")
                            .with_section(move || error.to_string().header("Reason:")) 
                        )
                };
                match self.bt_central.start_scan() {
                    Ok(_) => {
                        if initially_started {
                            warn!("(Re)starting Bluetooth scan due to hacky fix to random deaths.")
                        }
                    },
                    Err(error) => return Err(
                        eyre!("Unable to start Bluetooth scan")
                            .with_section(move || error.to_string().header("Reason:")) 
                        )
                };
                initially_started = true;
            }
    
            // peek into cnc channel to receive commands from iotcore
            match self.cnc_receiver.try_recv() {
                Ok(msg) => match msg.command {
                    CNCCommand::SHUTDOWN => {
                        warn!("CNC command received: SHUTDOWN software");
                        break;
                    },
                    _ => warn!("Unimplemented CNC message for Bluetooth scanner: {:?}", msg)
                },
                Err(error) => trace!("Unable to receive incoming CNC message: {}", error)
            };

            // check into the channel to see if there are beacons to relay to the mqtt broker
            match  self.bt_receiver.try_recv() {
                Ok(event) => {
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
                        trace!("No manufacturer data received in: {:?}", properties);
                    }    
                },
                Err(error) => trace!("Error on receiving Bluetooth event: {}", error)
            };

            // sleep for a while to reduce amount of CPU burn and idle for a while
            thread::sleep(time::Duration::from_millis(10));
        }

        match self.bt_central.stop_scan() {
            Ok(_) => {},
            // only log the error as we are shutting down correctly or otherwise we would not have reached here
            //  (this happens only from break; statement that is triggered by cnc channel command)
            Err(error) => error!("Unable to stop Bluetooth scan: {}", error)
        };

        Ok(())
    }

    pub fn build(config: &AppConfig, s: &channel::Sender<RuuviBluetoothBeacon>, cnc_r: &channel::Receiver<CNCCommandMessage>) -> Result<BluetoothScanner, Report> {
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

        let mut adapter = match adapters.into_iter().nth(config.bluetooth.adapter_index()) {
            Some(adapter) => adapter,
            None => return Err(
                eyre!("Configured Bluetooth adapter not found.")
                    .with_section(move || config.bluetooth.adapter_index().to_string().header("Configured adapter index:"))
                )
        };

        // reset the adapter -- clears out any errant state
        adapter = match manager.down(&adapter) {
            Ok(adapter) => adapter,
            Err(error) => return Err(
                eyre!("Unable to shutdown Bluetooth adapter")
                    .with_section(move || error.to_string().header("Reason:")) 
                )
        };
        adapter = match manager.up(&adapter) {
            Ok(adapter) => adapter,
            Err(error) => return Err(
                eyre!("Unable to (re)start Bluetooth adapter")
                    .with_section(move || error.to_string().header("Reason:")) 
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
        // use only passive scan as we are interested in beacons only
        central.active(false);

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
            channel_sender: s.clone(),
            cnc_receiver: cnc_r.clone()
        })

    }
}

// eof
