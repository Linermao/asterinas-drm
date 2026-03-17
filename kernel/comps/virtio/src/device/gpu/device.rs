use alloc::{boxed::Box, sync::Arc, vec, vec::Vec};
use core::{
    cmp::min,
    hint::spin_loop,
    sync::atomic::{AtomicU32, Ordering},
};

use aster_gpu::drm::{
    DrmDevice, DrmDeviceCaps, DrmError, DrmFeatures, MemfdallocatorType, VmaOffsetManager,
    gem::{DrmGemBackend, DrmGemObject},
    ioctl::{DrmModeCreateDumb, DrmModeFbCmd},
    mode_config::{DrmModeConfig, ObjectId},
};
use aster_util::mem_obj_slice::Slice;
use bitflags::bitflags;
use hashbrown::HashMap;
use log::{debug, info, warn};
use ostd::{
    arch::trap::TrapFrame,
    early_println,
    mm::{HasSize, PAGE_SIZE, VmIo, dma::DmaStream},
    sync::{Mutex, SpinLock},
};
use ostd_pod::Pod;

use crate::{
    device::{
        VirtioDeviceError,
        gpu::{
            GpuFeatures, VirtioGpuCmd, VirtioGpuConfig, VirtioGpuCtrlHdr, VirtioGpuDisplayOne,
            VirtioGpuFormat, VirtioGpuGetCapsetInfo, VirtioGpuGetEdid, VirtioGpuMemEntry,
            VirtioGpuQueue, VirtioGpuRect, VirtioGpuResourceAttachBacking,
            VirtioGpuResourceCreate2d, VirtioGpuResourceDetachBacking, VirtioGpuResourceFlush,
            VirtioGpuResourceUnref, VirtioGpuRespCapsetInfo, VirtioGpuRespDisplayInfo,
            VirtioGpuRespEdid, VirtioGpuRespOk, VirtioGpuSetScanout, VirtioGpuTransferToHost2d,
            gem::VirtioGemObject,
            output::{VirtioGpuFramebuffer, virtio_gpu_output_init},
        },
    },
    id_alloc::SyncIdAlloc,
    queue::{QueueError, VirtQueue},
    transport::{ConfigManager, VirtioTransport},
};

bitflags! {
    struct VirtioGpuCaps: u32 {
        const VIRGL_3D = 1 << 0;
        const EDID = 1 << 1;
        const INDIRECT_DESC = 1 << 2;
        const RESOURCE_ASSIGN_UUID = 1 << 3;
        const RESOURCE_BLOB = 1 << 4;
    }
}

const DRIVER_NAME: &'static str = "virtio_gpu";
const DRIVER_DESC: &'static str = "virtio GPU";
const DRIVER_DATE: &'static str = "";

const XRES_MIN: u32 = 32;
const YRES_MIN: u32 = 32;
const XRES_MAX: u32 = 8192;
const YRES_MAX: u32 = 8192;

const CTRL_QUEUE_SIZE: u16 = 64;
const CTRL_REQ_STRIDE: usize = {
    let mut max = size_of::<VirtioGpuCtrlHdr>();
    if size_of::<VirtioGpuGetCapsetInfo>() > max {
        max = size_of::<VirtioGpuGetCapsetInfo>();
    }
    if size_of::<VirtioGpuGetEdid>() > max {
        max = size_of::<VirtioGpuGetEdid>();
    }
    if size_of::<VirtioGpuResourceCreate2d>() > max {
        max = size_of::<VirtioGpuResourceCreate2d>();
    }
    if size_of::<VirtioGpuResourceUnref>() > max {
        max = size_of::<VirtioGpuResourceUnref>();
    }
    if size_of::<VirtioGpuResourceAttachBacking>() > max {
        max = size_of::<VirtioGpuResourceAttachBacking>();
    }
    if size_of::<VirtioGpuResourceDetachBacking>() > max {
        max = size_of::<VirtioGpuResourceDetachBacking>();
    }
    if size_of::<VirtioGpuResourceFlush>() > max {
        max = size_of::<VirtioGpuResourceFlush>();
    }
    if size_of::<VirtioGpuTransferToHost2d>() > max {
        max = size_of::<VirtioGpuTransferToHost2d>();
    }
    if size_of::<VirtioGpuSetScanout>() > max {
        max = size_of::<VirtioGpuSetScanout>();
    }
    max
};
const CTRL_RESP_STRIDE: usize = size_of::<VirtioGpuRespEdid>();
const VIRTIO_RING_F_INDIRECT_DESC: u64 = 1 << 28;

