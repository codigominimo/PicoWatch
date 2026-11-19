//! Interfaz interactiva de pantalla completa construida con ratatui.
//!
//! Los colectores son los mismos que usa la CLI: el TUI sólo presenta y
//! conserva un histórico en memoria para las gráficas. No hay hilos ni estado
//! global; el bucle sondea eventos y refresca cuando toca.

use std::collections::VecDeque;
use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{
    Bar, BarChart, BarGroup, Block, Borders, Cell, Gauge, Paragraph, Row, Sparkline, Table,
};
use ratatui::Terminal;

use crate::alerts::{Alert, Metric};
use crate::collectors::cpu::CpuReport;
use crate::collectors::disk::DiskInfo;
use crate::collectors::memory::MemoryInfo;
use crate::collectors::network::Interface;
use crate::collectors::processes::Process;
use crate::collectors::temperature::Sensor;
use crate::core::NAME;

const HISTORY_LEN: usize = 60;
const SAMPLE: Duration = Duration::from_millis(100);
const MIN_INTERVAL: Duration = Duration::from_millis(500);
const MAX_INTERVAL: Duration = Duration::from_secs(30);
const TOP_PROCESSES: usize = 12;

/// Restaura la terminal aunque el bucle termine con un error.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

/// Estado dibujado por el TUI.
struct App {
    interval: Duration,
    last_refresh: Instant,
    force_refresh: bool,
    paused: bool,
    cpu: Option<CpuReport>,
    memory: Option<MemoryInfo>,
    disk: Option<DiskInfo>,
    interfaces: Vec<Interface>,
    processes: Vec<Process>,
    sensors: Vec<Sensor>,
    alerts: Vec<Alert>,
    cpu_history: VecDeque<u64>,
    memory_history: VecDeque<u64>,
    rx_history: VecDeque<u64>,
    tx_history: VecDeque<u64>,
    previous_rx: u64,
    previous_tx: u64,
    error: Option<String>,
}

impl App {
    fn new() -> Self {
        Self {
            interval: Duration::from_secs(1),
            last_refresh: Instant::now() - Duration::from_secs(1),
            force_refresh: true,
            paused: false,
            cpu: None,
            memory: None,
            disk: None,
            interfaces: Vec::new(),
            processes: Vec::new(),
            sensors: Vec::new(),
            alerts: Vec::new(),
            cpu_history: VecDeque::with_capacity(HISTORY_LEN),
            memory_history: VecDeque::with_capacity(HISTORY_LEN),
            rx_history: VecDeque::with_capacity(HISTORY_LEN),
            tx_history: VecDeque::with_capacity(HISTORY_LEN),
            previous_rx: 0,
            previous_tx: 0,
            error: None,
        }
    }

    fn refresh(&mut self) {
        // La temperatura no falla en Windows (lista vacía) y se lee antes de
        // evaluar alertas para que el panel refleje la muestra actual.
        let sensors = crate::collectors::temperature::read().unwrap_or_default();
        let max_temperature = sensors.iter().map(|sensor| sensor.millicelsius).max();
        let cpu = crate::collectors::cpu::sample(SAMPLE);
        let processes = crate::collectors::processes::sample(SAMPLE);
        let memory = crate::collectors::memory::read();
        let disk = crate::collectors::disk::read();
        let interfaces = crate::collectors::network::read();
        let config = crate::config::load().unwrap_or_default();

        match (cpu, processes, memory, disk, interfaces) {
            (Ok(cpu), Ok(processes), Ok(memory), Ok(disk), Ok(interfaces)) => {
                self.alerts = crate::alerts::evaluate(
                    &config,
                    cpu.usage.basis_points,
                    memory.used_basis_points(),
                    disk.used_basis_points(),
                    max_temperature,
                );

                let rx: u64 = interfaces.iter().map(|item| item.rx_bytes).sum();
                let tx: u64 = interfaces.iter().map(|item| item.tx_bytes).sum();
                push(&mut self.rx_history, rx.saturating_sub(self.previous_rx));
                push(&mut self.tx_history, tx.saturating_sub(self.previous_tx));
                self.previous_rx = rx;
                self.previous_tx = tx;

                push(&mut self.cpu_history, u64::from(cpu.usage.basis_points));
                push(
                    &mut self.memory_history,
                    u64::from(memory.used_basis_points()),
                );

                self.sensors = sensors;
                self.cpu = Some(cpu);
                self.memory = Some(memory);
                self.disk = Some(disk);
                self.interfaces = interfaces;
                self.processes = processes;
                self.error = None;
            }
            (Err(error), _, _, _, _)
            | (_, Err(error), _, _, _)
            | (_, _, Err(error), _, _)
            | (_, _, _, Err(error), _)
            | (_, _, _, _, Err(error)) => {
                self.error = Some(error.to_string());
            }
        }
        self.last_refresh = Instant::now();
        self.force_refresh = false;
    }

