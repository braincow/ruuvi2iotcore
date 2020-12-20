use serde::{Serialize, Deserialize};
use color_eyre::{eyre::eyre, SectionExt, Section, eyre::Report};
use crossbeam::channel;
use paho_mqtt as mqtt;
use eui48::{MacAddress, MacAddressFormat};
use std::time::{Instant, Duration};
use std::{time, thread};
use std::sync::mpsc::Receiver;
use std::clone::Clone;
use std::str::FromStr;
use std::collections::HashMap;

use crate::configfile::AppConfig;
use crate::scanner::RuuviBluetoothBeacon;
use crate::jwt::IotCoreAuthToken;

#[derive(Debug,Clone)]
pub enum IOTCoreCNCMessageKind {
    COMMAND(Option<CNCCommandMessage>),
    CONFIG(Option<CollectConfig>)
}

#[derive(Debug,Deserialize, Clone)]
pub enum CNCCommand {
    #[serde(rename = "collect")]
    COLLECT,
    #[serde(rename = "pause")]
    PAUSE,
    #[serde(rename = "shutdown")]
    SHUTDOWN,
    #[serde(rename = "reset")]
    RESET,
}

#[derive(Debug,Deserialize, Clone)]
pub struct CNCCommandMessage {
    pub command: CNCCommand
}

#[derive(Debug,Deserialize,Serialize,Clone,PartialEq,PartialOrd)]
pub struct BluetoothConfig {
    pub adapter_index: usize
}

#[derive(Debug,Deserialize,Serialize,Clone,PartialEq,PartialOrd)]
pub struct CollectConfig {
    collecting: bool,
    event_subfolder: Option<String>,
    pub stuck_data_threshold: Option<i64>,
    collection_size: Option<usize>,
    pub bluetooth: Option<BluetoothConfig>
}
impl CollectConfig {
    pub fn collection_size(&self) -> usize {
        match self.collection_size {
            Some(size) => size,
            None => 0
        }
    }
}

pub struct IotCoreClient {
    ssl_opts: mqtt::SslOptions,
    conn_opts: mqtt::ConnectOptions,
    client: mqtt::Client,
    channel_receiver: channel::Receiver<RuuviBluetoothBeacon>,
    cnc_sender: channel::Sender<IOTCoreCNCMessageKind>,
    jwt_factory: IotCoreAuthToken,
    config_topic: String,
    state_topic: String,
    command_topic_root: String,
    consumer: Receiver<Option<mqtt::message::Message>>,
    collectconfig: Option<CollectConfig>,
    last_pause: Option<Instant>,
    last_seen: Instant,
    discovered_tags: HashMap<MacAddress, Vec<RuuviBluetoothBeacon>>,
}

impl IotCoreClient {
    fn publish_message(&mut self, topic: String, message: String) -> Result<(), Report> {
        trace!("in publish_message");
        debug!("outbound mqtt topic: {}", topic);
        trace!("outbound mqtt message: {}", message);

        let msg = message.as_bytes().to_vec();
        // fullfill IoT Core's odd JWT based authentication needs by disconnecting & connecting with new one
        //   when needed
        if !self.jwt_factory.is_valid(60) || !self.client.is_connected() {
            warn!("JWT token has/is about to expire or we have no connection. Initiating reconnect.");
            self.disconnect()?;
            self.conn_opts = mqtt::ConnectOptionsBuilder::new()
                .user_name("not_used")
                .password(&self.jwt_factory.renew()?)
                .ssl_options(self.ssl_opts.clone())
                .finalize();
            self.connect()?;
        }

        // create message and send it
        let mqtt_msg = mqtt::MessageBuilder::new()
            .topic(topic)
            .payload(msg)
            .qos(mqtt::QOS_1)
            .finalize();

            Ok(match self.client.publish(mqtt_msg) {
                Ok(retval) => retval,
                Err(error) => return Err(
                    eyre!("Error while publishing to MQTT")
                        .with_section(move || error.to_string().header("Reason:"))
                    )
            })
    }

