
use super::settings::{Base, TemperatureUnit, SETTINGS};

#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, Hash)]
enum Prefix {
    #[default]
    None,
    Kilo,
    Mega,
    Giga,
    Tera,
    Peta,
    Exa,
    Zetta,
    Yotta,
    Ronna,
    Quetta,
}

impl Prefix {
    pub fn into_iter() -> core::array::IntoIter<Prefix, 11> {
        [
            Prefix::None,
            Prefix::Kilo,
            Prefix::Mega,
            Prefix::Giga,
            Prefix::Tera,
            Prefix::Peta,
            Prefix::Exa,
            Prefix::Zetta,
            Prefix::Yotta,
            Prefix::Ronna,
            Prefix::Quetta,
        ]
        .into_iter()
    }
}

pub fn format_time(time_in_seconds: f64) -> String {
    let millis = ((time_in_seconds - time_in_seconds.floor()) * 100.0) as u8;
    let seconds = (time_in_seconds % 60.0) as u8;
    let minutes = ((time_in_seconds / 60.0) % 60.0) as u8;
    let hours = (time_in_seconds / (60.0 * 60.0)) as usize;
    format!("{hours}∶{minutes:02}∶{seconds:02}.{millis:02}")
}

pub fn convert_seconds(seconds: u64) -> String {
    use std::fmt::Write;
    if seconds == 0 {
        return String::from("00:00:00");
    }

    let durations = [
        ("y ", 365 * 24 * 60 * 60),
        ("d ", 24 * 60 * 60),
        (":", 60 * 60),
        (":", 60),
        ("", 1),
    ];

    let mut remaining = seconds;

    let mut sb = String::new();
    let mut end = 0;
    for (id, (unit, duration_in_seconds)) in durations.iter().enumerate() {
        if remaining >= *duration_in_seconds {
            let count = remaining / duration_in_seconds;
            remaining %= duration_in_seconds;
            write!(sb, "{:02}{}", count, unit).unwrap();
            end = id;
        }
    }
    if end > 1 {
        for (unit, _) in durations.iter().skip(end + 1) {
            write!(sb, "00{}", unit).unwrap();
        }
    }

    sb
}

fn to_largest_prefix(amount: f64, prefix_base: Base) -> (f64, Prefix) {
    let mut x = amount;
    let base = match prefix_base {
        Base::Decimal => 1000.0,
        Base::Binary => 1024.0,
    };
    for prefix in Prefix::into_iter() {
        if x < base {
            return (x, prefix);
        }
        x /= base;
    }
    (x, Prefix::Quetta)
}

fn celsius_to_fahrenheit(celsius: f64) -> f64 {
    celsius * 1.8 + 32.0
}

fn celsius_to_kelvin(celsius: f64) -> f64 {
    celsius + 273.15
}

pub fn convert_temperature(celsius: f64) -> String {
    match SETTINGS.temperature_unit() {
        TemperatureUnit::Kelvin => {
            format!("{} K", celsius_to_kelvin(celsius).round())
        }
        TemperatureUnit::Celsius => format!("{} °C", celsius.round()),
        TemperatureUnit::Fahrenheit => format!("{} °F", celsius_to_fahrenheit(celsius).round(),),
    }
}

pub fn convert_storage(bytes: f64, integer: bool) -> String {
    match SETTINGS.base() {
        Base::Decimal => convert_storage_decimal(bytes, integer),
        Base::Binary => convert_storage_binary(bytes, integer),
    }
}

fn convert_storage_decimal(bytes: f64, integer: bool) -> String {
    let (mut number, prefix) = to_largest_prefix(bytes, Base::Decimal);
    if integer {
        number = number.round();
        match prefix {
            Prefix::None => format!("{} B", number),
            Prefix::Kilo => format!("{} kB", number),
            Prefix::Mega => format!("{} MB", number),
            Prefix::Giga => format!("{} GB", number),
            Prefix::Tera => format!("{} TB", number),
            Prefix::Peta => format!("{} PB", number),
            Prefix::Exa => format!("{} EB", number),
            Prefix::Zetta => format!("{} ZB", number),
            Prefix::Yotta => format!("{} YB", number),
            Prefix::Ronna => format!("{} RB", number),
            Prefix::Quetta => format!("{} QB", number),
        }
    } else {
        match prefix {
            Prefix::None => format!("{} B", number.round()),
            Prefix::Kilo => format!("{number:.2} kB"),
            Prefix::Mega => format!("{number:.2} MB"),
            Prefix::Giga => format!("{number:.2} GB"),
            Prefix::Tera => format!("{number:.2} TB"),
            Prefix::Peta => format!("{number:.2} PB"),
            Prefix::Exa => format!("{number:.2} EB"),
            Prefix::Zetta => format!("{number:.2} ZB"),
            Prefix::Yotta => format!("{number:.2} YB"),
            Prefix::Ronna => format!("{number:.2} RB"),
            Prefix::Quetta => format!("{number:.2} QB"),
        }
    }
}

