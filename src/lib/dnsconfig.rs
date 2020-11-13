use trust_dns_resolver::Resolver;
use trust_dns_resolver::config::*;
use color_eyre::{eyre::eyre, SectionExt, Section, eyre::Report};
use std::str;

fn resolve_txt_record(resolver: &Resolver, record: &String) -> Result<String, Report> {
    match resolver.txt_lookup(record) {
        Ok(response) => {
            match response.iter().next() {
                Some(response_data) => match str::from_utf8(&response_data.txt_data()[0]) {
                    Ok(txt_str) => Ok(txt_str.to_string()),
                    Err(error) => return Err(
                        eyre!("Unable to parse DNS response TXT data into string.")
                            .with_section(move || format!("{:?}", response_data).header("Byte array:"))
                            .with_section(move || error.to_string().header("Reason:"))
                        )            
                },
                None => return Err(
                    eyre!("Empty DNS record.")
                        .with_section(move || record.to_string().header("Record:"))
                    )
            }
        },
        Err(error) => return Err(
            eyre!("Unable to query DNS record.")
                .with_section(move || record.to_string().header("Record:"))
                .with_section(move || error.to_string().header("Reason:"))
            )
    }
}

#[derive(Debug)]
pub struct IotCoreConfig {
    pub device_id: String,
    pub project_id: String,
    pub region: String,
    pub registry: String   
}

impl IotCoreConfig {
    pub fn build(device_id: &String, domain: &String) -> Result<IotCoreConfig, Report> {
        let resolver = match Resolver::new(ResolverConfig::default(), ResolverOpts::default()) {
            Ok(resolver) => resolver,
            Err(error) => return Err(
                eyre!("Unable to instantiate DNS resolver.")
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };

        Ok(IotCoreConfig {
            device_id: device_id.clone(),
            project_id: resolve_txt_record(&resolver, &format!("_project_id.{}", domain))?,
            region: resolve_txt_record(&resolver, &format!("_region.{}", domain))?,
            registry: resolve_txt_record(&resolver, &format!("_registry.{}", domain))?
        })
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