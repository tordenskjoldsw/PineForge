#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderMetrics {
    pub total_us: u64,
    pub compose_us: u64,
    pub transfer_us: u64,
    pub max_stripe_us: u64,
    pub stripe_count: u8,
}

/// How much of a paint is the bus and how much is the core, taken at boot.
///
/// A transition reports one "transfer" figure that covers pulling colours from
/// an iterator, packing them into bytes and moving those bytes over SPI. The
/// 8 MHz bus owes 115.2 ms for a frame, so the excess over that belongs to
/// something the figure cannot name. These are the parts, measured apart.
///
/// Taken once, before the panel is up, and never touched again - which is why
/// this screen can show them safely. Nothing that happens while they are on the
/// panel can make them stale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BusBench {
    /// A frame's worth of bytes onto the bus, one entry per write size, in
    /// ascending order from 255 bytes doubling to 4,080. Microseconds.
    pub bus_us: [u64; 5],
    /// A frame's worth of colours packed into bytes with no bus involved,
    /// through a concrete iterator and through a `dyn` one. The difference is
    /// what a vtable per pixel costs. Microseconds.
    pub pack_concrete_us: u64,
    pub pack_dyn_us: u64,
}