fn convert_storage_binary(bytes: f64, integer: bool) -> String {
    let (mut number, prefix) = to_largest_prefix(bytes, Base::Binary);
    if integer {
        number = number.round();
        match prefix {
            Prefix::None => format!("{} B", number),
            Prefix::Kilo => format!("{} KiB", number),
            Prefix::Mega => format!("{} MiB", number),
            Prefix::Giga => format!("{} GiB", number),
            Prefix::Tera => format!("{} TiB", number),
            Prefix::Peta => format!("{} PiB", number),
            Prefix::Exa => format!("{} EiB", number),
            Prefix::Zetta => format!("{} ZiB", number),
            Prefix::Yotta => format!("{} YiB", number),
            Prefix::Ronna => format!("{} RiB", number),
            Prefix::Quetta => format!("{} QiB", number),
        }
    } else {
        match prefix {
            Prefix::None => format!("{} B", number.round()),
            Prefix::Kilo => format!("{number:.2} KiB"),
            Prefix::Mega => format!("{number:.2} MiB"),
            Prefix::Giga => format!("{number:.2} GiB"),
            Prefix::Tera => format!("{number:.2} TiB"),
            Prefix::Peta => format!("{number:.2} PiB"),
            Prefix::Exa => format!("{number:.2} EiB"),
            Prefix::Zetta => format!("{number:.2} ZiB"),
            Prefix::Yotta => format!("{number:.2} YiB"),
            Prefix::Ronna => format!("{number:.2} RiB"),
            Prefix::Quetta => format!("{number:.2} QiB"),
        }
    }
}

pub fn conver_storage_width4(bytes: f64) -> String {
    if bytes < 10.0 {
        format!("{:.0}  B", bytes)
    } else if bytes < 100.0 {
        format!("{:.0} B", bytes)
    } else if bytes < 1000.0 {
        format!("{:.0}B", bytes)
    } else if bytes < 1024.0_f64 * 10.0 {
        format!("{:.1}K", bytes / 1024.0_f64)
    } else if bytes < 1024.0_f64 * 100.0 {
        format!("{:.0} K", bytes / 1024.0_f64)
    } else if bytes < 1024.0_f64 * 1000.0 {
        format!("{:.0}K", bytes / 1024.0_f64)
    } else if bytes < 1024.0_f64.powi(2) * 10.0 {
        format!("{:.1}M", bytes / 1024.0_f64.powi(2))
    } else if bytes < 1024.0_f64.powi(2) * 100.0 {
        format!("{:.0} M", bytes / 1024.0_f64.powi(2))
    } else if bytes < 1024.0_f64.powi(2) * 1000.0 {
        format!("{:.0}M", bytes / 1024.0_f64.powi(2))
    } else if bytes < 1024.0_f64.powi(3) * 10.0 {
        format!("{:.1}G", bytes / 1024.0_f64.powi(3))
    } else if bytes < 1024.0_f64.powi(3) * 100.0 {
        format!("{:.0} G", bytes / 1024.0_f64.powi(3))
    } else if bytes < 1024.0_f64.powi(3) * 1000.0 {
        format!("{:.0}G", bytes / 1024.0_f64.powi(3))
    } else if bytes < 1024.0_f64.powi(4) * 10.0 {
        format!("{:.1}T", bytes / 1024.0_f64.powi(4))
    } else if bytes < 1024.0_f64.powi(4) * 100.0 {
        format!("{:.0} T", bytes / 1024.0_f64.powi(4))
    } else if bytes < 1024.0_f64.powi(4) * 1000.0 {
        format!("{:.0}T", bytes / 1024.0_f64.powi(4))
    } else if bytes < 1024.0_f64.powi(5) * 10.0 {
        format!("{:.1}P", bytes / 1024.0_f64.powi(5))
    } else if bytes < 1024.0_f64.powi(5) * 100.0 {
        format!("{:.0} P", bytes / 1024.0_f64.powi(5))
    } else if bytes < 1024.0_f64.powi(5) * 1000.0 {
        format!("{:.0}P", bytes / 1024.0_f64.powi(5))
    } else if bytes < 1024.0_f64.powi(6) * 10.0 {
        format!("{:.1}E", bytes / 1024.0_f64.powi(6))
    } else if bytes < 1024.0_f64.powi(6) * 100.0 {
        format!("{:.0} E", bytes / 1024.0_f64.powi(6))
    } else if bytes < 1024.0_f64.powi(6) * 1000.0 {
        format!("{:.0}E", bytes / 1024.0_f64.powi(6))
    } else if bytes < 1024.0_f64.powi(7) * 10.0 {
        format!("{:.1}Z", bytes / 1024.0_f64.powi(7))
    } else if bytes < 1024.0_f64.powi(7) * 100.0 {
        format!("{:.0} Z", bytes / 1024.0_f64.powi(7))
    } else if bytes < 1024.0_f64.powi(7) * 1000.0 {
        format!("{:.0}Z", bytes / 1024.0_f64.powi(7))
    } else {
        format!("{:.1}Y", bytes / 1024.0_f64.powi(8))
    }
}

