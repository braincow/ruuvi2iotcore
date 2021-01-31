use btleplug::api::{Central, CentralEvent, Peripheral};
use btleplug::bluez::{adapter::ConnectedAdapter, manager::Manager};
use chrono;
use color_eyre::{eyre::eyre, eyre::Report, Section, SectionExt};
use crossbeam::channel;
use ruuvitag_dataformat::RuuviTagDataFormat5;
use serde::Serialize;
use std::clone::Clone;
use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::{thread, time};
use structview::View;

use crate::iotcore::{CNCCommand, IOTCoreCNCMessageKind};

#[derive(Debug, Serialize, Clone)]
pub struct RuuviBluetoothBeacon {
    pub data: RuuviTagDataFormat5,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub address: String,
}

pub struct BluetoothScanner {
    bt_central: Option<ConnectedAdapter>,
    bt_receiver: Option<Receiver<CentralEvent>>,
    channel_sender: channel::Sender<RuuviBluetoothBeacon>,
    cnc_receiver: channel::Receiver<IOTCoreCNCMessageKind>,
    adapter_index: Option<usize>,
    stuck_data_threshold: Option<i64>,
}

impl BluetoothScanner {
    fn reserve_adapter(&mut self) -> Result<(), Report> {
        debug!("Reserving Bluetooth adapter");

        let manager = match Manager::new() {
            Ok(manager) => manager,
            Err(error) => {
                return Err(eyre!("Unable to initialize Bluetooth manager")
                    .with_section(move || error.to_string().header("Reason:")))
            }
        };

        if self.adapter_index.is_none() {
            return Err(eyre!("No adapter_index setup for reserving adapter"));
        }
        let adapter_index = self.adapter_index.unwrap();

        let adapters = match manager.adapters() {
            Ok(adapters) => adapters,
            Err(error) => {
                return Err(eyre!("Unable to list Bluetooth adapters")
                    .with_section(move || error.to_string().header("Reason:")))
            }
        };

        let mut adapter = match adapters.into_iter().nth(adapter_index) {
            Some(adapter) => adapter,
            None => {
                return Err(
                    eyre!("Configured Bluetooth adapter not found.").with_section(move || {
                        adapter_index
                            .to_string()
                            .header("Configured adapter index:")
                    }),
                )
            }
        };

        // reset the adapter -- clears out any errant state
        adapter = match manager.down(&adapter) {
            Ok(adapter) => adapter,
            Err(error) => {
                return Err(eyre!("Unable to shutdown Bluetooth adapter")
                    .with_section(move || error.to_string().header("Reason:")))
            }
        };
        adapter = match manager.up(&adapter) {
            Ok(adapter) => adapter,
            Err(error) => {
                return Err(eyre!("Unable to (re)start Bluetooth adapter")
                    .with_section(move || error.to_string().header("Reason:")))
            }
        };

        let central = match adapter.connect() {
            Ok(central) => central,
            Err(error) => {
                return Err(eyre!("Unable to connect to Bluetooth adapter")
                    .with_section(move || {
                        adapter_index
                            .to_string()
                            .header("Configured adapter index:")
                    })
                    .with_section(move || error.to_string().header("Reason:")))
            }
        };
        self.bt_central = Some(central.clone());

        let receiver =
            match central.event_receiver() {
                Some(receiver) => receiver,
                None => return Err(eyre!(
                    "Unable to build Bluetooth receiver instance for configured Bluetooth adapter"
                )
                .with_section(move || {
                    adapter_index
                        .to_string()
                        .header("Configured adapter index:")
                })),
            };
        self.bt_receiver = Some(receiver);

        Ok(())
    }

    fn release_adapter(&mut self) -> Result<(), Report> {
        trace!("in release_adapter");
        if self.bt_central.is_some() {
            debug!("Releasing Bluetooth adapter.");
            match self.bt_central.as_ref().unwrap().stop_scan() {
                Ok(_) => {
                    self.bt_central = None;
                    self.bt_receiver = None;
                }
                Err(error) => {
                    return Err(eyre!("Unable to release Bluetooth adapter")
                        .with_section(move || error.to_string().header("Reason:")))
                }
            };
        }

        Ok(())
    }

