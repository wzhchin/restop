use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use restop::app::ResTop;

fn main() -> Result<(), Box<dyn std::error::Error>> {
/*     #[cfg(debug_assertions)]
    let file_appender = tracing_appender::rolling::daily("/tmp/", "resource-tui.log");
    #[cfg(debug_assertions)]
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    #[cfg(debug_assertions)]
    tracing_subscriber::fmt().with_writer(non_blocking).init(); */

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