    fn change_interval(&mut self, increase: bool) {
        self.interval = if increase {
            (self.interval + Duration::from_millis(500)).min(MAX_INTERVAL)
        } else {
            self.interval
                .checked_sub(Duration::from_millis(500))
                .unwrap_or(MIN_INTERVAL)
                .max(MIN_INTERVAL)
        };
    }
}

fn push(history: &mut VecDeque<u64>, value: u64) {
    if history.len() == HISTORY_LEN {
        history.pop_front();
    }
    history.push_back(value);
}

/// Arranca el TUI y no devuelve control hasta que el usuario sale con `q`.
pub fn run() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let _guard = TerminalGuard;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    loop {
        if !app.paused && (app.force_refresh || app.last_refresh.elapsed() >= app.interval) {
            app.refresh();
        }
        terminal.draw(|frame| render(frame, &app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('+') | KeyCode::Char('=') => app.change_interval(true),
                    KeyCode::Char('-') | KeyCode::Char('_') => app.change_interval(false),
                    KeyCode::Char('r') => app.force_refresh = true,
                    KeyCode::Char(' ') => app.paused = !app.paused,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn render(frame: &mut ratatui::Frame<'_>, app: &App) {
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(3),
    ])
    .split(area);

    render_header(frame, chunks[0], app);
    render_body(frame, chunks[1], app);
    render_footer(frame, chunks[2], app);
}

fn render_header(frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let cpu_bp = app.cpu.as_ref().map_or(0, |cpu| cpu.usage.basis_points);
    let memory_bp = app.memory.map_or(0, |memory| memory.used_basis_points());
    let disk_bp = app.disk.map_or(0, |disk| disk.used_basis_points());
    let title = format!(
        " {NAME} · CPU {} · RAM {} · DISCO {} · {} interfaces ",
        percent(cpu_bp),
        percent(memory_bp),
        percent(disk_bp),
        app.interfaces.len()
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Monitor del sistema")
        .border_style(Style::default().fg(Color::Cyan));
    let paragraph = Paragraph::new(Line::from(title))
        .style(Style::default().fg(Color::White))
        .block(block);
    frame.render_widget(paragraph, area);
}

fn render_body(frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let columns =
        Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area);
    render_left(frame, columns[0], app);
    render_right(frame, columns[1], app);
}

fn render_left(frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Percentage(35),
        Constraint::Length(12),
        Constraint::Min(8),
    ])
    .split(area);

    let cpu_bp = app.cpu.as_ref().map_or(0, |cpu| cpu.usage.basis_points);
    let cpu_block = Block::default().borders(Borders::ALL).title(" CPU ");
    let inner = cpu_block.inner(chunks[0]);
    frame.render_widget(cpu_block, chunks[0]);
    let gauge_area = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
    frame.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(Color::Cyan))
            .percent((cpu_bp / 100) as u16)
            .label(percent(cpu_bp)),
        gauge_area[0],
    );
    frame.render_widget(
        Sparkline::default()
            .data(&app.cpu_history)
            .max(10_000)
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().title(" histórico (100 %) ")),
        gauge_area[1],
    );

    render_cores(frame, chunks[1], app);
    render_processes(frame, chunks[2], app);
}

fn render_cores(frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let Some(report) = &app.cpu else {
        return;
    };
    let bars: Vec<Bar> = report
        .core_usages
        .iter()
        .enumerate()
        .map(|(index, usage)| {
            Bar::default()
                .label(Line::from(index.to_string()))
                .value(u64::from(usage.basis_points))
                .style(Style::default().fg(Color::Green))
        })
        .collect();
    let chart = BarChart::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Núcleos (0-100 %) "),
        )
        .data(BarGroup::default().bars(&bars))
        .bar_width(3)
        .bar_gap(1)
        .max(10_000);
    frame.render_widget(chart, area);
}