    fn disconnect(&mut self) -> Result<(), Report> {
        trace!("in disconnect");
        if self.client.is_connected() {
            warn!("Disconnecting from MQTT broker");
        }
        match self.client.disconnect(None) {
            Ok(_) => Ok(()),
            Err(error) => {
                if self.client.is_connected() {
                    Err(eyre!("Error while disconnecting MQTT broker")
                        .with_section(move || error.to_string().header("Reason:"))
                    )
                } else {
                    warn!("There was an error while disconnecting MQTT broker, but we are apparently disconnected anyway: {}", error);
                    Ok(())
                }
            }
        }
    }

    fn connect(&mut self) -> Result<(), Report> {
        trace!("in connect");
        // connect to the mqtt broker
        match self.client.connect(self.conn_opts.clone()) {
            Ok(_) => info!("Connected to IoT core service"),
            Err(error) => return Err(
                eyre!("Error while connecting to IoT core service")
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };

        // subscribe to command and control channels
        match self.client.subscribe_many(&[self.config_topic.to_string(), format!("{}/#", self.command_topic_root.to_string())],
                &[mqtt::QOS_1, mqtt::QOS_1]) {
            Ok(_) => {},
            Err(error) => return Err(
                eyre!("Error while subscribing to command and control topics")
                    .with_section(move || error.to_string().header("Reason:"))
                )
        }

        self.reattach_discovered_devices();

        Ok(())
    }

    fn set_collecting_state(&mut self, enabled: bool) -> Result<(), Report> {
        trace!("in set_collecting_state");
        debug!("set_collecting_state({})", enabled);
        if let Some(collectconfig) = &self.collectconfig {
            let mut newconfig = collectconfig.clone();
            newconfig.collecting = enabled;
            self.collectconfig = Some(newconfig);
            debug!("collectconfig is now: {:?}", self.collectconfig);
            self.publish_message(self.state_topic.clone(), serde_json::to_string_pretty(&self.collectconfig).unwrap())?;
        } else {
            error!("No collect config defined. Unable to change collect state to: {}", enabled);
        }

        Ok(())
    }

    fn enable_collecting(&mut self) -> Result<(), Report> {
        trace!("in enable_collecting");
        let retval = self.set_collecting_state(true);
        if retval.is_ok() {
            self.last_pause = None;
        }
        retval
    }

    fn disable_collecting(&mut self) -> Result<(), Report> {
        trace!("in disable_collecting");
        let retval = self.set_collecting_state(false);
        if retval.is_ok() {
            self.last_pause = Some(Instant::now());
        }
        retval
    }

    pub fn start_client(&mut self) -> Result<bool, Report> {
        trace!("in start_client");
        // cycle connection state
        if self.client.is_connected() {
            trace!("Entering to start_client() from unclean restart.");
            self.disconnect()?;
        }
        self.connect()?;
        
        self.last_seen = Instant::now();
        // loop messages and wait for a ready signal
        loop {
            // check that we are actually doing work, and if not then issue a restart to threads
            if self.last_seen.elapsed() >= Duration::from_secs(58) {
                warn!("No beacons detected for 58 seconds. Issuing thread restart.");
                // emit reset signal to the cnc channel
                self.cnc_sender.send(IOTCoreCNCMessageKind::COMMAND(
                    Some(CNCCommandMessage { command: CNCCommand::RESET }))).unwrap(); // TODO: fix unwrap
                // exit cleanly and issue restart from main loop
                if self.client.is_connected() {
                    self.disconnect()?;
                }
                return Ok(false)
            }

            // check into the subscriptions if there are any incoming cnc messages
            match self.consumer.try_recv() {
                Ok(optmsg) => {
                    if let Some(msg) = optmsg {
                        trace!("incoming CNC message: '{:?}'", msg);

                        if msg.topic() == self.config_topic {
                            // we received new config, decode it
                            let new_collectconfig: Option<CollectConfig> = match serde_json::from_str(&msg.payload_str()) {
                                Ok(config) => Some(config),
                                Err(error) => { 
                                    error!("Unable to parse new collect config: {}", error);
                                    None
                                }
                            };
                            if new_collectconfig != self.collectconfig && new_collectconfig.is_some() {
                                self.collectconfig = new_collectconfig;
                                debug!("New collect config activated is '{:?}'", self.collectconfig);
                                if !&self.collectconfig.as_ref().unwrap().collecting {
                                    self.disable_collecting()?;
                                } else {
                                    self.enable_collecting()?;
                                }
                                // send config to CNC channel
                                self.cnc_sender.send(IOTCoreCNCMessageKind::CONFIG(self.collectconfig.clone())).unwrap(); // TODO: fix unwrap    
                            } else {
                                debug!("Not replacing active collect config with identical one.");
                            }
                        } else if msg.topic().starts_with(&self.command_topic_root) {
                            // command was sent into root or subfolder of command channel
                            // TODO: implement subfolder support
                            let command: Option<CNCCommandMessage> = match serde_json::from_str(&msg.payload_str()) {
                                Ok(command) => Some(command),
                                Err(error) => { 
                                    error!("Unable to parse CNC command: {}", error);
                                    None
                                }
                            };
                            // also publish the command to CNC channel
                            self.cnc_sender.send(IOTCoreCNCMessageKind::COMMAND(command.clone())).unwrap(); // TODO: fix unwrap
                            if let Some(command) = command {
                                // react locally to the message as well
                                match command.command {
                                    CNCCommand::COLLECT => {
                                        info!("CNC command received: COLLECT beacons");
                                        self.enable_collecting()?;
                                    },
                                    CNCCommand::PAUSE => {
                                        warn!("CNC command received: PAUSE collecting beacons");
                                        self.disable_collecting()?;
                                    },
                                    CNCCommand::SHUTDOWN => {
                                        warn!("CNC command received: SHUTDOWN software");
                                        self.detach_devices();
                                        break;
                                    },
                                    CNCCommand::RESET => {
                                        warn!("CNC command received: RESET software");
                                        self.disconnect()?;
                                        // send the current collect configuration to cnc channel so that
                                        //  bluetooth thread can use it after it recovers
                                        self.cnc_sender.send(IOTCoreCNCMessageKind::CONFIG(self.collectconfig.clone())).unwrap(); // TODO: fix unwrap
                                        return Ok(false)
                                    },
                                };
                            }
                        } else {
                            debug!("Unimplemented CNC topic in received message.");
                        }
                    }
                },
                Err(_) => {}
            };

            // check into the channel to see if there are beacons to relay to the mqtt broker
            match self.channel_receiver.try_recv() {
                Ok(msg) => {
                    debug!("new incoming ruuvi tag beacon from bt thread: {:?}", msg);
                    // update the last_seen counter to verify internally that we are doing work
                    self.last_seen = Instant::now();

                    let address = MacAddress::from_str(&msg.address).unwrap();

                    let mut queue: Vec<RuuviBluetoothBeacon> = match self.discovered_tags.get(&address) {
                        Some(queue) => queue.to_vec(),
                        None => Vec::new()
                    };

                    // submit the beacon to iotcore if collecting them is enabled
                    if self.collectconfig.as_ref().unwrap().collecting {
                        if self.try_attach_device(&address) {
                            let topic = self.device_event_topic(&address).unwrap();

                            if &self.collectconfig.as_ref().unwrap().collection_size() <= &1 {
                                trace!("publish individual beacon");
                                match self.publish_message(topic, serde_json::to_string_pretty(&msg).unwrap()) {
                                    Ok(_) => {},
                                    Err(error) => error!("Error on publishing message to MQTT: '{}'. Beacon lost.", error)
                                };
                            } else if queue.len() >= self.collectconfig.as_ref().unwrap().collection_size() - 1 {
                                trace!("publish beacon queue");
                                queue.push(msg);
                                debug!("Message queue size for '{}': {}/{}", address, queue.len(), self.collectconfig.as_ref().unwrap().collection_size());
                                match self.publish_message(topic, serde_json::to_string_pretty(&queue).unwrap()) {
                                    Ok(_) => { self.discovered_tags.insert(address, Vec::new()); },
                                    Err(error) => error!("Error on publishing message queue to MQTT: '{}'. Will retry.", error)
                                };
                            } else {
                                trace!("add beacon to queue");
                                // add beacon to queue
                                queue.push(msg);
                                debug!("Message queue size for '{}': {}/{}", address, queue.len(), self.collectconfig.as_ref().unwrap().collection_size());
                                // replace in hashmap the message queue with new extended one
                                self.discovered_tags.insert(address, queue.to_vec());
                            }
                        }
                    } else {
                        trace!("beacon collection is paused");
                        if let Some(last_pause) = self.last_pause {
                            if last_pause.elapsed() >= Duration::from_secs(4*60) {
                                // we are paused, so to avoid timeout due to lack of published messages to broker we occasionally will need to
                                //  publish our state to avoid that. as a short hand we essentially do a pause again.
                                self.disable_collecting()?;
                                warn!("Beacon collection is paused.");
                            }
                        } else {
                            error!("Beacon collection is paused, but paused state was not established correctly.")
                        }
                    }
                },
                Err(_) => {}
            };

            // sleep for a while to reduce amount of CPU burn and idle for a while
            thread::sleep(time::Duration::from_millis(100));
        }
        
        self.disconnect()?;
        
        Ok(true)
    }

    fn try_attach_device(&mut self, address: &MacAddress) -> bool {
        trace!("in try_attach_device");
        if self.client.is_connected() && self.discovered_tags.get(address).is_none() {
            // try to attach a newly discovered beacon owner to this gateway
            //  (succesful only if bound)
            match self.publish_message(self.device_attach_topic(&address), "{}".to_string()) {
                Ok(_) => {
                    info!("Discovered Ruuvi tag ({}) attached to gateway succesfully.", address.to_string(MacAddressFormat::Canonical).to_uppercase());
                    self.discovered_tags.insert(*address, Vec::new());
                },
                Err(error) => {
                    warn!("Discovered Ruuvi tag ({}) attachment to gateway failed (possibly not bound): {}", 
                        address.to_string(MacAddressFormat::Canonical).to_uppercase(),
                        error);
                    return false;
                }
            };
        }

        true
    }

    fn reattach_discovered_devices(&mut self) {
        trace!("in reattach_discovered_devices");
        if self.client.is_connected() {
            for (tag, _) in self.discovered_tags.clone().iter() {
                match self.publish_message(self.device_attach_topic(&tag), "{}".to_string()) {
                    Ok(_) => info!("Discovered Ruuvi tag ({}) reattached to gateway succesfully.", tag.to_string(MacAddressFormat::Canonical).to_uppercase()),
                    Err(error) => {
                        // remove the tag from associated list as it failed this time around
                        self.discovered_tags.remove(tag);
                        warn!("Discovered Ruuvi tag ({}) reattached to gateway failed: {}", 
                            tag.to_string(MacAddressFormat::Canonical).to_uppercase(),
                            error);
                    }
                }
            }
        }
    }

    fn detach_devices(&mut self) {
        trace!("in detach_devices");
        if self.client.is_connected() {
            for (tag, _) in self.discovered_tags.clone().iter() {
                match self.publish_message(self.device_detach_topic(&tag), "{}".to_string()) {
                    Ok(_) => info!("Discovered Ruuvi tag ({}) detached from gateway succesfully.", tag.to_string(MacAddressFormat::Canonical).to_uppercase()),
                    Err(error) => warn!("Discovered Ruuvi tag ({}) detachment from gateway failed: {}", 
                        tag.to_string(MacAddressFormat::Canonical).to_uppercase(),
                        error)
                }
            }
        }
    }

    fn device_event_topic(&self, address: &MacAddress) -> Option<String> {
        trace!("in device_event_topic");
        let mut retval: Option<String> = None;
        if let Some(collectconfig) = &self.collectconfig {
            retval = match &collectconfig.event_subfolder {
                Some(folder) => Some(format!("/devices/{}/events/{}", address.to_string(MacAddressFormat::Canonical).to_uppercase(), folder)),
                None => Some(format!("/devices/{}/events", address.to_string(MacAddressFormat::Canonical).to_uppercase()))
            }
        }
        retval
    }

    fn device_attach_topic(&self, address: &MacAddress) -> String {
        let topic = format!("/devices/{}/attach", address.to_string(MacAddressFormat::Canonical).to_uppercase());
        debug!("device attach topic: {}", topic);
        topic
    }

    fn device_detach_topic(&self, address: &MacAddress) -> String {
        let topic = format!("/devices/{}/detach", address.to_string(MacAddressFormat::Canonical).to_uppercase());
        debug!("device detach topic: {}", topic);
        topic
    }

    pub fn build(appconfig: &AppConfig, r: &channel::Receiver<RuuviBluetoothBeacon>, cnc_s: &channel::Sender<IOTCoreCNCMessageKind>) -> Result<IotCoreClient, Report> {
        trace!("in build");
        let create_opts = mqtt::CreateOptionsBuilder::new()
            .client_id(appconfig.iotcore.client_id())
            .mqtt_version(mqtt::types::MQTT_VERSION_3_1_1)
            .server_uri("ssl://mqtt.googleapis.com:8883")
            .persistence(mqtt::PersistenceType::None)
            .finalize();

        let mut cli = match mqtt::Client::new(create_opts) {
            Ok(cli) => cli,
            Err(error) => return Err(
                eyre!("Unable to create Paho MQTT client instance")
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };
        cli.set_timeout(Duration::from_secs(5));

        let mut ssl_options_builder = mqtt::SslOptionsBuilder::new();
        ssl_options_builder.ssl_version(mqtt::SslVersion::Tls_1_2);
        if appconfig.identity.ca_certs.is_some() {
            match ssl_options_builder.trust_store(appconfig.identity.ca_certs.as_ref().unwrap()) {
                Ok(options_builder) => options_builder,
                Err(error) => return Err(
                    eyre!("Unable to use CA certificates in mqtt client")
                        .with_section(move || error.to_string().header("Reason:"))
                    )
            };    
        }
        match ssl_options_builder.key_store(&appconfig.identity.public_key) {
            Ok(options_builder) => options_builder,
            Err(error) => return Err(
                eyre!("Unable to use public key in mqtt client")
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };
        match ssl_options_builder.private_key(&appconfig.identity.private_key) {
            Ok(options_builder) => options_builder,
            Err(error) => return Err(
                eyre!("Unable to use private key in mqtt client")
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };
        let ssl_options = ssl_options_builder.finalize();

        let jwt_factory = IotCoreAuthToken::build(appconfig);
        let jwt_token = match jwt_factory.issue_new() {
            Ok(token) => token,
            Err(error) => return Err(
                eyre!("Unable to issue original JWT token")
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };

        let conn_opts = mqtt::ConnectOptionsBuilder::new()
            .user_name("not_used")
            .password(jwt_token)
            .ssl_options(ssl_options.clone())
            .keep_alive_interval(Duration::from_secs(5*60))
            .finalize();

        // thru mspc relay incoming messages from cnc topics
        let consumer = cli.start_consuming();

        let device_id = appconfig.iotcore.device_id.clone();

        Ok(IotCoreClient {
            ssl_opts: ssl_options,
            conn_opts: conn_opts,
            client: cli,
            jwt_factory: jwt_factory,
            channel_receiver: r.clone(),
            cnc_sender: cnc_s.clone(),
            config_topic: format!("/devices/{}/config", device_id),
            state_topic: format!("/devices/{}/state", device_id),
            command_topic_root: format!("/devices/{}/commands", device_id),
            consumer: consumer,
            collectconfig: None,
            last_pause: None,
            last_seen: Instant::now(),
            discovered_tags: HashMap::new(),
        })
    }
}

// eof
