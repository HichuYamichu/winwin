use winwin::*;
use winwin_common::*;

#[link(name = "hooks.dll", kind="dylib")]
extern "C" {
    fn add(left: usize, right: usize) -> usize;
}

fn main() {
    println!("Hello, world!");

    unsafe {
        println!("2+2={}", add(2, 2));
    }

    let mod_key = Key::AltLeft;
    let mut queue = EventQueue::new();
    let ctx = Context::new();

    loop {
        let event = queue.next_event(&ctx);
        match event {
            Event::KeyPress(input) => {
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
            Event::WindowOpen(window) => {
                dbg!(window);
                // let monitor = get_monitor_with_window(window);
                // keep_layout(&ctx, monitor, window);
            }
            Event::WindowClose(window) => {
                dbg!(window);
            }
            _ => {}
        }
    }
}
