use anyhow::Result;

#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, Hash)]
pub enum Base {
    #[default]
    Decimal,
    Binary,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, Hash)]
pub enum TemperatureUnit {
    #[default]
    Celsius,
    Kelvin,
    Fahrenheit,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, Hash)]
pub enum RefreshSpeed {
    VerySlow,
    Slow,
    #[default]
    Normal,
    Fast,
    VeryFast,
}

impl RefreshSpeed {
    pub fn ui_refresh_interval(&self) -> f32 {
        match self {
            RefreshSpeed::VerySlow => 3.0,
            RefreshSpeed::Slow => 2.0,
            RefreshSpeed::Normal => 1.0,
            RefreshSpeed::Fast => 0.5,
            RefreshSpeed::VeryFast => 0.25,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Hash)]
pub enum SidebarMeterType {
    #[default]
    ProgressBar,
    Graph,
}

pub const SETTINGS: Settings = Settings {};

#[derive(Clone, Debug, Hash)]
pub struct Settings {}

impl Settings {
    pub fn temperature_unit(&self) -> TemperatureUnit {
        TemperatureUnit::default()
    }

    pub fn set_temperature_unit(&self, value: TemperatureUnit) -> Result<()> {
        Ok(())
    }

    pub fn base(&self) -> Base {
        Base::default()
    }

    pub fn set_base(&self, value: Base) -> Result<()> {
        Ok(())
    }
    pub fn set_last_viewed_page<S: AsRef<str>>(&self, value: S) -> Result<()> {
        Ok(())
    }

    pub fn refresh_speed(&self) -> RefreshSpeed {
        Default::default()
    }

    pub fn set_refresh_speed(&self, value: RefreshSpeed) -> Result<()> {
        Ok(())
    }

    pub fn sidebar_meter_type(&self) -> SidebarMeterType {
        SidebarMeterType::default()
    }

    pub fn set_sidebar_meter_type(&self, value: SidebarMeterType) -> Result<()> {
        Ok(())
    }

    pub fn network_bits(&self) -> bool {
        false
    }
}
