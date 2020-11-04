use color_eyre::{eyre::eyre, SectionExt, Section, eyre::Report};
use crossbeam::channel;
use rumqttc::{MqttOptions, Client, QoS};

use crate::lib::config::AppConfig;
use crate::lib::scanner::RuuviBluetoothBeacon;
use crate::lib::jwt::IotCoreAuthToken;

pub struct IotCoreClient {
    channel_receiver: channel::Receiver<RuuviBluetoothBeacon>,
    jwt_factory: IotCoreAuthToken,
    mqttoptions: MqttOptions,
    event_topic: String,
    config_topic: String,
}

impl IotCoreClient {
    pub fn start_client(&self) -> Result<(), Report> {
        let (mut client, mut connection) = Client::new(self.mqttoptions.clone(), 10);
        
        client.subscribe(&self.config_topic, QoS::AtMostOnce).unwrap();

        let running = true;
        while running {
            match self.channel_receiver.recv() {
                Ok(msg) => {
                    match client.publish(&self.event_topic, QoS::AtMostOnce, false, msg.data.to_string()) {
                        Ok(_) => {
                            debug!("Sent to MQTT: {} {} {}", msg.timestamp, msg.address, msg.data);
                        },
                        Err(error) => return Err(
                            eyre!("Error while publishing to MQTT")
                                .with_section(move || error.to_string().header("Reason:"))
                            )
                    };
                    
                },
                Err(error) => {
                    trace!("No bluetooth beacon in channel: {}", error);
                }
            };
        }
        
        Ok(())
    }

    pub fn build(config: &AppConfig, r: &channel::Receiver<RuuviBluetoothBeacon>) -> Result<IotCoreClient, Report> {
        let jwt_factory = IotCoreAuthToken::build(config);
        let token = match jwt_factory.issue_new() {
            Ok(token) => token,
            Err(error) => return Err(
                eyre!("Unable to issue original JWT token")
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };

        let (host, port) = config.iotcore.mqtt_bridge();
        let mut mqttoptions = MqttOptions::new(config.iotcore.client_id(), host, port);
        mqttoptions.set_keep_alive(5)
            .set_client_auth(config.identity.certificate_from_pem_file()?, config.identity.key_from_pem_file()?)
            .set_credentials(&config.iotcore.device_id, &token);

        Ok(IotCoreClient {
            jwt_factory: jwt_factory,
            channel_receiver: r.clone(),
            mqttoptions: mqttoptions,
            event_topic: format!("devices/{}/events", config.iotcore.device_id),
            config_topic: format!("devices/{}/config", config.iotcore.device_id)
        })
    }
}

// eof
