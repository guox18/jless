use std::cmp;
use std::ops::RangeInclusive;

use crate::action::Action;
use crate::dimensions::Dimensions;
use crate::document::{CursorRange, Document};

/// The `DocumentViewer` manages what part of a document is displayed on screen
/// as the user takes actions to move the cursor or manipulate the document. Much
/// of the behavior here matches or is inspired by vim behavior. A type that
/// implements `Document` is responsible for deciding the actual content that goes
/// on each line, and when and where line wrapping should occur.
///
/// The basic objective here is to keep whatever part of the document is focused
/// (i.e., where the `Cursor` is) visible within the viewport, and for certain
/// scrolling actions that manipulate the viewport, the cursor should be updated
/// to a position in the document that is within the viewport.

pub struct DocumentViewer<D: Document> {
    pub doc: D,
    top_line: D::ScreenLine,
    // Soon: Make this private again.
    pub current_focus: D::Cursor,
    dimensions: Dimensions,

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
    range_after_considering_scrolloff: RangeInclusive<usize>,
    #[allow(dead_code)]
    #[cfg(debug_assertions)]
    range_after_considering_start_and_end_of_document: RangeInclusive<usize>,
    #[allow(dead_code)]
    #[cfg(debug_assertions)]
    range_after_expanding_due_to_cursor_height: RangeInclusive<usize>,
    // The actual fields
    start: usize,
    end: usize,
}

#[derive(Debug, Copy, Clone)]
enum PositionOfScreenLine {
    AboveTopLine,
    AtScreenIndex(usize),
    BelowBottomLine,
}

impl<D: Document> DocumentViewer<D> {
    pub fn new(
        doc: D,
        first_line: D::ScreenLine,
        initial_cursor: D::Cursor,
        dimensions: Dimensions,
        scrolloff: usize,
    ) -> Self {
        DocumentViewer {
            doc,
            top_line: first_line,
            current_focus: initial_cursor,
            dimensions,
            scrolloff_setting: scrolloff,
        }
    }

    pub fn set_scrolloff(&mut self, scrolloff: usize) {
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
        cmp::min(self.scrolloff_setting, (self.dimensions.height - 1) / 2)
    }

    // If the last line of the file appears before `screen_index`, this will return `None`.
    fn screen_line_at_screen_index(&self, mut screen_index: usize) -> Option<D::ScreenLine> {
        let mut curr_line = self.top_line.clone();
        while screen_index > 0 {
            curr_line = self.doc.next_screen_line(&curr_line)?;
            screen_index -= 1;
        }
        Some(curr_line)
    }

    // If the last line of the file appears before `screen_index`, this will return the
    // last line of the file.
    fn last_screen_line_at_or_before_screen_index(&self, mut screen_index: usize) -> D::ScreenLine {
        let mut curr_line = self.top_line.clone();
        while screen_index > 0 {
            let Some(next_screen_line) = self.doc.next_screen_line(&curr_line) else {
                return curr_line;
            };
            curr_line = next_screen_line;
            screen_index -= 1;
        }
        curr_line
    }

    fn position_of_screen_line(&self, screen_line: &D::ScreenLine) -> PositionOfScreenLine {
        if screen_line < &self.top_line {
            return PositionOfScreenLine::AboveTopLine;
        }

        let mut screen_index = 0;
        let mut curr_screen_line = self.top_line.clone();
        while screen_index < self.dimensions.height {
            if *screen_line == curr_screen_line {
                return PositionOfScreenLine::AtScreenIndex(screen_index);
            }

            screen_index += 1;
            curr_screen_line = self
                .doc
                .next_screen_line(&curr_screen_line)
                .expect("[screen_line] should exist after top line and before EOF");
        }

        PositionOfScreenLine::BelowBottomLine
    }

    pub fn move_cursor_down(&mut self, lines: usize) {
        self.update_so_new_cursor_is_visible(self.doc.move_cursor_down(lines, &self.current_focus));
    }

    pub fn move_cursor_up(&mut self, lines: usize) {
        self.update_so_new_cursor_is_visible(self.doc.move_cursor_up(lines, &self.current_focus));
    }

