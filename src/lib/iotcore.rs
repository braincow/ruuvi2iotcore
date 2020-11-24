use serde::{Serialize, Deserialize};
use std::time::{Instant, Duration};
use color_eyre::{eyre::eyre, SectionExt, Section, eyre::Report};
use crossbeam::channel;
use paho_mqtt as mqtt;
use std::{time, thread};
use std::sync::mpsc::Receiver;
use std::clone::Clone;

use crate::lib::configfile::AppConfig;
use crate::lib::scanner::RuuviBluetoothBeacon;
use crate::lib::jwt::IotCoreAuthToken;

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
    SHUTDOWN
}

#[derive(Debug,Deserialize, Clone)]
pub struct CNCCommandMessage {
    pub command: CNCCommand
}

#[derive(Debug,Deserialize,Serialize,Clone)]
enum CollectMode {
    #[serde(rename = "blacklist")]
    BLACKLIST,
    #[serde(rename = "whitelist")]
    WHITELIST
}

#[derive(Debug,Deserialize,Serialize,Clone)]
pub struct BluetoothConfig {
    pub adapter_index: usize
}

#[derive(Debug,Deserialize,Serialize,Clone)]
pub struct CollectConfig {
    mode: CollectMode,
    addresses: Vec<String>,
    collecting: bool,
    event_subfolder: Option<String>,
    collection_size: Option<usize>,
    pub bluetooth: BluetoothConfig
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
    device_id: String,
    ssl_opts: mqtt::SslOptions,
    conn_opts: mqtt::ConnectOptions,
    client: mqtt::Client,
    channel_receiver: channel::Receiver<RuuviBluetoothBeacon>,
    cnc_sender: channel::Sender<IOTCoreCNCMessageKind>,
    jwt_factory: IotCoreAuthToken,
    events_topic: Option<String>,
    config_topic: String,
    state_topic: String,
    command_topic_root: String,
    consumer: Receiver<Option<mqtt::message::Message>>,
    collectconfig: Option<CollectConfig>,
}

impl IotCoreClient {
    fn publish_message(&mut self, topic: String, msg: Vec<u8>) -> Result<(), Report> {
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

            match self.client.publish(mqtt_msg) {
                Ok(_) => {},
                Err(error) => return Err(
                    eyre!("Error while publishing to MQTT")
                        .with_section(move || error.to_string().header("Reason:"))
                    )
            };

            Ok(())
    }

