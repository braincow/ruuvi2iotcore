use std::time::{SystemTime, UNIX_EPOCH};
use std::path::{Path, PathBuf};

use frank_jwt::{Algorithm, encode};
use serde::Serialize;
use color_eyre::{eyre::eyre, SectionExt, Section, eyre::Report};

use crate::configfile::AppConfig;

#[derive(Debug, Serialize)]
pub struct JWTHeaders;

#[derive(Debug, Serialize)]
pub struct JWTPayload {
    iat: u64,
    exp: u64,
    aud: String
}

impl JWTPayload {
    fn new(audience: &String, lifetime: &u64) -> JWTPayload {
        trace!("in new");
        let now = SystemTime::now();
        let secs_since_epoc = now.duration_since(UNIX_EPOCH).unwrap();
    
        JWTPayload {
            iat: secs_since_epoc.as_secs(),
            exp: secs_since_epoc.as_secs() + lifetime,
            aud: audience.clone()
        }
    }
}

pub struct IotCoreAuthToken {
    headers: JWTHeaders,
    payload: JWTPayload,
    private_key: PathBuf,
    audience: String,
    lifetime: u64
}

impl IotCoreAuthToken {
    pub fn build(appconfig: &AppConfig) -> IotCoreAuthToken {
        trace!("in build");
        IotCoreAuthToken {
            headers: JWTHeaders,
            payload: JWTPayload::new(&appconfig.iotcore.project_id, &appconfig.identity.token_lifetime()),
            private_key: Path::new(&appconfig.identity.private_key).to_path_buf(),
            audience: appconfig.iotcore.project_id.clone(),
            lifetime: appconfig.identity.token_lifetime()
        }
    }

    pub fn issue_new(&self) -> Result<String, Report> {
        trace!("in issue_new");
        let token = match encode(json!(self.headers), &self.private_key, &json!(self.payload), Algorithm::RS256) {
            Ok(jwt) => Ok(jwt),
            Err(error) => Err(
                eyre!("Unable to issue new JWT token")
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };
        debug!("JWT token is: {:?}", token);
        token
    }

    pub fn renew(&mut self) -> Result<String, Report> {
        trace!("in renew");
        self.payload = JWTPayload::new(&self.audience, &self.lifetime);
        self.issue_new()
    }

    pub fn is_valid(&self, threshold: u64) -> bool {
        trace!("in is_valid");
        let now = SystemTime::now();
        let secs_since_epoc = now.duration_since(UNIX_EPOCH).unwrap();

        if secs_since_epoc.as_secs() > self.payload.exp - threshold {
            debug!("JWT token has expired / is expiring within the threshold.");
            return false
        }

        debug!("JWT token has not expired.");
        true
    }
}

// eof