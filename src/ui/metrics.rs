#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderMetrics {
    pub total_us: u64,
    pub compose_us: u64,
    pub transfer_us: u64,
    pub max_stripe_us: u64,
    pub stripe_count: u8,
}
