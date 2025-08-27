use std::{
    cell::Cell,
    io::{stdout, Stdout},
    sync::Arc,
    thread,
    time::{Duration, SystemTime},
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

        let last_sync_ts = Cell::new(SystemTime::UNIX_EPOCH);
        let lasy_draw_ts = Cell::new(SystemTime::UNIX_EPOCH);

        loop {
            let now = SystemTime::now();
            let sync_diff = now
                .duration_since(last_sync_ts.get())
                .unwrap_or(Duration::from_millis(300));

            if sync_diff > Duration::from_secs(1) {
                for ele in self.resources.iter() {
                    ele.fetch_data(worker_tx);
                }
                last_sync_ts.set(now);
            }

            if event_enum.contains(RedrawEventEnum::TERM)
                || event_enum.contains(RedrawEventEnum::SENSOR)
                || (event_enum.contains(RedrawEventEnum::INTERVAL)
                    && now
                        .duration_since(lasy_draw_ts.get())
                        .unwrap_or(Duration::from_millis(200))
                        > Duration::from_millis(300))
            {
                lasy_draw_ts.set(now);

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
            }

            match self.res_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(rsp) => match rsp {
                    ResourceEvent::Resize(w, h) => {
                        self.layout.update_layout(Rect {
                            x: 0,
                            y: 0,
                            width: w,
                            height: h,
                        });
                        event_enum = event_enum.union(RedrawEventEnum::TERM);
                    }
                    ResourceEvent::KeyEvent(key) => {
                        self.handle_key(&key);
                        event_enum = event_enum.union(RedrawEventEnum::TERM);
                    }
                    ResourceEvent::SensorRsp(rsp) => {
                        for ele in &mut self.resources {
                            if ele.updata_data(&rsp) {
                                break;
                            }
                        }
                        event_enum = event_enum.union(RedrawEventEnum::SENSOR);
                    }
                    ResourceEvent::Quit => {
                        break;
                    }
                    ResourceEvent::FocusedIndex(focused_index) => {
                        self.focused_index.replace(focused_index);
                    }
                },
                Err(RecvTimeoutError::Timeout) => {
                    event_enum = event_enum.union(RedrawEventEnum::INTERVAL);
                }
                _ => {}
            }
        }
        Ok(())
    }
}
