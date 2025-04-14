use std::time::Instant;

#[derive(Debug)]
pub(super) struct Acked {
    pub(super) pkt_num: u64,
    pub(super) time_sent: Instant,
}

#[derive(Debug)]
pub(super) struct Lost {
    pub(super) packet_number: u64,
    pub(super) bytes_lost: usize,
}