#[derive(Debug)]
pub struct GpuDevice {
    mode_config: Mutex<DrmModeConfig>,
    vma_offset_manager: Mutex<VmaOffsetManager>,

    resource_id_gem: HashMap<Arc<dyn DrmGemBackend>, u32>,

    config_manager: ConfigManager<VirtioGpuConfig>,
    control_queue: SpinLock<VirtQueue>,
    cursor_queue: SpinLock<VirtQueue>,
    transport: SpinLock<Box<dyn VirtioTransport>>,
    ctrl_requests: Arc<DmaStream>,
    ctrl_responses: Arc<DmaStream>,
    id_allocator: SyncIdAlloc,
    next_resource_id: AtomicU32,
    caps: VirtioGpuCaps,
    num_scanouts: SpinLock<u32>,
    display_infos: SpinLock<Vec<VirtioGpuDisplayOne>>,
    display_info_resp: SpinLock<Option<VirtioGpuRespDisplayInfo>>,
    edids: SpinLock<Vec<Option<VirtioGpuRespEdid>>>,
    num_capsets: SpinLock<u32>,
    capset_infos: SpinLock<Vec<VirtioGpuRespCapsetInfo>>,
}

impl GpuDevice {
    pub(crate) fn negotiate_features(device_features: u64) -> u64 {
        let supported_features = GpuFeatures::VIRGL
            | GpuFeatures::EDID
            | GpuFeatures::RESOURCE_UUID
            | GpuFeatures::RESOURCE_BLOB;
        (GpuFeatures::from_bits_truncate(device_features) & supported_features).bits()
    }

    pub(crate) fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let num_queues = transport.num_queues();
        if num_queues < 2 {
            return Err(VirtioDeviceError::QueuesAmountDoNotMatch(num_queues, 2));
        }

        let mut control_queue = VirtQueue::new(
            VirtioGpuQueue::QueueControl as u16,
            CTRL_QUEUE_SIZE,
            transport.as_mut(),
        )
        .expect("create virtio-gpu control queue failed");
        let cursor_queue = VirtQueue::new(
            VirtioGpuQueue::QueueCursor as u16,
            CTRL_QUEUE_SIZE,
            transport.as_mut(),
        )
        .expect("create virtio-gpu cursor queue failed");

        // We currently use synchronous command submission during initialization,
        // so queue callbacks are not required yet.
        control_queue.disable_callback();

        let ctrl_req_frames = (CTRL_REQ_STRIDE * CTRL_QUEUE_SIZE as usize).div_ceil(PAGE_SIZE);
        let ctrl_resp_frames = (CTRL_RESP_STRIDE * CTRL_QUEUE_SIZE as usize).div_ceil(PAGE_SIZE);

        let ctrl_requests = Arc::new(DmaStream::alloc(ctrl_req_frames, false).unwrap());
        let ctrl_responses = Arc::new(DmaStream::alloc(ctrl_resp_frames, false).unwrap());
        assert!(CTRL_REQ_STRIDE * CTRL_QUEUE_SIZE as usize <= ctrl_requests.size());
        assert!(CTRL_RESP_STRIDE * CTRL_QUEUE_SIZE as usize <= ctrl_responses.size());

        let device_features = transport.read_device_features();
        let gpu_features = GpuFeatures::from_bits_truncate(device_features);
        let mut caps = VirtioGpuCaps::empty();
        if cfg!(target_endian = "little") && gpu_features.contains(GpuFeatures::VIRGL) {
            caps.insert(VirtioGpuCaps::VIRGL_3D);
        }
        if gpu_features.contains(GpuFeatures::EDID) {
            caps.insert(VirtioGpuCaps::EDID);
        }
        if (device_features & VIRTIO_RING_F_INDIRECT_DESC) != 0 {
            caps.insert(VirtioGpuCaps::INDIRECT_DESC);
        }
        if gpu_features.contains(GpuFeatures::RESOURCE_UUID) {
            caps.insert(VirtioGpuCaps::RESOURCE_ASSIGN_UUID);
        }
        if gpu_features.contains(GpuFeatures::RESOURCE_BLOB) {
            caps.insert(VirtioGpuCaps::RESOURCE_BLOB);
        }

