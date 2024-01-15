// The lexical format of s-expressions is explained here in brief:
// https://github.com/janestreet/sexplib#lexical-conventions-of-s-expression
//
// > Whitespace, which consists of the space, newline, horizontal tab, and form
// > feed characters, > is ignored unless within an OCaml-string, where it is
// > treated according to OCaml-conventions. The left parenthesis opens a new
// > list, the right one closes it again. Lists can be empty.
//
// > The double quote denotes the beginning and end of a string following the
// > lexical conventions of OCaml (see the OCaml-manual for details). All
// > characters other than double quotes, left and right parentheses,
// > whitespace, carriage return, and comment-introducing characters or
// > sequences (see next paragraph) are considered part of a contiguous string.
//
// > Comments
//
// > There are three kinds of comments:
// > - line comments are introduced with ;, and end at the newline.
// > - sexp comments are introduced with #;, and end at the end of the
//     following s-expression.
// > - block comments are introduced with #| and end with |#. These can be
//     nested, and double-quotes within them must be balanced and be lexically
//     correct OCaml strings.
//
// We note empirically that the [sexp] parser not require valid escape sequences
// in quoted string literals:
// $ printf '"\123\n" | sexp print -m
// > {
// $ printf '"\12\n" | sexp print -m
// > "\\12"
//
// From the above we infer the following states:
//
// START: No processed input
// UNQUOTED_STRING: Reading an unquoted atom, will end at (), whitespace, '"', or ';'
// QUOTED_STRING: Reading a quoted string
// QUOTED_STRING_ESCAPE: Seen a '\' in a quoted string
// LINE_COMMENT: Line comment until '\n'
// POUND: We've seen a '#' and are waiting for either a ';', '|', or some other character
// BLOCK_COMMENT(n): We are in a block comment at depth n (i.e., requiring n instances
// of "|#" to escape.
// BLOCK_COMMENT_QUOTED_STRING: We are in a quoted string in a block comment
// BLOCK_COMMENT_QUOTED_STRING_ESCAPE: Seen a '\' in a quoted string in a block comment
// BLOCK_COMMENT_BAR: We have seen a '|' in a block comment
// BLOCK_COMMENT_POUND: We have seen a '#' in a block comment
//
// (Technically all of the "BLOCK_COMMENT_*" states have an associated n, but we'll just
// store it out of band.)

#[derive(Copy, Clone, Debug)]
enum State {
    Start,
    InUnquotedAtom,
    InQuotedAtom,
    InQuotedAtomEscape,
    LineComment,
    Pound,
    BlockComment,
    BlockCommentQuotedString,
    BlockCommentQuotedStringEscape,
    BlockCommentBar,
    BlockCommentPound,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Part {
    All,
    Start,
    Middle,
    End,
}

impl Part {
    fn create_at_end(reported_start: bool) -> Part {
        match reported_start {
            true => Part::End,
            false => Part::All,
        }
    }

