#[derive(Debug, Copy, Clone)]
pub enum Action {
    // Does nothing, for debugging, shouldn't modify any state.
    #[allow(dead_code)]
    NoOp,

    MoveCursorDown(usize),
    MoveCursorUp(usize),

    ScrollViewportDown(usize),
    ScrollViewportUp(usize),

    FocusTop,
    FocusBottom,
    MoveFocusedElemToCenter,
    MoveFocusedElemToTop,
    MoveFocusedElemToBottom,
}
