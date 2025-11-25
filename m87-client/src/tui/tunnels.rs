use crate::{
    config::Config,
    device::tunnel::{delete_tunnel, list_tunnels},
};
use anyhow::Result;
use m87_shared::forward::ForwardAccess;
use ratatui::{
    backend::TermionBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};
use termion::{
    async_stdin, event::Key, input::TermRead, raw::IntoRawMode, screen::IntoAlternateScreen,
};

pub async fn show_forwards_tui(device: &str) -> Result<()> {
    let mut forwards = list_tunnels(&device).await?;
    use std::collections::VecDeque;
    let config = Config::load()?;

    let stdout = std::io::stdout();
    let raw = stdout.into_raw_mode()?;
    let screen = raw.into_alternate_screen()?;
    let backend = TermionBackend::new(screen);
    let mut terminal = Terminal::new(backend)?;

    let mut keys = async_stdin().keys();

    let mut selected: usize = 0;

    loop {
        terminal.draw(|f| {
            let size = f.size();

            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(size);

            let left = chunks[0];
            let right = chunks[1];

            // ---------------------------
            // Left: Forward List
            // ---------------------------

            let items: Vec<ListItem> = forwards
                .iter()
                .enumerate()
                .map(|(i, fwd)| {
                    let mut text = format!(
                        "{}:{} → {}",
                        fwd.device_short_id,
                        fwd.target_port,
                        fwd.name.clone().unwrap_or_else(|| "(unnamed)".into()),
                    );

                    let style = if i == selected {
                        Style::default().fg(Color::Black).bg(Color::LightBlue)
                    } else {
                        Style::default()
                    };

                    ListItem::new(text).style(style)
                })
                .collect();

            let list = List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Tunnels (Up/Down, Enter=Details, d=Delete, q=Exit)"),
            );

            f.render_widget(list, left);

            // ---------------------------
            // Right: Details Pane
            // ---------------------------

            if let Some(fwd) = forwards.get(selected) {
                // Friendly name fallback
                let fname = fwd.name.clone().unwrap_or_else(|| "(unnamed)".into());

                // Build the hostname using your pattern
                //
                //   <name>-<short_id>.<server-host>
                //
                let hostname = format!(
                    "{}-{}.{}",
                    fname,
                    fwd.device_short_id,
                    config.get_server_hostname(),
                );

                // Access display
                let access_str = match &fwd.access {
                    ForwardAccess::Open => "open".into(),
                    ForwardAccess::IpWhitelist(list) => {
                        format!("IP Whitelist:\n{}", list.join("\n"))
                    }
                };

                // Full text shown in details pane
                let text = format!(
                    "ID: {}\n\
                        Device: {}\n\
                        Short ID: {}\n\
                        Name: {}\n\
                        Port: {}\n\
                        Access: {}\n\
                        URL:\n  {}\n\
                        Created: {}\n\
                        Updated: {}\n",
                    fwd.id,
                    fwd.device_id,
                    fwd.device_short_id,
                    fname,
                    fwd.target_port,
                    access_str,
                    hostname,
                    fwd.created_at.clone().unwrap_or_default(),
                    fwd.updated_at.clone().unwrap_or_default(),
                );

                let paragraph = Paragraph::new(text)
                    .block(Block::default().borders(Borders::ALL).title("Details"));

                f.render_widget(paragraph, right);
            }
        })?;

        // ---------------------------
        // Key handling
        // ---------------------------

        if let Some(Ok(key)) = keys.next() {
            match key {
                Key::Char('q') | Key::Esc | Key::Ctrl('c') => break,

                Key::Up => {
                    if selected > 0 {
                        selected -= 1;
                    }
                }

                Key::Down => {
                    if selected + 1 < forwards.len() {
                        selected += 1;
                    }
                }

                // Delete tunnel
                Key::Char('d') => {
                    if let Some(fwd) = forwards.get(selected) {
                        delete_tunnel(&device, fwd.target_port).await?;
                    }
                    forwards.remove(selected);
                    if selected >= forwards.len() && selected > 0 {
                        selected -= 1;
                    }
                }

                _ => {}
            }
        }
    }

    Ok(())
}
