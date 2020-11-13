use trust_dns_resolver::Resolver;
use trust_dns_resolver::config::*;
use color_eyre::eyre::Report;
use std::str;

#[derive(Debug)]
pub struct IotCoreConfig {
    pub device_id: String,
    pub project_id: String,
    pub region: String,
    pub registry: String   
}

impl IotCoreConfig {
    pub fn build(device_id: &String, domain: &String) -> Result<IotCoreConfig, Report> {
        let resolver = Resolver::new(ResolverConfig::default(), ResolverOpts::default()).unwrap();

        let project_id_response = resolver.txt_lookup(&format!("_project_id.{}", domain)).unwrap();
        let project_id = project_id_response.iter().next().unwrap();

        let region_response = resolver.txt_lookup(&format!("_region.{}", domain)).unwrap();
        let region = region_response.iter().next().unwrap();

        let registry_response = resolver.txt_lookup(&format!("_registry.{}", domain)).unwrap();
        let registry = registry_response.iter().next().unwrap();

        Ok(IotCoreConfig {
            device_id: device_id.clone(),
            project_id: str::from_utf8(&project_id.txt_data()[0]).unwrap().to_string(),
            region: str::from_utf8(&region.txt_data()[0]).unwrap().to_string(),
            registry: str::from_utf8(&registry.txt_data()[0]).unwrap().to_string()
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