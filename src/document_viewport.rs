use std::cmp;
use std::ops::Range;

use crate::document::{CursorRange, Document};

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

    // Someday: This needs to be a struct; right now it's (width, height).
    dimensions: (usize, usize),

    // We call this scrolloff_setting, to differentiate between
    // what it's set to, and what the scrolloff functionally is
    // if it's set to value >= height / 2.
    //
    // Access the functional value via .effective_scrolloff().
    scrolloff_setting: usize,
}

#[derive(Debug)]
struct AcceptableStartScreenIndexesToShowCursorNode {
    #[allow(dead_code)]
    #[cfg(debug_assertions)]
    cursor_height: usize,
    #[allow(dead_code)]
    #[cfg(debug_assertions)]
    last_screen_index: usize,
    #[allow(dead_code)]
    #[cfg(debug_assertions)]
    range_after_considering_scrolloff: Range<usize>,
    #[allow(dead_code)]
    #[cfg(debug_assertions)]
    range_after_considering_start_and_end_of_document: Range<usize>,
    #[allow(dead_code)]
    #[cfg(debug_assertions)]
    range_after_expanding_due_to_cursor_height: Range<usize>,
    // The actual fields
    start: usize,
    end: usize,
}

impl<D: Document> DocumentViewport<D> {
    pub fn new(
        first_line: D::ScreenLine,
        initial_cursor: D::Cursor,
        dimensions: (usize, usize),
        scrolloff: usize,
    ) -> Self {
        DocumentViewport {
            top_line: first_line,
            current_focus: initial_cursor,
            dimensions,
            scrolloff_setting: scrolloff,
        }
    }

    fn set_scrolloff(&mut self, scrolloff: usize) {
        self.scrolloff_setting = scrolloff;
    }

    // Cap scrolloff at half the size of the screen.
    //
    // Height | Scrolloff | Min between edge of screen and cursor
    //   15   |     3     |                  3
    //   15   |     7     |                  7
    //   15   |     8     |                  7
    //   16   |     7     |                  7
    //   16   |     8     |                  7
    //   17   |     8     |                  8
    fn effective_scrolloff(&self) -> usize {
        cmp::min(self.scrolloff_setting, (self.dimensions.1 - 1) / 2)
    }

    // If the last line of the file appears before `screen_index`, this will return `None`.
    fn screen_line_at_screen_index(
        &self,
        doc: &D,
        mut screen_index: usize,
    ) -> Option<D::ScreenLine> {
        let mut curr_line = self.top_line.clone();
        while screen_index > 0 {
            curr_line = doc.next_screen_line(&curr_line, self.dimensions.0)?;
            screen_index -= 1;
        }
        Some(curr_line)
    }

    // If the last line of the file appears before `screen_index`, this will return the
    // last line of the file.
    fn last_screen_line_at_or_before_screen_index(
        &self,
        doc: &D,
        mut screen_index: usize,
    ) -> D::ScreenLine {
        let mut curr_line = self.top_line.clone();
        while screen_index > 0 {
            let Some(next_screen_line) = doc.next_screen_line(&curr_line, self.dimensions.0) else {
                return curr_line;
            };
            curr_line = next_screen_line;
            screen_index -= 1;
        }
        curr_line
    }

    pub fn move_cursor_down(&mut self, doc: &D, lines: usize) {
        self.update_so_new_cursor_is_visible(doc, doc.move_cursor_down(lines, &self.current_focus));
    }

    pub fn move_cursor_up(&mut self, doc: &D, lines: usize) {
        self.update_so_new_cursor_is_visible(doc, doc.move_cursor_up(lines, &self.current_focus));
    }

    pub fn scroll_viewport_down(&mut self, doc: &D, mut lines: usize) {
        let mut lines_scrolled = 0;
        let mut next_top_line = self.top_line.clone();
        while lines > 0 {
            match doc.next_screen_line(&next_top_line, self.dimensions.0) {
                None => break,
                Some(line) => {
                    lines -= 1;
                    lines_scrolled += 1;
                    next_top_line = line;
                }
            }
        }

        if lines_scrolled > 0 {
            self.top_line = next_top_line;
            self.maybe_update_focused_node_after_scroll(doc);
        }
    }

