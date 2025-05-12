use tracing::Level;
use winwin::*;

use hylib::alloc::{Arena, GrowStrategy};
use hylib::KiB;

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

    // SAFETY: There can be only one `EventQueue` at a time.
    let mut wm = unsafe { WindowManager::new() };
    let alloc: Arena = Arena::new(KiB(16), GrowStrategy::None);
    let ctx = &mut Context::new(&alloc);
    loop {
        let event = wm.next_event(ctx); // Wait for next event to handle.
        match event {
            Event::KeyPress(input) => {
                if input.all_pressed(&[mod_key, Key::X]) {
                    wm.shutdown();
                    break;
                }

                // Focus switching.
                if input.all_pressed(&[mod_key, Key::ShiftLeft, Key::J]) {
                    focus_next_window(ctx);
                }

                if input.all_pressed(&[mod_key, Key::ShiftLeft, Key::K]) {
                    focus_prev_window(ctx);
                }

                // 2d window navigation.
                if input.all_pressed(&[mod_key, Key::L]) {
                    move_focus(ctx, Direction::Right);
                }

                if input.all_pressed(&[mod_key, Key::H]) {
                    move_focus(ctx, Direction::Left);
                }

                if input.all_pressed(&[mod_key, Key::J]) {
                    move_focus(ctx, Direction::Down);
                }

                if input.all_pressed(&[mod_key, Key::K]) {
                    move_focus(ctx, Direction::Up);
                }

                // Swap adjacent windows.
                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::L]) {
                    let window = focused_window(ctx);
                    swap_adjacent(ctx, window, Direction::Right);
                }

                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::H]) {
                    let window = focused_window(ctx);
                    swap_adjacent(ctx, window, Direction::Left);
                }

                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::J]) {
                    let window = focused_window(ctx);
                    swap_adjacent(ctx, window, Direction::Down);
                }

                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::K]) {
                    let window = focused_window(ctx);
                    swap_adjacent(ctx, window, Direction::Up);
                }

                // Apply selected layout.
                if input.all_pressed(&[mod_key, Key::Q]) {
                    let monitor = focused_monitor(ctx);
                    apply_layout(ctx, monitor, Layout::Stack);
                }

                if input.all_pressed(&[mod_key, Key::W]) {
                    let monitor = focused_monitor(ctx);
                    apply_layout(ctx, monitor, Layout::Full);
                }

                if input.all_pressed(&[mod_key, Key::E]) {
                    let monitor = focused_monitor(ctx);
                    apply_layout(ctx, monitor, Layout::Grid);
                }

                if input.all_pressed(&[mod_key, Key::R]) {
                    let monitor = focused_monitor(ctx);
                    apply_layout(ctx, monitor, Layout::None);
                }

                // Moving windows across monitors.
                if input.all_pressed(&[mod_key, Key::Right]) {
                    let window = focused_window(ctx);
                    send_in(ctx, window, Direction::Right);
                }

                if input.all_pressed(&[mod_key, Key::Left]) {
                    let window = focused_window(ctx);
                    send_in(ctx, window, Direction::Left);
                }

                // Swap windows on monitors.
                if input.all_pressed(&[mod_key, Key::P]) {
                    let monitors = monitors(ctx);
                    swap_monitors(ctx, monitors[0], monitors[2]);
                }

                // Window closing.
                if input.all_pressed(&[mod_key, Key::BackSlash]) {
                    let window = focused_window(ctx);
                    kill_window(window);
                }

                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::BackSlash]) {
                    kill_all_windows(ctx);
                }
            }
            Event::WindowOpen(_window, monitor) => {
                let layout = layout_on(ctx, monitor);
                apply_layout(ctx, monitor, layout);
            }
            Event::WindowClose(_window, monitor) => {
                // By the time this event is handled the window in question might have been
                // destroyed, as such it is not recommended to query for its properties. Regardles
                // of wheather the window is still around or not, it has been evicted from the cache
                // thus all `get_` functions called for this window will return default/invalid
                // values.
                // `monitor` value is valid and designates last monitor the window was on.
                let layout = layout_on(ctx, monitor);
                apply_layout(ctx, monitor, layout);
            } // TODO: Handle monitor connection/disconection.
        }
    }
}
