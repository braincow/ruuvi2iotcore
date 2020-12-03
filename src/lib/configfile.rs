use std::{fs, path::Path};
use serde::{Serialize, Deserialize};
use color_eyre::{eyre::eyre, SectionExt, Section, eyre::Report};

fn read_pem(path: &Path) -> Result<Vec<u8>, Report> {
    match fs::read_to_string(path) {
        Ok(pem) => match base64::decode(pem) {
            Ok(der_bytes) => return Ok(der_bytes),
            Err(error) => return Err(
                eyre!("Unable to decode certifiates file.")
                    .with_section(move || path.to_string_lossy().trim().to_string().header("File name:"))
                    .with_section(move || error.to_string().header("Reason:"))
                )
        },
        Err(error) => return Err(
            eyre!("Unable to read certifiates file")
                .with_section(move || path.to_string_lossy().trim().to_string().header("File name:"))
                .with_section(move || error.to_string().header("Reason:"))
            )
    }
}

#[derive(Debug,Deserialize,Serialize)]
pub struct IdentityConfig {
    pub public_key: String,
    pub private_key: String,
    pub ca_certs: Option<String>,
    token_lifetime: Option<u64>
}

impl IdentityConfig {
    pub fn token_lifetime(&self) -> u64 {
        trace!("in token_lifetime");
        if self.token_lifetime.is_none() {
            return 3600
        }

        self.token_lifetime.unwrap()
    }

    pub fn ca_as_vec(&self) -> Result<Option<Vec<u8>>, Report> {
        if let Some(ca_certs) = self.ca_certs.clone() {
            Ok(Some(read_pem(Path::new(&ca_certs))?))
        } else {
            Ok(None)
        }
    }

    pub fn cert_as_vec(&self) -> Result<Vec<u8>, Report> {
        Ok(read_pem(Path::new(&self.public_key))?)
    }

    pub fn key_as_vec(&self) -> Result<Vec<u8>, Report> {
        Ok(read_pem(Path::new(&self.private_key))?)
    }
}

#[derive(Debug,Deserialize,Serialize)]
pub struct IotCoreConfig {
    pub device_id: String,
    pub project_id: String,
    pub region: String,
    pub registry: String
}

impl IotCoreConfig {
    pub fn client_id(&self) -> String {
        trace!("in client_id");
        let client_id = format!("projects/{}/locations/{}/registries/{}/devices/{}",
            self.project_id,
            self.region,
            self.registry,
            self.device_id);
        debug!("client_id is '{}'", client_id);
        client_id
    }
}

#[derive(Debug,Deserialize,Serialize)]
pub struct AppConfig {
    pub identity: IdentityConfig,
    pub iotcore: IotCoreConfig
}

impl AppConfig {
    pub fn read_config(config_file_path: &Path) -> Result<AppConfig, Report> {
        trace!("in read_config");
        let config_yaml = match fs::read_to_string(config_file_path) {
            Ok(yaml) => yaml,
            Err(error) => return Err(
                eyre!("Unable to read config file")
                    .with_section(move || config_file_path.to_string_lossy().trim().to_string().header("File name:"))
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };
        let config: AppConfig = match serde_yaml::from_str(&config_yaml) {
            Ok(config) => config,
            Err(error) => return Err(
                eyre!("Unable to parse config file")
                    .with_section(move || config_file_path.to_string_lossy().trim().to_string().header("File name:"))
                    .with_section(move || error.to_string().header("Reason:")) 
                )
        };
        debug!("application configuration is: {:?}", config);
    
        Ok(config)
    }
}

// eof
