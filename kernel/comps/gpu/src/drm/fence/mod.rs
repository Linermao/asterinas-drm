/// Minimal in-kernel dma_fence-like abstraction.
///
/// This intentionally models only the essentials needed by syncobj import/
/// export paths today: identity (context/seqno) and signaling state.
struct DmaFence {
    context: u64,
    seqno: u64,
}
