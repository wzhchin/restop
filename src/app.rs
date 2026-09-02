use std::{
    io::{stdout, Stdout},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use chin_tools::AResult;
use crossterm::{
    event::{read, Event, KeyEvent},
    execute,
    terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate},
};

use flume::{Receiver, RecvTimeoutError, Sender};
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal};

use crate::{
    resource::{
        battery::ResBattery, cpu::ResCPU, drive::ResDrive, gpu::ResGPU, memory::ResMEM,
        network::ResNetwork, process::ResProcess, HardwareWorker, ResourceType, SensorRsp,
    },
    sensor::settings::SETTINGS,
    utils::is_ctrl_c,
    view::{
        sidebar_and_page::SidebarAndPage,
        theme::{SharedTheme, Theme},
        LayoutType, Navigator, NavigatorArgs,
    },
};

bitflags::bitflags! {
    #[derive(Debug, PartialOrd, PartialEq, Eq, Clone, Copy, Hash)]
    pub struct RedrawEventEnum: u8 {
        const SENSOR = 0b0000_0001;
        const INTERVAL = 0b0000_0010;
        const TERM = 0b0000_0100;
    }
}

/// Idle `recv_timeout` does not request a redraw. Sensor ticks and
/// real terminal input do.
pub(crate) fn recv_timeout_redraw_flags() -> RedrawEventEnum {
    RedrawEventEnum::empty()
}

const REDRAW_THROTTLE: Duration = Duration::from_millis(300);
const SENSOR_REDRAW_DEBOUNCE: Duration = Duration::from_millis(50);