    pub fn scroll_viewport_up(&mut self, doc: &D, mut lines: usize) {
        let mut lines_scrolled = 0;
        let mut next_top_line = self.top_line.clone();
        while lines > 0 {
            match doc.prev_screen_line(&next_top_line, self.dimensions.0) {
                None => break,
                Some(line) => {
                    lines -= 1;
                    lines_scrolled += 1;
                    next_top_line = line;
                }
            }
        }

        if lines_scrolled > 0 {
            self.top_line = next_top_line;
            self.maybe_update_focused_node_after_scroll(doc);
        }
    }

    fn update_so_new_cursor_is_visible(&mut self, doc: &D, new_cursor: Option<D::Cursor>) {
        // If an operation doesn't move the cursor, it will return `None`, so there's
        // nothing to do.
        let Some(new_cursor) = new_cursor else {
            return;
        };

        self.current_focus = new_cursor;

        let (cursor_range, acceptable_start_index_range) = self
            .calculate_acceptable_start_screen_indexes_to_show_cursor_node(
                doc,
                &self.current_focus,
            );

        let AcceptableStartScreenIndexesToShowCursorNode {
            start: start_index,
            end: end_index,
            ..
        } = acceptable_start_index_range;

        let screen_line_at_first_acceptable_start =
            self.screen_line_at_screen_index(doc, start_index);
        let screen_line_at_last_acceptable_start = self.screen_line_at_screen_index(doc, end_index);

        let cursor_start_is_before_first_acceptable_start =
            match screen_line_at_first_acceptable_start {
                // If there's no screen line af the first acceptable start, then must be the after
                // the end of the file, so the cursor start is definitely before then.
                None => true,
                Some(acceptable_start) => cursor_range.start < acceptable_start,
            };

        let cursor_start_is_at_or_before_last_acceptable_start =
            match screen_line_at_last_acceptable_start {
                None => true, // Same logic as above
                Some(acceptable_start) => cursor_range.start <= acceptable_start,
            };

        if cursor_start_is_before_first_acceptable_start {
            // Cursor is too close to the top of the screen (or past it); move the viewport so
            // the cursor is at the start of the acceptable range.
            self.top_line = self.n_screen_lines_before(doc, cursor_range.start, start_index);
        } else if cursor_start_is_at_or_before_last_acceptable_start {
            // Nothing to do, the cursor is in an acceptable range!
        } else {
            // Cursor is too close to the bottom of the screen (or past it); move the viewport
            // so the cursor is at the end of the acceptable range.
            self.top_line = self.n_screen_lines_before(doc, cursor_range.start, end_index);
        }
    }

