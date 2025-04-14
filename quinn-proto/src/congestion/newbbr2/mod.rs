mod bbr2;
mod bandwith;
mod bandwith_samper;
mod drain;
mod minmax;
mod mode;
mod network_model;
mod probe_bw;
mod probe_rtt;
mod rtt;
mod startup;
mod utils;
mod windowed_filter;

use self::bbr2::BBRv2;
use self::rtt::{INITIAL_RTT, RttStats};
use std::any::Any;
use std::collections::{BTreeMap, HashSet};
use std::{sync::Arc, time::Instant};
use self::utils::{Acked,Lost};
use self::bandwith::Bandwidth;
use super::{Controller, ControllerFactory, BASE_DATAGRAM_SIZE};
use std::time::Duration;
use crate::connection::RttEstimator;
// Congestion Control
const INITIAL_WINDOW_PACKETS: usize = 10;
const MAX_WINDOW_PACKETS: usize = 20_000;

/// Configuration for the [`Bbr`] congestion controller
#[derive(Debug, Clone)]
pub struct NewBbr2Config {
    initial_congestion_window: usize, 
    max_congestion_window: usize,
    max_segment_size: usize, 
    smoothed_rtt: Duration,
}

impl NewBbr2Config {
    pub fn initial_window(&mut self, value: u64) -> &mut Self {
        self.initial_congestion_window = value as usize;
        self.max_congestion_window = MAX_WINDOW_PACKETS;
        self.max_segment_size = BASE_DATAGRAM_SIZE as usize;
        self.smoothed_rtt = INITIAL_RTT;
        self
    }
}

impl Default for NewBbr2Config {
    fn default() -> Self {
        Self {
            initial_congestion_window: INITIAL_WINDOW_PACKETS,
            max_congestion_window: MAX_WINDOW_PACKETS,
            max_segment_size: BASE_DATAGRAM_SIZE as usize,
            smoothed_rtt: INITIAL_RTT
        }
    }
}

impl ControllerFactory for NewBbr2Config {
    fn build(self: Arc<Self>, now: Instant, current_mtu: u16) -> Box<dyn Controller> {
        Box::new(NewBbr2::new(self, now, current_mtu))
    }
}


#[derive(Debug, Clone)]
pub struct NewBbr2 {
    mybbr2: BBRv2,
    config: Arc<NewBbr2Config>,
    prior_in_flight: usize,
    bytes_in_flight: usize,
    rtt_stats: RttStats,
    has_received_ack: bool,
    next_acked_pkt_num: u64,
    // max_acked_pkt_time: Instant,
}

impl NewBbr2 {
    pub fn new(config: Arc<NewBbr2Config>, now: Instant, current_mtu: u16) -> Self {
        let mut mybbr2 = BBRv2::new(
            config.initial_congestion_window,
            config.max_congestion_window,
            config.max_segment_size.max(current_mtu as usize),
            config.smoothed_rtt,
        );
        let mut rtt_stats = RttStats::new(Duration::from_millis(0));
        Self {
            mybbr2,
            config,
            prior_in_flight:0,
            bytes_in_flight:0,
            rtt_stats,
            has_received_ack: false,
            next_acked_pkt_num:0,
            // max_acked_pkt_time:now,
        }
    }

    fn generate_packet_lists(
        &mut self,
        ack_begin_num: u64,
        ack_end_num: u64,
        new_bbr2_lost: &Vec<(u64, usize)>,
        // sent_packets: &BTreeMap<u64, Instant>,
    ) -> (Vec<Acked>, Vec<Lost>) {
        let mut acked_packets = Vec::new();
        let mut lost_packets = Vec::new();

        // 收集丢失的数据包
        for (packet_number, bytes_lost) in new_bbr2_lost {
            if *packet_number >= ack_begin_num && *packet_number <= ack_end_num {
                lost_packets.push(Lost {
                    packet_number: *packet_number,
                    bytes_lost: *bytes_lost,
                });
            }
        }

        // 收集已确认的数据包，排除丢失的数据包
        let lost_set: HashSet<u64> = lost_packets.iter().map(|lost| lost.packet_number).collect();
        // eprintln!("lost_set:{:?}", lost_set);
        for pkt_num in ack_begin_num..=ack_end_num {
            if !lost_set.contains(&pkt_num) {
                acked_packets.push(Acked {
                    pkt_num,
                    time_sent: Instant::now(),// new bbr2没用这个，所以我们直接赋值now就行了
                });
            }
        }

        (acked_packets, lost_packets)
    }
}