    pub fn scroll_viewport_down(&mut self, mut lines: usize) {
        let mut lines_scrolled = 0;
        let mut next_top_line = self.top_line.clone();
        while lines > 0 {
            match self.doc.next_screen_line(&next_top_line) {
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
            self.maybe_update_focused_node_after_scroll();
        }
    }

    pub fn scroll_viewport_up(&mut self, mut lines: usize) {
        let mut lines_scrolled = 0;
        let mut next_top_line = self.top_line.clone();
        while lines > 0 {
            match self.doc.prev_screen_line(&next_top_line) {
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
            self.maybe_update_focused_node_after_scroll();
        }
    }

    fn update_so_new_cursor_is_visible(&mut self, new_cursor: Option<D::Cursor>) {
        // If an operation doesn't move the cursor, it will return `None`, so there's
        // nothing to do.
        let Some(new_cursor) = new_cursor else {
            return;
        };

        self.current_focus = new_cursor;

        let (cursor_range, acceptable_start_index_range) =
            self.calculate_acceptable_start_screen_indexes_to_show_cursor_node(&self.current_focus);

        let AcceptableStartScreenIndexesToShowCursorNode {
            start: start_index,
            end: end_index,
            ..
        } = acceptable_start_index_range;

        let screen_line_at_first_acceptable_start = self.screen_line_at_screen_index(start_index);
        let screen_line_at_last_acceptable_start = self.screen_line_at_screen_index(end_index);

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
            self.top_line = self.n_screen_lines_before(cursor_range.start, start_index);
        } else if cursor_start_is_at_or_before_last_acceptable_start {
            // Nothing to do, the cursor is in an acceptable range!
        } else {
            // Cursor is too close to the bottom of the screen (or past it); move the viewport
            // so the cursor is at the end of the acceptable range.
            self.top_line = self.n_screen_lines_before(cursor_range.start, end_index);
        }
    }

    // Someday: We should compute the `CursorRange` outside of this function, pass it in,
    // and then get rid of `cursor_layout_details` in lieu of functions to directly
    // count `bounded_screen_lines_{before,after}_screen_line`.
    fn calculate_acceptable_start_screen_indexes_to_show_cursor_node(
        &self,
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

        let cursor_layout_details = self
            .doc
            .cursor_layout_details(&cursor, self.dimensions.height - 1);

        // Initial acceptable range is the whole screen.
        let mut first_acceptable_screen_index = 0;
        let last_screen_index = self.dimensions.height - 1;
        let mut last_acceptable_screen_index = last_screen_index;

        let min_screenlines_between_edge_of_screen_and_cursor = self.effective_scrolloff();

        // Shrink the acceptable range based on the scrolloff setting. (Because we capped
        // scrolloff at half the height of the screen, that ensures this doesn't cause
        // the values to cross.)
        first_acceptable_screen_index += min_screenlines_between_edge_of_screen_and_cursor;
        last_acceptable_screen_index -= min_screenlines_between_edge_of_screen_and_cursor;

        #[cfg(debug_assertions)]
        let range_after_considering_scrolloff =
            first_acceptable_screen_index..=last_acceptable_screen_index;

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
            first_acceptable_screen_index..=last_acceptable_screen_index;

        // Now we need to expand the acceptable range based on the height of the cursor, up
        // to the whole size of the screen again.

        let height_of_acceptable_range =
            last_acceptable_screen_index - first_acceptable_screen_index + 1;
        let cursor_height = cursor_layout_details.range.num_screen_lines;

        if cursor_height >= self.dimensions.height {
            // Simple case, the cursor is as big or bigger than the screen, so the whole screen
            // is available.
            first_acceptable_screen_index = 0;
            last_acceptable_screen_index = self.dimensions.height - 1;
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
            first_acceptable_screen_index..=last_acceptable_screen_index;

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

    fn screen_indexes_within_scrolloff(&self) -> RangeInclusive<usize> {
        let min_screenlines_between_edge_of_screen_and_cursor = self.effective_scrolloff();

        // If the top of the screen is the top of the document, we don't enforce scrolloff.
        let first_acceptable_screen_index =
            if self.doc.is_first_screen_line_of_document(&self.top_line) {
                0
            } else {
                min_screenlines_between_edge_of_screen_and_cursor
            };

        let last_screen_index = self.dimensions.height - 1;
        let last_acceptable_screen_index =
            last_screen_index - min_screenlines_between_edge_of_screen_and_cursor;

        first_acceptable_screen_index..=last_acceptable_screen_index
    }

    fn maybe_update_focused_node_after_scroll(&mut self) {
        // When scrolling, we'll allow scrolling wrapped lines partly off the screen as long
        // as any part of the focused node still obeys the scrolloff setting.
        let acceptable_screen_indexes = self.screen_indexes_within_scrolloff();

        // Using `last_screen_line_at_or_before_screen_index` handles the case when the end
        // of the file is on screen. In the extreme example, consider if the last line of the
        // document is at the top of the screen. If the screen is 10 lines tall, and scrolloff
        // is 3, then the first/last acceptable screen indexes will be 3 and 6, but the first
        // and last acceptable screen line will both be the current top line, which is the last
        // line of the document.

        let first_acceptable_screen_line =
            self.last_screen_line_at_or_before_screen_index(*acceptable_screen_indexes.start());
        let last_acceptable_screen_line =
            self.last_screen_line_at_or_before_screen_index(*acceptable_screen_indexes.end());

        let focused_range = self.doc.cursor_range(&self.current_focus);

        if focused_range.end < first_acceptable_screen_line {
            self.current_focus = self
                .doc
                .convert_screen_line_to_cursor(first_acceptable_screen_line, &self.current_focus);
        } else if last_acceptable_screen_line < focused_range.start {
            self.current_focus = self
                .doc
                .convert_screen_line_to_cursor(last_acceptable_screen_line, &self.current_focus);
        } else {
            // Current focused range overlaps with acceptable screen line ranges;
            // nothing to do!
        }
    }

    pub fn resize(&mut self, new_dimensions: Dimensions) {
        // Handle resizes in two parts: first resize the width, then the height.
        self.resize_width(new_dimensions.width);
        self.resize_height(new_dimensions.height);
        self.move_current_focus_within_scrolloff_after_resize();
    }

    fn update_dimensions_and_resize_doc(&mut self, dimensions: Dimensions) {
        self.dimensions = dimensions;
        self.doc.resize(dimensions.width);
    }

    fn resize_width(&mut self, new_width: usize) {
        if new_width == self.dimensions.width {
            return;
        }

        let new_dimensions = Dimensions {
            width: new_width,
            ..self.dimensions
        };

        let old_cursor_range = self.doc.cursor_range(&self.current_focus);
        let start_position = self.position_of_screen_line(&old_cursor_range.start);
        let end_position = self.position_of_screen_line(&old_cursor_range.end);

        match (start_position, end_position) {
            (PositionOfScreenLine::AboveTopLine, PositionOfScreenLine::AboveTopLine) => {
                panic!("entirety of focused node is before the start of the screen");
            }
            (PositionOfScreenLine::AboveTopLine, PositionOfScreenLine::AtScreenIndex(index)) => {
                // Don't top line to keep anchored, so we'll keep the end of the line
                // in the same place.
                self.update_dimensions_and_resize_doc(new_dimensions);
                let new_cursor_range = self.doc.cursor_range(&self.current_focus);
                self.top_line =
                    self.n_screen_lines_before_or_top_of_doc(new_cursor_range.end, index);
            }
            (PositionOfScreenLine::AboveTopLine, PositionOfScreenLine::BelowBottomLine) => {
                let lines_above_top_of_screen = self
                    .doc
                    .diff_screen_lines(&self.top_line, &old_cursor_range.start);
                let lines_below_bottom_of_screen = self
                    .doc
                    .diff_screen_lines(&old_cursor_range.end, &self.top_line)
                    - self.dimensions.height;

                self.update_dimensions_and_resize_doc(new_dimensions);
                let new_cursor_range = self.doc.cursor_range(&self.current_focus);

                if old_cursor_range.num_screen_lines == new_cursor_range.num_screen_lines {
                    // If the cursor is the same number of lines long, then we'll keep the lines in
                    // the same spot.
                    self.top_line = self
                        .n_screen_lines_after(new_cursor_range.start, lines_above_top_of_screen);
                } else {
                    // If the size of the cursor changed, we'll try to keep the content
                    // near the center of the screen in approximately the same place.
                    //
                    // For example, if the focused node took up 100 screen lines before,
                    // and lines 70-90 were on screen, that means the 80% percentile of
                    // the node was in the middle of the screen. If, after the resize, it
                    // takes up 60 screen lines, then we want screen line 48 in the middle
                    // of the screen, so the screen will show lines 38-58. This is all
                    // very approximate, so I'm not worried too much about off-by-one errors.
                    let percentile_of_cursor_in_middle_of_screen: f64 = ((lines_above_top_of_screen
                        as f64)
                        + (self.dimensions.height as f64 / 2.0))
                        / (old_cursor_range.num_screen_lines as f64);
                    let new_cursor_index_in_middle_of_screen: usize =
                        (percentile_of_cursor_in_middle_of_screen
                            * (new_cursor_range.num_screen_lines as f64))
                            as usize;

                    let screen_line_in_middle_of_screen = self.n_screen_lines_after(
                        new_cursor_range.start,
                        new_cursor_index_in_middle_of_screen,
                    );
                    self.top_line = self.n_screen_lines_before_or_top_of_doc(
                        screen_line_in_middle_of_screen,
                        self.dimensions.height / 2,
                    );
                }
            }
            (PositionOfScreenLine::AtScreenIndex(index), _) => {
                self.update_dimensions_and_resize_doc(new_dimensions);
                let new_cursor_range = self.doc.cursor_range(&self.current_focus);
                self.top_line =
                    self.n_screen_lines_before_or_top_of_doc(new_cursor_range.start, index);
            }
            (PositionOfScreenLine::BelowBottomLine, _) => {
                panic!("entirety of focused node is past the end of the screen");
            }
        }
    }

    fn resize_height(&mut self, new_height: usize) {
        let old_height = self.dimensions.height;
        if new_height == old_height {
            return;
        }

        // We'll go with vim's approach of keeping the focused in the same percentile of the
        // screen.

        let anchor_screen_line;
        let new_index;

        let convert_old_index_to_new_index = |index| -> usize {
            // We do `old_height - 1` so that when we're using the last line as an anchor, it'll
            // stay the last line.
            let percentile = (index as f64) / ((old_height - 1) as f64);
            (percentile * ((new_height - 1) as f64)).round() as usize
        };

        let cursor_range = self.doc.cursor_range(&self.current_focus);
        let start_position = self.position_of_screen_line(&cursor_range.start);
        let end_position = self.position_of_screen_line(&cursor_range.end);

        match (start_position, end_position) {
            (PositionOfScreenLine::AboveTopLine, PositionOfScreenLine::AboveTopLine) => {
                panic!("entirety of focused node is before the start of the screen");
            }
            (PositionOfScreenLine::AboveTopLine, PositionOfScreenLine::AtScreenIndex(index)) => {
                // Keep the end of focused node in the same percentile
                anchor_screen_line = cursor_range.end;
                new_index = convert_old_index_to_new_index(index);
            }
            (PositionOfScreenLine::AboveTopLine, PositionOfScreenLine::BelowBottomLine) => {
                // Keep middle of what's visible on screen in the middle.
                let half_old_height = self.dimensions.height / 2;
                anchor_screen_line = self.screen_line_at_screen_index(half_old_height).unwrap();
                new_index = new_height / 2;
            }
            (PositionOfScreenLine::AtScreenIndex(index), PositionOfScreenLine::AboveTopLine) => {
                panic!("start of focused node is on screen, but bottom is above the top line");
            }
            (
                PositionOfScreenLine::AtScreenIndex(start_index),
                PositionOfScreenLine::AtScreenIndex(end_index),
            ) => {
                // Keep the middle of focused node in the same percentile.
                let middle_index = (start_index + end_index) / 2;
                anchor_screen_line = self.screen_line_at_screen_index(middle_index).unwrap();
                new_index = convert_old_index_to_new_index(middle_index);
            }
            (PositionOfScreenLine::AtScreenIndex(index), PositionOfScreenLine::BelowBottomLine) => {
                // Keep the start of focused node in the same percentile.
                anchor_screen_line = cursor_range.start;
                new_index = convert_old_index_to_new_index(index);
            }
            (PositionOfScreenLine::BelowBottomLine, _) => {
                panic!("entirety of focused node is past the end of the screen");
            }
        }

        self.top_line = self.n_screen_lines_before_or_top_of_doc(anchor_screen_line, new_index);

        self.dimensions = Dimensions {
            height: new_height,
            ..self.dimensions
        };
    }

    fn move_current_focus_within_scrolloff_after_resize(&mut self) {
        // After a resize, we'll allow part of the focused node to be outside of scrolloff,
        // but if that's not the case we'll move the screen slightly to make it so.
        let acceptable_screen_indexes = self.screen_indexes_within_scrolloff();

        // We use `last_screen_line_at_or_before_screen_index` in `maybe_update_focused_node_after_scroll`
        // to allow scrolling the end of the file to the very top of the screen. We'll use the same
        // relaxation here, so that if you do that, and the resize the screen, the cursor won't
        // "jump" into the scrolloff zone.

        let first_acceptable_screen_line =
            self.last_screen_line_at_or_before_screen_index(*acceptable_screen_indexes.start());
        let last_acceptable_screen_line =
            self.last_screen_line_at_or_before_screen_index(*acceptable_screen_indexes.end());

        let focused_range = self.doc.cursor_range(&self.current_focus);

        if focused_range.end < first_acceptable_screen_line {
            // Put the end of the focused range at the first acceptable screen index.
            self.top_line =
                self.n_screen_lines_before(focused_range.end, *acceptable_screen_indexes.start());
        } else if last_acceptable_screen_line < focused_range.start {
            // Put the start of the focused range at the last acceptable screen index.
            self.top_line =
                self.n_screen_lines_before(focused_range.start, *acceptable_screen_indexes.end());
        } else {
            // Current focused range overlaps with acceptable screen line ranges;
            // nothing to do!
        }
    }

    // Assumes that this will always exist.
    fn n_screen_lines_before(&self, mut screen_line: D::ScreenLine, mut n: usize) -> D::ScreenLine {
        while n > 0 {
            screen_line = self.doc.prev_screen_line(&screen_line).unwrap();
            n -= 1;
        }
        screen_line
    }

    fn n_screen_lines_before_or_top_of_doc(
        &self,
        mut screen_line: D::ScreenLine,
        mut n: usize,
    ) -> D::ScreenLine {
        while n > 0 {
            let Some(prev_screen_line) = self.doc.prev_screen_line(&screen_line) else {
                return screen_line;
            };
            screen_line = prev_screen_line;
            n -= 1;
        }
        screen_line
    }

    // Assumes that this will always exist
    fn n_screen_lines_after(&self, mut screen_line: D::ScreenLine, mut n: usize) -> D::ScreenLine {
        while n > 0 {
            screen_line = self.doc.next_screen_line(&screen_line).unwrap();
            n -= 1;
        }
        screen_line
    }

    pub fn document_eof(&mut self) {
        self.doc.eof();
    }

    pub fn append_document_data(&mut self, data: &[u8]) {
        self.doc.append(data);
    }

    pub fn do_action(&mut self, action: Action) {
        match action {
            Action::NoOp => (),
            Action::MoveCursorDown(n) => self.move_cursor_down(n),
            Action::MoveCursorUp(n) => self.move_cursor_up(n),
            Action::ScrollViewportDown(n) => self.scroll_viewport_down(n),
            Action::ScrollViewportUp(n) => self.scroll_viewport_up(n),
        }
    }

    pub fn viewport_lines<'a>(&'a self) -> impl Iterator<Item = Option<D::ScreenLine>> + 'a {
        ViewportLinesIterator {
            document: &self.doc,
            next_line: Some(self.top_line.clone()),
            remaining_height: self.dimensions.height,
        }
    }
}

struct ViewportLinesIterator<'a, D: Document> {
    document: &'a D,
    next_line: Option<D::ScreenLine>,
    remaining_height: usize,
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

