use serde::Deserialize;
use std::path::Path;
use std::time::Duration;
use color_eyre::{eyre::eyre, SectionExt, Section, eyre::Report};
use crossbeam::channel;
use paho_mqtt as mqtt;
use std::{time, thread};
use std::sync::mpsc::Receiver;
use std::clone::Clone;

use crate::lib::config::AppConfig;
use crate::lib::scanner::RuuviBluetoothBeacon;
use crate::lib::jwt::IotCoreAuthToken;

static READY_MESSAGE: &str = "{state: \"RUNNING\"}";
static STOP_MESSAGE: &str = "{state: \"STOPPING\"}";

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

#[derive(Debug,Deserialize)]
enum CollectMode {
    #[serde(rename = "blacklist")]
    BLACKLIST,
    #[serde(rename = "whitelist")]
    WHITELIST
}

#[derive(Debug,Deserialize)]
struct CollectConfig {
    mode: CollectMode,
    addresses: Vec<String>
}

pub struct IotCoreClient {
    ssl_opts: mqtt::SslOptions,
    conn_opts: mqtt::ConnectOptions,
    client: mqtt::Client,
    channel_receiver: channel::Receiver<RuuviBluetoothBeacon>,
    cnc_sender: channel::Sender<CNCCommandMessage>,
    jwt_factory: IotCoreAuthToken,
    events_topic: String,
    config_topic: String,
    state_topic: String,
    command_topic_root: String,
    consumer: Receiver<Option<mqtt::message::Message>>,
    collectconfig: Option<CollectConfig>,
    collecting: bool,
    collection_size: usize
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
            Err(error) => Err(
                eyre!("Error while disconnecting MQTT broker")
                    .with_section(move || error.to_string().header("Reason:"))
                )
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

    pub fn start_client(&mut self) -> Result<(), Report> {

        // cycle connection state
        if self.client.is_connected() {
            self.disconnect()?;
        }
        self.connect()?;
        
        self.publish_message(self.state_topic.to_string(), READY_MESSAGE.as_bytes().to_vec())?; // if this fails our connection is dead anyway

        let mut message_queue: Vec<RuuviBluetoothBeacon> = Vec::new();

        // loop messages and wait for a ready signal
        loop {
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
                            info!("New collect config activated: {:?}", self.collectconfig);
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
                            if let Some(command) = command {
                                // also publish the command to CNC channel
                                self.cnc_sender.send(command.clone()).unwrap(); // TODO: fix unwrap
                                // react locally to the message as well
                                match command.command {
                                    CNCCommand::COLLECT => {
                                        self.collecting = true;
                                        info!("CNC command received: COLLECT beacons");
                                    },
                                    CNCCommand::PAUSE => {
                                        self.collecting = false;
                                        warn!("CNC command received: PAUSE collecting beacons");
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
                Err(error) => {
                    trace!("No incoming cnc messages in topic consumer: {}", error);
                }
            };

            // check into the channel to see if there are beacons to relay to the mqtt broker
            match self.channel_receiver.try_recv() {
                Ok(msg) => {
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
                    // submit the beacon to iotcore
                    if publish && self.collecting {
                        if message_queue.len() >= self.collection_size {
                            match self.publish_message(self.events_topic.to_string(), json!(message_queue).to_string().as_bytes().to_vec()) {
                                Ok(_) => trace!("iotcore publish: {:?}", message_queue),
                                Err(error) => error!("Error on publishing message queue to MQTT: '{}'. Will retry.", error)
                            };
                            message_queue = Vec::new();
                        } else {
                            message_queue.push(msg);
                        }
                    }
                    trace!("Message queue size: {}/{}", message_queue.len(), self.collection_size);
                },
                Err(error) => {
                    trace!("No bluetooth beacon in channel: {}", error);
                }
            };

            // sleep for a while to reduce amount of CPU burn and idle for a while
            thread::sleep(time::Duration::from_millis(10));
        }
        
        self.publish_message(self.state_topic.to_string(), STOP_MESSAGE.as_bytes().to_vec())?; // doesnt matter if we throw error here as we are gointo die anyway

        self.disconnect()?;
        
        Ok(())
    }

    pub fn build(config: &AppConfig, r: &channel::Receiver<RuuviBluetoothBeacon>, cnc_s: &channel::Sender<CNCCommandMessage>) -> Result<IotCoreClient, Report> {

        let create_opts = mqtt::CreateOptionsBuilder::new()
            .client_id(config.iotcore.client_id())
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

        let ssl_options = match mqtt::SslOptionsBuilder::new()
            .ssl_version(mqtt::SslVersion::Tls_1_2)
            .trust_store(Path::new(&config.identity.ca_certs).to_path_buf()) {
                Ok(options) => options.finalize(),
                Err(error) => return Err(
                    eyre!("Unable to instantiate Paho MQTT clients SSL options")
                        .with_section(move || error.to_string().header("Reason:"))
                    )
            };

        let jwt_factory = IotCoreAuthToken::build(config);
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

        // if we have subfolder defined for our telemetry use that in the events topic name
        let events_topic = match &config.iotcore.event_subfolder {
            Some(subfolder) => format!("/devices/{}/events/{}", config.iotcore.device_id, subfolder),
            None => format!("/devices/{}/events", config.iotcore.device_id)
        };

        Ok(IotCoreClient {
            ssl_opts: ssl_options,
            conn_opts: conn_opts,
            client: cli,
            jwt_factory: jwt_factory,
            channel_receiver: r.clone(),
            cnc_sender: cnc_s.clone(),
            events_topic: events_topic,
            config_topic: format!("/devices/{}/config", config.iotcore.device_id),
            state_topic: format!("/devices/{}/state", config.iotcore.device_id),
            command_topic_root: format!("/devices/{}/commands", config.iotcore.device_id),
            consumer: consumer,
            collectconfig: None,
            collecting: true,
            collection_size: config.iotcore.collection_size()
        })
    }
}

// eof