fn render_processes(frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let header = Row::new(["PID", "Nombre", "RSS", "Hilos", "CPU"])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .bottom_margin(0);
    let rows = app.processes.iter().take(TOP_PROCESSES).map(|process| {
        Row::new(vec![
            Cell::from(process.pid.to_string()),
            Cell::from(process.name.as_str()),
            Cell::from(mib(process.rss_bytes)),
            Cell::from(process.threads.to_string()),
            Cell::from(percent(process.cpu_basis_points)),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(7),
            Constraint::Min(14),
            Constraint::Length(10),
            Constraint::Length(6),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Procesos (top {TOP_PROCESSES}) ")),
    );
    frame.render_widget(table, area);
}

fn render_right(frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(9),
        Constraint::Length(7),
        Constraint::Min(4),
    ])
    .split(area);

    render_gauge(
        frame,
        chunks[0],
        " Memoria ",
        app.memory.map(|m| m.used_basis_points()),
        Color::Magenta,
    );
    render_gauge(
        frame,
        chunks[1],
        " Disco ",
        app.disk.map(|d| d.used_basis_points()),
        Color::Blue,
    );
    render_network(frame, chunks[2], app);
    render_temperature(frame, chunks[3], app);
    render_alerts(frame, chunks[4], app);
}

fn render_gauge(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    title: &str,
    value: Option<u32>,
    color: Color,
) {
    let value = value.unwrap_or(0);
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(title))
        .gauge_style(Style::default().fg(color))
        .percent((value / 100) as u16)
        .label(percent(value));
    frame.render_widget(gauge, area);
}

fn render_network(frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" Red ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let chunks = Layout::vertical([Constraint::Min(2), Constraint::Length(2)]).split(inner);
    let lines = app
        .interfaces
        .iter()
        .take(2)
        .map(|interface| {
            Line::from(format!(
                "{} ↓{} ↑{}",
                truncate(&interface.name, 22),
                mib(interface.rx_bytes),
                mib(interface.tx_bytes)
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), chunks[0]);
    let traffic = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);
    frame.render_widget(
        Sparkline::default()
            .data(&app.rx_history)
            .style(Style::default().fg(Color::Green))
            .block(Block::default().title(" RX ")),
        traffic[0],
    );
    frame.render_widget(
        Sparkline::default()
            .data(&app.tx_history)
            .style(Style::default().fg(Color::Red))
            .block(Block::default().title(" TX ")),
        traffic[1],
    );
}

fn render_temperature(frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let lines = if app.sensors.is_empty() {
        vec![Line::from("sin sensores disponibles")]
    } else {
        app.sensors
            .iter()
            .take(3)
            .map(|sensor| {
                Line::from(format!(
                    "{}: {}.{} °C",
                    truncate(&sensor.name, 18),
                    sensor.millicelsius / 1000,
                    (sensor.millicelsius.rem_euclid(1000)) / 100
                ))
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Temperatura "),
        ),
        area,
    );
}

fn render_alerts(frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let lines: Vec<Line<'_>> = app
        .alerts
        .iter()
        .take(3)
        .map(|alert| Line::from(alert_text(alert)))
        .collect();
    let lines = if lines.is_empty() {
        vec![Line::from("ninguna alerta activa")]
    } else {
        lines
    };
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL).title(" Alertas ")),
        area,
    );
}

