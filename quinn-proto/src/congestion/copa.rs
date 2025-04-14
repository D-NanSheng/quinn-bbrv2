use std::time::Instant;

// 定义 CopaDirection 枚举
#[derive(Clone, Copy, Debug, PartialEq)]
enum CopaDirection {
    Undef,
    Up,
    Down,
}

// 定义 CopaMode 枚举
#[derive(Clone, Copy, Debug, PartialEq)]
enum CopaMode {
    DelayMode,
    CompetitiveMode,
}

// 定义 WinFilterMin 结构体，用于跟踪最小 RTT
struct WinFilterMin {
    min_rtt: u64,
    window_duration: u64,
    last_update_time: Instant,
}

impl WinFilterMin {
    fn new(window_duration: u64) -> Self {
        WinFilterMin {
            min_rtt: u64::MAX,
            window_duration,
            last_update_time: Instant::now(),
        }
    }

    fn update(&mut self, now: Instant, rtt: u64) {
        if now.duration_since(self.last_update_time).as_millis() as u64 > self.window_duration {
            self.min_rtt = u64::MAX;
            self.last_update_time = now;
        }
        if rtt < self.min_rtt {
            self.min_rtt = rtt;
        }
    }

    fn get(&self) -> u64 {
        self.min_rtt
    }
}

// 定义 WinFilterMax 结构体，用于跟踪最大 RTT
struct WinFilterMax {
    max_rtt: u64,
    round_count: u32,
    last_round_count: u32,
}

impl WinFilterMax {
    fn new() -> Self {
        WinFilterMax {
            max_rtt: 0,
            round_count: 0,
            last_round_count: 0,
        }
    }

    fn update(&mut self, round_count: u32, rtt: u64) {
        if round_count != self.last_round_count {
            self.max_rtt = 0;
            self.last_round_count = round_count;
        }
        if rtt > self.max_rtt {
            self.max_rtt = rtt;
        }
    }

    fn get(&self) -> u64 {
        self.max_rtt
    }
}

// 定义 RttEstimator 结构体，用于估计 RTT
struct RttEstimator {
    latest_rtt: u64,
    srtt: u64,
}

// 定义 Copa 结构体
pub struct Copa {
    delta_ai_unit: f64,
    delta_base: f64,
    delta_max: f64,
    delta: f64,
    init_cwnd_bytes: u64,
    cwnd_bytes: u64,
    last_round_cwnd_bytes: u64,
    pacing_rate: u64,
    rtt_standing: WinFilterMin,
    rtt_min: WinFilterMin,
    rtt_max: WinFilterMax,
    v: f64,
    curr_dir: CopaDirection,
    prev_dir: CopaDirection,
    same_dir_cnt: u32,
    mode: CopaMode,
    t_last_delay_min: Instant,
    in_slow_start: bool,
    recovery_start_time: Option<Instant>,
    round_cnt: u32,
    next_round_threshold: u64,
    round_start: bool,
    cwnd_adjustment_accumulated: i64,
    current_mtu: u64,
}

impl Copa {
    // 初始化 Copa 结构体
    pub fn new(current_mtu: u64) -> Self {
        let init_cwnd_bytes = 32 * current_mtu;
        let rtt_min_window = 10 * 1000; // 10 seconds in milliseconds
        let rtt_sta_window = 0.5;
        let rtt_max_window = 4;

        Copa {
            delta_ai_unit: 1.0,
            delta_base: 0.05,
            delta_max: 0.5,
            delta: 0.05,
            init_cwnd_bytes,
            cwnd_bytes: init_cwnd_bytes,
            last_round_cwnd_bytes: 0,
            pacing_rate: 0,
            rtt_standing: WinFilterMin::new((rtt_sta_window * (current_mtu as f64)) as u64),
            rtt_min: WinFilterMin::new(rtt_min_window),
            rtt_max: WinFilterMax::new(),
            v: 1.0,
            curr_dir: CopaDirection::Undef,
            prev_dir: CopaDirection::Undef,
            same_dir_cnt: 0,
            mode: CopaMode::DelayMode,
            t_last_delay_min: Instant::now(),
            in_slow_start: true,
            recovery_start_time: None,
            round_cnt: 0,
            next_round_threshold: 0,
            round_start: false,
            cwnd_adjustment_accumulated: 0,
            current_mtu,
        }
    }

    // 设置 pacing rate
    fn set_pacing_rate(&mut self) {
        let rtt_standing = self.rtt_standing.get();
        if rtt_standing == u64::MAX {
            // 这里简单处理，可根据实际情况调整
            self.pacing_rate = (2 * self.cwnd_bytes * 1_000_000) / self.current_mtu;
        } else {
            self.pacing_rate = (2 * self.cwnd_bytes * 1_000_000) / rtt_standing;
        }
        self.pacing_rate = self.pacing_rate.max(self.current_mtu);
    }

