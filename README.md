# ruuvi2iotcore

Ruuvi2iotcore is a GNU/Linux based gateway for relaying selected Ruuvi tag Bluetooth beacons to Google Cloud IoT Core service and to configure and control the gateway from it.

## Installation

After cloning this Git repository you need to install a few external C-libraries that are dependencies for Rust crates this project uses. These are:

* OpenSSL development files (openssl-devel RPM package in Fedora)
* Paho MQTT development files (paho-c-devel RPM package in Fedora)
* Paho MQTT build also requires the "cmake" utility.

If you do not yet have the Rust development environment setup please follow instructions for [Installing Rust](https://rustup.rs/) first as well.

After C-language dependencies and Rust development environment have been set up you can compile the ruuvi2iotcore binary itself with Cargo:

```sh
cargo build --release
```

After the build is finished you can copy the binary ./target/release/ruuvi2iotcore to some location in your PATH e.g /usr/local/bin.

You need to execute the binary as a root user to have enough capabilities to work with Bluetooth devices on GNU/Linux unless you grant those permissions to the binary itself with:

```sh
sudo setcap 'cap_net_raw,cap_net_admin+eip' /usr/local/bin/ruuvi2iotcore
```

## Configuration

Ruuvi2iotcore has two local configuration files:

1. Software configuration file (example file: ruuvi2iotcore.toml) that configures identity and IoT Core registry settings to use.
    * See later section on setting up IoT Core if you do not have one running yet. (You need the registry name, region, and GCP project id for example so that you can configure them here.)
2. Logging configuration file (example file: log4rs.yaml) that configures verbosity of logging and the location of log files (if enabled). Log files without absolute path defined are written into in the default working directory of the binary which defaults to users home folder at ~/.local/share/ruuvi2iotcore/ (Default location can be verified with: ```ruuvi2iotcore --help```)

Configuration files are by default searched from users home folder at ~/.config/ruuvi2iotcore/ruuvi2iotcore.toml and ~/.config/ruuvi2iotcore/log4rs.yaml respectively. (Default locations can be verified with: ```ruuvi2iotcore --help```)

You also need an X509 certificate and key pair in PEM-formatted files that are used to authenticate and secure communications to IoT Core service. Generating such a keypair can be achieved with the OpenSSL command:

```sh
openssl req -x509 -sha256 -nodes -days 365 -newkey rsa:2048 -keyout ruuvi2iotcore.key -out ruuvi2iotcore.crt
```

Remember to secure your .key file properly and remove all unnecessary user privileges from it.

(In addition to local keypair, you may optionally also define Certificate Authority chain file that Google provides and can be download for example with: ```curl -O https://pki.goog/roots.pem```)

You configure the locations of these three identity files in ruuvi2iotcore.toml. Note: if you do not specify an absolute path the files are expected to be in the default working directory of the binary which defaults to users home folder at ~/.local/share/ruuvi2iotcore/ (Default location can be verified with: ```ruuvi2iotcore --help```)

## Setup in Google Cloud

Login to your GCP Project and enable and configure your IoT Core and Pub/Sub environment.

### Setup in IoT Core

Refer to Google Cloud Internet Of Things (IoT) Core [documentation](https://cloud.google.com/iot/docs) first.

1. Enable IoT Core API if not yet enabled.
2. Create a registry into IoT Core (if not yet created)
3. Create a gateway into the selected registry. For authentication use the RS256_X509. Upload or copy&paste the public key (certificate) to IoT Core you created earlier.
4. Using the file example_config.json as a template update the configuration of the gateway:
    * To "tags" configure list of Ruuvi tag mac addresses and IoT Core device_id's associated with those addresses.
    * If "collecting" is true will ruuvi2iotcore automatically start collecting beacons and relaying them. If it is false ruuvi2iotcore will wait for COLLECT command before starting collecting and relaying.
    * Optionally: Also "event_subfolder" in most cases will be empty or if you wish to use one you also need to set up the topic subfolder in IoT Core first. This can safely be omitted if not configured.
    * Optionally: Field "collection_size" is a buffer that dictates how many beacons should be collected before they are relayed to IoT Core; 0 or 1 will send every beacon individually and larger value will collect as many beacons first before publishing them via MQTT.
5. Optionally: bluetooth_config and its adapter_index define a value upwards from 0 which is the index of installed Bluetooth adapters on the hardware you are running ruuvitag2iotcore on. Normally you do not need to change this and bluetooth_config can also be omitted.

Once you have configured your gateway proceed to create devices into the registry:

1. Name of your device(s) need to be identical with the client_id's you configured into the gateways configuration in previous step.
2. Finally bind your device(s) to the registry through the gateway configuration screen.

### Setup in pub/sub

Refer to Google Cloud Pub/Sub [documentation](https://cloud.google.com/pubsub/docs) first.

Your IoT Core registry (and device) should be configured to publish received messages to some pub/sub topic. Note: if you plan to use subfolders you need individual topics for each.

Once a message is published to Pub/Sub topic you need to with either Cloud Function or Data Flow for example collect the beacons from the topic and store them in a SQL database or other desired location.

## Executing software

The help page is always available with the ```--help``` flag:

```sh
❯ ./target/debug/ruuvi2iotcore --help
ruuvi2iotcore x.y.z
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

If all your configuration and certificate files are in default locations just executing the binary itself is enough. Otherwise, you might need to adjust the default locations with the command line arguments first.

Happy collecting!

## Controlling the process from IoT Core

Few commands can be issued to the running ruuvi2iotcore process remotely. By sending one of the following commands through IoT Core:

* ```{"command": "pause"}``` will pause relay of Ruuvi tag beacons to IoT Core (if collecting).
* ```{"command": "collect"}``` will continue relay of Ruuvi tag beacons to IoT Core (if paused).
* ```{"command": "shutdown"}``` will force a clean shutdown (if possible) of the binary. All collection and relay will stop.
* ```{"command": "reset"}``` will force a clean reset (if possible) of the internal Bluetooth scanner and IoT Core client subthreads. Useful for cases where something is wrong and you do not have access to your ruuvi2iotcore installation otherwise.
