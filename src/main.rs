use std::fs::File;

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::LevelFilter;
use ratatui::{backend::CrosstermBackend, Terminal};
use restop::app::ResTop;
use simplelog::{CombinedLogger, Config, WriteLogger};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(debug_assertions)]
    CombinedLogger::init(vec![WriteLogger::new(
        LevelFilter::Debug,
        Config::default(),
        File::create("/tmp/restop.log").unwrap(),
    )])
    .unwrap();
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut term = Terminal::new(backend)?;

    execute!(
        term.backend_mut(),
        EnterAlternateScreen,
        crossterm::cursor::Hide
    )?;
    enable_raw_mode()?;

    let mut res_top = ResTop::new()?;

    match res_top.run(&mut term) {
        Ok(_) => {}
        Err(err) => {
            log::error!("Some error occurs handling the tui event: {}", err);
        }
    }

    execute!(
        term.backend_mut(),
        LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;
    disable_raw_mode()?;

    Ok(())
}
