use std::{fs, path::Path};
use serde::{Serialize, Deserialize};
use color_eyre::{eyre::eyre, SectionExt, Section, eyre::Report};

#[derive(Debug,Deserialize,Serialize)]
pub struct IdentityConfig {
    pub public_key: String,
    pub private_key: String,
    token_lifetime: Option<u64>
}

impl IdentityConfig {
    pub fn token_lifetime(&self) -> u64 {
        if self.token_lifetime.is_none() {
            return 3600
        }

        self.token_lifetime.unwrap()
    }

    pub fn certificate_from_pem_file(&self) -> Result<Vec<u8>, Report> {
        let certfile = match fs::read_to_string(&self.public_key) {
            Ok(certfile) => certfile,
            Err(error) => return Err(
                eyre!("Unable to read PEM certificate file")
                    .with_section(move || self.public_key.to_string().header("File name:"))
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };
        let parsed_pem = match pem::parse(certfile) {
            Ok(parsed_pem) => parsed_pem,
            Err(error) => return Err(
                eyre!("Unable to decode PEM certificate file")
                    .with_section(move || self.public_key.to_string().header("File name:"))
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };

        Ok(parsed_pem.contents)
    }

    pub fn key_from_pem_file(&self) -> Result<Vec<u8>, Report> {
        let certfile = match fs::read_to_string(&self.private_key) {
            Ok(certfile) => certfile,
            Err(error) => return Err(
                eyre!("Unable to read PEM key file")
                    .with_section(move || self.private_key.to_string().header("File name:"))
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };
        let parsed_pem = match pem::parse(certfile) {
            Ok(parsed_pem) => parsed_pem,
            Err(error) => return Err(
                eyre!("Unable to decode PEM key file")
                    .with_section(move || self.private_key.to_string().header("File name:"))
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };

        Ok(parsed_pem.contents)
    }
}

#[derive(Debug,Deserialize,Serialize)]
pub struct BluetoothConfig {
    adapter: Option<usize>,
}

impl BluetoothConfig {
    pub fn adapter_index(&self) -> usize {
        if self.adapter.is_some() {
            return self.adapter.unwrap()
        }
        // default value will be 0
        0
    }
}

#[derive(Debug,Deserialize,Serialize)]
pub struct IotCoreConfig {
    hostname: Option<String>,
    port: Option<u16>,
    pub project_id: String,
    region: String,
    pub registry: String,
    pub device_id: String
}

impl IotCoreConfig {

    pub fn mqtt_bridge(&self) -> (String, u16) {
        let mut srv = String::from("mqtt.googleapis.com");
        let mut prt = 8883;

        if let Some(hostname) = &self.hostname {
            srv = hostname.to_string();
        }

        if let Some(port) = &self.port {
            prt = *port
        }

        (srv, prt)
    }

    pub fn client_id(&self) -> String {
        let client_id = format!("projects/{}/locations/{}/registries/{}/devices/{}",
            self.project_id,
            self.region,
            self.registry,
            self.device_id);
        client_id
    }
}

#[derive(Debug,Deserialize,Serialize)]
pub struct AppConfig {
    pub identity: IdentityConfig,
    pub bluetooth: BluetoothConfig,
    pub iotcore: IotCoreConfig
}

impl AppConfig {
    pub fn read_config(config_file_path: &Path) -> Result<AppConfig, Report> {
        let config_toml = match fs::read_to_string(config_file_path) {
            Ok(toml) => toml,
            Err(error) => return Err(
                eyre!("Unable to read config file")
                    .with_section(move || config_file_path.to_string_lossy().trim().to_string().header("File name:"))
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };
        let config: AppConfig = match toml::from_str(&config_toml) {
            Ok(config) => config,
            Err(error) => return Err(
                eyre!("Unable to parse config file")
                    .with_section(move || config_file_path.to_string_lossy().trim().to_string().header("File name:"))
                    .with_section(move || error.to_string().header("Reason:")) 
                )
        };
    
        Ok(config)
    }
}

// eof