    fn start_scan(&self) -> Result<(), Report> {
        trace!("in start_scan");
        match self.bt_central {
            None => return Err(eyre!("No Bluetooth adapter reserved for use")),
            Some(_) => {
                // use only passive scan as we are interested in beacons only
                self.bt_central.as_ref().unwrap().active(false);
                match self.bt_central.as_ref().unwrap().start_scan() {
                    Ok(_) => info!("Started passive Bluetooth scan on configured adapter"),
                    Err(error) => {
                        return Err(eyre!("Unable to start Bluetooth scan on adapter")
                            .with_section(move || {
                                self.adapter_index
                                    .unwrap()
                                    .to_string()
                                    .header("Configured adapter index:")
                            })
                            .with_section(move || error.to_string().header("Reason:")))
                    }
                };
                Ok(())
            }
        }
    }

    fn stop_scan(&self) -> Result<(), Report> {
        trace!("in stop_scan");
        match self.bt_central {
            None => return Err(eyre!("No Bluetooth adapter reserved for use")),
            Some(_) => {
                match self.bt_central.as_ref().unwrap().stop_scan() {
                    Ok(_) => info!("Stopped passive Bluetooth scan on configured adapter"),
                    Err(error) => {
                        return Err(eyre!("Unable to stop Bluetooth scan on adapter")
                            .with_section(move || {
                                self.adapter_index
                                    .unwrap()
                                    .to_string()
                                    .header("Configured adapter index:")
                            })
                            .with_section(move || error.to_string().header("Reason:")))
                    }
                };
                Ok(())
            }
        }
    }

