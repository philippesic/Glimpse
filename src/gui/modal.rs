use gpui::{App, FocusHandle, Keystroke, Window};

pub fn tab(event: &Keystroke, scope: &FocusHandle, window: &mut Window, cx: &mut App) -> bool {
    if event.key != "tab" {
        return false;
    }
    for _ in 0..512 {
        if event.modifiers.shift {
            window.focus_prev();
        } else {
            window.focus_next();
        }
        if scope.contains_focused(window, cx) {
            break;
        }
    }
    if !scope.contains_focused(window, cx) {
        window.focus(scope);
    }
    window.prevent_default();
    cx.stop_propagation();
    true
}
