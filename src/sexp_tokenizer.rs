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

#[derive(Copy, Clone, Debug)]
pub enum Token<'a> {
    OpenParen,
    CloseParen,
    Atom(&'a [u8]),
    LineComment(&'a [u8]),
    SexpComment,
    BlockComment(&'a [u8]),
}

impl<'a> Token<'a> {
    fn requires_finishing(self) -> bool {
        matches!(
            self,
            Token::Atom(_) | Token::LineComment(_) | Token::BlockComment(_)
        )
    }
}

pub trait Parser<'a> {
    fn process_token(&mut self, token: Token<'a>) -> ();
    fn continue_token(&mut self, bytes: &'a [u8]) -> ();
    fn finish_token(&mut self) -> ();
}

pub struct Tokenizer {
    state: State,
    block_comment_depth: usize,
    line_num: usize,
    column_byte_num: usize,
}

#[derive(PartialEq, Eq, Debug)]
pub enum TokenizationError {
    UnterminatedQuotedAtom,
    UnterminatedBlockQuote,
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
                    b'(' => parser.process_token(Token::OpenParen),
                    b')' => parser.process_token(Token::CloseParen),
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
                        let range = &bytes[curr_token_start..i];
                        if reported_start_of_current_token {
                            parser.continue_token(range);
                        } else {
                            parser.process_token(Token::Atom(range));
                        }
                        parser.finish_token();

                        self.state = State::Start;
                        // [curr_token_start] is now invalid, but will be set
                        // on a transition out of [Start].
                    }
                    b'(' => {
                        // End current atom, and emit open paren
                        let range = &bytes[curr_token_start..i];
                        if reported_start_of_current_token {
                            parser.continue_token(range);
                        } else {
                            parser.process_token(Token::Atom(range));
                        }
                        parser.finish_token();
                        parser.process_token(Token::OpenParen);

                        self.state = State::Start;
                    }
                    b')' => {
                        let range = &bytes[curr_token_start..i];
                        if reported_start_of_current_token {
                            parser.continue_token(range);
                        } else {
                            parser.process_token(Token::Atom(range));
                        }
                        parser.finish_token();
                        parser.process_token(Token::CloseParen);

                        self.state = State::Start;
                    }
                    b'"' => {
                        // End current atom, and start a new quoted one
                        let range = &bytes[curr_token_start..i];
                        if reported_start_of_current_token {
                            parser.continue_token(range);
                        } else {
                            parser.process_token(Token::Atom(range));
                        }
                        parser.finish_token();

                        curr_token_start = i;
                        self.state = State::InQuotedAtom;
                        reported_start_of_current_token = false;
                    }
                    _ => { /* Do nothing, continue processing atom. */ }
                },
                State::InQuotedAtom => match byte {
                    b'"' => {
                        // End current atom

                        // Include trailing '"'
                        let range = &bytes[curr_token_start..=i];
                        if reported_start_of_current_token {
                            parser.continue_token(range);
                        } else {
                            parser.process_token(Token::Atom(range));
                        }
                        parser.finish_token();

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
                        let range = &bytes[curr_token_start..i];
                        if reported_start_of_current_token {
                            parser.continue_token(range);
                        } else {
                            parser.process_token(Token::LineComment(range));
                        }
                        parser.finish_token();

                        curr_token_start = i;
                        self.state = State::Start;
                        reported_start_of_current_token = false;
                    }
                    _ => { /* Do nothing, continue processing comment. */ }
                },
                State::Pound => match byte {
                    b';' => {
                        // Emit sexp comment ("#;"), then go back to start state.
                        parser.process_token(Token::SexpComment);
                        self.state = State::Start;
                    }
                    b'|' => {
                        // Report the start '#' (which may have been in a previous batch).
                        if curr_token_start == 0 {
                            parser.process_token(Token::BlockComment(b"#"));

                            curr_token_start = i;
                            reported_start_of_current_token = true;
                        } else {
                            curr_token_start = i - 1;
                            reported_start_of_current_token = false;
                        }

                        self.block_comment_depth = 1;
                        self.state = State::BlockComment;
                    }
                    b' ' | b'\n' | b'\t' | b'\r' | b'\x0C' => {
                        // Emit "#" atom, then go back to start state.
                        parser.process_token(Token::Atom(b"#"));
                        parser.finish_token();
                        self.state = State::Start;
                    }
                    b'(' => {
                        // End "#" atom, and emit open paren
                        parser.process_token(Token::Atom(b"#"));
                        parser.finish_token();
                        parser.process_token(Token::OpenParen);
                        self.state = State::Start;
                    }
                    b')' => {
                        // End "#" atom, and emit open paren
                        parser.process_token(Token::Atom(b"#"));
                        parser.finish_token();
                        parser.process_token(Token::CloseParen);
                        self.state = State::Start;
                    }
                    b'"' => {
                        // End "#" atom, move into quoted atom.
                        parser.process_token(Token::Atom(b"#"));
                        parser.finish_token();

                        curr_token_start = i;
                        self.state = State::InQuotedAtom;
                        reported_start_of_current_token = false;
                    }
                    _ => {
                        // Emit the start of an atom that starts with '#', then start
                        // processing that atom.
                        parser.process_token(Token::Atom(b"#"));
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
                            // Include trailing '#' char
                            let range = &bytes[curr_token_start..=i];

                            if reported_start_of_current_token {
                                parser.continue_token(range);
                            } else {
                                parser.process_token(Token::BlockComment(range));
                            }
                            parser.finish_token();

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
        let range = &bytes[curr_token_start..];

        match self.state {
            State::Start | State::Pound => {}
            State::InUnquotedAtom | State::InQuotedAtom | State::InQuotedAtomEscape => {
                if reported_start_of_current_token {
                    parser.continue_token(range);
                } else {
                    parser.process_token(Token::Atom(range));
                }
            }
            State::LineComment => {
                if reported_start_of_current_token {
                    parser.continue_token(range);
                } else {
                    parser.process_token(Token::LineComment(range));
                }
            }
            State::BlockComment
            | State::BlockCommentQuotedString
            | State::BlockCommentQuotedStringEscape
            | State::BlockCommentBar
            | State::BlockCommentPound => {
                if reported_start_of_current_token {
                    parser.continue_token(range);
                } else {
                    parser.process_token(Token::BlockComment(range));
                }
            }
        }
    }

    pub fn eof<'p, 't, P: Parser<'t>>(
        &mut self,
        parser: &'p mut P,
    ) -> Result<(), TokenizationError> {
        match self.state {
            State::Start => Ok(()),
            State::Pound => {
                parser.process_token(Token::BlockComment(b"#"));
                parser.finish_token();
                Ok(())
            }
            State::InUnquotedAtom | State::LineComment => {
                parser.finish_token();
                Ok(())
            }
            State::InQuotedAtom | State::InQuotedAtomEscape => {
                Err(TokenizationError::UnterminatedQuotedAtom)
            }
            State::BlockComment
            | State::BlockCommentQuotedString
            | State::BlockCommentQuotedStringEscape
            | State::BlockCommentBar
            | State::BlockCommentPound => Err(TokenizationError::UnterminatedBlockQuote),
        }
    }
}

#[cfg(test)]
mod tests {
    use bstr::BString;

    use super::*;

    #[derive(PartialEq, Eq, Debug)]
    enum OwnedToken {
        OpenParen,
        CloseParen,
        Atom(BString),
        LineComment(BString),
        SexpComment,
        BlockComment(BString),
    }

    fn op() -> OwnedToken {
        OwnedToken::OpenParen
    }

    fn cp() -> OwnedToken {
        OwnedToken::CloseParen
    }

    fn sc() -> OwnedToken {
        OwnedToken::SexpComment
    }

    fn atom(bytes: &[u8]) -> OwnedToken {
        OwnedToken::Atom(bytes.into())
    }

    fn line_comment(bytes: &[u8]) -> OwnedToken {
        OwnedToken::LineComment(bytes.into())
    }

    fn block_comment(bytes: &[u8]) -> OwnedToken {
        OwnedToken::BlockComment(bytes.into())
    }

    impl<'a> From<Token<'a>> for OwnedToken {
        fn from(token: Token<'a>) -> Self {
            match token {
                Token::OpenParen => OwnedToken::OpenParen,
                Token::CloseParen => OwnedToken::CloseParen,
                Token::Atom(s) => OwnedToken::Atom(s.into()),
                Token::LineComment(s) => OwnedToken::LineComment(s.into()),
                Token::SexpComment => OwnedToken::SexpComment,
                Token::BlockComment(s) => OwnedToken::BlockComment(s.into()),
            }
        }
    }

    #[derive(Debug)]
    struct TokenCollector {
        tokens: Vec<OwnedToken>,
        waiting_to_finish_token: bool,
    }

    impl TokenCollector {
        fn new() -> TokenCollector {
            TokenCollector {
                tokens: vec![],
                waiting_to_finish_token: false,
            }
        }
    }

    impl<'a> Parser<'a> for TokenCollector {
        fn process_token(&mut self, token: Token<'a>) {
            assert!(
                !self.waiting_to_finish_token,
                "process_token called, but expected token to be finished, {:?}, {:?}",
                token, &self
            );
            self.tokens.push(token.into());
            self.waiting_to_finish_token = token.requires_finishing();
        }

        fn continue_token(&mut self, bytes: &'a [u8]) {
            assert!(
                self.waiting_to_finish_token,
                "continue_token, but not waiting for token to be finished, {self:?}"
            );
            match self.tokens.last_mut() {
                None => panic!("continue_token called before receiving any tokens"),
                Some(OwnedToken::OpenParen | OwnedToken::CloseParen | OwnedToken::SexpComment) => {
                    panic!("most recent token does not require finishing")
                }
                Some(
                    OwnedToken::Atom(s) | OwnedToken::LineComment(s) | OwnedToken::BlockComment(s),
                ) => {
                    s.extend_from_slice(bytes);
                }
            }
        }

        fn finish_token(&mut self) {
            assert!(
                self.waiting_to_finish_token,
                "finish_token, but not waiting for token to be finished, {self:?}"
            );
            self.waiting_to_finish_token = false;
        }
    }

    #[track_caller]
    fn assert_tokenizes(
        byte_slices: Vec<&[u8]>,
        expected: &[OwnedToken],
    ) -> Result<(), TokenizationError> {
        let mut tokenizer = Tokenizer::new();
        let mut token_collector = TokenCollector::new();

        for bytes in byte_slices.iter() {
            tokenizer.tokenize(bytes, &mut token_collector);
        }

        let result = tokenizer.eof(&mut token_collector);

        assert_eq!(result.is_ok(), !token_collector.waiting_to_finish_token);
        assert_eq!(token_collector.tokens, expected);

        result
    }

    #[track_caller]
    fn assert_valid_tokens(bytes: &[u8], expected: Vec<OwnedToken>) {
        let result = assert_tokenizes(vec![bytes], &expected);
        assert_eq!(Ok(()), result);

        let mut one_byte_at_a_time = vec![];
        for i in 0..(bytes.len()) {
            one_byte_at_a_time.push(&bytes[i..(i + 1)]);
        }

        let result = assert_tokenizes(one_byte_at_a_time, &expected);
        assert_eq!(Ok(()), result);

        let mut byte_pairs = vec![];
        let mut offset_byte_pairs = vec![&bytes[0..1]];
        let mut i = 0;
        let len = bytes.len();
        while i < len {
            byte_pairs.push(&bytes[i..(len.min(i + 2))]);
            if i + 1 < bytes.len() {
                offset_byte_pairs.push(&bytes[(i + 1)..(len.min(i + 3))]);
            }
            i += 2;
        }

        let result = assert_tokenizes(byte_pairs, &expected);
        assert_eq!(Ok(()), result);

        let result = assert_tokenizes(offset_byte_pairs, &expected);
        assert_eq!(Ok(()), result);
    }

    #[track_caller]
    fn assert_unterminated_quoted_atom(bytes: &[u8], expected: Vec<OwnedToken>) {
        let result = assert_tokenizes(vec![bytes], &expected);
        assert_eq!(Err(TokenizationError::UnterminatedQuotedAtom), result);
    }

    #[track_caller]
    fn assert_unterminated_block_quote(bytes: &[u8], expected: Vec<OwnedToken>) {
        let result = assert_tokenizes(vec![bytes], &expected);
        assert_eq!(Err(TokenizationError::UnterminatedBlockQuote), result);
    }

    #[test]
    fn test_basic() {
        assert_valid_tokens(
            br#"(abc "def ghi" jkl)"#,
            vec![op(), atom(b"abc"), atom(b"\"def ghi\""), atom(b"jkl"), cp()],
        );

        assert_valid_tokens(
            b"abc ; comment\n def",
            vec![atom(b"abc"), line_comment(b"; comment"), atom(b"def")],
        );

        assert_valid_tokens(
            b"#; x #| abc \n def |#",
            vec![sc(), atom(b"x"), block_comment(b"#| abc \n def |#")],
        );
    }

    #[test]
    fn test_adjacent_atoms() {
        assert_valid_tokens(
            br#"abc"def""ghi"jkl"#,
            vec![
                atom(b"abc"),
                atom(b"\"def\""),
                atom(b"\"ghi\""),
                atom(b"jkl"),
            ],
        );
    }

    #[test]
    fn test_pound_transitions() {
        assert_valid_tokens(
            b"#(#)# #\t#\r#\x0C#\n#\"a\"",
            vec![
                atom(b"#"),
                op(),
                atom(b"#"),
                cp(),
                atom(b"#"),
                atom(b"#"),
                atom(b"#"),
                atom(b"#"),
                atom(b"#"),
                atom(b"#"),
                atom(b"\"a\""),
            ],
        );
    }

    #[test]
    fn test_quoted_atoms() {
        assert_valid_tokens(
            br#""""a" "\"a""#,
            vec![atom(br#""""#), atom(br#""a""#), atom(br#""\"a""#)],
        );

        assert_valid_tokens(b"\"a\nb#|\"", vec![atom(b"\"a\nb#|\"")]);

        assert_unterminated_quoted_atom(b"\"abc", vec![atom(b"\"abc")]);
        assert_unterminated_quoted_atom(b"\"abc\\\"", vec![atom(b"\"abc\\\"")]);
        assert_unterminated_quoted_atom(b"\"abc\n", vec![atom(b"\"abc\n")]);
    }

    #[test]
    fn test_block_comments() {
        assert_valid_tokens(
            b"#| |# a #| #|c|# |# #|# |#",
            vec![
                block_comment(b"#| |#"),
                atom(b"a"),
                block_comment(b"#| #|c|# |#"),
                block_comment(b"#|# |#"),
            ],
        );

        assert_valid_tokens(
            br#"#| "|#" |# a"#,
            vec![block_comment(b"#| \"|#\" |#"), atom(b"a")],
        );

        assert_unterminated_block_quote(b"#| #| |#|", vec![block_comment(b"#| #| |#|")]);

        assert_unterminated_block_quote(br#"#|"a\"|#"#, vec![block_comment(b"#|\"a\\\"|#")]);
    }
}
