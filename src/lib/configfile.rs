use std::{fs, path::Path};
use serde::{Serialize, Deserialize};
use color_eyre::{eyre::eyre, SectionExt, Section, eyre::Report};
use std::io::Cursor;
use x509_parser::pem::Pem;
use addr::Email;

#[derive(Debug,Deserialize,Serialize)]
pub struct IdentityConfig {
    pub public_key: String,
    pub private_key: String,
    pub ca_certs: String,
    token_lifetime: Option<u64>
}

impl IdentityConfig {
    pub fn token_lifetime(&self) -> u64 {
        if self.token_lifetime.is_none() {
            return 3600
        }

        self.token_lifetime.unwrap()
    }

    fn get_subject(&self) -> Result<Email, Report> {
        let cert_data = match fs::read(Path::new(&self.public_key)) {
            Ok(cert_data) => cert_data,
            Err(error) => return Err(
                eyre!("Unable to read certificate file")
                    .with_section(move || self.public_key.to_string().header("File name:"))
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };
        let reader = Cursor::new(cert_data);
        let (pem, _bytes_read) = match Pem::read(reader) {
            Ok(data) => data,
            Err(error) => return Err(
                eyre!("Unable to parse certificate file")
                    .with_section(move || self.public_key.to_string().header("File name:"))
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };
        let x509 = match pem.parse_x509() {
            Ok(data) => data,
            Err(error) => return Err(
                eyre!("Unable to parse PEM certificate to DER internally")
                    .with_section(move || self.public_key.to_string().header("File name:"))
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };

        let subject: Email;
        if let Some(cn) = x509.tbs_certificate.subject.iter_common_name().next().and_then(|cn| cn.as_str().ok()) {
            subject = match cn.parse() {
                Ok(subject)=>subject,
                Err(error) => return Err(
                    eyre!("Unable to parse certificate Common Name field as email address.")
                        .with_section(move || self.public_key.to_string().header("File name:"))
                        .with_section(move || error.to_string().header("Reason:"))
                    )
            };
        } else {
            return Err(
                eyre!("No Common Name field found in the certificate.")
                    .with_section(move || self.public_key.to_string().header("File name:"))
                )
        };

        Ok(subject)
    }

    pub fn device_id(&self) -> Result<String, Report> {
        Ok(self.get_subject()?.user().to_string())
    }

    pub fn domain(&self) -> Result<String, Report> {
        Ok(self.get_subject()?.host().to_string())
    }
}

#[derive(Debug,Deserialize,Serialize)]
pub struct AppConfig {
    pub identity: IdentityConfig
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
