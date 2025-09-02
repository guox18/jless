use std::ops::Range;
use std::rc::Rc;

use crate::document::Document;

// Precalculated break points when displaying a long line
// Someday: Maybe save info about whether the range contains non-ASCII, or
// escaped characters.
#[derive(Debug)]
struct BreakPoints(Vec<usize>);

#[derive(Clone, Debug)]
struct SegmentOfWrappedLine {
    break_points: Rc<BreakPoints>,
    index: usize,
}

impl SegmentOfWrappedLine {
    fn is_start(&self) -> bool {
        self.index == 0
    }

    fn is_end(&self) -> bool {
        self.index == self.break_points.len() - 1
    }

    fn is_after_start(&self) -> bool {
        self.index > 0
    }

    fn is_before_end(&self) -> bool {
        self.index < self.break_points.len() - 1
    }
}

impl BreakPoints {
    // Someday: Handle control characters, UTF-8, etc.
    fn calculate(bytes: &[u8], width: usize) -> Option<BreakPoints> {
        let len = bytes.len();
        if len <= width {
            return None;
        }

        let mut offsets = vec![];
        let mut curr_offset = 0;
        while curr_offset < len {
            offsets.push(curr_offset);
            curr_offset += width;
        }

        Some(BreakPoints(offsets))
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn nth_segment<'a, 'b>(&'a self, bytes: &'b [u8], n: usize) -> &'b [u8] {
        let start = self.0[n];
        if n + 1 < self.0.len() {
            let end = self.0[n + 1];
            &bytes[start..end]
        } else {
            &bytes[start..]
        }
    }
}

#[derive(Clone, Debug)]
pub struct ScreenLine {
    line_index: usize,
    // If we have to wrap the line, precalculated line breaks, and the specific line wrap
    // we're showing.
    segment_of_wrapped_line: Option<SegmentOfWrappedLine>,
}

// Cursor is just a plain line number
pub type Cursor = usize;

pub struct TextDocument {
    data: Vec<u8>,
    complete_line_ranges: Vec<Range<usize>>,
    next_start: usize,
    trailing_newline: Option<bool>,
}

impl TextDocument {
    pub fn new() -> Self {
        TextDocument {
            data: vec![],
            complete_line_ranges: vec![],
            next_start: 0,
            trailing_newline: None,
        }
    }

    fn num_lines(&self) -> usize {
        self.complete_line_ranges.len()
    }

    fn line_zero_indexed(&self, n: usize) -> &[u8] {
        &self.data[self.complete_line_ranges[n].clone()]
    }

    fn trailing_newline(&self) -> Option<bool> {
        self.trailing_newline
    }

    fn create_ref_to_start_of_line(&self, line_index: usize, display_width: usize) -> ScreenLine {
        let line = self.line_zero_indexed(line_index);
        let segment_of_wrapped_line = match BreakPoints::calculate(line, display_width) {
            None => None,
            Some(break_points) => Some(SegmentOfWrappedLine {
                break_points: Rc::new(break_points),
                index: 0,
            }),
        };

        ScreenLine {
            line_index,
            segment_of_wrapped_line,
        }
    }

    fn screen_line_contents(&self, screen_line: &ScreenLine) -> &[u8] {
        let full_line = self.line_zero_indexed(screen_line.line_index);
        match &screen_line.segment_of_wrapped_line {
            None => full_line,
            Some(SegmentOfWrappedLine {
                break_points,
                index,
            }) => break_points.nth_segment(full_line, *index),
        }
    }
}

impl Document for TextDocument {
    type ScreenLine = ScreenLine;
    type Cursor = Cursor;

    fn append(&mut self, data: &[u8]) {
        let len = self.data.len();
        self.data.extend_from_slice(data);

        for newline_offset in memchr::memchr_iter(b'\n', data) {
            let end = len + newline_offset;
            let line_range = if end > 0 && self.data[end - 1] == b'\r' {
                self.next_start..(end - 1)
            } else {
                self.next_start..end
            };

            self.complete_line_ranges.push(line_range);
            self.next_start = len + newline_offset + 1;
        }
    }

    fn eof(&mut self) {
        let end = self.data.len();
        if end > self.next_start {
            self.trailing_newline = Some(false);
            self.complete_line_ranges.push(self.next_start..end);
        } else {
            self.trailing_newline = Some(true);
        }
    }

    fn init_top_screen_line_and_cursor(
        &self,
        display_width: usize,
    ) -> Option<(ScreenLine, Cursor)> {
        if self.complete_line_ranges.is_empty() {
            return None;
        }

        let screen_line = self.create_ref_to_start_of_line(0, display_width);
        let cursor = 0;
        Some((screen_line, cursor))
    }

    fn next_screen_line(
        &self,
        screen_line: &ScreenLine,
        display_width: usize,
    ) -> Option<ScreenLine> {
        let num_lines = self.num_lines();
        let next_line_index = screen_line.line_index + 1;

        match &screen_line.segment_of_wrapped_line {
            None => {
                if next_line_index == num_lines {
                    None
                } else {
                    Some(self.create_ref_to_start_of_line(next_line_index, display_width))
                }
            }
            Some(SegmentOfWrappedLine {
                break_points,
                index,
            }) => {
                if *index + 1 < break_points.len() {
                    Some(ScreenLine {
                        line_index: screen_line.line_index,
                        segment_of_wrapped_line: Some(SegmentOfWrappedLine {
                            break_points: Rc::clone(break_points),
                            index: *index + 1,
                        }),
                    })
                } else if next_line_index == num_lines {
                    None
                } else {
                    Some(self.create_ref_to_start_of_line(next_line_index, display_width))
                }
            }
        }
    }