pub fn convert_speed(bytes_per_second: f64, network: bool) -> String {
    match SETTINGS.base() {
        Base::Decimal => {
            if network && SETTINGS.network_bits() {
                convert_speed_bits_decimal(bytes_per_second * 8.0)
            } else {
                convert_speed_decimal(bytes_per_second)
            }
        }
        Base::Binary => {
            if network && SETTINGS.network_bits() {
                convert_speed_bits_binary(bytes_per_second * 8.0)
            } else {
                convert_speed_binary(bytes_per_second)
            }
        }
    }
}

fn convert_speed_decimal(bytes_per_second: f64) -> String {
    let (number, prefix) = to_largest_prefix(bytes_per_second, Base::Decimal);
    match prefix {
        Prefix::None => format!("{} B/s", number.round()),
        Prefix::Kilo => format!("{number:.2} kB/s"),
        Prefix::Mega => format!("{number:.2} MB/s"),
        Prefix::Giga => format!("{number:.2} GB/s"),
        Prefix::Tera => format!("{number:.2} TB/s"),
        Prefix::Peta => format!("{number:.2} PB/s"),
        Prefix::Exa => format!("{number:.2} EB/s"),
        Prefix::Zetta => format!("{number:.2} ZB/s"),
        Prefix::Yotta => format!("{number:.2} YB/s"),
        Prefix::Ronna => format!("{number:.2} RB/s"),
        Prefix::Quetta => format!("{number:.2} QB/s"),
    }
}

fn convert_speed_binary(bytes_per_second: f64) -> String {
    let (number, prefix) = to_largest_prefix(bytes_per_second, Base::Binary);
    match prefix {
        Prefix::None => format!("{} B/s", number.round()),
        Prefix::Kilo => format!("{number:.2} KiB/s"),
        Prefix::Mega => format!("{number:.2} MiB/s"),
        Prefix::Giga => format!("{number:.2} GiB/s"),
        Prefix::Tera => format!("{number:.2} TiB/s"),
        Prefix::Peta => format!("{number:.2} PiB/s"),
        Prefix::Exa => format!("{number:.2} EiB/s"),
        Prefix::Zetta => format!("{number:.2} ZiB/s"),
        Prefix::Yotta => format!("{number:.2} YiB/s"),
        Prefix::Ronna => format!("{number:.2} RiB/s"),
        Prefix::Quetta => format!("{number:.2} QiB/s"),
    }
}

fn convert_speed_bits_decimal(bits_per_second: f64) -> String {
    let (number, prefix) = to_largest_prefix(bits_per_second, Base::Decimal);
    match prefix {
        Prefix::None => format!("{} b/s", number.round()),
        Prefix::Kilo => format!("{number:.2} kb/s"),
        Prefix::Mega => format!("{number:.2} Mb/s"),
        Prefix::Giga => format!("{number:.2} Gb/s"),
        Prefix::Tera => format!("{number:.2} Tb/s"),
        Prefix::Peta => format!("{number:.2} Pb/s"),
        Prefix::Exa => format!("{number:.2} Eb/s"),
        Prefix::Zetta => format!("{number:.2} Zb/s"),
        Prefix::Yotta => format!("{number:.2} Yb/s"),
        Prefix::Ronna => format!("{number:.2} Rb/s"),
        Prefix::Quetta => format!("{number:.2} Qb/s"),
    }
}