    // Someday: We should compute the `CursorRange` outside of this function, pass it in,
    // and then get rid of `cursor_layout_details` in lieu of functions to directly
    // count `bounded_screen_lines_{before,after}_screen_line`.
    fn calculate_acceptable_start_screen_indexes_to_show_cursor_node(
        &self,
        doc: &D,
        cursor: &D::Cursor,
    ) -> (
        CursorRange<D::ScreenLine>,
        AcceptableStartScreenIndexesToShowCursorNode,
    ) {
        // We want to make sure that as much of the newly focused node is visible. The high level
        // logic here is to first determine the range of "acceptable" places for the cursor to be
        // This is by default the entire screen, but then it gets shrunk based on the `scrolloff`
        // setting, and then increased again based and the height of the focused node, or proximity
        // to the start/end of the document.
        //
        // Once we have this range, we will snap the position of the cursor into that range.

        let cursor_layout_details =
            doc.cursor_layout_details(&cursor, self.dimensions.0, self.dimensions.1 - 1);

        // Initial acceptable range is the whole screen.
        let mut first_acceptable_screen_index = 0;
        let last_screen_index = self.dimensions.1 - 1;
        let mut last_acceptable_screen_index = last_screen_index;

        let min_screenlines_between_edge_of_screen_and_cursor = self.effective_scrolloff();

        // Shrink the acceptable range based on the scrolloff setting. (Because we capped
        // scrolloff at half the height of the screen, that ensures this doesn't cause
        // the values to cross.)
        first_acceptable_screen_index += min_screenlines_between_edge_of_screen_and_cursor;
        last_acceptable_screen_index -= min_screenlines_between_edge_of_screen_and_cursor;

        #[cfg(debug_assertions)]
        let range_after_considering_scrolloff =
            first_acceptable_screen_index..last_acceptable_screen_index;

        // Now we take into account the start/end of the document, where we don't enforce the
        // scrolloff setting. If we enforce scrolloff at the top of the file, then we'd have
        // to show empty lines before the start of the file, which would be silly. At the bottom
        // of the file, we won't enforce scrolloff, so that if you just hold the down arrow,
        // eventually you'll focus the last line of the file on the bottom of the screen.
        // (We do allow scrolling past the end of the file, but that decreases the start index,
        // and this operation relaxes the constraint by increasing the allowable start index,
        // so it is not relevant here.)

        if let Some(lines_before_start) =
            cursor_layout_details.bounded_doc_screen_lines_before_start
        {
            first_acceptable_screen_index =
                cmp::min(first_acceptable_screen_index, lines_before_start);
        }

        if let Some(lines_after_end) = cursor_layout_details.bounded_doc_screen_lines_after_end {
            last_acceptable_screen_index = cmp::max(
                last_acceptable_screen_index,
                // The bound is approximate, to prevent us from having to look at the whole
                // document, not exact;
                // last_screen_index.saturating_sub(lines_after_end),
                last_screen_index - lines_after_end,
            );
        }

        #[cfg(debug_assertions)]
        let range_after_considering_start_and_end_of_document =
            first_acceptable_screen_index..last_acceptable_screen_index;

        // Now we need to expand the acceptable range based on the height of the cursor, up
        // to the whole size of the screen again.

        let height_of_acceptable_range =
            last_acceptable_screen_index - first_acceptable_screen_index + 1;
        let cursor_height = cursor_layout_details.range.num_screen_lines;

        if cursor_height >= self.dimensions.1 {
            // Simple case, the cursor is as big or bigger than the screen, so the whole screen
            // is available.
            first_acceptable_screen_index = 0;
            last_acceptable_screen_index = self.dimensions.1 - 1;
        } else if cursor_height > height_of_acceptable_range {
            let mut additional_space_needed = cursor_height - height_of_acceptable_range;
            let space_to_reclaim_at_start = first_acceptable_screen_index;
            let space_to_reclaim_at_end = last_screen_index - last_acceptable_screen_index;

            // If we have more space to reclaim on one end (because of the start/end of the file)
            // we'll push that side back first. Imagine the second line of a file is 8 lines tall
            // (first line is 1 line tall), the height of the window is 10, and scrolloff is 4.
            // Then we have:
            //
            // Screen index range:            [0, 9]
            // Acceptable range w/ scrolloff: [4, 5]
            // After start/end of file:       [1, 5]
            // Height of acceptable range:    5
            //
            // If we subtracted evenly from both sides, we'd expand it to [0, 6], and then [0, 7],
            // which would be weird. If you loaded the file, and hit the down arrow, even though
            // the line is perfectly centered (it takes up [1, 8]), we would move it up to the top
            // of the screen.
            //
            // (I originally just thought "oh, we should try to keep it centered", and then while
            // trying to explain why this was better, I found this example that clearly shows the
            // other behavior is wrong.)
            let diff_between_sides =
                usize::abs_diff(space_to_reclaim_at_start, space_to_reclaim_at_end);
            let space_to_reclaim_from_one_side =
                cmp::min(diff_between_sides, additional_space_needed);

            additional_space_needed -= space_to_reclaim_from_one_side;
            if space_to_reclaim_at_start > space_to_reclaim_at_end {
                first_acceptable_screen_index -= space_to_reclaim_from_one_side;
            } else if space_to_reclaim_at_end > space_to_reclaim_at_start {
                last_acceptable_screen_index += space_to_reclaim_from_one_side;
            }

            // Now both sides are equal; let's chop off equally from both sides now.
            // If we have an odd amount of additional space neeed, we'll round up and
            // take that amount from both sides so that we don't always have the bigger
            // half of the cursor on top.

            let to_reclaim = (additional_space_needed + 1) / 2;
            first_acceptable_screen_index -= to_reclaim;
            last_acceptable_screen_index += to_reclaim;
        } else {
            // Cursor height <= height of acceptable range, so we don't need to
            // make any updates.
        }

        #[cfg(debug_assertions)]
        let range_after_expanding_due_to_cursor_height =
            first_acceptable_screen_index..last_acceptable_screen_index;

        // Final step: convert the acceptable screen index range into an acceptable
        // range for the start of the cursor.
        let first_acceptable_screen_index_for_start_of_cursor = first_acceptable_screen_index;
        let last_acceptable_screen_index_for_start_of_cursor = cmp::max(
            first_acceptable_screen_index_for_start_of_cursor,
            // Subtract (size - 1); for example, if the cursor takes up two lines, then
            // the last acceptable start is one line before the last acceptable screen index.
            last_acceptable_screen_index
                .saturating_sub(cursor_layout_details.range.num_screen_lines - 1),
        );

        let acceptable_start_indexes = AcceptableStartScreenIndexesToShowCursorNode {
            #[cfg(debug_assertions)]
            cursor_height,
            #[cfg(debug_assertions)]
            last_screen_index,
            #[cfg(debug_assertions)]
            range_after_considering_scrolloff,
            #[cfg(debug_assertions)]
            range_after_considering_start_and_end_of_document,
            #[cfg(debug_assertions)]
            range_after_expanding_due_to_cursor_height,
            // The actual fields
            start: first_acceptable_screen_index_for_start_of_cursor,
            end: last_acceptable_screen_index_for_start_of_cursor,
        };

        (cursor_layout_details.range, acceptable_start_indexes)
    }

