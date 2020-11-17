#[macro_use] extern crate log;
#[macro_use] extern crate serde_json;

mod lib;

use std::path::Path;
use std::env;
use clap::{App, Arg};
use color_eyre::{eyre::eyre, SectionExt, Section, eyre::Report};
use dotenv::dotenv;
use directories::ProjectDirs;
use crossbeam::thread;
use crossbeam::channel::unbounded;

use crate::lib::configfile::AppConfig;
use crate::lib::dnsconfig::IotCoreConfig;
use crate::lib::scanner::BluetoothScanner;
use crate::lib::iotcore::IotCoreClient;

fn main() -> Result<(), Report> {
    // initialize error handling
    color_eyre::install()?;

    // initialize dot environment so we can pull arguments from env, env files, config file
    //  commandline or as hardcoded values in code
    dotenv().ok();

    // project dirs are located somewhere in the system based on arch and os
    let project_dirs = ProjectDirs::from("me", "bcow", env!("CARGO_PKG_NAME")).unwrap();
    let default_config_file_path = Path::new(project_dirs.config_dir()).join(format!("{}.toml", env!("CARGO_PKG_NAME")));
    let default_logging_config_file_path = Path::new(project_dirs.config_dir()).join("log4rs.yaml");
    let default_working_dir_path = Path::new(project_dirs.data_dir());

    // initialize Clap (Command line argument parser)
    let matches = App::new(env!("CARGO_PKG_NAME")) // get the application name from package name
        .version(env!("CARGO_PKG_VERSION")) // read the version string from cargo.toml
        .author(env!("CARGO_PKG_AUTHORS")) // and for the author(s) information as well
        .about(env!("CARGO_PKG_DESCRIPTION")) // do the same for about, read it from env (cargo.toml)
            .arg(Arg::with_name("workdir") // working directory default
                .long("workdir")
                .short("w")
                .help("Specify alternate location of working directory.")
                .default_value(default_working_dir_path.to_str().unwrap())
                .global(true))
            .arg(Arg::with_name("config") // define config file path and as a default use the autodetected one.
                .long("config")
                .short("c")
                .help("Specify alternate config file location.")
                .default_value(default_config_file_path.to_str().unwrap())
                .global(true))
            .arg(Arg::with_name("logging") // define logconfig file path and as a default use the autodetected one.
                .long("log")
                .short("l")
                .help("Specify alternate logging config file location.")
                .default_value(default_logging_config_file_path.to_str().unwrap())
                .global(true))
            .arg(Arg::with_name("nologging") // define logconfig file path and as a default use the autodetected one.
                .long("no-log")
                .short("n")
                .help("Disable logging.")
                .conflicts_with("logging")
                .global(true))
        // from App instance parse all matches to determine selected commandline arguments and options
        .get_matches();

    // change working directory to configured path
    let working_dir_path = Path::new(matches.value_of("workdir").unwrap());
    match env::set_current_dir(working_dir_path) {
        Ok(_) => {},
        Err(error) => return Err(
            eyre!("Unable to change working directory")
                .with_section(move || working_dir_path.to_string_lossy().trim().to_string().header("Directory name:"))
                .with_section(move || error.to_string().header("Reason:"))
            )
    }

    // read logging configuration (if present)
    if matches.is_present("logging") {
        let logging_config_path = Path::new(matches.value_of("logging").unwrap());
        match log4rs::init_file(logging_config_path, Default::default()) {
            Ok(_) => {},
            Err(error) => return Err(
                eyre!("Unable to start logging")
                    .with_section(move || logging_config_path.to_string_lossy().trim().to_string().header("Config file name:"))
                    .with_section(move || error.to_string().header("Reason:"))
                )
        };
    }
    info!("Starting {} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

    // read configuration
    let appconfig = AppConfig::read_config(Path::new(matches.value_of("config").unwrap()))?;
    debug!("appconfig is '{:?}'", appconfig);
    // autodetect registry settings from dns based on the certificate (configured in configfile)
    //  CN= info.
    let iotconfig = IotCoreConfig::build(&appconfig.identity.device_id()?, &appconfig.identity.domain()?)?;
    debug!("iotcnfig is '{:?}'", iotconfig);

    let (cnc_s, cnc_r) = unbounded();
    let (event_s, event_r) = unbounded();
    let mut scanner = BluetoothScanner::build(&event_s, &cnc_r)?;
    let mut iotcore = IotCoreClient::build(&appconfig, &iotconfig, &event_r, &cnc_s)?;

    thread::scope(|scope| {
        // spawn the mqtt thread
        scope.spawn(move|_| {
            loop {
                match iotcore.start_client() {
                    Ok(_) => break,
                    Err(error) => error!("Restarting iotcore client: {}", error)
                };
            }
            info!("Shutting down IotCore client thread.");
        });

        // spawn bt scan thread
        scope.spawn(move|_| {
            loop {
                match scanner.start_scanner() {
                    Ok(exit) => if exit {
                        break;
                    } else {
                        info!("Restarting Bluetooth scanner due to adapter index change or adapter index reset.");
                    },
                    Err(error) => error!("Restarting bluetooth scanner: {}", error)
                };
            }
            info!("Shutting down Bluetooth scanner thread.");
        });
    }).unwrap();

    info!("Shutting down {}", env!("CARGO_PKG_NAME"));
    // return with Ok (success)
    Ok(())
}

// eof
