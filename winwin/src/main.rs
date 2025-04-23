use tracing::Level;
use winwin::*;

fn main() {
    println!("Hello, world!");
    // TODO:
    // Reneme EventQueue to WindowManager
    // Move context into WindowManager
    // Put incoming events in ring buffer
    // Relplace IOCP system with channel like ICP


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
    loop {
        let event = wm.next_event();
        match event {
            Event::KeyPress(input) => {
                if input.all_pressed(&[mod_key, Key::X]) {
                    wm.shutdown();
                    break;
                }

                // Focus switching.
                if input.all_pressed(&[mod_key, Key::ShiftLeft, Key::J]) {
                    focus_next_window(wm.ctx());
                }

                if input.all_pressed(&[mod_key, Key::ShiftLeft, Key::K]) {
                    focus_prev_window(wm.ctx());
                }

                // 2d window navigation.
                if input.all_pressed(&[mod_key, Key::L]) {
                    move_focus(wm.ctx_mut(), Direction::Right);
                }

                if input.all_pressed(&[mod_key, Key::H]) {
                    move_focus(wm.ctx_mut(), Direction::Left);
                }

                if input.all_pressed(&[mod_key, Key::J]) {
                    move_focus(wm.ctx_mut(), Direction::Down);
                }

                if input.all_pressed(&[mod_key, Key::K]) {
                    move_focus(wm.ctx_mut(), Direction::Up);
                }

                // Swap adjacent windows.
                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::L]) {
                    let window = focused_window(wm.ctx());
                    swap_adjacent(wm.ctx_mut(), window, Direction::Right);
                }

                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::H]) {
                    let window = focused_window(wm.ctx());
                    swap_adjacent(wm.ctx_mut(), window, Direction::Left);
                }

                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::J]) {
                    let window = focused_window(wm.ctx());
                    swap_adjacent(wm.ctx_mut(), window, Direction::Down);
                }

                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::K]) {
                    let window = focused_window(wm.ctx());
                    swap_adjacent(wm.ctx_mut(), window, Direction::Up);
                }

                // Apply selected layout.
                if input.all_pressed(&[mod_key, Key::Q]) {
                    let monitor = focused_monitor(wm.ctx());
                    apply_layout(wm.ctx_mut(), monitor, Layout::Stack);
                }

                if input.all_pressed(&[mod_key, Key::W]) {
                    let monitor = focused_monitor(wm.ctx());
                    apply_layout(wm.ctx_mut(), monitor, Layout::Full);
                }

                if input.all_pressed(&[mod_key, Key::E]) {
                    let monitor = focused_monitor(wm.ctx());
                    apply_layout(wm.ctx_mut(), monitor, Layout::Grid);
                }

                if input.all_pressed(&[mod_key, Key::R]) {
                    let monitor = focused_monitor(wm.ctx());
                    apply_layout(wm.ctx_mut(), monitor, Layout::None);
                }

                // Moving windows across monitors.
                if input.all_pressed(&[mod_key, Key::Right]) {
                    let window = focused_window(wm.ctx());
                    send_in(wm.ctx_mut(), window, Direction::Right);
                }

                if input.all_pressed(&[mod_key, Key::Left]) {
                    let window = focused_window(wm.ctx());
                    send_in(wm.ctx_mut(), window, Direction::Left);
                }

                // Swap windows on monitors.
                if input.all_pressed(&[mod_key, Key::P]) {
                    let monitors = monitors(wm.ctx());
                    swap_monitors(wm.ctx_mut(), monitors[0], monitors[2]);
                }

                // Window closing.
                if input.all_pressed(&[mod_key, Key::BackSlash]) {
                    let window = focused_window(wm.ctx());
                    kill_window(window);
                }

                if input.all_pressed(&[mod_key, Key::CtrlLeft, Key::BackSlash]) {
                    kill_all_windows(wm.ctx());
                }
            }
            Event::WindowOpen(window, monitor) => {
                let layout = layout_on(wm.ctx(), monitor);
                apply_layout(wm.ctx_mut(), monitor, layout);
            }
            Event::WindowClose(window, monitor) => {
                // By the time this event is handled the window in question might have been
                // destroyed, as such it is not recommended to query for its properties. Regardles
                // of wheather the window is still around or not, it has been evicted from the cache
                // thus all `get_` functions called for this window will return default/invalid
                // values.
                // `monitor` value is valid and designates last monitor the window was on.
                let layout = layout_on(wm.ctx(), monitor);
                apply_layout(wm.ctx_mut(), monitor, layout);
            } // TODO: Handle monitor connection/disconection.
        }
    }
}
