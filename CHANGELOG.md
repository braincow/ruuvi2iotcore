# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added
### Changed
- fix: using correct stuck beacon interval value on error message
### Removed

## [0.2.3] - 2021-01-04
### Added
- openssl-sys and paho-mqtt crates now built as vendored to simplify cross compiling
- using cargo rpm and cross to cross build binaries and RPM install packages

### Changed
- feature: Data stuck detection threshold is now configurable via IoT Core config message.
- feature: No beacons detection threshold is now configurable via IoT Core config message.

### Removed

## [0.2.2] - 2020-12-13
### Added
- bugfix: detecting "stuck values" in beacons. if values in beacons for a ruuvi tag are identical three minutes apart issue bluetooth scanner thread restart
### Changed

### Removed
- enhancement: last_seen is tracked now only in iot core thread and both threads (scanner included) are restarted based on that trigger instead of both having their own triggers.
## [0.2.1] - 2020-12-06
### Added

### Changed
- enhancement: loops in bt scanner and iotcore threads now sleep for 100ms instead of 10ms on each loop
- bugfix: only succesful submit of beacon queue will lead the queue to be emptied

### Removed

## [0.2.0] - 2020-12-01
### Added

### Changed
- ruuvi2iotcore now functions as a IoT Core gateway instead of a device. Ruuvi tags need to be added as individual devices and then bounded to gateway.

### Removed

## [0.1.2] - 2020-11-28
### Added

### Changed
- fix: in rare case of BT hardware lockup scanner thread cant recover, but gets stuck in an never ending loop of trying to reset its state.

### Removed

## [0.1.1] - 2020-11-27
### Added

### Changed
- fixed: when pausing beacon collection mqtt connection will time out.

### Removed

## [0.1.0] - 2020-11-26
### Added
- Ruuvi tag beacons are recorded & relayed to Google Cloud Platform IoT Core.

### Changed

### Removed
