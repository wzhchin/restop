use anyhow::Result;
use hashbrown::HashMap;
use once_cell::sync::Lazy;

use regex::Regex;

use std::path::PathBuf;

use crate::sensor::{
    pci::{self, Device, PciSlot},
    IS_FLATPAK,
};

use super::GpuImpl;

static RE_AMDGPU_IDS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"([0-9A-F]{4}),\s*([0-9A-F]{2}),\s*(.*)").unwrap());

static AMDGPU_IDS: Lazy<HashMap<(u16, u8), String>> =
    Lazy::new(|| AmdGpu::read_libdrm_ids().unwrap_or_default());

#[derive(Debug, Clone, Default)]

pub struct AmdGpu {
    pub device: Option<&'static Device>,
    pub pci_slot: PciSlot,
    pub driver: String,
    sysfs_path: PathBuf,
    first_hwmon_path: Option<PathBuf>,
}

impl AmdGpu {
    pub fn new(
        device: Option<&'static Device>,
        pci_slot: PciSlot,
        driver: String,
        sysfs_path: PathBuf,
        first_hwmon_path: Option<PathBuf>,
    ) -> Self {
        Self {
            device,
            pci_slot,
            driver,
            sysfs_path,
            first_hwmon_path,
        }
    }

    pub fn read_libdrm_ids() -> Result<HashMap<(u16, u8), String>> {
        let path = if *IS_FLATPAK {
            PathBuf::from("/run/host/usr/share/libdrm/amdgpu.ids")
        } else {
            PathBuf::from("/usr/share/libdrm/amdgpu.ids")
        };

        let mut map = HashMap::new();

        let amdgpu_ids_raw = std::fs::read_to_string(path)?;

        for capture in RE_AMDGPU_IDS.captures_iter(&amdgpu_ids_raw) {
            if let (Some(device_id), Some(revision), Some(name)) =
                (capture.get(1), capture.get(2), capture.get(3))
            {
                let device_id = u16::from_str_radix(device_id.as_str().trim(), 16).unwrap();
                let revision = u8::from_str_radix(revision.as_str().trim(), 16).unwrap();
                let name = name.as_str().into();
                map.insert((device_id, revision), name);
            }
        }

        Ok(map)
    }
}

impl GpuImpl for AmdGpu {
    fn device(&self) -> Option<&'static Device> {
        self.device
    }

    fn pci_slot(&self) -> PciSlot {
        self.pci_slot
    }

    fn driver(&self) -> String {
        self.driver.clone()
    }

    fn sysfs_path(&self) -> PathBuf {
        self.sysfs_path.clone()
    }

    fn first_hwmon(&self) -> Option<PathBuf> {
        self.first_hwmon_path.clone()
    }

    fn name(&self) -> Result<String> {
        let revision =
            u8::from_str_radix(&self.read_device_file("revision")?.replace("0x", ""), 16)?;
        Ok(AMDGPU_IDS
            .get(&(self.device().map_or(0, pci::Device::pid), revision))
            .cloned()
            .unwrap_or_else(|| {
                if let Ok(drm_name) = self.drm_name() {
                    format!("AMD Radeon Graphics ({drm_name})")
                } else {
                    "AMD Radeon Graphics".into()
                }
            }))
    }

    fn usage(&self) -> Result<isize> {
        self.drm_usage()
    }

    /// AMD exposes a single VCN block (shared by encode + decode) as
    /// `vcn_busy_percent`. Fall back to overall GPU busy percent if absent.
    fn vcn_usage(&self) -> Result<isize> {
        self.read_device_int("vcn_busy_percent").or_else(|_| self.drm_usage())
    }

    fn used_vram(&self) -> Result<isize> {
        self.drm_used_vram()
    }

    fn total_vram(&self) -> Result<isize> {
        self.drm_total_vram()
    }

    fn temperature(&self) -> Result<f64> {
        self.hwmon_temperature()
    }

    fn power_usage(&self) -> Result<f64> {
        self.hwmon_power_usage()
    }

    fn core_frequency(&self) -> Result<f64> {
        // hwmon `freq1_input` is sclk in Hz; fall back to the active pp_dpm_sclk state.
        self.hwmon_core_frequency()
            .or_else(|_| self.read_pp_dpm("pp_dpm_sclk").map(|(active, _)| active))
    }

    fn vram_frequency(&self) -> Result<f64> {
        // hwmon `freq2_input` (mclk) is often missing on AMD; use the active
        // mclk power state from `pp_dpm_mclk` instead.
        self.hwmon_vram_frequency()
            .or_else(|_| self.read_pp_dpm("pp_dpm_mclk").map(|(active, _)| active))
    }

    fn max_core_frequency(&self) -> Result<f64> {
        self.read_pp_dpm("pp_dpm_sclk").map(|(_, max)| max)
    }

    fn power_cap(&self) -> Result<f64> {
        self.hwmon_power_cap()
    }

    fn power_cap_max(&self) -> Result<f64> {
        self.hwmon_power_cap_max()
    }
}