fn render_footer(frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let status = if app.paused { "PAUSADO" } else { "ACTIVO" };
    let error = app.error.as_deref().unwrap_or("");
    let text = format!(
        " q/Esc salir · +/- intervalo ({:?}) · r refrescar · espacio pausa [{status}] {error}",
        app.interval
    );
    frame.render_widget(
        Paragraph::new(Line::from(truncate(
            &text,
            area.width.saturating_sub(2) as usize,
        )))
        .block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn percent(basis_points: u32) -> String {
    format!("{}.{:02}%", basis_points / 100, basis_points % 100)
}

/// Formatea una alerta: temperatura en miligrados, el resto en puntos base.
fn alert_text(alert: &Alert) -> String {
    match alert.metric {
        Metric::Temperature => format!(
            "{}: {}.{} °C",
            alert.metric.label(),
            alert.value / 1000,
            (alert.value % 1000) / 100
        ),
        _ => format!("{}: {}", alert.metric.label(), percent(alert.value as u32)),
    }
}

fn mib(bytes: u64) -> String {
    format!("{} MiB", bytes / (1024 * 1024))
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut result: String = text.chars().take(max.saturating_sub(1)).collect();
    result.push('…');
    result
}

#[cfg(test)]
mod tests {
    use super::{alert_text, mib, percent, push, render, truncate, App, HISTORY_LEN};
    use crate::alerts::{Alert, Metric};
    use crate::collectors::cpu::{CpuReport, CpuUsage};
    use crate::collectors::disk::DiskInfo;
    use crate::collectors::memory::MemoryInfo;
    use crate::collectors::network::Interface;
    use crate::collectors::processes::Process;
    use crate::collectors::temperature::Sensor;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Dibuja en un buffer en memoria: permite comprobar el panel sin terminal.
    fn draw(app: &App) -> String {
        let backend = TestBackend::new(140, 44);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                text.push_str(buffer.cell((x, y)).unwrap().symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn porcentaje_usa_enteros() {
        assert_eq!(percent(0), "0.00%");
        assert_eq!(percent(416), "4.16%");
        assert_eq!(percent(10_000), "100.00%");
    }

    #[test]
    fn mib_redondea_hacia_abajo() {
        assert_eq!(mib(1024 * 1024), "1 MiB");
        assert_eq!(mib((1024 * 1024) - 1), "0 MiB");
    }

    #[test]
    fn trunca_utf8() {
        assert_eq!(truncate("hola", 10), "hola");
        assert_eq!(truncate("hola mundo", 5), "hola…");
        assert_eq!(truncate("áéíóú", 3), "áé…");
    }

    #[test]
    fn historico_esta_acotado() {
        let mut app = App::new();
        for value in 0..(HISTORY_LEN as u64 + 10) {
            super::push(&mut app.cpu_history, value);
        }
        assert_eq!(app.cpu_history.len(), HISTORY_LEN);
        assert_eq!(app.cpu_history.front(), Some(&10));
        assert_eq!(app.cpu_history.back(), Some(&(HISTORY_LEN as u64 + 9)));
    }

    #[test]
    fn texto_de_alerta_usa_unidades_correctas() {
        assert_eq!(
            alert_text(&Alert {
                metric: Metric::Cpu,
                value: 9500
            }),
            "CPU: 95.00%"
        );
        assert_eq!(
            alert_text(&Alert {
                metric: Metric::Temperature,
                value: 81_234
            }),
            "Temperatura: 81.2 °C"
        );
    }

    #[test]
    fn panel_vacio_muestra_estados_neutros() {
        let text = draw(&App::new());
        assert!(text.contains("picowatch"), "{text}");
        assert!(text.contains("sin sensores disponibles"), "{text}");
        assert!(text.contains("ninguna alerta activa"), "{text}");
        assert!(text.contains("q/Esc salir"), "{text}");
    }

    #[test]
    fn panel_con_datos_muestra_graficas_y_detalles() {
        let mut app = App::new();
        app.cpu = Some(CpuReport {
            usage: CpuUsage { basis_points: 1250 },
            core_usages: vec![
                CpuUsage { basis_points: 5000 },
                CpuUsage { basis_points: 1000 },
            ],
            core_count: 2,
        });
        app.memory = Some(MemoryInfo {
            total_bytes: 16 * 1024 * 1024 * 1024,
            available_bytes: 8 * 1024 * 1024 * 1024,
        });
        app.disk = Some(DiskInfo {
            total_bytes: 100 * 1024 * 1024 * 1024,
            available_bytes: 40 * 1024 * 1024 * 1024,
        });
        app.interfaces = vec![Interface {
            name: "eth0".into(),
            rx_bytes: 5 * 1024 * 1024,
            tx_bytes: 2 * 1024 * 1024,
        }];
        app.processes = vec![Process {
            pid: 42,
            name: "init".into(),
            threads: 3,
            rss_bytes: 7 * 1024 * 1024,
            cpu_basis_points: 2500,
        }];
        app.sensors = vec![Sensor {
            name: "thermal_zone0".into(),
            millicelsius: 45_200,
        }];
        app.alerts = vec![
            Alert {
                metric: Metric::Cpu,
                value: 9500,
            },
            Alert {
                metric: Metric::Temperature,
                value: 81_000,
            },
        ];
        for value in 0..10 {
            push(&mut app.cpu_history, value * 100);
            push(&mut app.memory_history, value * 100);
            push(&mut app.rx_history, value * 1000);
            push(&mut app.tx_history, value * 1000);
        }

        let text = draw(&app);
        for expected in [
            "12.50%",
            "50.00%",
            "init",
            "25.00%",
            "eth0",
            "thermal_zone0",
            "45.2 °C",
            "CPU: 95.00%",
            "Temperatura: 81.0 °C",
        ] {
            assert!(text.contains(expected), "falta {expected:?} en:\n{text}");
        }
    }

    #[test]
    fn intervalo_esta_limitado() {
        let mut app = App::new();
        for _ in 0..100 {
            app.change_interval(false);
        }
        assert_eq!(app.interval.as_millis(), 500);
        for _ in 0..100 {
            app.change_interval(true);
        }
        assert_eq!(app.interval.as_secs(), 30);
    }
}