        let config_manager = VirtioGpuConfig::new_manager(transport.as_ref());
        let config = config_manager.read_config();

        let num_scanouts = config.num_scanouts;
        let num_capsets = config.num_capsets;

        let mut mode_config = DrmModeConfig::new(
            XRES_MIN,
            XRES_MAX, 
            YRES_MIN, 
            YRES_MAX,
            16,
            0,
            0,
            0,
            true,
            false,
        );

        for scanout_id in 0..num_scanouts {
            virtio_gpu_output_init(scanout_id, &mut mode_config)
                .map_err(|_| VirtioDeviceError::QueueUnknownError)?;
        }

        let device = Arc::new(Self {
            mode_config: Mutex::new(mode_config),
            vma_offset_manager: Mutex::new(VmaOffsetManager::new()),

            resource_id_gem: HashMap::new(),

            config_manager,
            control_queue: SpinLock::new(control_queue),
            cursor_queue: SpinLock::new(cursor_queue),
            transport: SpinLock::new(transport),
            ctrl_requests,
            ctrl_responses,
            id_allocator: SyncIdAlloc::with_capacity(CTRL_QUEUE_SIZE as usize),
            next_resource_id: AtomicU32::new(1),
            caps,
            num_scanouts: SpinLock::new(num_scanouts),
            display_infos: SpinLock::new(Vec::new()),
            display_info_resp: SpinLock::new(None),
            edids: SpinLock::new(Vec::new()),
            num_capsets: SpinLock::new(num_capsets),
            capset_infos: SpinLock::new(Vec::new()),
        });

        {
            fn config_space_change(_: &TrapFrame) {
                debug!("virtio-gpu config space changed");
            }

            let mut transport = device.transport.lock();
            transport
                .register_cfg_callback(Box::new(config_space_change))
                .unwrap();
            transport.finish_init();
        }

        // Register this bus-level GPU device to the common GPU subsystem.
        if let Err(err) = aster_gpu::register_device(device.clone()) {
            warn!(
                "failed to register virtio-gpu device into gpu subsystem: {:?}",
                err
            );
        }

        let mut capset_infos = Vec::new();
        if config.num_capsets > 0 {
            for capset_index in 0..config.num_capsets {
                match device.get_capset_info(capset_index) {
                    Ok(info) => {
                        info!(
                            "virtio-gpu capset[{capset_index}]: id={}, max_version={}, max_size={}",
                            info.capset_id, info.capset_max_version, info.capset_max_size
                        );
                        capset_infos.push(info);
                    }
                    Err(err) => {
                        warn!(
                            "virtio-gpu get capset info failed at index {capset_index}: {:?}",
                            err
                        );
                    }
                }
            }
        }
        *device.capset_infos.lock() = capset_infos;

        let mut display_infos = Vec::new();
        let mut edids = vec![None; config.num_scanouts as usize];

        if config.num_scanouts > 0 {
            if device.has_edid() {
                for scanout in 0..config.num_scanouts {
                    match device.get_edid(scanout) {
                        Ok(edid) => {
                            let edid_size = min(edid.size as usize, edid.edid.len());
                            info!("virtio-gpu edid fetched: scanout={scanout}, size={edid_size}");
                            edids[scanout as usize] = Some(edid);
                        }
                        Err(err) => {
                            warn!("virtio-gpu get edid failed at scanout {scanout}: {:?}", err);
                        }
                    }
                }
            }

            match device.get_display_info() {
                Ok(resp) => {
                    *device.display_info_resp.lock() = Some(resp);
                    let scanout_count = min(config.num_scanouts as usize, resp.pmodes.len());
                    display_infos.extend_from_slice(&resp.pmodes[..scanout_count]);

                    let active_scanouts = display_infos
                        .iter()
                        .filter(|mode| {
                            mode.enabled != 0 && mode.rect.width != 0 && mode.rect.height != 0
                        })
                        .count();
                    info!("virtio-gpu display info fetched, active scanouts={active_scanouts}");
                }
                Err(err) => {
                    warn!("virtio-gpu get display info failed: {:?}", err);
                }
            }
        }

        *device.display_infos.lock() = display_infos;
        *device.edids.lock() = edids;

        Ok(())
    }
}

impl DrmDevice for GpuDevice {
    fn name(&self) -> &str {
        DRIVER_NAME
    }