    pub fn start_scanner(&mut self) -> Result<bool, Report> {
        trace!("in start_scanner");
        if self.adapter_index.is_some() {
            trace!("Entering to start_scanner() from unclean restart.");
            // i am restarting from main loop as I got here and I have some adapter index
            //  already configured
            match self.release_adapter() {
                Ok(_) => match self.reserve_adapter() {
                    Ok(_) => self.start_scan()?,
                    Err(error) => {
                        error!("{}", error);
                        self.release_adapter()?;
                        self.adapter_index = None;
                        // force exit to main loop and restart in clean state
                        return Ok(false);
                    }
                },
                Err(error) => {
                    error!("{}", error);
                    match self.release_adapter() {
                        Ok(_) => {},
                        Err(error) => error!("Compound error while trying to recover from unclean thread restart: {}", error)
                    }
                    self.bt_central = None;
                    self.bt_receiver = None;
                    self.adapter_index = None;
                    warn!("Scanner internal configuration reset now forced. Expecting RESET command or new configuration from MQTT broker.");
                    // force exit to main loop and restart in clean state
                    return Ok(false);
                }
            };
        }

        let mut beacon_stuck_inventory: HashMap<String, RuuviBluetoothBeacon> = HashMap::new();

        loop {
            // peek into cnc channel to receive commands from iotcore
            match self.cnc_receiver.try_recv() {
                Ok(msg) => match msg {
                    IOTCoreCNCMessageKind::COMMAND(command) => match command {
                        Some(command) => match command.command {
                            CNCCommand::SHUTDOWN => {
                                warn!("CNC command received: SHUTDOWN software");
                                self.release_adapter()?;
                                break;
                            }
                            CNCCommand::RESET => {
                                warn!("CNC command received: RESET software");
                                self.release_adapter()?;
                                return Ok(false);
                            }
                            _ => warn!(
                                "Unimplemented CNC message for Bluetooth scanner: {:?}",
                                command
                            ),
                        },
                        None => debug!("Empty command received from CNC channel"),
                    },
                    IOTCoreCNCMessageKind::CONFIG(collectconfig) => match collectconfig {
                        Some(collectconfig) => {
                            let new_adapter_index = match collectconfig.bluetooth {
                                Some(bluetooth) => bluetooth.adapter_index,
                                None => 0,
                            };
                            self.stuck_data_threshold = collectconfig.stuck_data_threshold;
                            if self.adapter_index.is_none() {
                                trace!("Associate Bluetooth adapter for the first time");
                                // associate the adapter
                                self.adapter_index = Some(new_adapter_index);
                                self.reserve_adapter()?;
                            } else if self.adapter_index != Some(new_adapter_index) {
                                //  store the adapter_index and exit with boolean value that causes main loop
                                //  to restart us cleanly
                                self.stop_scan()?;
                                self.adapter_index = Some(new_adapter_index);
                                trace!("Restarting through main loop to finalize change of associated Bluetooth adapter");
                                return Ok(false);
                            } else {
                                trace!("No change to associated Bluetooth adapter");
                            }
                            // (re)start scanning as a precaution against timeouts on some hardware or for the first time
                            self.stop_scan()?;
                            self.start_scan()?;
                        }
                        None => debug!("Empty collect config received from CNC channel"),
                    },
                },
                Err(_) => {}
            };

            // check into the channel to see if there are beacons to relay to the mqtt broker
            if self.bt_receiver.is_some() && self.bt_central.is_some() {
                match self.bt_receiver.as_ref().unwrap().try_recv() {
                    Ok(event) => {
                        let bd_addr = match event {
                            CentralEvent::DeviceDiscovered(bd_addr) => Some(bd_addr),
                            CentralEvent::DeviceUpdated(bd_addr) => Some(bd_addr),
                            _ => None,
                        };

                        // FIXME: unwrap()
                        let peripheral = self
                            .bt_central
                            .as_ref()
                            .unwrap()
                            .peripheral(bd_addr.unwrap())
                            .unwrap();
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

                                        let beacon = RuuviBluetoothBeacon {
                                            data: *payload,
                                            timestamp: chrono::Utc::now(),
                                            address: bd_addr.unwrap().to_string(),
                                        };

                                        // check against value measured 3 minutes ago and if it is identical
                                        //  something is wrong in the stack in which case restart thread to recover.
                                        if beacon_stuck_inventory.contains_key(&beacon.address) {
                                            trace!(
                                                "Comparing beacon data to see if scanner is stuck"
                                            );
                                            let old_beacon = beacon_stuck_inventory
                                                .get(&beacon.address)
                                                .unwrap();
                                            if chrono::Utc::now()
                                                .signed_duration_since(old_beacon.timestamp)
                                                >= self.stuck_data_threshold()
                                            {
                                                if beacon.data.to_string()
                                                    == old_beacon.data.to_string()
                                                {
                                                    error!("Values from {} seconds ago are identical for Ruuvi tag: {}", 
                                                        self.stuck_data_threshold(), beacon.address);
                                                    warn!("Bluetooth stack probably stuck.");
                                                    return Ok(false);
                                                } else {
                                                    debug!("Updating Ruuvi tag: {} in beacon_stuck_inventory after succesful test.", beacon.address);
                                                    // values from 3 minutes ago seemed to differ as expected. update inventory with this beacon
                                                    let beacon_clone = beacon.clone();
                                                    beacon_stuck_inventory.insert(
                                                        beacon_clone.address.clone(),
                                                        beacon_clone,
                                                    );
                                                }
                                            }
                                        } else {
                                            debug!("Adding discovered Ruuvi tag: {} to beacon_stuck_inventory to track stuck beacons (if any)", beacon.address);
                                            // first time im seeing this Ruuvi tag. add initial beacon
                                            let beacon_clone = beacon.clone();
                                            beacon_stuck_inventory
                                                .insert(beacon_clone.address.clone(), beacon_clone);
                                        }

                                        Some(beacon)
                                    }
                                    _ => {
                                        warn!(
                                            "Ruuvitag data format '{}' not implemented yet.",
                                            data[2]
                                        );
                                        None
                                    }
                                };

                                if let Some(packet) = packet {
                                    self.channel_sender.send(packet).unwrap();
                                }
                            }
                        }
                    }
                    Err(_) => {}
                };
            }

            // sleep for a while to reduce amount of CPU burn and idle for a while
            thread::sleep(time::Duration::from_millis(100));
        }

        self.release_adapter()?;

        Ok(true)
    }

    fn stuck_data_threshold(&self) -> chrono::Duration {
        let default = 180;
        if self.stuck_data_threshold.is_some() {
            let mut threshold = self.stuck_data_threshold.unwrap();
            if threshold <= 0 {
                warn!("Configured stuck data threshold can not be less or equal to zero. Defaulting to {} seconds.", default);
                threshold = default;
            }
            chrono::Duration::seconds(threshold)
        } else {
            // three minutes
            chrono::Duration::seconds(default)
        }
    }

    pub fn build(
        s: &channel::Sender<RuuviBluetoothBeacon>,
        cnc_r: &channel::Receiver<IOTCoreCNCMessageKind>,
    ) -> Result<BluetoothScanner, Report> {
        trace!("in build");
        Ok(BluetoothScanner {
            adapter_index: None,
            bt_central: None,
            bt_receiver: None,
            channel_sender: s.clone(),
            cnc_receiver: cnc_r.clone(),
            stuck_data_threshold: None,
        })
    }
}

// eof