    fn maybe_update_focused_node_after_scroll(&mut self, doc: &D) {
        // When scrolling, we'll allow scrolling wrapped lines partly off the screen as long
        // as any part of the focused node still obeys the scrolloff setting.
        let min_screenlines_between_edge_of_screen_and_cursor = self.effective_scrolloff();

        let first_acceptable_screen_index = if doc.is_first_screen_line_of_document(&self.top_line)
        {
            0
        } else {
            min_screenlines_between_edge_of_screen_and_cursor
        };

        let last_screen_index = self.dimensions.1 - 1;
        let last_acceptable_screen_index =
            last_screen_index - min_screenlines_between_edge_of_screen_and_cursor;

        // Using `last_screen_line_at_or_before_screen_index` handles the case when the end
        // of the file is on screen. In the extreme example, consider if the last line of the
        // document is at the top of the screen. If the screen is 10 lines tall, and scrolloff
        // is 3, then the first/last acceptable screen indexes will be 3 and 6, but the first
        // and last acceptable screen line will both be the current top line, which is the last
        // line of the document.

        let first_acceptable_screen_line =
            self.last_screen_line_at_or_before_screen_index(doc, first_acceptable_screen_index);
        let last_acceptable_screen_line =
            self.last_screen_line_at_or_before_screen_index(doc, last_acceptable_screen_index);

        let focused_range = doc.cursor_range(&self.current_focus, self.dimensions.0);

        if focused_range.end < first_acceptable_screen_line {
            self.current_focus = doc.convert_screen_line_to_cursor(
                first_acceptable_screen_line,
                &self.current_focus,
                self.dimensions.0,
            );
        } else if last_acceptable_screen_line < focused_range.start {
            self.current_focus = doc.convert_screen_line_to_cursor(
                last_acceptable_screen_line,
                &self.current_focus,
                self.dimensions.0,
            );
        } else {
            // Current focused range overlaps with acceptable screen line ranges;
            // nothing to do!
        }
    }