impl Controller for NewBbr2 {
    
    fn rtt_update(&mut self, latest_rtt: Duration, mut ack_delay: Duration, 
        now: Instant, handshake_confirmed: bool){
        self.rtt_stats.update_rtt(latest_rtt, ack_delay, now, handshake_confirmed);
    }
    fn can_send(&self, bytes_in_flight: usize) -> bool {
        self.mybbr2.can_send(bytes_in_flight)
    }

    fn on_sent_info(&mut self, sent_time: std::time::Instant, bytes_in_flight: usize,
        packet_number: u64, bytes: usize, is_retransmissible: bool) {
        // eprintln!("=====on_sent_info=======, now:{:?}", Instant::now());
        self.mybbr2.on_packet_sent(sent_time, bytes_in_flight, packet_number, bytes, is_retransmissible, &self.rtt_stats);
    }

    fn on_new_bbr2_congestion(&mut self, prior_in_flight: usize, bytes_in_flight: usize, event_time: Instant,
        max_acked_pkt_num: u64, max_acked_pkts_acked_time: Instant, new_bbr2_lost: & Vec<(u64, usize)>,least_unacked: u64) {
        // eprintln!("=====on_new_bbr2_congestion=======");
        let rtt_updated = false;// new bbr2没用，所以这里可以随便定义
        let (acked_packets, lost_packets) =self.generate_packet_lists(self.next_acked_pkt_num, max_acked_pkt_num, & new_bbr2_lost);
        // eprintln!("self.max_acked_pkt_num:{}, max_acked_pkt_num:{}, len of acked_packets:{}, len of lost_packets:{}",
        // self.next_acked_pkt_num, max_acked_pkt_num, acked_packets.len(), lost_packets.len());
        // eprintln!("sent_packets map is :{:?}", sent_packets);
        let new_bbr2_acked_packets: &[Acked] = &acked_packets;
        let new_bbr2_lost_packets: &[Lost] =  &lost_packets;
        
        self.mybbr2.on_congestion_event(rtt_updated, prior_in_flight, bytes_in_flight, event_time, new_bbr2_acked_packets, new_bbr2_lost_packets, least_unacked, &self.rtt_stats);
        self.next_acked_pkt_num = max_acked_pkt_num+1;
        // self.max_acked_pkt_time = max_acked_pkts_acked_time;
    }

    fn on_app_limited(&mut self, bytes_in_flight: usize, app_limited: bool) {
        if app_limited {
            self.mybbr2.on_app_limited(bytes_in_flight);
        }
    }


    fn on_sent(&mut self, now: Instant, bytes: u64, last_packet_number: u64) {
        return;
    }
    fn on_ack(
        &mut self,
        now: Instant,
        sent: Instant,
        bytes: u64,
        app_limited: bool,
        rtt: &RttEstimator,
    ) { 
        return;
    }
    
    fn on_congestion_event(
        &mut self,
        now: Instant,
        sent: Instant,
        is_persistent_congestion: bool,
        _lost_bytes: u64,
    ) {
        let mut rtt_update: bool = false; // new bbr2没用这个，所以这个可以自己定义

    }

    fn on_mtu_update(&mut self, new_mtu: u16) {
        // 这个mss和mtu概念是不一样的，但换成窗口的话其实还可以啦
        self.mybbr2.update_mss(new_mtu as usize);
    }

    fn window(&self) -> u64 {
        let wid = self.mybbr2.get_congestion_window() as u64;
        // eprintln!("congestion window is {}", wid);
        wid
        
    }

