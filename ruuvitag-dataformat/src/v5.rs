use serde::ser::{SerializeStruct, Serializer};
use serde::Serialize;
use std::fmt;

#[derive(Debug, Serialize)]
pub struct RuuviTagAccelaration {
    on_x_axis: f32,
    on_y_axis: f32,
    on_z_axis: f32,
}

impl RuuviTagAccelaration {
    fn sqrt(&self) -> f32 {
        (self.on_x_axis * self.on_x_axis
            + self.on_y_axis * self.on_y_axis
            + self.on_z_axis * self.on_z_axis)
            .sqrt()
            .round()
    }
}

impl fmt::Display for RuuviTagAccelaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(acceleration={}mG, on_x={}mG, on_y={}mG, on_z={}mG)",
            self.sqrt(),
            self.on_x_axis,
            self.on_y_axis,
            self.on_z_axis
        )
    }
}

// https://github.com/ruuvi/ruuvi-sensor-protocols/blob/master/dataformat_05.md
#[derive(Debug, Clone, Copy, structview::View)]
#[repr(C)]
pub struct RuuviTagDataFormat5 {
    temperature: structview::i16_be,
    humidity: structview::u16_be,
    atmospheric_pressure: structview::u16_be,
    acceleration_x: structview::i16_be,
    acceleration_y: structview::i16_be,
    acceleration_z: structview::i16_be,
    powerinfo: structview::u16_be,
    movement_counter: u8,
    measurement_sequence_number: structview::u16_be,
}

impl Serialize for RuuviTagDataFormat5 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("RuuviTagDataFormat5", 7)?;
        state.serialize_field("temperature", &self.get_temperature())?;
        state.serialize_field("humidity", &self.get_humidity())?;
        state.serialize_field("atmospheric_pressure", &self.get_pressure())?;
        state.serialize_field("acceleration", &self.get_accelaration())?;
        state.serialize_field("powerinfo", &self.get_battery())?;
        state.serialize_field("movement_counter", &self.get_movement_counter())?;
        state.serialize_field(
            "measurement_sequence_number",
            &self.get_measurement_sequence_number(),
        )?;
        state.end()
    }
}

impl RuuviTagDataFormat5 {
    pub fn get_temperature(&self) -> f32 {
        self.temperature.to_int() as f32 / 200.0
    }

    pub fn get_humidity(&self) -> f32 {
        self.humidity.to_int() as f32 / 400.0
    }

    pub fn get_pressure(&self) -> f32 {
        (self.atmospheric_pressure.to_int() as f32 + 50000.0) / 100.0
    }

    pub fn get_accelaration(&self) -> RuuviTagAccelaration {
        RuuviTagAccelaration {
            on_x_axis: self.acceleration_x.to_int() as f32,
            on_y_axis: self.acceleration_y.to_int() as f32,
            on_z_axis: self.acceleration_z.to_int() as f32,
        }
    }

    pub fn get_battery(&self) -> u16 {
        let powerinfo = self.powerinfo.to_int();
        let battery_voltage = powerinfo >> 5;
        (battery_voltage + 1600) as u16
    }

    pub fn get_tx_power(&self) -> i8 {
        let powerinfo = self.powerinfo.to_int();
        let tx_power = powerinfo & 0b11111;
        (tx_power * 2) as i8 - 40
    }

    pub fn get_movement_counter(&self) -> u8 {
        self.movement_counter
    }

    pub fn get_measurement_sequence_number(&self) -> u16 {
        self.measurement_sequence_number.to_int()
    }
}

impl fmt::Display for RuuviTagDataFormat5 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(temperature={:.2}\u{00B0}C, humidity={:.2}%, pressure={:.2}hPa, acceleration={}, battery={}mV, tx_power={}dBm, movement_counter={}, measurement_sequence={})",
            self.get_temperature(),
            self.get_humidity(),
            self.get_pressure(),
            self.get_accelaration(),
            self.get_battery(),
            self.get_tx_power(),
            self.get_movement_counter(),
            self.get_measurement_sequence_number())
    }
}

#[cfg(test)]
mod tests {
    /*
     * About test cases:
     *
     * https://github.com/ruuvi/ruuvi-sensor-protocols/blob/master/dataformat_05.md
     * outlines the test cases for valid, min and max values
     */
    use crate::lib::ruuvi::RuuviTagDataFormat5;
    use structview::View;
    #[test]
    fn valid_values() {
        let hex_string = "0512FC5394C37C0004FFFC040CAC364200CDCBB8334C884F";
        let data = hex::decode(hex_string).unwrap();
        let beacon = RuuviTagDataFormat5::view(&data[1..]).unwrap();
        assert_eq!(beacon.get_temperature(), 24.3);
        assert_eq!(beacon.get_pressure(), 1000.44);
        assert_eq!(beacon.get_humidity(), 53.49);
        assert_eq!(beacon.get_accelaration().on_x_axis / 1000.0, 0.004);
        assert_eq!(beacon.get_accelaration().on_y_axis / 1000.0, -0.004);
        assert_eq!(beacon.get_accelaration().on_z_axis / 1000.0, 1.036);
        assert_eq!(beacon.get_tx_power(), 4);
        assert_eq!(beacon.get_battery(), 2977);
        assert_eq!(beacon.get_movement_counter(), 66);
        assert_eq!(beacon.get_measurement_sequence_number(), 205);
    }

    #[test]
    fn min_values() {
        let hex_string = "058001000000008001800180010000000000CBB8334C884F";
        let data = hex::decode(hex_string).unwrap();
        let beacon = RuuviTagDataFormat5::view(&data[1..]).unwrap();
        assert_eq!(beacon.get_temperature(), -163.835);
        assert_eq!(beacon.get_pressure(), 500.0);
        assert_eq!(beacon.get_humidity(), 0.000);
        assert_eq!(beacon.get_accelaration().on_x_axis / 1000.0, -32.767);
        assert_eq!(beacon.get_accelaration().on_y_axis / 1000.0, -32.767);
        assert_eq!(beacon.get_accelaration().on_z_axis / 1000.0, -32.767);
        assert_eq!(beacon.get_tx_power(), -40);
        assert_eq!(beacon.get_battery(), 1600);
        assert_eq!(beacon.get_movement_counter(), 0);
        assert_eq!(beacon.get_measurement_sequence_number(), 0);
    }

    #[test]
    fn max_values() {
        let hex_string = "057FFFFFFEFFFE7FFF7FFF7FFFFFDEFEFFFECBB8334C884F";
        let data = hex::decode(hex_string).unwrap();
        let beacon = RuuviTagDataFormat5::view(&data[1..]).unwrap();
        assert_eq!(beacon.get_temperature(), 163.835);
        assert_eq!(beacon.get_pressure(), 1155.34);
        assert_eq!(beacon.get_humidity(), 163.8350);
        assert_eq!(beacon.get_accelaration().on_x_axis / 1000.0, 32.767);
        assert_eq!(beacon.get_accelaration().on_y_axis / 1000.0, 32.767);
        assert_eq!(beacon.get_accelaration().on_z_axis / 1000.0, 32.767);
        assert_eq!(beacon.get_tx_power(), 20);
        assert_eq!(beacon.get_battery(), 3646);
        assert_eq!(beacon.get_movement_counter(), 254);
        assert_eq!(beacon.get_measurement_sequence_number(), 65534);
    }
}
