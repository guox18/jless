use crate::document::Document;

/// The `DocumentViewport` manages what part of a document is displayed on screen
/// as the user takes actions to move the cursor or manipulate the document. Much
/// of the behavior here matches or is inspired by vim behavior. A type that
/// implements `Document` is responsible for deciding the actual content that goes
/// on each line, and when and where line wrapping should occur.
///
/// The basic objective here is to keep whatever part of the document is focused
/// (i.e., where the `Cursor` is) visible within the viewport, and for certain
/// scrolling actions that manipulate the viewport, the cursor should be updated
/// to a position in the document that is within the viewport.

pub struct DocumentViewport<D: Document> {
    top_line: D::ScreenLine,
    current_focus: D::Cursor,

    dimensions: (usize, usize),
}

struct ViewportLinesIterator<'a, D: Document> {
    document: &'a D,
    next_line: Option<D::ScreenLine>,
    remaining_height: usize,
    display_width: usize,
}

impl<'a, D: Document> Iterator for ViewportLinesIterator<'a, D> {
    type Item = Option<D::ScreenLine>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining_height == 0 {
            return None;
        }
        self.remaining_height -= 1;

        let Some(curr_line) = self.next_line.take() else {
            return Some(None);
        };

        self.next_line = self
            .document
            .next_screen_line(&curr_line, self.display_width);

        Some(Some(curr_line))
    }
}

impl<D: Document> DocumentViewport<D> {
    pub fn new(
        first_line: D::ScreenLine,
        initial_cursor: D::Cursor,
        dimensions: (usize, usize),
    ) -> Self {
        DocumentViewport {
            top_line: first_line,
            current_focus: initial_cursor,
            dimensions,
        }
    }

    pub fn viewport_lines<'a, 'b>(
        &'a self,
        document: &'b D,
    ) -> impl Iterator<Item = Option<D::ScreenLine>> + 'b {
        ViewportLinesIterator {
            document,
            next_line: Some(self.top_line.clone()),
            remaining_height: self.dimensions.1,
            display_width: self.dimensions.0,
        }
    }

    pub fn move_cursor_up(&mut self) {}
    pub fn move_cursor_down(&mut self) {}
    pub fn scroll_up(&mut self) {}
    pub fn scroll_down(&mut self) {}
    pub fn resize(&mut self) {}
}

#[cfg(test)]
mod test {
    use super::*;

    use bstr::ByteSlice;
    use insta::assert_snapshot;

    use std::fmt::Write;

    use crate::text_document::TextDocument;

    fn init(
        contents: &[u8],
        width: usize,
        height: usize,
    ) -> (TextDocument, DocumentViewport<TextDocument>) {
        let mut doc = TextDocument::new();
        doc.append(contents);
        doc.eof();

        let (top_line, initial_cursor) = doc.init_top_screen_line_and_cursor(width).unwrap();
        let viewport = DocumentViewport::new(top_line, initial_cursor, (width, height));

        (doc, viewport)
    }

    impl<D: Document> DocumentViewport<D> {
        fn render(&self, doc: &D) -> String {
            // |1234       5|
            // | ## <width> |
            let content_width = self.dimensions.0;
            let inner_width = content_width + 5;
            let width = inner_width + 2;
            let mut s = String::new();
            writeln!(s, "┌{:─<inner_width$}┐", "");
            for screen_line in self.viewport_lines(doc) {
                let Some(screen_line) = screen_line else {
                    writeln!(s, "│ ~  {: <content_width$} │", "");
                    continue;
                };

                let is_focused = false;
                let line_number = doc.line_number(&screen_line);
                let wraps_from_prev_line = doc.is_after_start_of_wrapped_line(&screen_line);
                let wraps_onto_next_line = doc.is_before_end_of_wrapped_line(&screen_line);

                writeln!(
                    s,
                    "│{}{:<2}{}{: <content_width$}{}│",
                    if is_focused { '*' } else { ' ' },
                    line_number,
                    if wraps_from_prev_line { '↪' } else { ' ' },
                    doc.debug_text_content(&screen_line).as_bstr(),
                    if wraps_onto_next_line { '↩' } else { ' ' },
                );
            }
            writeln!(s, "└{:─<inner_width$}┘", "");
            s
        }
    }

    #[test]
    fn test_render() {
        let (doc, viewport) = init(b"aaa\nbb\ncccc\ndddddd\ne\n", 4, 7);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌─────────┐
        │ 1  aaa  │
        │ 2  bb   │
        │ 3  cccc │
        │ 4  dddd↩│
        │ 4 ↪dd   │
        │ 5  e    │
        │ ~       │
        └─────────┘
        ");
    }
}
