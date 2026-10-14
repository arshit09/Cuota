// Release builds are GUI-only; debug builds keep a console for logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod animations;
mod claude;
mod details;
mod glance;
mod log;
mod mascot;
mod overlay;
mod scripts;
mod settings;
mod sound;
mod stats;
mod status;
mod tray;
mod vector;
mod view;
mod widgets;
mod win;

use chrono::TimeDelta;
use eframe::egui::{IconData, ViewportBuilder, WindowLevel};
use settings::{Mode, Settings};
use std::sync::Arc;

const USAGE: &str = "\
Usage: cuota [--interval <seconds>] [--once] [--demo]
  --interval <s>  How often to refresh usage while Claude is in use (default 60, minimum 30)
  --once          Print usage to stdout and exit
  --demo          Show the overlay with fake data";

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);

    if flag("--help") || flag("-h") {
        println!("{USAGE}");
        return Ok(());
    }
    log::start();
    let build = if cfg!(debug_assertions) { " (debug build)" } else { "" };
    let with = if args.is_empty() { String::new() } else { format!(" with {}", args.join(" ")) };
    log::info!(
        "Cuota {}{build} started{with}, on {} at {:.0}% scale",
        env!("CARGO_PKG_VERSION"),
        win::windows_version(),
        win::system_scale() * 100.0
    );
    if flag("--once") {
        print_once();
        return Ok(());
    }
    if !win::first_instance() {
        // Already running, say from startup: show that one's settings instead of a second copy.
        log::info!("Cuota was already running, so this copy opened its settings and quit");
        tray::open_running_settings();
        return Ok(());
    }
    let interval = args
        .iter()
        .position(|a| a == "--interval")
        .and_then(|i| args.get(i + 1)?.parse().ok())
        .unwrap_or(60)
        .max(30);
    let demo = flag("--demo");

    let mut settings = Settings::load();
    let position = initial_position(&mut settings);
    let spark = mascot::spark_rgba(mascot::SPARK_PIXELS);
    let side = mascot::SPARK_PIXELS as u32;
    let icon = Arc::new(IconData { rgba: spark.clone(), width: side, height: side });
    let level = if settings.pinned() == Some(Mode::Desktop) { WindowLevel::AlwaysOnBottom } else { WindowLevel::AlwaysOnTop };

    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("Cuota")
            .with_inner_size([300.0, 70.0])
            .with_position(position)
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_taskbar(false)
            .with_window_level(level)
            // Without Always on top or Desktop it starts as tray icons only; the panel opens from them.
            .with_visible(settings.pinned().is_some())
            .with_icon(icon.clone()),
        ..Default::default()
    };
    let run = eframe::run_native(
        "Cuota",
        options,
        Box::new(move |cc| {
            let interval = TimeDelta::seconds(interval);
            Ok(Box::new(overlay::Overlay::new(cc, settings, interval, demo, &spark, icon)))
        }),
    );
    match &run {
        Ok(()) => log::info!("Cuota quit"),
        // Such as OpenGL missing, which a release build has no console to say.
        Err(e) => log::error!("Cuota's window couldn't run: {e}"),
    }
    run
}

/// The saved panel position if it's still on a monitor, otherwise near the top-left.
fn initial_position(settings: &mut Settings) -> [f32; 2] {
    let scale = win::system_scale();
    let corner = |[x, y]: [f32; 2]| ((x * scale) as i32 + 8, (y * scale) as i32 + 8);
    settings.panel_pos = settings.panel_pos.filter(|&pos| {
        let (x, y) = corner(pos);
        win::on_screen(x, y)
    });
    settings.panel_pos.unwrap_or([40.0, 40.0])
}

fn print_once() {
    match claude::fetch() {
        Ok(usage) => {
            println!("{}", claude::local_account().plan.as_deref().unwrap_or("Claude"));
            let now = chrono::Utc::now();
            for limit in usage.limits {
                let reset = limit
                    .resets_at
                    .map(|t| format!("resets in {}", claude::until(t, now)))
                    .unwrap_or_default();
                println!("  {:<10} {:>4.0}%  {reset}", limit.label, limit.percent);
            }
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