    // 处理 ACK 事件
    pub fn on_ack(&mut self, now: Instant, sent: Instant, bytes: u64, app_limited: bool, rtt_estimator: &RttEstimator) {
        let latest_rtt = rtt_estimator.latest_rtt;
        let srtt = rtt_estimator.srtt;

        // 更新总确认字节数
        let total_acked = self.next_round_threshold + bytes;
        if total_acked >= self.next_round_threshold {
            self.round_cnt += 1;
            self.next_round_threshold = total_acked;
            self.round_start = true;
        } else {
            self.round_start = false;
        }

        // 处理恢复状态
        if let Some(recovery_start_time) = self.recovery_start_time {
            if sent > recovery_start_time {
                self.recovery_start_time = None;
            }
        }

        // 更新 RTT 统计信息
        self.rtt_min.update(now, latest_rtt);
        self.rtt_standing.update(now, latest_rtt);
        self.rtt_max.update(self.round_cnt, latest_rtt);

        // 计算延迟和目标速率
        let rtt_standing = self.rtt_standing.get();
        let rtt_min = self.rtt_min.get();
        if rtt_standing < rtt_min {
            return;
        }
        let delay = rtt_standing - rtt_min;

        if delay <= (self.rtt_max.get() - rtt_min) / 10 {
            self.t_last_delay_min = now;
            if self.mode != CopaMode::DelayMode {
                self.mode = CopaMode::DelayMode;
                self.delta = self.delta_base;
            }
        }

        let target_rate = if delay == 0 {
            u64::MAX as f64
        } else {
            (self.current_mtu as f64 * 1_000_000.0) / (delay as f64 * self.delta)
        };
        let current_rate = (self.cwnd_bytes as f64 * 1_000_000.0) / rtt_standing as f64;

        // 慢启动阶段
        if self.in_slow_start {
            if current_rate > target_rate {
                self.in_slow_start = false;
            } else {
                if bytes > self.cwnd_bytes {
                    self.cwnd_bytes *= 2;
                } else {
                    self.cwnd_bytes += bytes;
                }
                self.set_pacing_rate();
                return;
            }
        }

        // 稳态阶段
        if self.round_start && self.mode != CopaMode::CompetitiveMode && now.duration_since(self.t_last_delay_min).as_millis() as u64 >= (5 * srtt) {
            self.mode = CopaMode::CompetitiveMode;
        }

        if self.round_start {
            let new_dir = if self.cwnd_bytes > self.last_round_cwnd_bytes {
                CopaDirection::Up
            } else {
                CopaDirection::Down
            };

            self.last_round_cwnd_bytes = self.cwnd_bytes;

            if new_dir != self.curr_dir {
                self.v = 1.0;
                self.same_dir_cnt = 0;
            } else {
                self.same_dir_cnt += 1;
            }

            self.prev_dir = self.curr_dir;
            self.curr_dir = new_dir;

            if self.same_dir_cnt > 3 {
                self.v *= 2.0;
            }

            if (self.v * self.current_mtu as f64) >= (self.delta * self.cwnd_bytes as f64) {
                self.v /= 2.0;
            }
            self.v = self.v.max(1.0);
        }

        let aiad_sign = if current_rate > target_rate {
            -1
        } else {
            1
        };

        let numerator_bytes = (self.v * bytes as f64 / self.delta) as u64;
        self.cwnd_adjustment_accumulated += aiad_sign * numerator_bytes as i64;

        if self.cwnd_adjustment_accumulated > 0 && self.cwnd_adjustment_accumulated >= self.cwnd_bytes as i64 {
            let d = self.cwnd_adjustment_accumulated / self.cwnd_bytes as i64;
            self.cwnd_adjustment_accumulated -= d * self.cwnd_bytes as i64;
            self.cwnd_bytes += (d * self.current_mtu) as u64;
        } else if self.cwnd_adjustment_accumulated < 0 && self.cwnd_adjustment_accumulated <= -self.cwnd_bytes as i64 {
            let d = -self.cwnd_adjustment_accumulated / self.cwnd_bytes as i64;
            self.cwnd_adjustment_accumulated += d * self.cwnd_bytes as i64;
            let d = d * self.current_mtu;
            if d <= self.cwnd_bytes {
                self.cwnd_bytes -= d;
            } else {
                self.cwnd_bytes = 0;
            }
        }

        self.cwnd_bytes = self.cwnd_bytes.max(4 * self.current_mtu);
        self.set_pacing_rate();
    }

    // 处理拥塞事件
    pub fn on_congestion_event(&mut self, sent_time: Instant, is_persistent: bool) {
        if let Some(recovery_start_time) = self.recovery_start_time {
            if sent_time < recovery_start_time {
                return;
            }
        }

        self.recovery_start_time = Some(Instant::now());
        if self.mode == CopaMode::CompetitiveMode {
            self.delta = (self.delta * 2.0).min(self.delta_max);
        }

        if is_persistent {
            self.rtt_min = WinFilterMin::new(10 * 1000);
            self.rtt_max = WinFilterMax::new();
            self.rtt_standing = WinFilterMin::new((0.5 * self.current_mtu as f64) as u64);
            self.v = 1.0;
            self.curr_dir = CopaDirection::Undef;
            self.prev_dir = CopaDirection::Undef;
            self.same_dir_cnt = 0;
            self.mode = CopaMode::DelayMode;
            self.t_last_delay_min = Instant::now();
            self.in_slow_start = true;
            self.delta = self.delta_base;
            self.cwnd_adjustment_accumulated = 0;
            self.cwnd_bytes = 4 * self.current_mtu;
            self.set_pacing_rate();
        }
    }

    // 更新 MTU
    pub fn on_mtu_update(&mut self, new_mtu: u64) {
        self.current_mtu = new_mtu;
        self.cwnd_bytes = self.cwnd_bytes.max(4 * new_mtu);
        self.set_pacing_rate();
    }

    // 获取当前拥塞窗口大小
    pub fn window(&self) -> u64 {
        self.cwnd_bytes
    }

    // 获取初始拥塞窗口大小
    pub fn initial_window(&self) -> u64 {
        self.init_cwnd_bytes
    }

    // 获取 pacing window，这里简单返回当前拥塞窗口大小
    pub fn pacing_window(&self) -> u64 {
        self.cwnd_bytes
    }
}    