        self.next_line = self.document.next_screen_line(&curr_line);

        Some(Some(curr_line))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use bstr::ByteSlice;
    use insta::{allow_duplicates, assert_debug_snapshot, assert_snapshot};

    use std::fmt::Write;

    use crate::dimensions::Dimensions;
    use crate::text_document::{Cursor, TextDocument};

    fn init(
        contents: &[u8],
        width: usize,
        height: usize,
        scrolloff: usize,
    ) -> DocumentViewer<TextDocument> {
        let mut doc = TextDocument::new(width);
        doc.append(contents);
        doc.eof();

        let (top_line, initial_cursor) = doc.top_screen_line_and_cursor().unwrap();
        let dimensions = Dimensions { width, height };
        DocumentViewer::new(doc, top_line, initial_cursor, dimensions, scrolloff)
    }

    impl<D: Document> DocumentViewer<D> {
        fn render(&self) -> String {
            // |12345678       9|
            // | ##|##| <width> |
            let content_width = self.dimensions.width;
            let mut s = String::new();
            writeln!(s, "┌SI┬─L#┬─{:─<content_width$}─┐", "").unwrap();
            for (screen_index, screen_line) in self.viewport_lines().enumerate() {
                let Some(screen_line) = screen_line else {
                    writeln!(s, "│{:>2}│ ~ │ {: <content_width$} │", screen_index, "").unwrap();
                    continue;
                };

                let is_focused = self
                    .doc
                    .does_screen_line_intersect_cursor(&screen_line, &self.current_focus);
                let line_number = self.doc.line_number(&screen_line);
                let wraps_from_prev_line = self.doc.is_after_start_of_wrapped_line(&screen_line);
                let wraps_onto_next_line = self.doc.is_before_end_of_wrapped_line(&screen_line);

                writeln!(
                    s,
                    "│{:>2}│{}{:<2}│{}{: <content_width$}{}│",
                    screen_index,
                    if is_focused { '*' } else { ' ' },
                    line_number,
                    if wraps_from_prev_line { '↪' } else { ' ' },
                    self.doc.debug_text_content(&screen_line).as_bstr(),
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
        let viewer = init(b"aaa\nbb\ncccc\ndddddd\ne\n", 4, 7, 0);
        assert_snapshot!(viewer.render(), @r"
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
        viewer: &DocumentViewer<TextDocument>,
        cursor: &Cursor,
    ) -> AcceptableStartScreenIndexesToShowCursorNode {
        viewer
            .calculate_acceptable_start_screen_indexes_to_show_cursor_node(cursor)
            .1
    }

    #[test]
    fn test_acceptable_start_screen_indexes() {
        let mut viewer = init(b"a\nbbbb\nc\nd\ne\nf\ng\n", 1, 10, 0);
        assert_snapshot!(viewer.render(), @r"
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

        let line_2 = viewer.doc.cursor_to_line_n(2);
        assert_debug_snapshot!(acceptable_screen_indexes(&viewer, &line_2), @r"
        AcceptableStartScreenIndexesToShowCursorNode {
            cursor_height: 4,
            last_screen_index: 9,
            range_after_considering_scrolloff: 0..=9,
            range_after_considering_start_and_end_of_document: 0..=9,
            range_after_expanding_due_to_cursor_height: 0..=9,
            start: 0,
            end: 6,
        }
        ");

        viewer.set_scrolloff(3);
        assert_debug_snapshot!(acceptable_screen_indexes(&viewer, &line_2), @r"
        AcceptableStartScreenIndexesToShowCursorNode {
            cursor_height: 4,
            last_screen_index: 9,
            range_after_considering_scrolloff: 3..=6,
            range_after_considering_start_and_end_of_document: 1..=6,
            range_after_expanding_due_to_cursor_height: 1..=6,
            start: 1,
            end: 3,
        }
        ");

        // Example from the comment in `calculate_acceptable_start_screen_indexes_to_show_cursor_node`:
        let viewer = init(b"a\nbbbbbbbb\nc\nd\ne\nf\n", 1, 10, 4);
        assert_snapshot!(viewer.render(), @r"
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

        let line_2 = viewer.doc.cursor_to_line_n(2);
        assert_debug_snapshot!(acceptable_screen_indexes(&viewer, &line_2), @r"
        AcceptableStartScreenIndexesToShowCursorNode {
            cursor_height: 8,
            last_screen_index: 9,
            range_after_considering_scrolloff: 4..=5,
            range_after_considering_start_and_end_of_document: 1..=5,
            range_after_expanding_due_to_cursor_height: 1..=8,
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
                let viewer = init(input.as_bytes(), 1, 3, 0);
                assert_snapshot!(viewer.render(), @r"
                ┌SI┬─L#┬───┐
                │ 0│*1 │ a │
                │ 1│ 2 │ b │
                │ 2│ 3 │ c │
                └──┴───┴───┘
                ");

                let line_4 = viewer.doc.cursor_to_line_n(4);
                let mut acceptable_screen_indexes = acceptable_screen_indexes(&viewer, &line_4);
                assert_eq!(acceptable_screen_indexes.cursor_height, *height);
                // Clear for the snapshot, since it differs
                acceptable_screen_indexes.cursor_height = 0;
                assert_debug_snapshot!(acceptable_screen_indexes, @r"
                AcceptableStartScreenIndexesToShowCursorNode {
                    cursor_height: 0,
                    last_screen_index: 2,
                    range_after_considering_scrolloff: 0..=2,
                    range_after_considering_start_and_end_of_document: 0..=2,
                    range_after_expanding_due_to_cursor_height: 0..=2,
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
                let viewer = init(input.as_bytes(), 1, 4, 0);
                assert_snapshot!(viewer.render(), @r"
                ┌SI┬─L#┬───┐
                │ 0│*1 │ a │
                │ 1│ 2 │ b │
                │ 2│ 3 │ c │
                │ 3│ 4 │ d↩│
                └──┴───┴───┘
                ");

                let line_4 = viewer.doc.cursor_to_line_n(4);
                let mut acceptable_screen_indexes = acceptable_screen_indexes(&viewer, &line_4);
                assert_eq!(acceptable_screen_indexes.cursor_height, *height);
                // Clear for the snapshot, since it differs
                acceptable_screen_indexes.cursor_height = 0;
                assert_debug_snapshot!(acceptable_screen_indexes, @r"
                AcceptableStartScreenIndexesToShowCursorNode {
                    cursor_height: 0,
                    last_screen_index: 3,
                    range_after_considering_scrolloff: 0..=3,
                    range_after_considering_start_and_end_of_document: 0..=3,
                    range_after_expanding_due_to_cursor_height: 0..=3,
                    start: 0,
                    end: 0,
                }
                ");
            }
        }
    }

    #[test]
    fn test_move_cursor_up_and_down() {
        let mut viewer = init(b"aaa\nbb\ncccc\ndddddd\ne\n", 4, 7, 0);
        assert_snapshot!(viewer.render(), @r"
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

        viewer.move_cursor_down(1);
        assert_snapshot!(viewer.render(), @r"
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

        viewer.move_cursor_down(3);
        assert_snapshot!(viewer.render(), @r"
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

        viewer.move_cursor_up(1);
        assert_snapshot!(viewer.render(), @r"
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

        viewer.move_cursor_up(10);
        assert_snapshot!(viewer.render(), @r"
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
        let mut viewer = init(
            b"aaa\nbb\ncccc\ndddddd\neeeeeee\nff\nggggg\nhh\ni\n",
            4,
            5,
            1,
        );
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│*1 │ aaa  │
        │ 1│ 2 │ bb   │
        │ 2│ 3 │ cccc │
        │ 3│ 4 │ dddd↩│
        │ 4│ 4 │↪dd   │
        └──┴───┴──────┘
        ");

        viewer.move_cursor_down(1);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 1 │ aaa  │
        │ 1│*2 │ bb   │
        │ 2│ 3 │ cccc │
        │ 3│ 4 │ dddd↩│
        │ 4│ 4 │↪dd   │
        └──┴───┴──────┘
        ");

        viewer.move_cursor_down(2);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 2 │ bb   │
        │ 1│ 3 │ cccc │
        │ 2│*4 │ dddd↩│
        │ 3│*4 │↪dd   │
        │ 4│ 5 │ eeee↩│
        └──┴───┴──────┘
        ");

        viewer.move_cursor_down(1);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 4 │ dddd↩│
        │ 1│ 4 │↪dd   │
        │ 2│*5 │ eeee↩│
        │ 3│*5 │↪eee  │
        │ 4│ 6 │ ff   │
        └──┴───┴──────┘
        ");

        viewer.move_cursor_up(1);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 3 │ cccc │
        │ 1│*4 │ dddd↩│
        │ 2│*4 │↪dd   │
        │ 3│ 5 │ eeee↩│
        │ 4│ 5 │↪eee  │
        └──┴───┴──────┘
        ");

        viewer.move_cursor_down(100);
        assert_snapshot!(viewer.render(), @r"
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
        let mut viewer = init(
            b"aaa\nbb\ncccc\ndddddd\neeeeeee\nff\nggggg\nhh\ni\n",
            4,
            5,
            1,
        );
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│*1 │ aaa  │
        │ 1│ 2 │ bb   │
        │ 2│ 3 │ cccc │
        │ 3│ 4 │ dddd↩│
        │ 4│ 4 │↪dd   │
        └──┴───┴──────┘
        ");

        viewer.scroll_viewport_down(1);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 2 │ bb   │
        │ 1│*3 │ cccc │
        │ 2│ 4 │ dddd↩│
        │ 3│ 4 │↪dd   │
        │ 4│ 5 │ eeee↩│
        └──┴───┴──────┘
        ");

        viewer.scroll_viewport_down(1);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 3 │ cccc │
        │ 1│*4 │ dddd↩│
        │ 2│*4 │↪dd   │
        │ 3│ 5 │ eeee↩│
        │ 4│ 5 │↪eee  │
        └──┴───┴──────┘
        ");

        viewer.scroll_viewport_down(1);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│*4 │ dddd↩│
        │ 1│*4 │↪dd   │
        │ 2│ 5 │ eeee↩│
        │ 3│ 5 │↪eee  │
        │ 4│ 6 │ ff   │
        └──┴───┴──────┘
        ");

        viewer.scroll_viewport_down(1);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 4 │↪dd   │
        │ 1│*5 │ eeee↩│
        │ 2│*5 │↪eee  │
        │ 3│ 6 │ ff   │
        │ 4│ 7 │ gggg↩│
        └──┴───┴──────┘
        ");

        viewer.scroll_viewport_down(10);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│*9 │ i    │
        │ 1│ ~ │      │
        │ 2│ ~ │      │
        │ 3│ ~ │      │
        │ 4│ ~ │      │
        └──┴───┴──────┘
        ");

        viewer.scroll_viewport_up(4);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 6 │ ff   │
        │ 1│ 7 │ gggg↩│
        │ 2│ 7 │↪g    │
        │ 3│*8 │ hh   │
        │ 4│ 9 │ i    │
        └──┴───┴──────┘
        ");

        viewer.scroll_viewport_up(1);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 5 │↪eee  │
        │ 1│ 6 │ ff   │
        │ 2│*7 │ gggg↩│
        │ 3│*7 │↪g    │
        │ 4│ 8 │ hh   │
        └──┴───┴──────┘
        ");

        viewer.scroll_viewport_up(1);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 5 │ eeee↩│
        │ 1│ 5 │↪eee  │
        │ 2│ 6 │ ff   │
        │ 3│*7 │ gggg↩│
        │ 4│*7 │↪g    │
        └──┴───┴──────┘
        ");

        viewer.scroll_viewport_up(1);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│ 4 │↪dd   │
        │ 1│ 5 │ eeee↩│
        │ 2│ 5 │↪eee  │
        │ 3│*6 │ ff   │
        │ 4│ 7 │ gggg↩│
        └──┴───┴──────┘
        ");

        viewer.scroll_viewport_up(10);
        assert_snapshot!(viewer.render(), @r"
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
        let mut viewer = init(b"a\nb\nc1c2c3c4c5c6c7c8\nd\ne\n", 2, 4, 1);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬────┐
        │ 0│*1 │ a  │
        │ 1│ 2 │ b  │
        │ 2│ 3 │ c1↩│
        │ 3│ 3 │↪c2↩│
        └──┴───┴────┘
        ");

        viewer.scroll_viewport_down(2);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬────┐
        │ 0│*3 │ c1↩│
        │ 1│*3 │↪c2↩│
        │ 2│*3 │↪c3↩│
        │ 3│*3 │↪c4↩│
        └──┴───┴────┘
        ");

        viewer.scroll_viewport_down(4);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬────┐
        │ 0│*3 │↪c5↩│
        │ 1│*3 │↪c6↩│
        │ 2│*3 │↪c7↩│
        │ 3│*3 │↪c8 │
        └──┴───┴────┘
        ");

        viewer.scroll_viewport_down(2);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬────┐
        │ 0│*3 │↪c7↩│
        │ 1│*3 │↪c8 │
        │ 2│ 4 │ d  │
        │ 3│ 5 │ e  │
        └──┴───┴────┘
        ");

        viewer.scroll_viewport_down(1);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬────┐
        │ 0│ 3 │↪c8 │
        │ 1│*4 │ d  │
        │ 2│ 5 │ e  │
        │ 3│ ~ │    │
        └──┴───┴────┘
        ");

        viewer.scroll_viewport_up(2);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬────┐
        │ 0│*3 │↪c6↩│
        │ 1│*3 │↪c7↩│
        │ 2│*3 │↪c8 │
        │ 3│ 4 │ d  │
        └──┴───┴────┘
        ");

        viewer.scroll_viewport_up(7);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬────┐
        │ 0│ 1 │ a  │
        │ 1│ 2 │ b  │
        │ 2│*3 │ c1↩│
        │ 3│*3 │↪c2↩│
        └──┴───┴────┘
        ");
    }

    #[test]
    fn test_resize_width() {
        let text = b"a\n\
            b\n\
            c\n\
            d\n\
            1eeee2eeee3eeee4e!ee5eeee6eeee7eeee8eeee9eeee0eeee\n\
            f\n\
            g\n\
            h\n\
            i\n";
        let mut viewer = init(text, 5, 5, 0);
        viewer.move_cursor_down(4);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬───────┐
        │ 0│*5 │ 1eeee↩│
        │ 1│*5 │↪2eeee↩│
        │ 2│*5 │↪3eeee↩│
        │ 3│*5 │↪4e!ee↩│
        │ 4│*5 │↪5eeee↩│
        └──┴───┴───────┘
        ");
        viewer.scroll_viewport_down(6);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬───────┐
        │ 0│*5 │↪7eeee↩│
        │ 1│*5 │↪8eeee↩│
        │ 2│*5 │↪9eeee↩│
        │ 3│*5 │↪0eeee │
        │ 4│ 6 │ f     │
        └──┴───┴───────┘
        ");

        // start AboveTopLine, end AtScreenIndex case, keep end of cursor in same spot
        viewer.resize_width(25);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬───────────────────────────┐
        │ 0│ 3 │ c                         │
        │ 1│ 4 │ d                         │
        │ 2│*5 │ 1eeee2eeee3eeee4e!ee5eeee↩│
        │ 3│*5 │↪6eeee7eeee8eeee9eeee0eeee │
        │ 4│ 6 │ f                         │
        └──┴───┴───────────────────────────┘
        ");

        // start AtScreenIndex case, keep start of cursor in same spot
        viewer.resize_width(5);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬───────┐
        │ 0│ 3 │ c     │
        │ 1│ 4 │ d     │
        │ 2│*5 │ 1eeee↩│
        │ 3│*5 │↪2eeee↩│
        │ 4│*5 │↪3eeee↩│
        └──┴───┴───────┘
        ");

        viewer.scroll_viewport_down(3);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬───────┐
        │ 0│*5 │↪2eeee↩│
        │ 1│*5 │↪3eeee↩│
        │ 2│*5 │↪4e!ee↩│
        │ 3│*5 │↪5eeee↩│
        │ 4│*5 │↪6eeee↩│
        └──┴───┴───────┘
        ");

        // Now we're showing chars 6-30 on screen, so char 18 out of 50 is in the middle, aka 36th
        // percentile. If we make the screen 2 chars wide, we should see that in the middle of the
        // screen.

        viewer.resize_width(2);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬────┐
        │ 0│*5 │↪ee↩│
        │ 1│*5 │↪e4↩│
        │ 2│*5 │↪e!↩│
        │ 3│*5 │↪ee↩│
        │ 4│*5 │↪5e↩│
        └──┴───┴────┘
        ");

        viewer.resize_width(3);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬─────┐
        │ 0│*5 │↪e3e↩│
        │ 1│*5 │↪eee↩│
        │ 2│*5 │↪4e!↩│
        │ 3│*5 │↪ee5↩│
        │ 4│*5 │↪eee↩│
        └──┴───┴─────┘
        ");

        viewer.resize_width(4);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬──────┐
        │ 0│*5 │↪ee3e↩│
        │ 1│*5 │↪eee4↩│
        │ 2│*5 │↪e!ee↩│
        │ 3│*5 │↪5eee↩│
        │ 4│*5 │↪e6ee↩│
        └──┴───┴──────┘
        ");

        viewer.resize_width(5);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬───────┐
        │ 0│*5 │↪2eeee↩│
        │ 1│*5 │↪3eeee↩│
        │ 2│*5 │↪4e!ee↩│
        │ 3│*5 │↪5eeee↩│
        │ 4│*5 │↪6eeee↩│
        └──┴───┴───────┘
        ");

        // Guess there's a slight off-by-one here, but seems fine.
        viewer.resize_width(6);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬────────┐
        │ 0│*5 │↪eeee3e↩│
        │ 1│*5 │↪eee4e!↩│
        │ 2│*5 │↪ee5eee↩│
        │ 3│*5 │↪e6eeee↩│
        │ 4│*5 │↪7eeee8↩│
        └──┴───┴────────┘
        ");
    }

    #[test]
    fn test_resize_height() {
        let text = b"a\n\
            b\n\
            c\n\
            d\n\
            1ee2ee3ee4ee5ee6ee7ee8ee9ee0ee\n\
            f\n\
            g\n\
            h\n\
            i\n";
        let mut viewer = init(text, 3, 5, 0);
        viewer.move_cursor_down(4);
        viewer.scroll_viewport_up(1);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬─────┐
        │ 0│ 4 │ d   │
        │ 1│*5 │ 1ee↩│
        │ 2│*5 │↪2ee↩│
        │ 3│*5 │↪3ee↩│
        │ 4│*5 │↪4ee↩│
        └──┴───┴─────┘
        ");

        viewer.resize_height(10);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬─────┐
        │ 0│ 3 │ c   │
        │ 1│ 4 │ d   │
        │ 2│*5 │ 1ee↩│
        │ 3│*5 │↪2ee↩│
        │ 4│*5 │↪3ee↩│
        │ 5│*5 │↪4ee↩│
        │ 6│*5 │↪5ee↩│
        │ 7│*5 │↪6ee↩│
        │ 8│*5 │↪7ee↩│
        │ 9│*5 │↪8ee↩│
        └──┴───┴─────┘
        ");

        let mut viewer = init(text, 3, 5, 0);
        viewer.move_cursor_down(4);
        viewer.scroll_viewport_down(8);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬─────┐
        │ 0│*5 │↪9ee↩│
        │ 1│*5 │↪0ee │
        │ 2│ 6 │ f   │
        │ 3│ 7 │ g   │
        │ 4│ 8 │ h   │
        └──┴───┴─────┘
        ");

        viewer.resize_height(10);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬─────┐
        │ 0│*5 │↪8ee↩│
        │ 1│*5 │↪9ee↩│
        │ 2│*5 │↪0ee │
        │ 3│ 6 │ f   │
        │ 4│ 7 │ g   │
        │ 5│ 8 │ h   │
        │ 6│ 9 │ i   │
        │ 7│ ~ │     │
        │ 8│ ~ │     │
        │ 9│ ~ │     │
        └──┴───┴─────┘
        ");

        let mut viewer = init(text, 3, 5, 0);
        viewer.move_cursor_down(4);
        viewer.scroll_viewport_down(1);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬─────┐
        │ 0│*5 │↪2ee↩│
        │ 1│*5 │↪3ee↩│
        │ 2│*5 │↪4ee↩│
        │ 3│*5 │↪5ee↩│
        │ 4│*5 │↪6ee↩│
        └──┴───┴─────┘
        ");
        viewer.resize_height(3);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬─────┐
        │ 0│*5 │↪3ee↩│
        │ 1│*5 │↪4ee↩│
        │ 2│*5 │↪5ee↩│
        └──┴───┴─────┘
        ");

        let mut viewer = init(text, 10, 5, 0);
        viewer.move_cursor_down(4);
        viewer.scroll_viewport_down(1);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬────────────┐
        │ 0│ 4 │ d          │
        │ 1│*5 │ 1ee2ee3ee4↩│
        │ 2│*5 │↪ee5ee6ee7e↩│
        │ 3│*5 │↪e8ee9ee0ee │
        │ 4│ 6 │ f          │
        └──┴───┴────────────┘
        ");
        viewer.resize_height(7);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬────────────┐
        │ 0│ 3 │ c          │
        │ 1│ 4 │ d          │
        │ 2│*5 │ 1ee2ee3ee4↩│
        │ 3│*5 │↪ee5ee6ee7e↩│
        │ 4│*5 │↪e8ee9ee0ee │
        │ 5│ 6 │ f          │
        │ 6│ 7 │ g          │
        └──┴───┴────────────┘
        ");
    }

    #[test]
    fn test_resize() {
        let text = b"\
            01\n02\n03\n04\n55\n06\n07\n08\n09\n10\n\
            11\n12\n13\n14\n15\n16\n17\n18\n19\n20\n";
        let mut viewer = init(text, 3, 15, 0);
        viewer.move_cursor_down(13);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬─────┐
        │ 0│ 1 │ 01  │
        │ 1│ 2 │ 02  │
        │ 2│ 3 │ 03  │
        │ 3│ 4 │ 04  │
        │ 4│ 5 │ 55  │
        │ 5│ 6 │ 06  │
        │ 6│ 7 │ 07  │
        │ 7│ 8 │ 08  │
        │ 8│ 9 │ 09  │
        │ 9│ 10│ 10  │
        │10│ 11│ 11  │
        │11│ 12│ 12  │
        │12│ 13│ 13  │
        │13│*14│ 14  │
        │14│ 15│ 15  │
        └──┴───┴─────┘
        ");

        let new_dimensions = Dimensions {
            width: 3,
            height: 4,
        };
        viewer.resize(new_dimensions);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬─────┐
        │ 0│ 11│ 11  │
        │ 1│ 12│ 12  │
        │ 2│ 13│ 13  │
        │ 3│*14│ 14  │
        └──┴───┴─────┘
        ");

        // Same as above, but now with scrolloff = 1; the new cursor position obeys scrolloff.
        let mut viewer = init(text, 3, 15, 1);
        viewer.move_cursor_down(13);
        viewer.resize(new_dimensions);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬─────┐
        │ 0│ 12│ 12  │
        │ 1│ 13│ 13  │
        │ 2│*14│ 14  │
        │ 3│ 15│ 15  │
        └──┴───┴─────┘
        ");

        let text = b"a\nb\nc\nd\nxxxxxxxxxxxxxxxxxxxx\ne\nf\ng\n";
        let mut viewer = init(text, 3, 5, 2);
        viewer.move_cursor_down(3);
        viewer.scroll_viewport_down(2);
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬─────┐
        │ 0│ 4 │ d   │
        │ 1│*5 │ xxx↩│
        │ 2│*5 │↪xxx↩│
        │ 3│*5 │↪xxx↩│
        │ 4│*5 │↪xxx↩│
        └──┴───┴─────┘
        ");

        // Anchor point is second line, but snaps into viewport.
        viewer.resize(Dimensions {
            width: 30,
            height: 5,
        });
        assert_snapshot!(viewer.render(), @r"
        ┌SI┬─L#┬────────────────────────────────┐
        │ 0│ 3 │ c                              │
        │ 1│ 4 │ d                              │
        │ 2│*5 │ xxxxxxxxxxxxxxxxxxxx           │
        │ 3│ 6 │ e                              │
        │ 4│ 7 │ f                              │
        └──┴───┴────────────────────────────────┘
        ");
    }
}
