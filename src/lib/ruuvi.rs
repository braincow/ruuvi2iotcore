use std::fmt;

#[derive(Debug)]
pub struct RuuviTagAccelaration {
    on_x_axis: f32,
    on_y_axis: f32,
    on_z_axis: f32
}

impl RuuviTagAccelaration {
    fn sqrt(&self) -> f32 {
        (self.on_x_axis * self.on_x_axis + self.on_y_axis * self.on_y_axis + self.on_z_axis * self.on_z_axis).sqrt().round()
    }
}

impl fmt::Display for RuuviTagAccelaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(acceleration={}mG, on_x={}mG, on_y={}mG, on_z={}mG)",
            self.sqrt(),
            self.on_x_axis,
            self.on_y_axis,
            self.on_z_axis)
    }
}

#[derive(Debug,Clone, Copy, structview::View)]
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
    measurement_sequence_number: structview::u16_be
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
        battery_voltage + 1600
    }

    pub fn get_tx_power(&self) -> i8 {
        let powerinfo = self.powerinfo.to_int();
        let tx_power = powerinfo & 0b11111;
        (tx_power * 2 - 40) as i8
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