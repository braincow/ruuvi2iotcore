# ruuvi2iotcore

Ruuvi2iotcore is a Linux based gateway for relaying selected Ruuvi tag Bluetooth beacons to Google Cloud IoT Core service and to control the gateway from IoT Core.

## Installation

After cloning this Git repository you need to install few external C-libraries that are dependencies for Rust crates the project uses. These are:

* OpenSSL development files (openssl-devel in Fedora/CentOS)
* Paho MQTT development files (paho-c-devel in Fedora/CentOS)
* Paho MQTT build also requires cmake utility.

If you do not yet have Rust development environment setup please follow instructions for [Installing Rust](https://rustup.rs/).

After C-language dependencies and Rust development environment have been setup you can compile the ruuvi2iotcore binary itself with Cargo:

```sh
cargo build --release
```

After build is finished you can copy the binary ./target/release/ruuvi2iotcore to some location in your PATH e.g /usr/local/bin.

You need to execute the binary as root user to have enough capabilities to work with Bluetooth devices on Linux unless you grant those permissions for the binary with:

```sh
sudo setcap 'cap_net_raw,cap_net_admin+eip' /usr/local/bin/ruuvi2iotcore
```

## Configuration

Ruuvi2iotcore has two local configuration files:

1. Software configuration file (example file: ruuvi2iotcore.toml) that configures identity and IoT Core registry settings to use.
2. Logging configuration file (example file: log4rs.yaml) that configures verbosity of logging and the location of log files (if enabled). Log files without absolute path defined are written into in the default working directory of the binary which defaults to users home folder at ~/.local/share/ruuvi2iotcore/ (Default location can be verified with: ```ruuvi2iotcore --help```)

Configuration files are by default searched from users home folder at ~/.config/ruuvi2iotcore/ruuvi2iotcore.toml and ~/.config/ruuvi2iotcore/log4rs.yaml respectively. (Default locations can be verified with: ```ruuvi2iotcore --help```)

You also need an X509 certificate and key pair in PEM-formatted files that are used to authenticate and secure communications to IoT Core service. Generating a such a keypair can be achieved with OpenSSL command:

```sh
openssl req -x509 -sha256 -nodes -days 365 -newkey rsa:2048 -keyout ruuvi2iotcore.key -out ruuvi2iotcore.crt
```

Remember to secure your .key file properly and remove all unnecessary user privileges from it.

In addition to local keypair you also need a Certificate Authority chain that Google provides and can be download for example with: ```curl -O https://pki.goog/roots.pem```

You configure locations of these three identity files in ruuvi2iotcore.toml. Note: if you do not spesify an absolute path the files are expected to be in the default working directory of the binary which defaults to users home folder at ~/.local/share/ruuvi2iotcore/ (Default location can be verified with: ```ruuvi2iotcore --help```)

## Setup in Google Cloud

Login to your GCP Project and enable and configure your IoT Core and Pub/Sub environment.

### Setup in IoT Core

1. Enable IoT Core API if not yet enabled.
2. Create a registry into IoT Core (if not yet created)
3. Create a device into the selected registry. For authentication use the RS256_X509. Upload or copy&paste the public key (certificate) to IoT Core you created earlier.
4. Using the file example_config.json as a template update the configuration of the device:
    * Select either blacklist or whitelist mode of operation and list the mac addresses of your Ruuvi tags you want to include or exclude from forwarding.
    * Only if you do not want your ruuvi2iotcore to automatically start the collection and forwarding of the beacons should you change the collecting mode to false by default.
    * Also event_subfolder in production should be empty or if you wish to use one you also need to setup the topic subfolder in IoT Core first.
    * collection_size is a buffer that dictates how many beacons should be collected before they are relayed to IoT Core; 0 or 1 will send every beacon individually and larger value will collect as many beacons first before publishing them via MQTT.
5. bluetooth_config and its adapter_index define a value upwards from 0 that is the index of installed Bluetooth adapters on the hardware you are running ruuvitag2iotcore at. Normally you do not need to change this.

### Setup in pub/sub

Your IoT Core registry (and device) should be configured to publish received messages to some pub/sub topic. Note: if you plan to use subfolders you need individual topics for each.

Once message is published to Pub/Sub topic you need to with either Cloud Function or Data Flow for example collect the beacons from the topic and store them in a SQL database or other desired location.

## Executing software

Help page is always available with the ```--help``` flag:

```sh
❯ ./target/debug/ruuvi2iotcore --help
ruuvi2iotcore 0.1.1
Antti Peltonen <antti.peltonen@iki.fi>
Ruuvi tag beacons to GCP iot core

USAGE:
    ruuvi2iotcore [FLAGS] [OPTIONS]

FLAGS:
    -h, --help       Prints help information
    -n, --no-log     Disable logging.
    -V, --version    Prints version information

OPTIONS:
    -c, --config <config>      Specify alternate config file location. [default:
                               /home/bcow/.config/ruuvi2iotcore/ruuvi2iotcore.toml]
    -l, --log <logging>        Specify alternate logging config file location. [default:
                               /home/bcow/.config/ruuvi2iotcore/log4rs.yaml]
    -w, --workdir <workdir>    Specify alternate location of working directory. [default:
                               /home/bcow/.local/share/ruuvi2iotcore]
```

If all your configuration and certificate files are in default locations just executing the binary itself is enough. Otherwise you might need to adjust the default locations with the command line arguments first.

Happy collecting!

## Controlling the process from IoT Core

Few commands can be issued to the running ruuvi2iotcore process remotely. By sending one of following commands through IoT Core:

* ```{"command": "pause"}``` will pause relay of Ruuvi tag beacons to IoT Core (if collecting).
* ```{"command": "collect"}``` will continue relay of Ruuvi tag beacons to IoT Core (if paused).
* ```{"command": "shutdown"}``` will force a clean shutdown (if possible) of the binary. All collection and relay will obviously stop.
* ```{"command": "reset"}``` will force a clean reset (if possible) of the internal Bluetooth scanner and IoT Core client subthreads. Useful for cases where something is wrong and you do not have console nearby.
