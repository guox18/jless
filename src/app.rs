use std::fmt::Write;
use std::io;

use rustyline::history::MemHistory;
use rustyline::Editor;
use termion::event::{Event as TermionEvent, Key};

use crate::dimensions::Dimensions;
use crate::document::Document;
use crate::document_viewport::DocumentViewport;
use crate::terminal::{AnsiTerminal, Terminal};

pub struct App<D: Document> {
    doc: D,
    viewport: Option<DocumentViewport<D>>,
    readline_editor: Editor<(), MemHistory>,
    dimensions: Dimensions,
    stdout: Box<dyn std::io::Write>,
}

pub struct Break;

impl<D: Document> App<D> {
    pub fn new(
        doc: D,
        readline_editor: Editor<(), MemHistory>,
        dimensions: Dimensions,
        stdout: Box<dyn std::io::Write>,
    ) -> Self {
        App {
            doc,
            viewport: None,
            dimensions,
            readline_editor,
            stdout,
        }
    }

    pub fn handle_document_data(&mut self, data: Option<&[u8]>) {
        match data {
            None => self.doc.eof(),
            Some(data) => {
                self.doc.append(data);
                if self.viewport.is_none() {
                    if let Some((top_screen_line, cursor)) = self.doc.top_screen_line_and_cursor() {
                        self.viewport = Some(DocumentViewport::new(
                            top_screen_line,
                            cursor,
                            self.dimensions,
                            0,
                        ));
                    }
                }
            }
        }
        self.draw_screen();
    }

    pub fn handle_window_resize(&mut self, new_dimensions: Dimensions) {
        self.dimensions = new_dimensions;
        if let Some(viewport) = &mut self.viewport {
            viewport.resize(&mut self.doc, new_dimensions);
        };
        self.draw_screen();
    }

    pub fn handle_tty_event(&mut self, tty_event: TermionEvent) -> Option<Break> {
        if let Some(viewport) = &mut self.viewport {
            let doc = &self.doc;
            match tty_event {
                TermionEvent::Key(Key::Char('j')) => viewport.move_cursor_down(doc, 1),
                TermionEvent::Key(Key::Char('k')) => viewport.move_cursor_up(doc, 1),
                TermionEvent::Key(Key::Ctrl('e')) => viewport.scroll_viewport_down(doc, 1),
                TermionEvent::Key(Key::Ctrl('y')) => viewport.scroll_viewport_up(doc, 1),
                _ => (),
            }
        }

        self.draw_screen();

        match tty_event {
            TermionEvent::Key(Key::Ctrl('c')) => Some(Break),
            TermionEvent::Key(Key::Char(':')) => {
                // These [unwrap]s should be handled once this is moved out of
                // a proof-of-concept phase.
                write!(self.stdout, "{}", termion::cursor::Show).unwrap();
                let result = self.readline_editor.readline("Enter command: ");
                write!(self.stdout, "{}", termion::cursor::Hide).unwrap();
                print!("\rGot command: {result:?}\r\n");
                None
            }
            _ => None,
        }
    }

    // Someday: Do something here.
    pub fn handle_tty_input_error(&mut self, io_error: io::Error) {
        eprintln!("TTY Input Error: {io_error:?}");
    }

    pub fn handle_data_input_error(&mut self, io_error: io::Error) {
        eprintln!("Data Input Error: {io_error:?}");
    }

    fn draw_screen(&mut self) {
        let mut terminal = AnsiTerminal::new(String::new());

        terminal.clear_screen();

        match &self.viewport {
            None => {
                let _ = write!(terminal, "Waiting for input...");
            }
            Some(viewport) => {
                let mut row = 1;
                for screen_line in viewport.viewport_lines(&self.doc) {
                    terminal.position_cursor(1, row);
                    terminal.reset_style();
                    match screen_line {
                        None => {
                            let _ = write!(terminal, "~");
                        }
                        Some(screen_line) => {
                            if self.doc.does_screen_line_intersect_cursor(
                                &screen_line,
                                &viewport.current_focus,
                            ) {
                                terminal.set_inverted(true);
                            };

                            let line = self.doc.debug_text_content(&screen_line);
                            let _ = match std::str::from_utf8(line) {
                                Ok(s) => write!(terminal, "{s}"),
                                Err(_) => write!(terminal, "line is not valid UTF-8"),
                            };
                        }
                    }
                    row += 1;
                }
            }
        }

        terminal.position_cursor(1, 1);
        terminal.flush_contents(&mut self.stdout);
    }
}
