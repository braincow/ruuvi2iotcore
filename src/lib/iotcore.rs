use serde::{Serialize, Deserialize};
use serde::de::{self, Deserializer, Visitor, SeqAccess, MapAccess};
use serde::ser::{Serializer, SerializeStruct};
use color_eyre::{eyre::eyre, SectionExt, Section, eyre::Report};
use crossbeam::channel;
use paho_mqtt as mqtt;
use eui48::{MacAddress, MacAddressFormat};
use std::time::{Instant, Duration};
use std::{time, thread};
use std::sync::mpsc::Receiver;
use std::clone::Clone;
use std::fmt;
use std::str::FromStr;
use std::collections::HashMap;

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

#[derive(Debug,Clone,PartialEq,PartialOrd)]
pub struct RuuviTag {
    device_id: String,
    address: MacAddress,
}
impl RuuviTag {
    pub fn addr_as_hex_string(&self) -> String {
        self.address.to_string(MacAddressFormat::HexString)
    }
}
impl Serialize for RuuviTag {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("RuuviTag", 2)?;
        state.serialize_field("name", &self.device_id)?;
        state.serialize_field("address", &self.addr_as_hex_string())?;
        state.end()
    }
}
impl<'de> Deserialize<'de> for RuuviTag {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field { 
            #[serde(alias = "device_id")]
            DeviceId,
            Address
        };

        struct RuuviTagVisitor;

        impl<'de> Visitor<'de> for RuuviTagVisitor {
            type Value = RuuviTag;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct RuuviTag")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<RuuviTag, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let device_id = seq.next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let address = seq.next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                Ok(RuuviTag{
                    device_id: device_id,
                    address: MacAddress::from_str(address).unwrap() //TODO: fix unwrap https://serde.rs/deserialize-struct.html
                })
            }

            fn visit_map<V>(self, mut map: V) -> Result<RuuviTag, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut device_id = None;
                let mut address = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::DeviceId => {
                            if device_id.is_some() {
                                return Err(de::Error::duplicate_field("name"));
                            }
                            device_id = Some(map.next_value()?);
                        }
                        Field::Address => {
                            if address.is_some() {
                                return Err(de::Error::duplicate_field("address"));
                            }
                            address = Some(map.next_value()?);
                        }
                    }
                }
                let device_id = device_id.ok_or_else(|| de::Error::missing_field("name"))?;
                let address = address.ok_or_else(|| de::Error::missing_field("address"))?;
                Ok(RuuviTag{
                    device_id: device_id,
                    address: MacAddress::from_str(address).unwrap() //TODO: fix unwrap https://serde.rs/deserialize-struct.html
                })
            }
        }

        const FIELDS: &'static [&'static str] = &["name", "address"];
        deserializer.deserialize_struct("RuuviTag", FIELDS, RuuviTagVisitor)
    }
}

#[derive(Debug,Deserialize,Serialize,Clone,PartialEq,PartialOrd)]
pub struct CollectConfig {
    tags: Vec<RuuviTag>,
    collecting: bool,
    event_subfolder: Option<String>,
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

