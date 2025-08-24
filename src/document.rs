pub trait Document {
    type HardLineBreakRef;
    type Cursor;

    fn append(&mut self, data: &[u8]);
    fn eof(&mut self);
    fn has_content_to_display(&self) -> bool;
}
