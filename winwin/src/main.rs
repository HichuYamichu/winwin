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
    let ctx = Context::new();

    // SAFETY: There can be only one `EventQueue` at a time and it must be pulled for events or shutdown.
    // This is the only place and time where `EventQueue` is constructed and we pool right after so
    // this is safe and correct.
    let mut queue = unsafe { EventQueue::new(&ctx) };
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
                    let window = get_focused_window(&ctx);
                    swap_adjacent(&ctx, window, Direction::Right);
                }

                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::H]) {
                    let window = get_focused_window(&ctx);
                    swap_adjacent(&ctx, window, Direction::Left);
                }

                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::J]) {
                    let window = get_focused_window(&ctx);
                    swap_adjacent(&ctx, window, Direction::Down);
                }

                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::K]) {
                    let window = get_focused_window(&ctx);
                    swap_adjacent(&ctx, window, Direction::Up);
                }

                // Apply selected layout.
                if input.all_pressed(&[mod_key, Key::Q]) {
                    let monitor = get_focused_monitor(&ctx);
                    apply_layout(&ctx, monitor, Layout::Stack);
                }

                if input.all_pressed(&[mod_key, Key::W]) {
                    let monitor = get_focused_monitor(&ctx);
                    apply_layout(&ctx, monitor, Layout::Full);
                }

                if input.all_pressed(&[mod_key, Key::E]) {
                    let monitor = get_focused_monitor(&ctx);
                    apply_layout(&ctx, monitor, Layout::Grid);
                }

                if input.all_pressed(&[mod_key, Key::R]) {
                    let monitor = get_focused_monitor(&ctx);
                    apply_layout(&ctx, monitor, Layout::None);
                }

                // Moving windows across monitors.
                if input.all_pressed(&[mod_key, Key::Right]) {
                    let window = get_focused_window(&ctx);
                    send_in(&ctx, window, Direction::Right);
                }

                // Window closing.
                if input.all_pressed(&[mod_key, Key::BackSlash]) {
                    let window = get_focused_window(&ctx);
                    kill_window(window);
                }

                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::BackSlash]) {
                    kill_all_windows(&ctx);
                }
            }
            Event::WindowOpen(window, monitor) => {
                let layout = layout_on(&ctx, monitor);
                apply_layout(&ctx, monitor, layout);
            }
            Event::WindowClose(window, monitor) => {
                // By the time this event is handled the window in question might have been
                // destroyed, as such it is not recommended to query for its properties. Regardles
                // of wheather the window is still around or not, it has been evicted from the cache
                // thus all `get_` functions called for this window will return default/invalid
                // values.
                // `monitor` value is valid and designates last monitor the window was on.
                let layout = layout_on(&ctx, monitor);
                apply_layout(&ctx, monitor, layout);
            } // TODO: Handle monitor connection/disconection.
        }
    }
}
