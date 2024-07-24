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

                // Apply selected layout.
                if input.all_pressed(&[mod_key, Key::Q]) {
                    apply_layout(&ctx, get_focused_monitor(), Layout::Stack);
                }

                if input.all_pressed(&[mod_key, Key::W]) {
                    apply_layout(&ctx, get_focused_monitor(), Layout::Full);
                }

                if input.all_pressed(&[mod_key, Key::E]) {
                    apply_layout(&ctx, get_focused_monitor(), Layout::Grid);
                }
            }
            Event::WindowOpen(window, /*monitor*/ rect) => {
                // dbg!(window, rect);
                // keep_layout(&ctx, monitor, window, rect);
            }
            Event::WindowClose(window) => {
                // dbg!(window);
            }
            Event::Shutdown => {
                break;
            }
        }
    }
}