    fn desc(&self) -> &str {
        DRIVER_DESC
    }

    fn date(&self) -> &str {
        DRIVER_DATE
    }

    fn features(&self) -> DrmFeatures {
        DrmFeatures::GEM
            | DrmFeatures::MODESET
            | DrmFeatures::RENDER
            | DrmFeatures::ATOMIC
            | DrmFeatures::SYNCOBJ
            | DrmFeatures::SYNCOBJ_TIMELINE
            | DrmFeatures::CURSOR_HOTSPOT
    }

    fn capbilities(&self) -> DrmDeviceCaps {
        DrmDeviceCaps::DUMB_CREATE
    }

    fn mode_config(&self) -> &Mutex<DrmModeConfig> {
        &self.mode_config
    }

    fn vma_offset_manager(&self) -> &Mutex<VmaOffsetManager> {
        &self.vma_offset_manager 
    }

    fn create_dumb(
        &self,
        args: &DrmModeCreateDumb,
        memfd_allocator: MemfdallocatorType,
    ) -> Result<Arc<dyn DrmGemObject>, DrmError> {
        if args.bpp != 32 {
            return Err(DrmError::Invalid);
        }

        let pitch = args.width.checked_mul(4).ok_or(DrmError::Invalid)?;
        let size = pitch.checked_mul(args.height).ok_or(DrmError::Invalid)?;
        let size = size as u64;

        let backend = memfd_allocator("virtio-gpu-dumb", size)?;
        let sg_table = backend.get_pages_sgt()?;

        let resource_id = self.alloc_resource_id();
        self.resource_create_2d(resource_id, args.width, args.height)
            .map_err(|_| DrmError::Invalid)?;

        let entries: Vec<VirtioGpuMemEntry> = sg_table
            .entries
            .iter()
            .map(|e| VirtioGpuMemEntry {
                addr: e.addr,
                length: e.len,
                padding: 0,
            })
            .collect();

        self.resource_attach_backing_sg(resource_id, entries.as_slice())
            .map_err(|_| DrmError::Invalid)?;

        let gem_object = VirtioGemObject::new(pitch, size, backend, resource_id);

        Ok(Arc::new(gem_object))
    }

    fn map_dumb(&self, handle: u32) -> Result<u64, DrmError> {
        self.vma_offset_manager.lock().alloc(handle)
    }

    fn fb_create(
        &self,
        fb_cmd: &DrmModeFbCmd,
        gem_object: Arc<dyn DrmGemObject>,
    ) -> Result<ObjectId, DrmError> {
        let fb = Arc::new(VirtioGpuFramebuffer::new(
            fb_cmd.width,
            fb_cmd.height,
            gem_object,
        ));

        let handle = self.mode_config.lock().add_framebuffer(fb);

        Ok(handle)
    }
}

