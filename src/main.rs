// Release builds are GUI-only; debug builds keep a console for logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod animations;
mod claude;
mod details;
mod dodge;
mod glance;
mod install;
mod log;
mod mascot;
mod monitors;
mod overlay;
mod pet;
mod scripts;
mod settings;
mod sound;
mod stats;
mod status;
mod theme;
mod tray;
mod update;
mod usage;
mod vector;
mod view;
mod widgets;
mod win;

use chrono::TimeDelta;
use eframe::egui::{IconData, ViewportBuilder, WindowLevel};
use settings::{Mode, Settings};
use std::sync::Arc;
use std::time::Duration;

const USAGE: &str = "\
Usage: cuota [--interval <seconds>] [--once] [--demo]
  --interval <s>  How often to refresh usage while Claude is in use (default 60, minimum 30)
  --once          Print usage to stdout and exit
  --demo          Show the overlay with fake data
  --updated       Wait for the Cuota this one replaced to quit (an update starts it so)
  --uninstall     Remove the copy setup installed (Settings > Apps runs this)";

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
    if flag(install::UNINSTALL) {
        uninstall();
        return Ok(());
    }
    // Started by an update, it takes over once the copy it replaces has quit.
    let updated = flag(update::UPDATED);
    if !win::first_instance(if updated { update::HANDOVER } else { Duration::ZERO }) {
        // Already running, say from startup: show that one's settings instead of a second copy.
        log::info!("Cuota was already running, so this copy opened its settings and quit");
        tray::open_running_settings();
        return Ok(());
    }
    if updated {
        update::clean_up();
        install::note_version(env!("CARGO_PKG_VERSION"));
    }
    if flag(install::INSTALLED) {
        install::take_over_startup();
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
    // Windows halves it for the title bar's small icon, which keeps the pixels crisp.
    let icon = Arc::new(IconData { rgba: tray::draw_icon(32), width: 32, height: 32 });
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

/// Asks, then takes the installed Cuota off this PC, as Settings › Apps does.
fn uninstall() {
    if !win::message("Uninstall Cuota? Your settings are kept.", true) {
        return;
    }
    match install::uninstall() {
        Ok(()) => {
            log::info!("Cuota was uninstalled");
            win::message("Cuota was uninstalled.", false);
            if let Err(e) = install::delete_files_once_closed() {
                log::warn!("{e}");
            }
        }
        Err(e) => {
            log::warn!("Couldn't uninstall Cuota: {e}");
            win::message(&e, false);
        }
    }
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
