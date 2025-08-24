use std::ops::Range;

use crate::document::Document;

struct HardLineBreakRef {
    line: usize,
    offset: usize,
}

type Cursor = usize;

struct TextDocument {
    data: Vec<u8>,
    complete_line_ranges: Vec<Range<usize>>,
    next_start: usize,
    trailing_newline: Option<bool>,
}

impl TextDocument {
    fn new() -> Self {
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
}

impl Document for TextDocument {
    type HardLineBreakRef = HardLineBreakRef;
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

    fn has_content_to_display(&self) -> bool {
        self.complete_line_ranges.len() > 0
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
            writeln!(s, "{}: {:?}", n + 1, doc.line_zero_indexed(n).as_bstr());
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
}