fn redraw_is_due(
    events: RedrawEventEnum,
    now: Instant,
    last_draw_at: Option<Instant>,
    last_sensor_event_at: Option<Instant>,
) -> bool {
    if events.contains(RedrawEventEnum::TERM) {
        return true;
    }

    let has_refresh = events.intersects(RedrawEventEnum::SENSOR | RedrawEventEnum::INTERVAL);
    let sensor_burst_is_quiet =
        last_sensor_event_at.is_none_or(|last| now.duration_since(last) >= SENSOR_REDRAW_DEBOUNCE);
    let redraw_is_throttled =
        last_draw_at.is_some_and(|last| now.duration_since(last) <= REDRAW_THROTTLE);

    has_refresh && sensor_burst_is_quiet && !redraw_is_throttled
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DueSamples {
    hardware: bool,
    process: bool,
}

#[derive(Debug)]
struct SamplingSchedule {
    hardware_interval: Duration,
    process_interval: Duration,
    last_hardware: Option<Duration>,
    last_process: Option<Duration>,
}

impl SamplingSchedule {
    fn new(hardware_interval: Duration, process_interval: Duration) -> Self {
        Self {
            hardware_interval,
            process_interval,
            last_hardware: None,
            last_process: None,
        }
    }

    fn take_due(&mut self, elapsed: Duration) -> DueSamples {
        let hardware = self
            .last_hardware
            .is_none_or(|last| elapsed.saturating_sub(last) >= self.hardware_interval);
        let process = self
            .last_process
            .is_none_or(|last| elapsed.saturating_sub(last) >= self.process_interval);

        if hardware {
            self.last_hardware = Some(elapsed);
        }
        if process {
            self.last_process = Some(elapsed);
        }

        DueSamples { hardware, process }
    }
}

pub struct ResTop {
    resources: Vec<ResourceType>,
    focused_index: Option<usize>,

    layout: LayoutType,

    res_tx: Sender<ResourceEvent>,
    res_rx: Receiver<ResourceEvent>,
}

pub enum ResourceEvent {
    Resize(u16, u16),
    KeyEvent(KeyEvent),
    SensorRsp(Arc<SensorRsp>),
    FocusedIndex(usize),
    Quit,
}

impl ResTop {
    pub fn new() -> AResult<Self> {
        let theme = SharedTheme::new(Theme::default());
        let (tx, rx) = flume::unbounded::<ResourceEvent>();

        let mut resources = vec![];

        if let Ok(p) = ResProcess::spawn(theme.clone(), &tx) {
            resources.push(ResourceType::Process(p));
        }

        resources.push(ResourceType::CPU(ResCPU::new(theme.clone())?));
        resources.push(ResourceType::Memory(ResMEM::new(theme.clone())?));
        let gpus = ResGPU::new(theme.clone())?;
        gpus.into_iter().for_each(|g| {
            resources.push(ResourceType::GPU(g));
        });

        let drives = ResDrive::new(theme.clone())?;
        drives.into_iter().for_each(|d| {
            resources.push(ResourceType::Drive(d));
        });

        let nets = ResNetwork::new(theme.clone())?;
        nets.into_iter()
            .for_each(|d| resources.push(ResourceType::Network(d)));

        let bats = ResBattery::new(theme.clone())?;
        bats.into_iter()
            .for_each(|b| resources.push(ResourceType::Battery(b)));

        Ok(ResTop {
            resources,
            res_tx: tx,
            res_rx: rx,
            focused_index: None,
            layout: LayoutType::SidebarAndPage(SidebarAndPage::default()),
        })
    }

    pub fn handle_key(&mut self, key: &KeyEvent) {
        self.layout.handle_event(
            &crate::view::NavigatorEvent::KeyEvent(*key),
            NavigatorArgs {
                resources: &mut self.resources,
            },
        )
    }

    pub fn run(&mut self, term: &mut Terminal<CrosstermBackend<Stdout>>) -> AResult<()> {
        let mut event_enum = RedrawEventEnum::all();

        let hardware_worker = HardwareWorker::spawn(&self.res_tx);
        let worker_tx = &hardware_worker.tx;

        {
            let tx = self.res_tx.clone();
            thread::Builder::new()
                .name("termevent".to_string())
                .spawn(move || loop {
                    if let Ok(e) = read() {
                        match e {
                            Event::Key(key) => {
                                if is_ctrl_c(&key) {
                                    let _ = tx.send(ResourceEvent::Quit);
                                } else {
                                    let _ = tx.send(ResourceEvent::KeyEvent(key));
                                }
                            }
                            Event::Resize(w, h) => {
                                let _ = tx.send(ResourceEvent::Resize(w, h));
                            }
                            _event => {}
                        }
                    }
                })
                .unwrap();
        }

        let intervals = SETTINGS.refresh_intervals();
        let started_at = Instant::now();
        let mut schedule = SamplingSchedule::new(intervals.hardware, intervals.process);
        let mut last_draw_at = None;
        let mut last_sensor_event_at = None;

        loop {
            let now = Instant::now();
            let due = schedule.take_due(now.duration_since(started_at));

            if due.hardware || due.process {
                for ele in &self.resources {
                    let should_fetch = if ele.is_process() {
                        due.process
                    } else {
                        due.hardware
                    };
                    if should_fetch {
                        ele.fetch_data(worker_tx);
                    }
                }
            }

            // 2026-09-02 sensor-redraw-debounce
            // Hardware and process workers finish independently. Waiting for a short quiet
            // period collapses one sampling round into one frame; terminal input stays instant.
            let should_draw = redraw_is_due(event_enum, now, last_draw_at, last_sensor_event_at);

            if should_draw {
                last_draw_at = Some(now);

                let draw_result = term.draw(|f| {
                    let render_result: AResult<()> = (|| {
                        execute!(stdout(), BeginSynchronizedUpdate)?;

                        self.layout
                            .render(f, &mut self.resources, self.focused_index);

                        Ok(())
                    })();

                    if let Err(err) = render_result {
                        log::error!("unable to render {}", err);
                    }
                });
                if let Err(err) = draw_result {
                    log::error!("Unable to draw components: {}", err);
                };

                execute!(stdout(), EndSynchronizedUpdate)?;
                event_enum = RedrawEventEnum::empty();
                last_sensor_event_at = None;
            }

            // Block for the first event, then drain the rest without blocking
            // so a sensor flood becomes one update + one draw.
            match self.res_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(rsp) => {
                    if matches!(&rsp, ResourceEvent::SensorRsp(_)) {
                        last_sensor_event_at = Some(Instant::now());
                    }
                    if self.apply_event(rsp, &mut event_enum) {
                        break;
                    }
                    while let Ok(rsp) = self.res_rx.try_recv() {
                        if matches!(&rsp, ResourceEvent::SensorRsp(_)) {
                            last_sensor_event_at = Some(Instant::now());
                        }
                        if self.apply_event(rsp, &mut event_enum) {
                            return Ok(());
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    event_enum = event_enum.union(recv_timeout_redraw_flags());
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Apply one resource event. Returns true if the app should quit.
    fn apply_event(&mut self, rsp: ResourceEvent, event_enum: &mut RedrawEventEnum) -> bool {
        match rsp {
            ResourceEvent::Resize(w, h) => {
                self.layout.update_layout(Rect {
                    x: 0,
                    y: 0,
                    width: w,
                    height: h,
                });
                *event_enum = event_enum.union(RedrawEventEnum::TERM);
            }
            ResourceEvent::KeyEvent(key) => {
                self.handle_key(&key);
                *event_enum = event_enum.union(RedrawEventEnum::TERM);
            }
            ResourceEvent::SensorRsp(rsp) => {
                for ele in &mut self.resources {
                    if ele.updata_data(&rsp) {
                        break;
                    }
                }
                *event_enum = event_enum.union(RedrawEventEnum::SENSOR);
            }
            ResourceEvent::Quit => {
                return true;
            }
            ResourceEvent::FocusedIndex(focused_index) => {
                self.focused_index.replace(focused_index);
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{recv_timeout_redraw_flags, redraw_is_due, RedrawEventEnum, SamplingSchedule};

    #[test]
    fn recv_timeout_does_not_request_redraw() {
        let flags = recv_timeout_redraw_flags();
        assert!(flags.is_empty());
        assert!(!flags.contains(RedrawEventEnum::INTERVAL));
        assert!(!flags.contains(RedrawEventEnum::SENSOR));
        assert!(!flags.contains(RedrawEventEnum::TERM));
    }

    #[test]
    fn sampling_schedule_runs_both_groups_immediately_then_at_interval() {
        let mut schedule = SamplingSchedule::new(Duration::from_secs(2), Duration::from_secs(2));

        let initial = schedule.take_due(Duration::ZERO);
        assert!(initial.hardware);
        assert!(initial.process);

        let early = schedule.take_due(Duration::from_secs(1));
        assert!(!early.hardware);
        assert!(!early.process);

        let due = schedule.take_due(Duration::from_secs(2));
        assert!(due.hardware);
        assert!(due.process);
    }

    #[test]
    fn hardware_and_process_intervals_advance_independently() {
        let mut schedule = SamplingSchedule::new(Duration::from_secs(1), Duration::from_secs(3));
        schedule.take_due(Duration::ZERO);

        let first = schedule.take_due(Duration::from_secs(1));
        assert!(first.hardware);
        assert!(!first.process);

        let third = schedule.take_due(Duration::from_secs(3));
        assert!(third.hardware);
        assert!(third.process);
    }

    #[test]
    fn sensor_redraw_waits_for_the_response_burst_to_go_quiet() {
        let sensor_at = Instant::now();
        let events = RedrawEventEnum::SENSOR;

        assert!(!redraw_is_due(
            events,
            sensor_at + Duration::from_millis(49),
            None,
            Some(sensor_at),
        ));
        assert!(redraw_is_due(
            events,
            sensor_at + Duration::from_millis(50),
            None,
            Some(sensor_at),
        ));
    }

    #[test]
    fn terminal_input_redraws_immediately_during_sensor_debounce() {
        let now = Instant::now();

        assert!(redraw_is_due(
            RedrawEventEnum::TERM | RedrawEventEnum::SENSOR,
            now,
            Some(now),
            Some(now),
        ));
    }
}
