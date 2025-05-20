use tracing::Level;
use winwin::events::Event;
use winwin::input::Key;
use winwin::types::Layout;
use winwin::wm::*;
use winwin::*;

fn main() {
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

    // Creating more than one `WindowManager` at a time will panic.
    let mut wm = WindowManager::new();
    let ctx = &mut Context::new();

    // TODO: rewrite this:

    // Most `wm` functions return Result because the actual Windows DWM state could
    // have changed, and we might not have observed that change yet. For example, we
    // receive a KeyPress event right as a window gets destroyed. The window is still
    // available to us here, but all Windows operations we do on it will fail. That's
    // why you should either use let-else pattern (just continue in else block) or
    // `try_or_bail` macro that does the same thing as let-else but is more concise.

    let m = monitors(ctx)[2];
    let _ = apply_layout(ctx, m, Layout::Stack);

    loop {
        let event = wm.next_event(ctx); // Wait for next event to handle.
        match event {
            // `Input` structure allows us to check what keys are pressed. It is strongly
            // recommended to use input ASAP since it contains a rendezvous channel used to notify
            // the keyboard handler that a keypress should be intercepted and not passed further.
            // If you don't want to intercept a key press, use `pressed_no_intercept` instead of
            // `pressed`. Moreover, it should be noted that only the last key that completes the
            // sequence will be intercepted. This is because, for example, we don't know if the
            // user is going to press X after ALT, so we can't intercept the ALT press.
            Event::KeyPress(input) if input.pressed(&[mod_key, Key::X]) => {
                wm.shutdown();
                break;
            }

            // Layouts.
            Event::KeyPress(input) if input.pressed(&[mod_key, Key::Q]) => {
                // Set stack layout on currently focused monitor.
                // let monitor = focused_monitor(ctx);
                // let _ = apply_layout(ctx, monitor, Layout::Stack);
            }
            Event::KeyPress(input) if input.pressed(&[mod_key, Key::W]) => {
                // Set full layout on currently focused monitor.
            }
            Event::KeyPress(input) if input.pressed(&[mod_key, Key::E]) => {
                // Set grid layout on currently focused monitor.
            }

            // Order based navigation.
            Event::KeyPress(input) if input.pressed(&[mod_key, Key::ShiftLeft, Key::J]) => {
                // Focus next window on current monitor queue.
            }
            Event::KeyPress(input) if input.pressed(&[mod_key, Key::ShiftLeft, Key::K]) => {
                // Focus previous window on current monitor queue.
            }

            // 2d Navigation.
            Event::KeyPress(input) if input.pressed(&[mod_key, Key::ShiftLeft, Key::H]) => {
                // Focus window to the right of the current one.
            }
            Event::KeyPress(input) if input.pressed(&[mod_key, Key::ShiftLeft, Key::J]) => {
                // Focus window on top of the current one.
            }
            Event::KeyPress(input) if input.pressed(&[mod_key, Key::ShiftLeft, Key::K]) => {
                // Focus window under the current one.
            }
            Event::KeyPress(input) if input.pressed(&[mod_key, Key::ShiftLeft, Key::L]) => {
                // Focus window to the left of the current one.
            }

            // 2d window swaps.
            Event::KeyPress(input) if input.pressed(&[mod_key, Key::CtrlLeft, Key::H]) => {
                // Swap current window with the one to the right.
            }
            Event::KeyPress(input) if input.pressed(&[mod_key, Key::CtrlLeft, Key::J]) => {
                // Swap current window with the one on top.
            }
            Event::KeyPress(input) if input.pressed(&[mod_key, Key::CtrlLeft, Key::H]) => {
                // Swap current window with the one underneath.
            }
            Event::KeyPress(input) if input.pressed(&[mod_key, Key::CtrlLeft, Key::H]) => {
                // Swap current window with the one to the right.
            }

            // TODO:
            // Send window to a monitor.
            // Close window.
            // Swap windows on monitors.
            Event::KeyPress(_) => {}
            Event::WindowCreate(window) => {
                // We figure out on what monitor did the window open.
                // Then we figure out what layout was on that monitor.
                // And then we re-apply the layout in order to update window positions.

                // You could customize this by detecting what window did open and then
                // setting a specific layout instead.

                let m = try_or_bail!(monitor_from_window(ctx, window));
                let layout = try_or_bail!(layout_on(ctx, m));
                apply_layout(ctx, m, layout);
            }
            Event::WindowDestroy(_window, monitor) => {
                // When this event is signaled the window has already been evicted from the
                // internal cache, it is only returned in case you maintain your own collection
                // with windows in it. `monitor` param is the monitor the window was on before its
                // destruction. You can use it to re-layout that monitor.
                let layout = try_or_bail!(layout_on(ctx, monitor));
                apply_layout(ctx, monitor, layout);
            }
            Event::WindowMove {
                window,
                old_rect,
                new_rect,
            } => {
                // TODO: if window intersects a lot with other window swap them.

                tracing::debug!("move");
            }
            Event::WindowResize { .. } => {
                tracing::debug!("resize");
            }
        }
    }
}
