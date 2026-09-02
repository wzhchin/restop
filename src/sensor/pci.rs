use std::{
    collections::BTreeMap,
    error::Error,
    fmt::{self, Display},
    io::BufRead,
    str::FromStr,
};

use anyhow::{Context, Result};
use append_only_vec::AppendOnlyVec;
use log::error;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

/// A PCI domain:bus:device.function address.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, Default, Hash, PartialEq, Eq, PartialOrd, Ord,
)]
pub struct PciSlot {
    pub domain: u16,
    pub bus: u8,
    pub number: u8,
    pub function: u8,
}

impl PciSlot {
    pub fn new(domain: u16, bus: u8, number: u8, function: u8) -> Self {
        Self {
            domain,
            bus,
            number,
            function,
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ParseError(String);

impl Error for ParseError {}

impl Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unable to parse to PCI ID")
    }
}

impl FromStr for PciSlot {
    type Err = ParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let dot_split: Vec<&str> = value.split('.').collect();
        if dot_split.len() != 2 {
            return Err(ParseError("amount of '.' != 1".into()));
        }

        let colon_split: Vec<&str> = dot_split[0].split(':').collect();
        if colon_split.len() != 3 {
            return Err(ParseError("amount of ':' != 2".into()));
        }

        let domain = u16::from_str_radix(colon_split[0], 16)
            .map_err(|_| ParseError("unable to parse domain".into()))?;
        let bus = u8::from_str_radix(colon_split[1], 16)
            .map_err(|_| ParseError("unable to parse bus".into()))?;
        let number = u8::from_str_radix(colon_split[2], 16)
            .map_err(|_| ParseError("unable to parse number".into()))?;
        let function = u8::from_str_radix(dot_split[1], 16)
            .map_err(|_| ParseError("unable to parse function".into()))?;

        Ok(Self {
            domain,
            bus,
            number,
            function,
        })
    }
}

impl Display for PciSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04x}:{:02x}:{:02x}.{:x}",
            self.domain, self.bus, self.number, self.function
        )
    }
}

static DEVICES: Lazy<AppendOnlyVec<Device>> = Lazy::new(AppendOnlyVec::new);

pub fn get_device(vid: &u16, pid: &u16) -> Option<&'static Device> {
    let vendor = DEVICES.iter().find(|e| e.vendor_id == *vid && e.id == *pid);
    if vendor.is_none() {
        match get_device_raw(vid, pid) {
            Ok(dev) => {
                DEVICES.push(dev);
            }
            Err(err) => {
                error!("unable to read devices: {err}");
            }
        }
    }

    DEVICES.iter().find(|e| e.vendor_id == *vid && e.id == *pid)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subdevice {
    id: u16,
    vendor_id: u16,
    name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    id: u16,
    name: String,

    vendor_id: u16,
    pub vendor_name: String,

    sub_devices: Vec<Subdevice>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vendor {
    id: u16,
    name: String,
    devices: BTreeMap<u16, Device>,
}

impl Device {
    pub fn subdevices(&self) -> impl Iterator<Item = &Subdevice> {
        self.sub_devices.iter()
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn pid(&self) -> u16 {
        self.id
    }
}

impl Vendor {
    pub fn devices(&self) -> impl Iterator<Item = &Device> {
        self.devices.values()
    }

    pub fn get_device(&self, pid: u16) -> Option<&Device> {
        self.devices.get(&pid)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn vid(&self) -> u16 {
        self.id
    }
}

fn get_device_raw(in_vid: &u16, in_pid: &u16) -> Result<Device> {
    // first check if we can use flatpak's FS to get to the (probably newer) host's pci.ids file
    //
    // if that doesn't work, we're either not on flatpak or we're not allowed to see the host's pci.ids for some reason,
    // so try to either access flatpak's own (probably older) pci.ids or the host's if we're not on flatpak
    let file = std::fs::File::open("/run/host/usr/share/hwdata/pci.ids")
        .or_else(|_| std::fs::File::open("/usr/share/hwdata/pci.ids"))?;

    // debug!("Parsing pci.ids…");

    let reader = std::io::BufReader::new(file);

    let mut o_vendor_name: Option<String> = None;
    let mut o_device: Option<Device> = None;
    let mut o_vid = None;
    let mut breakp = false;

    for line in reader.lines().map_while(Result::ok) {
        if line.starts_with('C') {
            // case 1: we've reached the classes, time to stop
            break;
        } else if line.starts_with('#') || line.is_empty() {
            // case 2: we're seeing a comment, don't care
            // case 3: we're seeing an empty line, also don't care
            continue;
        } else if line.starts_with("\t\t") {
            if o_device.is_some() {
                // case 4: we're seeing a new sub device of the last seen device
                let mut split = line.trim_start().splitn(4, ' ');

                let sub_vid = u16::from_str_radix(
                    split
                        .next()
                        .with_context(|| format!("this subdevice has no vid (line: {line})"))?,
                    16,
                )?;

                let sub_pid = u16::from_str_radix(
                    split
                        .next()
                        .with_context(|| format!("this subdevice has no pid (line: {line})"))?,
                    16,
                )?;

                let name = split
                    .last()
                    .map(str::to_string)
                    .with_context(|| format!("this vendor has no name (line: {line})"))?;

                let subdevice = Subdevice {
                    id: sub_pid,
                    vendor_id: sub_vid,
                    name,
                };

                if let Some(dev) = o_device.as_mut() {
                    dev.sub_devices.push(subdevice);
                }
                breakp = true;
            }
        } else if line.starts_with('\t') {
            if o_vid.is_some() {
                // case 5: we're seeing a new device of the last seen vendor
                if breakp {
                    break;
                }

                let mut split = line.trim_start().split("  ");

                let pid = u16::from_str_radix(
                    split
                        .next()
                        .with_context(|| format!("this device has no pid (line: {line})"))?,
                    16,
                )?;

                if *in_pid == pid {
                    let name = split
                        .next()
                        .map(str::to_string)
                        .with_context(|| format!("this vendor has no name (line: {line})"))?;

                    let device = Device {
                        id: pid,
                        vendor_id: *in_vid,
                        name,
                        sub_devices: Vec::new(),
                        vendor_name: o_vendor_name.as_ref().unwrap().to_string(),
                    };

                    o_device.replace(device);
                }
            }
        } else if !line.starts_with('\t') {
            if breakp {
                break;
            }

            // case 6: we're seeing a new vendor
            let mut split = line.split("  ");

            let vid = u16::from_str_radix(
                split
                    .next()
                    .with_context(|| format!("this vendor has no vid (line: {line})"))?,
                16,
            )?;

            if *in_vid == vid {
                let name = split
                    .next()
                    .map(str::to_string)
                    .with_context(|| format!("this vendor has no name (line: {line})"))?;

                o_vid.replace(*in_pid);
                o_vendor_name.replace(name);
            }
        }
    }

    o_device.context("Unable to fine this device")
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::PciSlot;

    #[test]
    fn pci_id_from_string() {
        let pci_id = PciSlot::new(0x0, 0x1, 0xfe, 0x3);
        assert_eq!(pci_id, PciSlot::from_str("0000:01:fe.3").unwrap());
    }

    #[test]
    fn pci_id_to_string() {
        let pci_id = PciSlot::new(0x0, 0x1, 0xfe, 0x3);
        assert_eq!("0000:01:fe.3", pci_id.to_string());
    }
}
