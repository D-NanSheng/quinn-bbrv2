//! Logic for controlling the rate at which data is sent

use crate::connection::RttEstimator;
use std::collections::BTreeMap;
use std::{any::Any, time::Duration};
use std::sync::Arc;
use std::time::Instant;

mod bbr;
mod cubic;
mod new_reno;
mod bbr2;
mod newbbr2;

pub use bbr::{Bbr, BbrConfig};
pub use cubic::{Cubic, CubicConfig};
pub use new_reno::{NewReno, NewRenoConfig};
pub use  bbr2::{Bbr2, BbrConfig2};
pub use newbbr2::{NewBbr2,NewBbr2Config};

/// Common interface for different congestion controllers
pub trait Controller: Send + Sync {
    /// One or more packets were just sent
    #[allow(unused_variables)]
    fn on_sent(&mut self, now: Instant, bytes: u64, last_packet_number: u64) {}

    /// Packet deliveries were confirmed
    ///
    /// `app_limited` indicates whether the connection was blocked on outgoing
    /// application data prior to receiving these acknowledgements.
    #[allow(unused_variables)]
    fn on_ack(
        &mut self,
        now: Instant,
        sent: Instant,
        bytes: u64,
        app_limited: bool,
        rtt: &RttEstimator,
    ) {
    }

    /// Packets are acked in batches, all with the same `now` argument. This indicates one of those batches has completed.
    #[allow(unused_variables)]
    fn on_end_acks(
        &mut self,
        now: Instant,
        in_flight: u64,
        app_limited: bool,
        largest_packet_num_acked: Option<u64>,
    ) {
    }

    /// Packets were deemed lost or marked congested
    ///
    /// `in_persistent_congestion` indicates whether all packets sent within the persistent
    /// congestion threshold period ending when the most recent packet in this batch was sent were
    /// lost.
    /// `lost_bytes` indicates how many bytes were lost. This value will be 0 for ECN triggers.
    fn on_congestion_event(
        &mut self,
        now: Instant,
        sent: Instant,
        is_persistent_congestion: bool,
        lost_bytes: u64,
    );

    /// The known MTU for the current network path has been updated
    fn on_mtu_update(&mut self, new_mtu: u16);

    /// Number of ack-eliciting bytes that may be in flight
    fn window(&self) -> u64;

    /// Duplicate the controller's state
    fn clone_box(&self) -> Box<dyn Controller>;

    /// Initial congestion window
    fn initial_window(&self) -> u64;

    /// Returns Self for use in down-casting to extract implementation details
    fn into_any(self: Box<Self>) -> Box<dyn Any>;

    /// return pacing window for connection/pacing
    fn pacing_window(&self) -> u64;

    // update rtt stats for new bbr2
    fn rtt_update(&mut self, latest_rtt: Duration, mut ack_delay: Duration, now: Instant, handshake_confirmed: bool){}

    // cansend for new bbr2
    fn can_send(&self, bytes_in_flight: usize) -> bool {true}

    // sent packet info for new bbr2
    fn on_sent_info( &mut self, sent_time: std::time::Instant, bytes_in_flight: usize,
        packet_number: u64, bytes: usize, is_retransmissible: bool) {}
    
    // congestion info including info of lost pks
    fn on_new_bbr2_congestion(&mut self, prior_in_flight: usize, bytes_in_flight: usize, event_time: Instant,
    max_acked_pkt_num: u64, max_acked_pkts_acked_time: Instant, new_bbr2_lost: & Vec<(u64, usize)>,least_unacked: u64) {}

    // for new bbr2
    fn on_app_limited(&mut self, bytes_in_flight: usize, app_limited: bool) {}
}

/// Constructs controllers on demand
pub trait ControllerFactory {
    /// Construct a fresh `Controller`
    fn build(self: Arc<Self>, now: Instant, current_mtu: u16) -> Box<dyn Controller>;
}

const BASE_DATAGRAM_SIZE: u64 = 1400; // 如果mtu没有更新，那么我们的算法会依据这个来计算。 此时，如果最小数据包大于BASE_DATAGRAM_SIZE，就会导致无法发送数据包，继而导致连接断开。
