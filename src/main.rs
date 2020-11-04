#[macro_use] extern crate log;
#[macro_use] extern crate serde_json;

mod lib;

use std::path::Path;
use std::env;
use clap::{App, Arg};
use color_eyre::eyre::{Report, Result};
use dotenv::dotenv;
use directories::ProjectDirs;
use crossbeam::thread;
use crossbeam::channel::unbounded;

use crate::lib::config::AppConfig;
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

    // initialize Clap (Command line argument parser)
    let matches = App::new(env!("CARGO_PKG_NAME")) // get the application name from package name
        .version(env!("CARGO_PKG_VERSION")) // read the version string from cargo.toml
        .author(env!("CARGO_PKG_AUTHORS")) // and for the author(s) information as well
        .about(env!("CARGO_PKG_DESCRIPTION")) // do the same for about, read it from env (cargo.toml)
            .arg(Arg::with_name("config") // define config file path and as a default use the autodetected one.
                .long("config")
                .short("c")
                .help("Specify alternate config file location.")
                .default_value(default_config_file_path.to_str().unwrap())
                .global(true))
            .arg(Arg::with_name("verbose") // define verbosity flag
                .long("verbose")
                .short("v")
                .multiple(true)
                .help("Sets the level of verbosity. Specifying multiple flags increases verbosity.")
                .global(true))
        // from App instance parse all matches to determine selected commandline arguments and options
        .get_matches();

    // if there are environment variable(s) set for rust log
    //  overwrite them here since command line arguments have higher priority
    match matches.occurrences_of("verbose") {
        0 => env::set_var("RUST_LOG", "error"),
        1 => env::set_var("RUST_LOG", "warn"),
        2 => env::set_var("RUST_LOG", "info"),
        3 => env::set_var("RUST_LOG", "debug"),
        _ => env::set_var("RUST_LOG", "trace")
    }
    // initialize logger
    pretty_env_logger::try_init_timed().unwrap();
    info!("Starting {} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

    // read configuration
    let config = AppConfig::read_config(Path::new(matches.value_of("config").unwrap()))?;

    // TODO: determine subcommand from App matches
    //let (mqtt_out, mqtt_in) = unbounded();
    let (event_s, event_r) = unbounded();
    let scanner = BluetoothScanner::build(&config, &event_s)?;
    let mut iotcore = IotCoreClient::build(&config, &event_r)?;

    thread::scope(|scope| {
        // spawn the mqtt thread
        scope.spawn(move|_| {
            iotcore.start_client().unwrap();
        });

        // spawn bt scan thread
        scope.spawn(move|_| {
            scanner.start_scanner().unwrap();
        });
    }).unwrap();

    // nothing to do so print the usage and version information
    // TODO: print usage only when none of the subcommands matches
    println!("{}", matches.usage());

    // return with Ok (success)
    Ok(())
}

// eof
