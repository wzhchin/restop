use std::time::Duration;

use anyhow::Result;

const DEFAULT_UPDATE_MS: u64 = 2_000;
const MIN_UPDATE_MS: u64 = 250;
const MAX_UPDATE_MS: u64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefreshIntervals {
    pub hardware: Duration,
    pub process: Duration,
}

fn parse_interval_ms(value: Option<&str>, fallback_ms: u64) -> Duration {
    let millis = value
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(fallback_ms)
        .clamp(MIN_UPDATE_MS, MAX_UPDATE_MS);
    Duration::from_millis(millis)
}

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

    /// Collection intervals are read once when the application starts.
    /// Process collection is separate because it scales with the PID count.
    pub fn refresh_intervals(&self) -> RefreshIntervals {
        let hardware = parse_interval_ms(
            std::env::var("RESTOP_UPDATE_MS").ok().as_deref(),
            DEFAULT_UPDATE_MS,
        );
        let process = parse_interval_ms(
            std::env::var("RESTOP_PROCESS_UPDATE_MS").ok().as_deref(),
            hardware.as_millis() as u64,
        );
        RefreshIntervals { hardware, process }
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

#[cfg(test)]
mod tests {
    use super::{parse_interval_ms, DEFAULT_UPDATE_MS, MAX_UPDATE_MS, MIN_UPDATE_MS};
    use std::time::Duration;

    #[test]
    fn refresh_interval_defaults_to_two_seconds() {
        assert_eq!(
            parse_interval_ms(None, DEFAULT_UPDATE_MS),
            Duration::from_secs(2)
        );
        assert_eq!(
            parse_interval_ms(Some("invalid"), DEFAULT_UPDATE_MS),
            Duration::from_secs(2)
        );
    }

    #[test]
    fn refresh_interval_is_bounded() {
        assert_eq!(
            parse_interval_ms(Some("1"), DEFAULT_UPDATE_MS),
            Duration::from_millis(MIN_UPDATE_MS)
        );
        assert_eq!(
            parse_interval_ms(Some("999999"), DEFAULT_UPDATE_MS),
            Duration::from_millis(MAX_UPDATE_MS)
        );
        assert_eq!(
            parse_interval_ms(Some("1500"), DEFAULT_UPDATE_MS),
            Duration::from_millis(1500)
        );
    }
}