impl GpuDevice {
    pub fn alloc_resource_id(&self) -> u32 {
        self.next_resource_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn get_display_info(&self) -> Result<VirtioGpuRespDisplayInfo, VirtioGpuCommandError> {
        let req = VirtioGpuCtrlHdr {
            type_: VirtioGpuCmd::GetDisplayInfo as u32,
            ..Default::default()
        };
        self.submit_control_command::<VirtioGpuCtrlHdr, VirtioGpuRespDisplayInfo>(
            &req,
            VirtioGpuRespOk::DisplayInfo as u32,
        )
    }

    pub fn get_capset_info(
        &self,
        capset_index: u32,
    ) -> Result<VirtioGpuRespCapsetInfo, VirtioGpuCommandError> {
        let req = VirtioGpuGetCapsetInfo {
            hdr: VirtioGpuCtrlHdr {
                type_: VirtioGpuCmd::GetCapsetInfo as u32,
                ..Default::default()
            },
            capset_index,
            padding: 0,
        };
        self.submit_control_command::<VirtioGpuGetCapsetInfo, VirtioGpuRespCapsetInfo>(
            &req,
            VirtioGpuRespOk::CapsetInfo as u32,
        )
    }

    pub fn get_edid(&self, scanout: u32) -> Result<VirtioGpuRespEdid, VirtioGpuCommandError> {
        let req = VirtioGpuGetEdid {
            hdr: VirtioGpuCtrlHdr {
                type_: VirtioGpuCmd::GetEdid as u32,
                ..Default::default()
            },
            scanout,
            padding: 0,
        };
        self.submit_control_command::<VirtioGpuGetEdid, VirtioGpuRespEdid>(
            &req,
            VirtioGpuRespOk::Edid as u32,
        )
    }

    pub fn resource_create_2d(
        &self,
        resource_id: u32,
        width: u32,
        height: u32,
    ) -> Result<(), VirtioGpuCommandError> {
        let req = VirtioGpuResourceCreate2d {
            hdr: VirtioGpuCtrlHdr {
                type_: VirtioGpuCmd::ResourceCreate2D as u32,
                ..Default::default()
            },
            resource_id,
            format: VirtioGpuFormat::B8G8R8X8Unorm as u32,
            width,
            height,
        };
        let _: VirtioGpuCtrlHdr = self
            .submit_control_command::<VirtioGpuResourceCreate2d, VirtioGpuCtrlHdr>(
                &req,
                VirtioGpuRespOk::Nodata as u32,
            )?;
        Ok(())
    }

    pub fn resource_attach_backing(
        &self,
        resource_id: u32,
        addr: u64,
        length: u32,
    ) -> Result<(), VirtioGpuCommandError> {
        let req = VirtioGpuResourceAttachBacking {
            hdr: VirtioGpuCtrlHdr {
                type_: VirtioGpuCmd::ResourceAttachBacking as u32,
                ..Default::default()
            },
            resource_id,
            nr_entries: 1,
            entries: [VirtioGpuMemEntry {
                addr,
                length,
                padding: 0,
            }],
        };
        let _: VirtioGpuCtrlHdr = self
            .submit_control_command::<VirtioGpuResourceAttachBacking, VirtioGpuCtrlHdr>(
                &req,
                VirtioGpuRespOk::Nodata as u32,
            )?;
        Ok(())
    }

    pub fn resource_unref(&self, resource_id: u32) -> Result<(), VirtioGpuCommandError> {
        let req = VirtioGpuResourceUnref {
            hdr: VirtioGpuCtrlHdr {
                type_: VirtioGpuCmd::ResourceUnref as u32,
                ..Default::default()
            },
            resource_id,
            padding: 0,
        };
        let _: VirtioGpuCtrlHdr = self
            .submit_control_command::<VirtioGpuResourceUnref, VirtioGpuCtrlHdr>(
                &req,
                VirtioGpuRespOk::Nodata as u32,
            )?;
        Ok(())
    }

    pub fn resource_detach_backing(&self, resource_id: u32) -> Result<(), VirtioGpuCommandError> {
        let req = VirtioGpuResourceDetachBacking {
            hdr: VirtioGpuCtrlHdr {
                type_: VirtioGpuCmd::ResourceDetachBacking as u32,
                ..Default::default()
            },
            resource_id,
            padding: 0,
        };
        let _: VirtioGpuCtrlHdr = self
            .submit_control_command::<VirtioGpuResourceDetachBacking, VirtioGpuCtrlHdr>(
                &req,
                VirtioGpuRespOk::Nodata as u32,
            )?;
        Ok(())
    }

    pub fn resource_attach_backing_sg(
        &self,
        resource_id: u32,
        entries: &[VirtioGpuMemEntry],
    ) -> Result<(), VirtioGpuCommandError> {
        if entries.is_empty() {
            return Err(VirtioGpuCommandError::InvalidParameter);
        }

        let nr_entries =
            u32::try_from(entries.len()).map_err(|_| VirtioGpuCommandError::InvalidParameter)?;

        let hdr = VirtioGpuResourceAttachBackingHdr {
            hdr: VirtioGpuCtrlHdr {
                type_: VirtioGpuCmd::ResourceAttachBacking as u32,
                ..Default::default()
            },
            resource_id,
            nr_entries,
        };

        let req_len = size_of::<VirtioGpuResourceAttachBackingHdr>()
            + entries.len() * size_of::<VirtioGpuMemEntry>();
        let req_frames = req_len.div_ceil(PAGE_SIZE);
        let req_dma = Arc::new(DmaStream::alloc(req_frames, false).unwrap());
        let req_slice = Slice::new(req_dma, 0..req_len);

        req_slice.write_val(0, &hdr).unwrap();
        let mut off = size_of::<VirtioGpuResourceAttachBackingHdr>();
        for entry in entries {
            req_slice.write_val(off, entry).unwrap();
            off += size_of::<VirtioGpuMemEntry>();
        }
        req_slice.sync_to_device().unwrap();

        let resp_len = size_of::<VirtioGpuCtrlHdr>();
        let resp_frames = resp_len.div_ceil(PAGE_SIZE);
        let resp_dma = Arc::new(DmaStream::alloc(resp_frames, false).unwrap());
        let resp_slice = Slice::new(resp_dma, 0..resp_len);
        resp_slice
            .write_val(0, &VirtioGpuCtrlHdr::default())
            .unwrap();
        resp_slice.sync_to_device().unwrap();

        let token = loop {
            let mut queue = self.control_queue.disable_irq().lock();
            if queue.available_desc() >= 2 {
                let token = queue
                    .add_dma_buf(&[&req_slice], &[&resp_slice])
                    .map_err(VirtioGpuCommandError::Queue)?;
                if queue.should_notify() {
                    queue.notify();
                }
                break token;
            }
            drop(queue);
            spin_loop();
        };

        loop {
            let mut queue = self.control_queue.disable_irq().lock();
            if !queue.can_pop() {
                drop(queue);
                spin_loop();
                continue;
            }
            queue
                .pop_used_with_token(token)
                .map_err(VirtioGpuCommandError::Queue)?;
            break;
        }

        resp_slice.sync_from_device().unwrap();
        let resp_hdr: VirtioGpuCtrlHdr = resp_slice.read_val(0).unwrap();
        if resp_hdr.type_ != VirtioGpuRespOk::Nodata as u32 {
            return Err(VirtioGpuCommandError::UnexpectedResponse(resp_hdr.type_));
        }

        Ok(())
    }

    pub fn resource_flush(
        &self,
        resource_id: u32,
        rect: VirtioGpuRect,
    ) -> Result<(), VirtioGpuCommandError> {
        let req = VirtioGpuResourceFlush {
            hdr: VirtioGpuCtrlHdr {
                type_: VirtioGpuCmd::ResourceFlush as u32,
                ..Default::default()
            },
            rect,
            resource_id,
            _padding: 0,
        };
        let _: VirtioGpuCtrlHdr = self
            .submit_control_command::<VirtioGpuResourceFlush, VirtioGpuCtrlHdr>(
                &req,
                VirtioGpuRespOk::Nodata as u32,
            )?;
        Ok(())
    }

    pub fn transfer_to_host_2d(
        &self,
        resource_id: u32,
        rect: VirtioGpuRect,
        offset: u64,
    ) -> Result<(), VirtioGpuCommandError> {
        let req = VirtioGpuTransferToHost2d {
            hdr: VirtioGpuCtrlHdr {
                type_: VirtioGpuCmd::TransferToHost2D as u32,
                ..Default::default()
            },
            rect,
            offset,
            resource_id,
            _padding: 0,
        };
        let _: VirtioGpuCtrlHdr = self
            .submit_control_command::<VirtioGpuTransferToHost2d, VirtioGpuCtrlHdr>(
                &req,
                VirtioGpuRespOk::Nodata as u32,
            )?;
        Ok(())
    }

    pub fn set_scanout(
        &self,
        scanout_id: u32,
        resource_id: u32,
        rect: VirtioGpuRect,
    ) -> Result<(), VirtioGpuCommandError> {
        let req = VirtioGpuSetScanout {
            hdr: VirtioGpuCtrlHdr {
                type_: VirtioGpuCmd::SetScanout as u32,
                ..Default::default()
            },
            rect,
            scanout_id,
            resource_id,
        };
        let _: VirtioGpuCtrlHdr = self
            .submit_control_command::<VirtioGpuSetScanout, VirtioGpuCtrlHdr>(
                &req,
                VirtioGpuRespOk::Nodata as u32,
            )?;
        Ok(())
    }

    pub fn num_queues(&self) -> usize {
        2
    }

    pub fn num_capsets(&self) -> u32 {
        *self.num_capsets.lock()
    }

    pub fn num_scanouts(&self) -> u32 {
        *self.num_scanouts.lock()
    }

    pub fn has_virgl_3d(&self) -> bool {
        self.caps.contains(VirtioGpuCaps::VIRGL_3D)
    }

    pub fn has_edid(&self) -> bool {
        self.caps.contains(VirtioGpuCaps::EDID)
    }

    pub fn has_indirect(&self) -> bool {
        self.caps.contains(VirtioGpuCaps::INDIRECT_DESC)
    }

    pub fn has_resource_assign_uuid(&self) -> bool {
        self.caps.contains(VirtioGpuCaps::RESOURCE_ASSIGN_UUID)
    }

    pub fn has_resource_blob(&self) -> bool {
        self.caps.contains(VirtioGpuCaps::RESOURCE_BLOB)
    }

    pub fn display_infos(&self) -> Vec<VirtioGpuDisplayOne> {
        self.display_infos.lock().clone()
    }

    pub fn edids(&self) -> Vec<Option<VirtioGpuRespEdid>> {
        self.edids.lock().clone()
    }

    pub fn display_info_resp(&self) -> Option<VirtioGpuRespDisplayInfo> {
        *self.display_info_resp.lock()
    }

    pub fn capset_infos(&self) -> Vec<VirtioGpuRespCapsetInfo> {
        self.capset_infos.lock().clone()
    }

    pub(crate) fn submit_control_command<Req, Resp>(
        &self,
        req: &Req,
        expected_resp_type: u32,
    ) -> Result<Resp, VirtioGpuCommandError>
    where
        Req: Pod,
        Resp: Pod + Default,
    {
        if size_of::<Req>() > CTRL_REQ_STRIDE {
            early_println!(
                "[virtio-gpu] error: control command request size {} exceeds the stride {}",
                size_of::<Req>(),
                CTRL_REQ_STRIDE
            );
            return Err(VirtioGpuCommandError::RequestTooLarge(size_of::<Req>()));
        }
        if size_of::<Resp>() > CTRL_RESP_STRIDE {
            early_println!(
                "[virtio-gpu] error: control command response size {} exceeds the stride {}",
                size_of::<Resp>(),
                CTRL_RESP_STRIDE
            );
            return Err(VirtioGpuCommandError::ResponseTooLarge(size_of::<Resp>()));
        }

        let id = self.id_allocator.alloc();

        let req_slice = {
            let req_slice = Slice::new(
                self.ctrl_requests.clone(),
                id * CTRL_REQ_STRIDE..(id + 1) * CTRL_REQ_STRIDE,
            );
            req_slice.write_val(0, req).unwrap();
            req_slice.sync_to_device().unwrap();
            req_slice
        };

        let resp_slice = {
            let resp_slice = Slice::new(
                self.ctrl_responses.clone(),
                id * CTRL_RESP_STRIDE..(id + 1) * CTRL_RESP_STRIDE,
            );
            let default_resp = Resp::default();
            resp_slice.write_val(0, &default_resp).unwrap();
            resp_slice.sync_to_device().unwrap();
            resp_slice
        };

        let token = loop {
            let mut queue = self.control_queue.disable_irq().lock();
            if queue.available_desc() >= 2 {
                let token = queue
                    .add_dma_buf(&[&req_slice], &[&resp_slice])
                    .map_err(VirtioGpuCommandError::Queue)?;
                if queue.should_notify() {
                    queue.notify();
                }
                break token;
            }
            drop(queue);
            spin_loop();
        };

        loop {
            let mut queue = self.control_queue.disable_irq().lock();
            if !queue.can_pop() {
                drop(queue);
                spin_loop();
                continue;
            }
            queue
                .pop_used_with_token(token)
                .map_err(VirtioGpuCommandError::Queue)?;
            break;
        }

        self.id_allocator.dealloc(id);

        resp_slice.sync_from_device().unwrap();
        let resp_hdr: VirtioGpuCtrlHdr = resp_slice.read_val(0).unwrap();
        if resp_hdr.type_ != expected_resp_type && resp_hdr.type_ != VirtioGpuRespOk::Nodata as u32
        {
            return Err(VirtioGpuCommandError::UnexpectedResponse(resp_hdr.type_));
        }

        let resp: Resp = resp_slice.read_val(0).unwrap();
        Ok(resp)
    }

    pub fn has_cursor_queue(&self) -> bool {
        let queue = self.cursor_queue.lock();
        queue.size() > 0
    }
}

#[derive(Debug)]
pub enum VirtioGpuCommandError {
    Queue(QueueError),
    InvalidParameter,
    RequestTooLarge(usize),
    ResponseTooLarge(usize),
    UnexpectedResponse(u32),
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
struct VirtioGpuResourceAttachBackingHdr {
    hdr: VirtioGpuCtrlHdr,
    resource_id: u32,
    nr_entries: u32,
}
