use winwin::*;

fn main() {
    println!("Hello, world!");

    let mod_key = Key::AltLeft;
    let mut queue = EventQueue::new();
    let ctx = Context::new();

    loop {
        let event = queue.next_event(&ctx);
        match event {
            Event::KeyPress(input) => {
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
            Event::WindowOpen(window) => {
                let monitor = get_monitor_with_window(window);
                keep_layout(&ctx, monitor, window);
            }
            Event::WindowClose(window) => {}
            _ => {}
        }
    }
}