    fn clone_box(&self) -> Box<dyn Controller> {
        Box::new(self.clone())
    }

    fn initial_window(&self) -> u64 {
        self.config.initial_congestion_window as u64 * BASE_DATAGRAM_SIZE
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }

    fn pacing_window(&self) -> u64 {
        // self.mybbr2.get_congestion_window() as u64
        let wd :u64 = (self.mybbr2.pacing_rate(0,&self.rtt_stats).to_bits_per_second() as f64 / 8.0 // B/s
         * (self.rtt_stats.rtt().as_millis() as f64 / 1000.0)) as u64; // 乘以一个Rtt
        // eprintln!("pacing_window is {}, pacing_rate is {} bps", wd, (self.mybbr2.pacing_rate(0,&self.rtt_stats).to_bits_per_second()));
        wd
    }
}

#[enum_dispatch::enum_dispatch]
pub(super) trait CCLL{
    /// Returns the size of the current congestion window in bytes. Note, this
    /// is not the *available* window. Some send algorithms may not use a
    /// congestion window and will return 0.
    fn get_congestion_window(&self) -> usize;

    /// Returns the size of the current congestion window in packets. Note, this
    /// is not the *available* window. Some send algorithms may not use a
    /// congestion window and will return 0.
    fn get_congestion_window_in_packets(&self) -> usize;

    /// Make decision on whether the sender can send right now.  Note that even
    /// when this method returns true, the sending can be delayed due to pacing.
    fn can_send(&self, bytes_in_flight: usize) -> bool;

    /// Inform that we sent `bytes` to the wire, and if the packet is
    /// retransmittable. `bytes_in_flight` is the number of bytes in flight
    /// before the packet was sent. Note: this function must be called for
    /// every packet sent to the wire.
    fn on_packet_sent(
        &mut self, sent_time: Instant, bytes_in_flight: usize,
        packet_number: u64, bytes: usize, is_retransmissible: bool,
        rtt_stats: &RttStats,
    );

    /// Inform that `packet_number` has been neutered.
    fn on_packet_neutered(&mut self, _packet_number: u64) {}

    /// Indicates an update to the congestion state, caused either by an
    /// incoming ack or loss event timeout. `rtt_updated` indicates whether a
    /// new `latest_rtt` sample has been taken, `prior_in_flight` the bytes in
    /// flight prior to the congestion event. `acked_packets` and `lost_packets`
    /// are any packets considered acked or lost as a result of the
    /// congestion event.
    #[allow(clippy::too_many_arguments)]
    fn on_congestion_event(
        &mut self, rtt_updated: bool, prior_in_flight: usize,
        bytes_in_flight: usize, event_time: Instant, acked_packets: &[Acked],
        lost_packets: &[Lost], least_unacked: u64, rtt_stats: &RttStats,
    );

    /// Called when an RTO fires.  Resets the retransmission alarm if there are
    /// remaining unacked packets.
    fn on_retransmission_timeout(&mut self, packets_retransmitted: bool);

    /// Called when connection migrates and cwnd needs to be reset.
    #[allow(dead_code)]
    fn on_connection_migration(&mut self);

    /// Adjust the current cwnd to a new maximal size
    fn limit_cwnd(&mut self, _max_cwnd: usize) {}

    fn is_in_recovery(&self) -> bool;

    #[allow(dead_code)]
    fn is_cwnd_limited(&self, bytes_in_flight: usize) -> bool;

    #[cfg(test)]
    fn is_app_limited(&self, bytes_in_flight: usize) -> bool {
        !self.is_cwnd_limited(bytes_in_flight)
    }

    fn pacing_rate(
        &self, bytes_in_flight: usize, rtt_stats: &RttStats,
    ) -> Bandwidth;

    fn bandwidth_estimate(&self, rtt_stats: &RttStats) -> Bandwidth;

    fn update_mss(&mut self, new_mss: usize);

    fn on_app_limited(&mut self, _bytes_in_flight: usize) {}

    #[cfg(feature = "qlog")]
    fn ssthresh(&self) -> Option<u64> {
        None
    }
}