    // Assumes that this will always exist.
    fn n_screen_lines_before(
        &self,
        doc: &D,
        mut screen_line: D::ScreenLine,
        mut n: usize,
    ) -> D::ScreenLine {
        while n > 0 {
            screen_line = doc
                .prev_screen_line(&screen_line, self.dimensions.0)
                .unwrap();
            n -= 1;
        }
        screen_line
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

#[cfg(test)]
mod test {
    use super::*;

    use bstr::ByteSlice;
    use insta::{allow_duplicates, assert_debug_snapshot, assert_snapshot};

    use std::fmt::Write;

    use crate::text_document::{Cursor, TextDocument};

    fn init(
        contents: &[u8],
        width: usize,
        height: usize,
        scrolloff: usize,
    ) -> (TextDocument, DocumentViewport<TextDocument>) {
        let mut doc = TextDocument::new();
        doc.append(contents);
        doc.eof();

        let (top_line, initial_cursor) = doc.init_top_screen_line_and_cursor(width).unwrap();
        let viewport = DocumentViewport::new(top_line, initial_cursor, (width, height), scrolloff);

        (doc, viewport)
    }

    impl<D: Document> DocumentViewport<D> {
        fn render(&self, doc: &D) -> String {
            // |12345678       9|
            // | ##|##| <width> |
            let content_width = self.dimensions.0;
            let mut s = String::new();
            writeln!(s, "┌SI┬─L#┬─{:─<content_width$}─┐", "").unwrap();
            for (screen_index, screen_line) in self.viewport_lines(doc).enumerate() {
                let Some(screen_line) = screen_line else {
                    writeln!(s, "│{:>2}│ ~ │ {: <content_width$} │", screen_index, "").unwrap();
                    continue;
                };

                let is_focused = doc.does_screen_line_intersect_cursor(
                    &screen_line,
                    &self.current_focus,
                    self.dimensions.0,
                );
                let line_number = doc.line_number(&screen_line);
                let wraps_from_prev_line = doc.is_after_start_of_wrapped_line(&screen_line);
                let wraps_onto_next_line = doc.is_before_end_of_wrapped_line(&screen_line);

                writeln!(
                    s,
                    "│{:>2}│{}{:<2}│{}{: <content_width$}{}│",
                    screen_index,
                    if is_focused { '*' } else { ' ' },
                    line_number,
                    if wraps_from_prev_line { '↪' } else { ' ' },
                    doc.debug_text_content(&screen_line).as_bstr(),
                    if wraps_onto_next_line { '↩' } else { ' ' },
                )
                .unwrap();
            }
            writeln!(s, "└──┴───┴─{:─<content_width$}─┘", "").unwrap();
            s
        }
    }

    #[test]
    fn test_render() {
        let (doc, viewport) = init(b"aaa\nbb\ncccc\ndddddd\ne\n", 4, 7, 0);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│*1 │ aaa  │
        │ 1│ 2 │ bb   │
        │ 2│ 3 │ cccc │
        │ 3│ 4 │ dddd↩│
        │ 4│ 4 │↪dd   │
        │ 5│ 5 │ e    │
        │ 6│ ~ │      │
        └──┴───┴──────┘
        ");
    }

    fn acceptable_screen_indexes(
        doc: &TextDocument,
        viewport: &DocumentViewport<TextDocument>,
        cursor: &Cursor,
    ) -> AcceptableStartScreenIndexesToShowCursorNode {
        viewport
            .calculate_acceptable_start_screen_indexes_to_show_cursor_node(doc, cursor)
            .1
    }

    #[test]
    fn test_acceptable_start_screen_indexes() {
        let (doc, mut viewport) = init(b"a\nbbbb\nc\nd\ne\nf\ng\n", 1, 10, 0);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬───┐
        │ 0│*1 │ a │
        │ 1│ 2 │ b↩│
        │ 2│ 2 │↪b↩│
        │ 3│ 2 │↪b↩│
        │ 4│ 2 │↪b │
        │ 5│ 3 │ c │
        │ 6│ 4 │ d │
        │ 7│ 5 │ e │
        │ 8│ 6 │ f │
        │ 9│ 7 │ g │
        └──┴───┴───┘
        ");

        let line_2 = doc.cursor_to_line_n(2);
        assert_debug_snapshot!(acceptable_screen_indexes(&doc, &viewport, &line_2), @r"
        AcceptableStartScreenIndexesToShowCursorNode {
            cursor_height: 4,
            last_screen_index: 9,
            range_after_considering_scrolloff: 0..9,
            range_after_considering_start_and_end_of_document: 0..9,
            range_after_expanding_due_to_cursor_height: 0..9,
            start: 0,
            end: 6,
        }
        ");

        viewport.set_scrolloff(3);
        assert_debug_snapshot!(acceptable_screen_indexes(&doc, &viewport, &line_2), @r"
        AcceptableStartScreenIndexesToShowCursorNode {
            cursor_height: 4,
            last_screen_index: 9,
            range_after_considering_scrolloff: 3..6,
            range_after_considering_start_and_end_of_document: 1..6,
            range_after_expanding_due_to_cursor_height: 1..6,
            start: 1,
            end: 3,
        }
        ");

        // Example from the comment in `calculate_acceptable_start_screen_indexes_to_show_cursor_node`:
        let (doc, viewport) = init(b"a\nbbbbbbbb\nc\nd\ne\nf\n", 1, 10, 4);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬───┐
        │ 0│*1 │ a │
        │ 1│ 2 │ b↩│
        │ 2│ 2 │↪b↩│
        │ 3│ 2 │↪b↩│
        │ 4│ 2 │↪b↩│
        │ 5│ 2 │↪b↩│
        │ 6│ 2 │↪b↩│
        │ 7│ 2 │↪b↩│
        │ 8│ 2 │↪b │
        │ 9│ 3 │ c │
        └──┴───┴───┘
        ");

        let line_2 = doc.cursor_to_line_n(2);
        assert_debug_snapshot!(acceptable_screen_indexes(&doc, &viewport, &line_2), @r"
        AcceptableStartScreenIndexesToShowCursorNode {
            cursor_height: 8,
            last_screen_index: 9,
            range_after_considering_scrolloff: 4..5,
            range_after_considering_start_and_end_of_document: 1..5,
            range_after_expanding_due_to_cursor_height: 1..8,
            start: 1,
            end: 1,
        }
        ");
    }

    #[test]
    fn test_acceptable_start_screen_indexes_when_focused_node_bigger_than_viewport() {
        // Odd height
        allow_duplicates! {
            // Odd and even heights of the focused node
            for (input, height) in [("a\nb\nc\nddd\nc\nd\ne", 3), ("a\nb\nc\ndddd\nc\nd\ne", 4)].iter() {
                let (doc, viewport) = init(input.as_bytes(), 1, 3, 0);
                assert_snapshot!(viewport.render(&doc), @r"
                ┌SI┬─L#┬───┐
                │ 0│*1 │ a │
                │ 1│ 2 │ b │
                │ 2│ 3 │ c │
                └──┴───┴───┘
                ");

                let line_4 = doc.cursor_to_line_n(4);
                let mut acceptable_screen_indexes = acceptable_screen_indexes(&doc, &viewport, &line_4);
                assert_eq!(acceptable_screen_indexes.cursor_height, *height);
                // Clear for the snapshot, since it differs
                acceptable_screen_indexes.cursor_height = 0;
                assert_debug_snapshot!(acceptable_screen_indexes, @r"
                AcceptableStartScreenIndexesToShowCursorNode {
                    cursor_height: 0,
                    last_screen_index: 2,
                    range_after_considering_scrolloff: 0..2,
                    range_after_considering_start_and_end_of_document: 0..2,
                    range_after_expanding_due_to_cursor_height: 0..2,
                    start: 0,
                    end: 0,
                }
                ");
            }
        }

        // Even height
        allow_duplicates! {
            // Odd and even heights of the focused node
            for (input, height) in [("a\nb\nc\nddddd\nc\nd\ne", 5), ("a\nb\nc\ndddd\nc\nd\ne", 4)].iter() {
                let (doc, viewport) = init(input.as_bytes(), 1, 4, 0);
                assert_snapshot!(viewport.render(&doc), @r"
                ┌SI┬─L#┬───┐
                │ 0│*1 │ a │
                │ 1│ 2 │ b │
                │ 2│ 3 │ c │
                │ 3│ 4 │ d↩│
                └──┴───┴───┘
                ");

                let line_4 = doc.cursor_to_line_n(4);
                let mut acceptable_screen_indexes = acceptable_screen_indexes(&doc, &viewport, &line_4);
                assert_eq!(acceptable_screen_indexes.cursor_height, *height);
                // Clear for the snapshot, since it differs
                acceptable_screen_indexes.cursor_height = 0;
                assert_debug_snapshot!(acceptable_screen_indexes, @r"
                AcceptableStartScreenIndexesToShowCursorNode {
                    cursor_height: 0,
                    last_screen_index: 3,
                    range_after_considering_scrolloff: 0..3,
                    range_after_considering_start_and_end_of_document: 0..3,
                    range_after_expanding_due_to_cursor_height: 0..3,
                    start: 0,
                    end: 0,
                }
                ");
            }
        }
    }

    #[test]
    fn test_move_cursor_up_and_down() {
        let (doc, mut viewport) = init(b"aaa\nbb\ncccc\ndddddd\ne\n", 4, 7, 0);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│*1 │ aaa  │
        │ 1│ 2 │ bb   │
        │ 2│ 3 │ cccc │
        │ 3│ 4 │ dddd↩│
        │ 4│ 4 │↪dd   │
        │ 5│ 5 │ e    │
        │ 6│ ~ │      │
        └──┴───┴──────┘
        ");

        viewport.move_cursor_down(&doc, 1);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 1 │ aaa  │
        │ 1│*2 │ bb   │
        │ 2│ 3 │ cccc │
        │ 3│ 4 │ dddd↩│
        │ 4│ 4 │↪dd   │
        │ 5│ 5 │ e    │
        │ 6│ ~ │      │
        └──┴───┴──────┘
        ");

        viewport.move_cursor_down(&doc, 3);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 1 │ aaa  │
        │ 1│ 2 │ bb   │
        │ 2│ 3 │ cccc │
        │ 3│ 4 │ dddd↩│
        │ 4│ 4 │↪dd   │
        │ 5│*5 │ e    │
        │ 6│ ~ │      │
        └──┴───┴──────┘
        ");

        viewport.move_cursor_up(&doc, 1);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 1 │ aaa  │
        │ 1│ 2 │ bb   │
        │ 2│ 3 │ cccc │
        │ 3│*4 │ dddd↩│
        │ 4│*4 │↪dd   │
        │ 5│ 5 │ e    │
        │ 6│ ~ │      │
        └──┴───┴──────┘
        ");

        viewport.move_cursor_up(&doc, 10);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│*1 │ aaa  │
        │ 1│ 2 │ bb   │
        │ 2│ 3 │ cccc │
        │ 3│ 4 │ dddd↩│
        │ 4│ 4 │↪dd   │
        │ 5│ 5 │ e    │
        │ 6│ ~ │      │
        └──┴───┴──────┘
        ");
    }

    #[test]
    fn test_move_cursor_up_and_down_and_move_viewport() {
        let (doc, mut viewport) = init(
            b"aaa\nbb\ncccc\ndddddd\neeeeeee\nff\nggggg\nhh\ni\n",
            4,
            5,
            1,
        );
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│*1 │ aaa  │
        │ 1│ 2 │ bb   │
        │ 2│ 3 │ cccc │
        │ 3│ 4 │ dddd↩│
        │ 4│ 4 │↪dd   │
        └──┴───┴──────┘
        ");

        viewport.move_cursor_down(&doc, 1);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 1 │ aaa  │
        │ 1│*2 │ bb   │
        │ 2│ 3 │ cccc │
        │ 3│ 4 │ dddd↩│
        │ 4│ 4 │↪dd   │
        └──┴───┴──────┘
        ");

        viewport.move_cursor_down(&doc, 2);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 2 │ bb   │
        │ 1│ 3 │ cccc │
        │ 2│*4 │ dddd↩│
        │ 3│*4 │↪dd   │
        │ 4│ 5 │ eeee↩│
        └──┴───┴──────┘
        ");

        viewport.move_cursor_down(&doc, 1);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 4 │ dddd↩│
        │ 1│ 4 │↪dd   │
        │ 2│*5 │ eeee↩│
        │ 3│*5 │↪eee  │
        │ 4│ 6 │ ff   │
        └──┴───┴──────┘
        ");

        viewport.move_cursor_up(&doc, 1);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 3 │ cccc │
        │ 1│*4 │ dddd↩│
        │ 2│*4 │↪dd   │
        │ 3│ 5 │ eeee↩│
        │ 4│ 5 │↪eee  │
        └──┴───┴──────┘
        ");

        viewport.move_cursor_down(&doc, 100);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 6 │ ff   │
        │ 1│ 7 │ gggg↩│
        │ 2│ 7 │↪g    │
        │ 3│ 8 │ hh   │
        │ 4│*9 │ i    │
        └──┴───┴──────┘
        ");
    }

    #[test]
    fn test_scroll_up_and_down() {
        let (doc, mut viewport) = init(
            b"aaa\nbb\ncccc\ndddddd\neeeeeee\nff\nggggg\nhh\ni\n",
            4,
            5,
            1,
        );
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│*1 │ aaa  │
        │ 1│ 2 │ bb   │
        │ 2│ 3 │ cccc │
        │ 3│ 4 │ dddd↩│
        │ 4│ 4 │↪dd   │
        └──┴───┴──────┘
        ");

        viewport.scroll_viewport_down(&doc, 1);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 2 │ bb   │
        │ 1│*3 │ cccc │
        │ 2│ 4 │ dddd↩│
        │ 3│ 4 │↪dd   │
        │ 4│ 5 │ eeee↩│
        └──┴───┴──────┘
        ");

        viewport.scroll_viewport_down(&doc, 1);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 3 │ cccc │
        │ 1│*4 │ dddd↩│
        │ 2│*4 │↪dd   │
        │ 3│ 5 │ eeee↩│
        │ 4│ 5 │↪eee  │
        └──┴───┴──────┘
        ");

        viewport.scroll_viewport_down(&doc, 1);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│*4 │ dddd↩│
        │ 1│*4 │↪dd   │
        │ 2│ 5 │ eeee↩│
        │ 3│ 5 │↪eee  │
        │ 4│ 6 │ ff   │
        └──┴───┴──────┘
        ");

        viewport.scroll_viewport_down(&doc, 1);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 4 │↪dd   │
        │ 1│*5 │ eeee↩│
        │ 2│*5 │↪eee  │
        │ 3│ 6 │ ff   │
        │ 4│ 7 │ gggg↩│
        └──┴───┴──────┘
        ");

        viewport.scroll_viewport_down(&doc, 10);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│*9 │ i    │
        │ 1│ ~ │      │
        │ 2│ ~ │      │
        │ 3│ ~ │      │
        │ 4│ ~ │      │
        └──┴───┴──────┘
        ");

        viewport.scroll_viewport_up(&doc, 4);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 6 │ ff   │
        │ 1│ 7 │ gggg↩│
        │ 2│ 7 │↪g    │
        │ 3│*8 │ hh   │
        │ 4│ 9 │ i    │
        └──┴───┴──────┘
        ");

        viewport.scroll_viewport_up(&doc, 1);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 5 │↪eee  │
        │ 1│ 6 │ ff   │
        │ 2│*7 │ gggg↩│
        │ 3│*7 │↪g    │
        │ 4│ 8 │ hh   │
        └──┴───┴──────┘
        ");

        viewport.scroll_viewport_up(&doc, 1);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 5 │ eeee↩│
        │ 1│ 5 │↪eee  │
        │ 2│ 6 │ ff   │
        │ 3│*7 │ gggg↩│
        │ 4│*7 │↪g    │
        └──┴───┴──────┘
        ");

        viewport.scroll_viewport_up(&doc, 1);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 4 │↪dd   │
        │ 1│ 5 │ eeee↩│
        │ 2│ 5 │↪eee  │
        │ 3│*6 │ ff   │
        │ 4│ 7 │ gggg↩│
        └──┴───┴──────┘
        ");

        viewport.scroll_viewport_up(&doc, 10);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 1 │ aaa  │
        │ 1│ 2 │ bb   │
        │ 2│ 3 │ cccc │
        │ 3│*4 │ dddd↩│
        │ 4│*4 │↪dd   │
        └──┴───┴──────┘
        ");
    }

    #[test]
    fn test_scrolling_with_very_long_line() {
        let (doc, mut viewport) = init(b"a\nb\nc1c2c3c4c5c6c7c8\nd\ne\n", 2, 4, 1);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬────┐
        │ 0│*1 │ a  │
        │ 1│ 2 │ b  │
        │ 2│ 3 │ c1↩│
        │ 3│ 3 │↪c2↩│
        └──┴───┴────┘
        ");

        viewport.scroll_viewport_down(&doc, 2);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬────┐
        │ 0│*3 │ c1↩│
        │ 1│*3 │↪c2↩│
        │ 2│*3 │↪c3↩│
        │ 3│*3 │↪c4↩│
        └──┴───┴────┘
        ");

        viewport.scroll_viewport_down(&doc, 4);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬────┐
        │ 0│*3 │↪c5↩│
        │ 1│*3 │↪c6↩│
        │ 2│*3 │↪c7↩│
        │ 3│*3 │↪c8 │
        └──┴───┴────┘
        ");

        viewport.scroll_viewport_down(&doc, 2);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬────┐
        │ 0│*3 │↪c7↩│
        │ 1│*3 │↪c8 │
        │ 2│ 4 │ d  │
        │ 3│ 5 │ e  │
        └──┴───┴────┘
        ");

        viewport.scroll_viewport_down(&doc, 1);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬────┐
        │ 0│ 3 │↪c8 │
        │ 1│*4 │ d  │
        │ 2│ 5 │ e  │
        │ 3│ ~ │    │
        └──┴───┴────┘
        ");

        viewport.scroll_viewport_up(&doc, 2);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬────┐
        │ 0│*3 │↪c6↩│
        │ 1│*3 │↪c7↩│
        │ 2│*3 │↪c8 │
        │ 3│ 4 │ d  │
        └──┴───┴────┘
        ");

        viewport.scroll_viewport_up(&doc, 7);
        assert_snapshot!(viewport.render(&doc), @r"
        ┌SI┬─L#┬────┐
        │ 0│ 1 │ a  │
        │ 1│ 2 │ b  │
        │ 2│*3 │ c1↩│
        │ 3│*3 │↪c2↩│
        └──┴───┴────┘
        ");
    }
}