fn convert_speed_bits_binary(bits_per_second: f64) -> String {
    let (number, prefix) = to_largest_prefix(bits_per_second, Base::Binary);
    match prefix {
        Prefix::None => format!("{} b/s", number.round()),
        Prefix::Kilo => format!("{number:.2} Kib/s"),
        Prefix::Mega => format!("{number:.2} Mib/s"),
        Prefix::Giga => format!("{number:.2} Gib/s"),
        Prefix::Tera => format!("{number:.2} Tib/s"),
        Prefix::Peta => format!("{number:.2} Pib/s"),
        Prefix::Exa => format!("{number:.2} Eib/s"),
        Prefix::Zetta => format!("{number:.2} Zib/s"),
        Prefix::Yotta => format!("{number:.2} Yib/s"),
        Prefix::Ronna => format!("{number:.2} Rib/s"),
        Prefix::Quetta => format!("{number:.2} Qib/s"),
    }
}

pub fn convert_frequency(hertz: f64) -> String {
    let (number, prefix) = to_largest_prefix(hertz, Base::Decimal);
    match prefix {
        Prefix::None => format!("{number:.2} Hz"),
        Prefix::Kilo => format!("{number:.2} kHz"),
        Prefix::Mega => format!("{number:.2} MHz"),
        Prefix::Giga => format!("{number:.2} GHz"),
        Prefix::Tera => format!("{number:.2} THz"),
        Prefix::Peta => format!("{number:.2} PHz"),
        Prefix::Exa => format!("{number:.2} EHz"),
        Prefix::Zetta => format!("{number:.2} ZHz"),
        Prefix::Yotta => format!("{number:.2} YHz"),
        Prefix::Ronna => format!("{number:.2} RHz"),
        Prefix::Quetta => format!("{number:.2} QHz"),
    }
}

pub fn convert_power(watts: f64) -> String {
    let (number, prefix) = to_largest_prefix(watts, Base::Decimal);
    match prefix {
        Prefix::None => format!("{number:.1} W"),
        Prefix::Kilo => format!("{number:.2} kW"),
        Prefix::Mega => format!("{number:.2} MW"),
        Prefix::Giga => format!("{number:.2} GW"),
        Prefix::Tera => format!("{number:.2} TW"),
        Prefix::Peta => format!("{number:.2} PW"),
        Prefix::Exa => format!("{number:.2} EW"),
        Prefix::Zetta => format!("{number:.2} ZW"),
        Prefix::Yotta => format!("{number:.2} YW"),
        Prefix::Ronna => format!("{number:.2} RW"),
        Prefix::Quetta => format!("{number:.2} QW"),
    }
}

pub fn convert_energy(watthours: f64, integer: bool) -> String {
    let (mut number, prefix) = to_largest_prefix(watthours, Base::Decimal);
    if integer {
        number = number.round();
        match prefix {
            Prefix::None => format!("{} Wh", number),
            Prefix::Kilo => format!("{} kWh", number),
            Prefix::Mega => format!("{} MWh", number),
            Prefix::Giga => format!("{} GWh", number),
            Prefix::Tera => format!("{} TWh", number),
            Prefix::Peta => format!("{} PWh", number),
            Prefix::Exa => format!("{} EWh", number),
            Prefix::Zetta => format!("{} ZWh", number),
            Prefix::Yotta => format!("{} YWh", number),
            Prefix::Ronna => format!("{} RWh", number),
            Prefix::Quetta => format!("{} QWh", number),
        }
    } else {
        match prefix {
            Prefix::None => format!("{number:.1} Wh"),
            Prefix::Kilo => format!("{number:.2} kWh"),
            Prefix::Mega => format!("{number:.2} MWh"),
            Prefix::Giga => format!("{number:.2} GWh"),
            Prefix::Tera => format!("{number:.2} TWh"),
            Prefix::Peta => format!("{number:.2} PWh"),
            Prefix::Exa => format!("{number:.2} EWh"),
            Prefix::Zetta => format!("{number:.2} ZWh"),
            Prefix::Yotta => format!("{number:.2} YWh"),
            Prefix::Ronna => format!("{number:.2} RWh"),
            Prefix::Quetta => format!("{number:.2} QWh"),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::sensor::units::convert_seconds;

    #[test]
    fn test_convert_seconds() {
        println!("{}", convert_seconds(0));
        println!("{}", convert_seconds(60));
        println!("{}", convert_seconds(120));
        println!("{}", convert_seconds(200));
        println!("{}", convert_seconds(231));
        println!("{}", convert_seconds(1002));
    }
}