    fn create_after_middle(reported_start: bool) -> Part {
        match reported_start {
            true => Part::Middle,
            false => Part::Start,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum PartialToken<'a> {
    OpenParen,
    CloseParen,
    Atom(Part, &'a [u8]),
    LineComment(Part, &'a [u8]),
    SexpComment,
    BlockComment(Part, &'a [u8]),
}

pub trait Parser<'a> {
    fn process_token(&mut self, partial_token: PartialToken<'a>) -> ();
}

pub struct Tokenizer {
    state: State,
    block_comment_depth: usize,
    line_num: usize,
    column_byte_num: usize,
}

impl Tokenizer {
    pub fn new() -> Tokenizer {
        Tokenizer {
            state: State::Start,
            block_comment_depth: 0,
            line_num: 1,
            column_byte_num: 1,
        }
    }

    pub fn tokenize<'p, 't, P: Parser<'t>>(&mut self, bytes: &'p [u8], parser: &'p mut P)
    where
        'p: 't,
    {
        let mut reported_start_of_current_token = match self.state {
            State::Start | State::Pound => false,
            State::InUnquotedAtom
            | State::InQuotedAtom
            | State::InQuotedAtomEscape
            | State::LineComment
            | State::BlockComment
            | State::BlockCommentQuotedString
            | State::BlockCommentQuotedStringEscape
            | State::BlockCommentBar
            | State::BlockCommentPound => true,
        };

        let mut curr_token_start = 0;

        for (i, &byte) in bytes.iter().enumerate() {
            if byte == b'\n' {
                self.line_num += 1;
                self.column_byte_num = 1;
            } else {
                self.column_byte_num += 1;
            }

            match self.state {
                State::Start => match byte {
                    b' ' | b'\n' | b'\t' | b'\r' | b'\x0C' => { /* Do nothing */ }
                    b'(' => parser.process_token(PartialToken::OpenParen),
                    b')' => parser.process_token(PartialToken::CloseParen),
                    b'#' => {
                        curr_token_start = i;
                        self.state = State::Pound;
                        reported_start_of_current_token = false;
                    }
                    b';' => {
                        curr_token_start = i;
                        self.state = State::LineComment;
                        reported_start_of_current_token = false;
                    }
                    b'"' => {
                        curr_token_start = i;
                        self.state = State::InQuotedAtom;
                        reported_start_of_current_token = false;
                    }
                    _ => {
                        curr_token_start = i;
                        self.state = State::InUnquotedAtom;
                        reported_start_of_current_token = false;
                    }
                },
                State::InUnquotedAtom => match byte {
                    b' ' | b'\n' | b'\t' | b'\r' | b'\x0C' => {
                        let part = Part::create_at_end(reported_start_of_current_token);
                        let range = &bytes[curr_token_start..i];
                        parser.process_token(PartialToken::Atom(part, range));

                        self.state = State::Start;
                        // [curr_token_start] is now invalid, but will be set
                        // on a transition out of [Start].
                    }
                    b'(' => {
                        // End current atom, and emit open paren
                        let part = Part::create_at_end(reported_start_of_current_token);
                        let range = &bytes[curr_token_start..i];
                        parser.process_token(PartialToken::Atom(part, range));
                        parser.process_token(PartialToken::OpenParen);

                        self.state = State::Start;
                    }
                    b')' => {
                        let part = Part::create_at_end(reported_start_of_current_token);
                        let range = &bytes[curr_token_start..i];
                        parser.process_token(PartialToken::Atom(part, range));
                        parser.process_token(PartialToken::CloseParen);

                        self.state = State::Start;
                    }
                    b'"' => {
                        // End current atom, and start a new quoted one
                        let part = Part::create_at_end(reported_start_of_current_token);
                        let range = &bytes[curr_token_start..i];
                        parser.process_token(PartialToken::Atom(part, range));

                        curr_token_start = i;
                        self.state = State::InQuotedAtom;
                        reported_start_of_current_token = false;
                    }
                    _ => { /* Do nothing, continue processing atom. */ }
                },
                State::InQuotedAtom => match byte {
                    b'"' => {
                        // End current atom
                        let part = Part::create_at_end(reported_start_of_current_token);
                        // Include trailing '"'
                        let range = &bytes[curr_token_start..=i];
                        parser.process_token(PartialToken::Atom(part, range));

                        curr_token_start = i;
                        self.state = State::Start;
                        reported_start_of_current_token = false;
                    }
                    b'\\' => {
                        // Enter escape state
                        self.state = State::InQuotedAtomEscape;
                    }
                    _ => { /* Do nothing, continue processing atom. */ }
                },
                State::InQuotedAtomEscape => {
                    /* Don't even need to check the byte. */
                    self.state = State::InQuotedAtom;
                }
                State::LineComment => match byte {
                    b'\n' => {
                        // End line comment
                        let part = Part::create_at_end(reported_start_of_current_token);
                        let range = &bytes[curr_token_start..i];
                        parser.process_token(PartialToken::LineComment(part, range));
                    }
                    _ => { /* Do nothing, continue processing comment. */ }
                },
                State::Pound => match byte {
                    b';' => {
                        // Emit sexp comment ("#;"), then go back to start state.
                        parser.process_token(PartialToken::SexpComment);
                        self.state = State::Start;
                    }
                    b'|' => {
                        // Report the start '#' (which may have been in a previous batch).
                        parser.process_token(PartialToken::BlockComment(Part::Start, b"#"));
                        self.block_comment_depth = 1;

                        curr_token_start = i;
                        self.state = State::BlockComment;
                        reported_start_of_current_token = true;
                    }
                    b' ' | b'\n' | b'\t' | b'\r' | b'\x0C' => {
                        // Emit "#" atom, then go back to start state.
                        parser.process_token(PartialToken::Atom(Part::All, b"#"));
                        self.state = State::Start;
                    }
                    b'(' => {
                        // End "#" atom, and emit open paren
                        parser.process_token(PartialToken::Atom(Part::All, b"#"));
                        parser.process_token(PartialToken::OpenParen);
                        self.state = State::Start;
                    }
                    b')' => {
                        // End "#" atom, and emit open paren
                        parser.process_token(PartialToken::Atom(Part::All, b"#"));
                        parser.process_token(PartialToken::CloseParen);
                        self.state = State::Start;
                    }
                    _ => {
                        // Emit the start of an atom that starts with '#', then start
                        // processing that atom.
                        parser.process_token(PartialToken::Atom(Part::Start, b"#"));
                        curr_token_start = i;
                        self.state = State::InUnquotedAtom;
                        reported_start_of_current_token = true;
                    }
                },
                State::BlockComment => match byte {
                    // Maybe update state, but we won't ever emit a token.
                    b'"' => {
                        self.state = State::BlockCommentQuotedString;
                    }
                    b'|' => {
                        self.state = State::BlockCommentBar;
                    }
                    b'#' => {
                        self.state = State::BlockCommentPound;
                    }
                    _ => { /* Otherwise do nothing. */ }
                },
                State::BlockCommentQuotedString => match byte {
                    // Maybe end the quote, or ignore the next '"', but no
                    // bookkeeping to be done.
                    b'"' => {
                        self.state = State::BlockComment;
                    }
                    b'\\' => {
                        self.state = State::BlockCommentQuotedStringEscape;
                    }
                    _ => { /* Otherwise do nothing. */ }
                },
                State::BlockCommentQuotedStringEscape => {
                    /* Don't even need to check the byte. */
                    self.state = State::BlockCommentQuotedString;
                }
                State::BlockCommentBar => match byte {
                    b'#' => {
                        self.block_comment_depth -= 1;
                        if self.block_comment_depth == 0 {
                            let part = Part::create_at_end(reported_start_of_current_token);
                            // Include trailing '#' char
                            let range = &bytes[curr_token_start..=i];
                            parser.process_token(PartialToken::BlockComment(part, range));

                            self.state = State::Start;
                        } else {
                            // Still processing a block comment
                            self.state = State::BlockComment;
                        }
                    }
                    b'"' => {
                        self.state = State::BlockCommentQuotedString;
                    }
                    b'|' => { /* Stay in this state. */ }
                    _ => self.state = State::BlockComment,
                },
                State::BlockCommentPound => match byte {
                    b'|' => {
                        self.block_comment_depth += 1;
                        self.state = State::BlockComment;
                    }
                    b'"' => {
                        self.state = State::BlockCommentQuotedString;
                    }
                    _ => {
                        self.state = State::BlockComment;
                    }
                },
            }
        }

        // Emit the start/middle of whatever token we're currently processing.
        let part = Part::create_after_middle(reported_start_of_current_token);
        let range = &bytes[curr_token_start..];

        match self.state {
            Start | Pound => {}
            InUnquotedAtom | InQuotedAtom | InQuotedAtomEscape => {
                parser.process_token(PartialToken::Atom(part, range));
            }
            LineComment => {
                parser.process_token(PartialToken::LineComment(part, range));
            }
            BlockComment
            | BlockCommentQuotedString
            | BlockCommentQuotedStringEscape
            | BlockCommentBar
            | BlockCommentPound => {
                parser.process_token(PartialToken::BlockComment(part, range));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bstr::BString;

    use super::*;

    #[derive(PartialEq, Eq, Debug)]
    enum OwnedPartialToken {
        OpenParen,
        CloseParen,
        Atom(Part, BString),
        LineComment(Part, BString),
        SexpComment,
        BlockComment(Part, BString),
    }

    impl<'a> From<PartialToken<'a>> for OwnedPartialToken {
        fn from(partial_token: PartialToken<'a>) -> Self {
            match partial_token {
                PartialToken::OpenParen => OwnedPartialToken::OpenParen,
                PartialToken::CloseParen => OwnedPartialToken::CloseParen,
                PartialToken::Atom(part, s) => OwnedPartialToken::Atom(part, s.into()),
                PartialToken::LineComment(part, s) => {
                    OwnedPartialToken::LineComment(part, s.into())
                }
                PartialToken::SexpComment => OwnedPartialToken::SexpComment,
                PartialToken::BlockComment(part, s) => {
                    OwnedPartialToken::BlockComment(part, s.into())
                }
            }
        }
    }

    #[derive(Debug)]
    struct TokenCollector {
        partial_tokens: Vec<OwnedPartialToken>,
    }

    impl TokenCollector {
        fn new() -> TokenCollector {
            TokenCollector {
                partial_tokens: vec![],
            }
        }
    }

    impl<'a> Parser<'a> for TokenCollector {
        fn process_token(&mut self, partial_token: PartialToken<'a>) {
            self.partial_tokens.push(partial_token.into())
        }
    }

    #[test]
    fn test_basic() {
        let mut tokenizer = Tokenizer::new();
        let mut token_collector = TokenCollector::new();
        tokenizer.tokenize(br#"(abc "def ghi" jkl)"#, &mut token_collector);
        dbg!(token_collector);
        assert_eq!(1, 2);
    }
}
