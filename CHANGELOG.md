# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added

### Changed

### Removed

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
