bitflags::bitflags! {
    pub struct DrmAtomicFlags: u32 {
        const PAGE_FLIP_EVENT    = 0x0001;
        const PAGE_FLIP_ASYNC    = 0x0002;

        const TEST_ONLY          = 0x0100;
        const NONBLOCK           = 0x0200;
        const ALLOW_MODESET      = 0x0400;
    }
}