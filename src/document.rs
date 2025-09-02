// Misc. notes:
//
// Maybe want:
//    LineRef::is_wrap_continuation(&self) -> bool
// or Document::line_number_for_break(&self, &LineRef) -> Option<usize>
//
// TextDocument has `LineWrapping`; JsonDocument/SexpDocument might `ContainerWrapping`?,
// and a `ContainerWrapping` can have a `LineWrapping` inside it.

pub trait Document {
    type ScreenLine: Clone;
    type Cursor;

    // new(dimensions)?

    fn append(&mut self, data: &[u8]);
    fn eof(&mut self);

    // Someday: This initialization is clumsy, but we need to know how
    // many lines there are before we know how much space we'll have...
    // Someday: The "lines" here and the "line ref" below are different lines; I need to come
    // up with different words.
    fn init_top_screen_line_and_cursor(
        &self,
        display_width: usize,
    ) -> Option<(Self::ScreenLine, Self::Cursor)>;

    // Someday: This should be probably stored inside the document (so it can clear any caches?).
    fn next_screen_line(
        &self,
        screen_line: &Self::ScreenLine,
        display_width: usize,
    ) -> Option<Self::ScreenLine>;

    // 1-indexed
    fn line_number(&self, screen_line: &Self::ScreenLine) -> usize;

    fn is_wrapped_line(&self, screen_line: &Self::ScreenLine) -> bool;
    fn is_start_of_wrapped_line(&self, screen_line: &Self::ScreenLine) -> bool;
    fn is_end_of_wrapped_line(&self, screen_line: &Self::ScreenLine) -> bool;
    fn is_after_start_of_wrapped_line(&self, screen_line: &Self::ScreenLine) -> bool;
    fn is_before_end_of_wrapped_line(&self, screen_line: &Self::ScreenLine) -> bool;

    #[cfg(test)]
    fn debug_text_content(&self, screen_line: &Self::ScreenLine) -> &[u8];

    fn move_cursor_up(&self, cursor: &Self::Cursor) -> Option<Self::Cursor>;
    fn move_cursor_down(&self, cursor: &Self::Cursor) -> Option<Self::Cursor>;
}
