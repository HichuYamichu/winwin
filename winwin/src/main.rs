use std::collections::HashMap;

use tracing::Level;
use winwin::*;

fn main() {
    println!("Hello, world!");

    let subscriber = tracing_subscriber::fmt()
        .compact()
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(true)
        .with_target(false)
        .with_max_level(Level::TRACE)
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let mod_key = Key::AltLeft;
    let mut queue = EventQueue::new();
    let ctx = Context::new();
    let mut monitor_configs = HashMap::new();

    loop {
        let event = queue.next_event(&ctx);
        match event {
            Event::KeyPress(input) => {
                if input.all_pressed(&[mod_key, Key::X]) {
                    queue.shutdown();
                    break;
                }

                // Change focused window.
                if input.all_pressed(&[mod_key, Key::L]) {
                    move_focus(&ctx, Direction::Right);
                }

                if input.all_pressed(&[mod_key, Key::H]) {
                    move_focus(&ctx, Direction::Left);
                }

                if input.all_pressed(&[mod_key, Key::J]) {
                    move_focus(&ctx, Direction::Down);
                }

                if input.all_pressed(&[mod_key, Key::K]) {
                    move_focus(&ctx, Direction::Up);
                }

                // Swap adjacent windows.
                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::L]) {
                    swap_adjacent(&ctx, get_focused_window(), Direction::Right);
                }

                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::H]) {
                    swap_adjacent(&ctx, get_focused_window(), Direction::Left);
                }

                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::J]) {
                    swap_adjacent(&ctx, get_focused_window(), Direction::Down);
                }

                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::K]) {
                    swap_adjacent(&ctx, get_focused_window(), Direction::Up);
                }

                // Apply selected layout.
                if input.all_pressed(&[mod_key, Key::Q]) {
                    let monitor = get_focused_monitor();
                    apply_layout(&ctx, monitor, Layout::Stack);
                    monitor_configs.insert(monitor, Layout::Stack);
                }

                if input.all_pressed(&[mod_key, Key::W]) {
                    let monitor = get_focused_monitor();
                    apply_layout(&ctx, monitor, Layout::Full);
                    monitor_configs.insert(monitor, Layout::Full);
                }

                if input.all_pressed(&[mod_key, Key::E]) {
                    let monitor = get_focused_monitor();
                    apply_layout(&ctx, monitor, Layout::Grid);
                    monitor_configs.insert(monitor, Layout::Grid);
                }

                if input.all_pressed(&[mod_key, Key::R]) {
                    let monitor = get_focused_monitor();
                    apply_layout(&ctx, monitor, Layout::None);
                    monitor_configs.insert(monitor, Layout::None);
                }
            }
            Event::WindowOpen(window) => {
                let monitor = get_monitor_with_window(window);
                let layout = monitor_configs.get(&monitor).unwrap_or(&Layout::None);
                apply_layout(&ctx, monitor, *layout);
            }
            Event::WindowClose(window) => {
                let monitor = get_monitor_with_window(window);
                let layout = monitor_configs.get(&monitor).unwrap_or(&Layout::None);
                apply_layout(&ctx, monitor, *layout);
            }
        }
    }
}