    fn line_number(&self, screen_line: &ScreenLine) -> usize {
        screen_line.line_index + 1
    }

    fn is_wrapped_line(&self, screen_line: &ScreenLine) -> bool {
        screen_line.segment_of_wrapped_line.is_some()
    }

    fn is_start_of_wrapped_line(&self, screen_line: &ScreenLine) -> bool {
        screen_line
            .segment_of_wrapped_line
            .as_ref()
            .map_or(false, SegmentOfWrappedLine::is_start)
    }

    fn is_end_of_wrapped_line(&self, screen_line: &ScreenLine) -> bool {
        screen_line
            .segment_of_wrapped_line
            .as_ref()
            .map_or(false, SegmentOfWrappedLine::is_end)
    }

    fn is_after_start_of_wrapped_line(&self, screen_line: &ScreenLine) -> bool {
        screen_line
            .segment_of_wrapped_line
            .as_ref()
            .map_or(false, SegmentOfWrappedLine::is_after_start)
    }

    fn is_before_end_of_wrapped_line(&self, screen_line: &ScreenLine) -> bool {
        screen_line
            .segment_of_wrapped_line
            .as_ref()
            .map_or(false, SegmentOfWrappedLine::is_before_end)
    }

    #[cfg(test)]
    fn debug_text_content(&self, screen_line: &Self::ScreenLine) -> &[u8] {
        self.screen_line_contents(screen_line)
    }

    fn move_cursor_up(&self, cursor: &Cursor) -> Option<Cursor> {
        if *cursor == 0 {
            None
        } else {
            Some(*cursor - 1)
        }
    }

    fn move_cursor_down(&self, cursor: &Cursor) -> Option<Cursor> {
        let next = *cursor + 1;
        if next >= self.num_lines() {
            None
        } else {
            Some(next)
        }
    }
}

#[cfg(test)]

mod tests {
    use super::*;

    use bstr::ByteSlice;
    use insta::assert_snapshot;

    use std::fmt::Write;

    fn print_lines(doc: &TextDocument) -> String {
        let mut s = String::new();
        for n in 0..doc.num_lines() {
            writeln!(s, "{}: {:?}", n + 1, doc.line_zero_indexed(n).as_bstr()).unwrap();
        }
        s
    }

    #[test]
    fn test_text_document() {
        let mut doc = TextDocument::new();
        assert_snapshot!(print_lines(&doc), @"");

        doc.append(b"abc");
        assert_snapshot!(print_lines(&doc), @"");

        doc.append(b"def\n");
        assert_snapshot!(print_lines(&doc), @r#"1: "abcdef""#);

        doc.append(b"ghi\n\njkl");
        assert_snapshot!(print_lines(&doc), @r#"
        1: "abcdef"
        2: "ghi"
        3: ""
        "#);
        assert_eq!(None, doc.trailing_newline());

        doc.eof();
        assert_snapshot!(print_lines(&doc), @r#"
        1: "abcdef"
        2: "ghi"
        3: ""
        4: "jkl"
        "#);
        assert_eq!(Some(false), doc.trailing_newline());
    }

    #[test]
    fn test_leading_and_trailing_newline() {
        let mut doc = TextDocument::new();
        doc.append(b"\nabc\n");
        assert_snapshot!(print_lines(&doc), @r#"
        1: ""
        2: "abc"
        "#);

        doc.eof();
        assert_snapshot!(print_lines(&doc), @r#"
        1: ""
        2: "abc"
        "#);
        assert_eq!(Some(true), doc.trailing_newline());
    }

    #[test]
    fn test_crlf_line_endings() {
        let mut doc = TextDocument::new();
        doc.append(b"abc\r\n");
        doc.append(b"def\r");
        doc.append(b"\nghi\r");
        doc.eof();
        assert_snapshot!(print_lines(&doc), @r#"
        1: "abc"
        2: "def"
        3: "ghi\r"
        "#);
        assert_eq!(Some(false), doc.trailing_newline());
    }

    fn print_screen_lines(doc: &TextDocument, width: usize) -> String {
        let mut s = String::new();
        let mut screen_line = Some(doc.init_top_screen_line_and_cursor(width).unwrap().0);

        while let Some(line) = screen_line {
            write!(
                s,
                "|{: <width$}",
                doc.screen_line_contents(&line).as_bstr(),
                width = width,
            );
            writeln!(s, "|");
            screen_line = doc.next_screen_line(&line, width);
        }

        s
    }

    #[test]
    fn test_next_line() {
        let mut doc = TextDocument::new();
        doc.append(b"line.1\n");
        //           0123456789012345
        doc.append(b"long.long.line.2\n");
        doc.append(b"line.3\n");

        assert_snapshot!(print_screen_lines(&doc, 20), @r"
        |line.1              |
        |long.long.line.2    |
        |line.3              |
        ");

        assert_snapshot!(print_screen_lines(&doc, 16), @r"
        |line.1          |
        |long.long.line.2|
        |line.3          |
        ");

        assert_snapshot!(print_screen_lines(&doc, 15), @r"
        |line.1         |
        |long.long.line.|
        |2              |
        |line.3         |
        ");

        assert_snapshot!(print_screen_lines(&doc, 4), @r"
        |line|
        |.1  |
        |long|
        |.lon|
        |g.li|
        |ne.2|
        |line|
        |.3  |
        ");
    }
}
