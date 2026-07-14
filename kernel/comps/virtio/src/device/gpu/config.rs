// SPDX-License-Identifier: MPL-2.0

use core::mem::offset_of;

use crate::transport::{ConfigManager, VirtioTransport};

bitflags::bitflags! {
    /// VirtIO GPU features defined by the specification.
    pub(super) struct VirtioGpuFeatures: u64 {
        const VIRGL = 1 << 0;
        const EDID = 1 << 1;
        const RESOURCE_UUID = 1 << 2;
        const RESOURCE_BLOB = 1 << 3;
        const CONTEXT_INIT = 1 << 4;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioGpuConfig {
    events_read: u32,
    events_clear: u32,
    num_scanouts: u32,
    num_capsets: u32,
}

impl VirtioGpuConfig {
    pub(super) fn new_manager(transport: &dyn VirtioTransport) -> ConfigManager<Self> {
        let safe_ptr = transport.device_config_mem().map(|mem| {
            // The config starts from offset 0.
            aster_util::safe_ptr::SafePtr::new(mem, 0)
        });
        let bar_space = transport.device_config_bar();
        ConfigManager::new(safe_ptr, bar_space)
    }

    #[expect(dead_code)]
    pub(super) fn events_read(&self) -> u32 {
        self.events_read
    }

    #[expect(dead_code)]
    pub(super) fn events_clear(&self) -> u32 {
        self.events_clear
    }

    pub(super) fn num_scanouts(&self) -> u32 {
        self.num_scanouts
    }

    pub(super) fn num_capsets(&self) -> u32 {
        self.num_capsets
    }
}

impl ConfigManager<VirtioGpuConfig> {
    pub(super) fn read_config(&self) -> VirtioGpuConfig {
        VirtioGpuConfig {
            events_read: self
                .read_once::<u32>(offset_of!(VirtioGpuConfig, events_read))
                .unwrap(),
            events_clear: self
                .read_once::<u32>(offset_of!(VirtioGpuConfig, events_clear))
                .unwrap(),
            num_scanouts: self
                .read_once::<u32>(offset_of!(VirtioGpuConfig, num_scanouts))
                .unwrap(),
            num_capsets: self
                .read_once::<u32>(offset_of!(VirtioGpuConfig, num_capsets))
                .unwrap(),
        }
    }
}
