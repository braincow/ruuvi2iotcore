use std::path::Path;
use std::time::Duration;
use color_eyre::{eyre::eyre, SectionExt, Section, eyre::Report};
use crossbeam::channel;
use paho_mqtt as mqtt;

use crate::lib::config::AppConfig;
use crate::lib::scanner::RuuviBluetoothBeacon;
use crate::lib::jwt::IotCoreAuthToken;

static READY_MESSAGE: &str = "{state: \"RUNNING\"}";
static STOP_MESSAGE: &str = "{state: \"STOPPING\"}";

pub struct IotCoreClient {
    ssl_opts: mqtt::SslOptions,
    conn_opts: mqtt::ConnectOptions,
    client: mqtt::Client,
    channel_receiver: channel::Receiver<RuuviBluetoothBeacon>,
    jwt_factory: IotCoreAuthToken,
    events_topic: String,
    //config_topic: String,
    state_topic: String,
    //command_topic: String
}

impl IotCoreClient {
    fn publish_message(&mut self, topic: String, msg: Vec<u8>) -> Result<(), Report> {
        // fullfill IoT Core's odd JWT based authentication
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

        Ok(())
    }

    pub fn start_client(&mut self) -> Result<(), Report> {

        self.connect()?;
        
        self.publish_message(self.state_topic.to_string(), READY_MESSAGE.as_bytes().to_vec())?;

        // loop messages and wait for a ready signal
        let running = true;
        while running {
            match self.channel_receiver.recv() {
                Ok(msg) => {
                    self.publish_message(self.events_topic.to_string(), json!(msg).to_string().as_bytes().to_vec())?;
                },
                Err(error) => {
                    trace!("No bluetooth beacon in channel: {}", error);
                }
            };
        }

        self.publish_message(self.state_topic.to_string(), STOP_MESSAGE.as_bytes().to_vec())?;
        
        Ok(())
    }

    pub fn build(config: &AppConfig, r: &channel::Receiver<RuuviBluetoothBeacon>) -> Result<IotCoreClient, Report> {

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

        Ok(IotCoreClient {
            ssl_opts: ssl_options,
            conn_opts: conn_opts,
            client: cli,
            jwt_factory: jwt_factory,
            channel_receiver: r.clone(),
            events_topic: format!("/devices/{}/events", config.iotcore.device_id),
            //config_topic: format!("/devices/{}/config", config.iotcore.device_id),
            state_topic: format!("/devices/{}/state", config.iotcore.device_id),
            //command_topic: format!("/devices/{}/commands/#", config.iotcore.device_id)
        })
    }
}

// eof
