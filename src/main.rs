use winwin::*;

fn main() {
    println!("Hello, world!");

    // let focused_window = winwin::get_focused_window();
    // let all_windows = winwin::get_all_windows();
    // let monitors = winwin::get_all_monitors();
    //
    // monitors[2].set_layout(Layout::Stack);

    let mod_key = Key::AltLeft;
    let queue = EventQueue::new();
    let ctx = Context::new();

    loop {
        let event = queue.next_event(&ctx);
        match event {
            WMEvent::InputChange(input) => {
                dbg!(&input);
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
            WMEvent::WindowOpen(window) => {
                let monitor = get_monitor_with_window(window);
                keep_layout(&ctx, monitor, window);
            }
            WMEvent::WindowClose(window) => {}
            _ => {}
        }
    }
}