    fn disconnect(&self) -> Result<(), Report> {
        warn!("Disconnecting from MQTT broker");
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

    fn connect(&self) -> Result<(), Report> {
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

        Ok(())
    }

    fn enable_collecting(&mut self, enabled: bool) -> Result<(), Report> {
        if let Some(collectconfig) = &self.collectconfig {
            let mut newconfig = collectconfig.clone();
            newconfig.collecting = enabled;
            self.collectconfig = Some(newconfig);
            self.publish_message(self.state_topic.clone(), json!(&self.collectconfig).to_string().as_bytes().to_vec())?;
        }

        Ok(())
    }

    pub fn start_client(&mut self) -> Result<bool, Report> {

        // cycle connection state
        if self.client.is_connected() {
            self.disconnect()?;
        }
        self.connect()?;
        
        let mut message_queue: Vec<RuuviBluetoothBeacon> = Vec::new();

        let mut last_seen = Instant::now();

        // loop messages and wait for a ready signal
        loop {
            // check that we are actually doing work, and if not then issue a restart
            //  we have 60 seconds here to facilitate possible restart of the bluetooth stack first
            if last_seen.elapsed() >= Duration::from_secs(58) {
                warn!("No beacons detected for 58 seconds. Issuing thread clean restart.");
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
                        trace!("{:?}", msg);

                        if msg.topic() == self.config_topic {
                            // we received new config, decode it
                            self.collectconfig = match serde_json::from_str(&msg.payload_str()) {
                                Ok(config) => Some(config),
                                Err(error) => { 
                                    error!("Unable to parse new collect config: {}", error);
                                    None
                                }
                            };
                            if self.collectconfig.is_some() {
                                let events_topic = match &self.collectconfig.as_ref().unwrap().event_subfolder {
                                    Some(subfolder) => format!("/devices/{}/events/{}", self.device_id, subfolder),
                                    None => format!("/devices/{}/events", self.device_id)
                                };
                                self.events_topic = Some(events_topic);
                                // send new state back after activating the configuration
                                self.publish_message(self.state_topic.clone(), json!(&self.collectconfig).to_string().as_bytes().to_vec())?;
                                // send config to CNC channel
                                self.cnc_sender.send(IOTCoreCNCMessageKind::CONFIG(self.collectconfig.clone())).unwrap(); // TODO: fix unwrap    
                                debug!("New collect config activated is '{:?}'", self.collectconfig);
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
                                        self.enable_collecting(true)?;
                                    },
                                    CNCCommand::PAUSE => {
                                        warn!("CNC command received: PAUSE collecting beacons");
                                        self.enable_collecting(false)?;
                                    },
                                    CNCCommand::SHUTDOWN => {
                                        warn!("CNC command received: SHUTDOWN software");
                                        break;
                                    },
                                };
                            }
                        } else {
                            warn!("Unimplemented CNC topic in received message.");
                        }
                    }
                },
                Err(_) => {}
            };

            // check into the channel to see if there are beacons to relay to the mqtt broker
            match self.channel_receiver.try_recv() {
                Ok(msg) => {
                    // update the last_seen counter to verify internally that we are doing work
                    last_seen = Instant::now();
                    // check against collectconfig if this beacon shall be submitted
                    let publish = match &self.collectconfig {
                        Some(collectconfig) => {
                            match collectconfig.mode {
                                CollectMode::BLACKLIST => {
                                    if collectconfig.addresses.contains(&msg.address) {
                                        false
                                    } else {
                                        true
                                    }
                                },
                                CollectMode::WHITELIST => {
                                    if collectconfig.addresses.contains(&msg.address) {
                                        true
                                    } else {
                                        false
                                    }
                                }
                            }
                        },
                        None => true
                    };
                    let mut collect = false;
                    if let Some(collectconfig) = &self.collectconfig {
                        collect = collectconfig.collecting;
                    }
                    // submit the beacon to iotcore
                    if publish && collect && self.collectconfig.is_some() {
                        if &self.collectconfig.as_ref().unwrap().collection_size() <= &1 {
                            match self.publish_message(self.events_topic.as_ref().unwrap().to_string(), json!(msg).to_string().as_bytes().to_vec()) {
                                Ok(_) => trace!("iotcore publish message: {:?}", message_queue),
                                Err(error) => error!("Error on publishing message to MQTT: '{}'. Will retry.", error)
                            };
                        } else if message_queue.len() >= self.collectconfig.as_ref().unwrap().collection_size() {
                            match self.publish_message(self.events_topic.as_ref().unwrap().to_string(), json!(message_queue).to_string().as_bytes().to_vec()) {
                                Ok(_) => trace!("iotcore publish message queue: {:?}", message_queue),
                                Err(error) => error!("Error on publishing message queue to MQTT: '{}'. Will retry.", error)
                            };
                            message_queue = Vec::new();
                        } else {
                            message_queue.push(msg);
                        }
                    }
                    trace!("Message queue size: {}/{}", message_queue.len(), self.collectconfig.as_ref().unwrap().collection_size());
                },
                Err(_) => {}
            };

            // sleep for a while to reduce amount of CPU burn and idle for a while
            thread::sleep(time::Duration::from_millis(10));
        }
        
        self.disconnect()?;
        
        Ok(false)
    }

    pub fn build(appconfig: &AppConfig, r: &channel::Receiver<RuuviBluetoothBeacon>, cnc_s: &channel::Sender<IOTCoreCNCMessageKind>) -> Result<IotCoreClient, Report> {

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
        match ssl_options_builder.trust_store(&appconfig.identity.ca_certs) {
            Ok(options_builder) => options_builder,
            Err(error) => return Err(
                eyre!("Unable to use CA certificates in mqtt client")
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };
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
            .finalize();

        // thru mspc relay incoming messages from cnc topics
        let consumer = cli.start_consuming();

        let device_id = appconfig.iotcore.device_id.clone();

        Ok(IotCoreClient {
            device_id: device_id.clone(),
            ssl_opts: ssl_options,
            conn_opts: conn_opts,
            client: cli,
            jwt_factory: jwt_factory,
            channel_receiver: r.clone(),
            cnc_sender: cnc_s.clone(),
            events_topic: None,
            config_topic: format!("/devices/{}/config", device_id),
            state_topic: format!("/devices/{}/state", device_id),
            command_topic_root: format!("/devices/{}/commands", device_id),
            consumer: consumer,
            collectconfig: None,
        })
    }
}

// eof