            Ok(match self.client.publish(mqtt_msg) {
                Ok(retval) => retval,
                Err(error) => return Err(
                    eyre!("Error while publishing to MQTT")
                        .with_section(move || error.to_string().header("Reason:"))
                    )
            })
    }

    fn disconnect(&mut self) -> Result<(), Report> {
        if self.client.is_connected() {
            warn!("Disconnecting from MQTT broker");
            // send deassociation control messages for all the devices under our "control"
            self.detach_tags()?;
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

        // send association control messages for all the devices under our "control"
        self.attach_tags()?;

        Ok(())
    }

    fn set_collecting_state(&mut self, enabled: bool) -> Result<(), Report> {
        if let Some(collectconfig) = &self.collectconfig {
            let mut newconfig = collectconfig.clone();
            newconfig.collecting = enabled;
            self.collectconfig = Some(newconfig);
            self.publish_message(self.state_topic.clone(), serde_json::to_vec(&self.collectconfig).unwrap())?;
        } else {
            error!("No collect config defined. Unable to change collect state to: {}", enabled);
        }

        Ok(())
    }

    fn enable_collecting(&mut self) -> Result<(), Report> {
        let retval = self.set_collecting_state(true);
        if retval.is_ok() {
            self.last_pause = None;
        }
        retval
    }

    fn disable_collecting(&mut self) -> Result<(), Report> {
        let retval = self.set_collecting_state(false);
        if retval.is_ok() {
            self.last_pause = Some(Instant::now());
        }
        retval
    }

    fn attach_tags(&mut self) -> Result<(), Report> {
        let tags = match self.collectconfig.as_ref() {
            Some(config) => config.tags.clone(),
            None => Vec::new()
        };
        for tag in tags.iter() {
            match self.publish_message(format!("/devices/{}/attach", tag.device_id), "{}".as_bytes().to_vec()) {
                Ok(_) => info!("Associated Ruuvi tag: {} ({})", tag.device_id, tag.addr_as_hex_string()),
                Err(error) => error!("Error while associating tag {} ({}): {}", tag.device_id, tag.addr_as_hex_string(), error)
            };
        }

        Ok(())
    }

    fn detach_tags(&mut self) -> Result<(), Report> {
        let tags = match self.collectconfig.as_ref() {
            Some(config) => config.tags.clone(),
            None => Vec::new()
        };
        for tag in tags.iter() {
            self.publish_message(format!("/devices/{}/detach", tag.device_id), "{}".as_bytes().to_vec())?;
            debug!("De-associated Ruuvi tag: {} ({})", tag.device_id, tag.addr_as_hex_string());
        }

        Ok(())
    }

    pub fn start_client(&mut self) -> Result<bool, Report> {

        // cycle connection state
        if self.client.is_connected() {
            trace!("Entering to start_client() from unclean restart.");
            self.disconnect()?;
        }
        self.connect()?;
        
        let mut message_queue: HashMap<MacAddress, Vec<RuuviBluetoothBeacon>> = HashMap::new();

        self.last_seen = Instant::now();

        // loop messages and wait for a ready signal
        loop {
            // check that we are actually doing work, and if not then issue a restart
            //  we have 60 seconds here to facilitate possible restart of the bluetooth stack first
            if self.last_seen.elapsed() >= Duration::from_secs(58) {
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
                                // send association control messages for all the devices under our "control"
                                self.attach_tags()?;
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
                    self.last_seen = Instant::now();

                    let address = MacAddress::from_str(&msg.address).unwrap();
                    let topic = self.device_event_topic(&address);

                    // submit the beacon to iotcore if collecting them is enabled
                    if let Some(topic) = topic {
                        let mut queue: Vec<RuuviBluetoothBeacon> = match message_queue.get(&address) {
                            Some(queue) => queue.to_vec(),
                            None => vec![]
                        };
    
                        if self.collectconfig.as_ref().unwrap().collecting {
                            trace!("Message queue size for '{}': {}/{}", address, queue.len(), self.collectconfig.as_ref().unwrap().collection_size());
                            if &self.collectconfig.as_ref().unwrap().collection_size() <= &1 {
                                match self.publish_message(topic, serde_json::to_vec(&msg).unwrap()) {
                                    Ok(_) => trace!("iotcore publish message: {:?}", msg),
                                    Err(error) => error!("Error on publishing message to MQTT: '{}'. Will retry.", error)
                                };
                            } else if queue.len() >= self.collectconfig.as_ref().unwrap().collection_size() {
                                match self.publish_message(topic, serde_json::to_vec(&queue).unwrap()) {
                                    Ok(_) => trace!("iotcore publish message queue: {:?}", queue),
                                    Err(error) => error!("Error on publishing message queue to MQTT: '{}'. Will retry.", error)
                                };
                                // empty the message queue
                                message_queue.remove(&address);
                            } else {
                                // replace in hashmap the message queue with new extended one
                                queue.push(msg);
                                message_queue.insert(address, queue.to_vec());
                            }
                        } else {
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
                    }
                },
                Err(_) => {}
            };

            // sleep for a while to reduce amount of CPU burn and idle for a while
            thread::sleep(time::Duration::from_millis(10));
        }
        
        self.disconnect()?;
        
        Ok(true)
    }

    fn device_event_topic(&self, address: &MacAddress) -> Option<String> {
        let tags = self.collectconfig.as_ref().unwrap().tags.clone();
        for tag in tags.iter() {
            if &tag.address == address {
                return match &self.collectconfig.as_ref().unwrap().event_subfolder {
                    Some(subfolder) => Some(format!("/devices/{}/events/{}", tag.device_id, subfolder)),
                    None => Some(format!("/devices/{}/events", tag.device_id))
                };
            }
        }
        None
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
        })
    }
}

// eof
