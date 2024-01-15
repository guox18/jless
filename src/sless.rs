use bstr::ByteSlice;
use rustyline::history::MemHistory;
use rustyline::Editor;
use signal_hook::consts::SIGWINCH;
use termion::cursor::HideCursor;
use termion::event::{Event, Key};
use termion::input::{MouseTerminal, TermRead};
use termion::raw::IntoRawMode;
use termion::screen::AlternateScreen;

use std::fs;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::sync::{mpsc, Arc};
use std::thread;

pub fn main() {
    println!("Hello, world!");

    let (events_sender, events_receiver) = mpsc::channel();
    // This should be a oneshot channel.
    let (databuf_sender, databuf_receiver) = mpsc::channel();

    let sigwinch_sender = events_sender.clone();
    let tty_sender = events_sender.clone();
    let text_sender = events_sender.clone();

    let _ = databuf_sender.send(Databuf::new(16));

    let tty_mutex = Arc::new(std::sync::Mutex::new(()));
    let tty_condvar = Arc::new(std::sync::Condvar::new());

    // Introduce scope to ensure stdout gets dropped.
    {
        let stdout = Box::new(MouseTerminal::from(HideCursor::from(
            AlternateScreen::from(std::io::stdout()),
        ))) as Box<dyn std::io::Write>;
        let _raw_stdout = stdout.into_raw_mode().unwrap();

        // Start 3 threads:
        register_sigwinch_handler(sigwinch_sender);
        let _ = get_text_input(text_sender, databuf_receiver);
        get_tty_input(tty_sender, Arc::clone(&tty_mutex), Arc::clone(&tty_condvar));

        let editor_config = rustyline::config::Config::builder()
            .behavior(rustyline::config::Behavior::PreferTerm)
            .keyseq_timeout(0)
            .build();

        let mut editor =
            match Editor::<(), MemHistory>::with_history(editor_config, MemHistory::default()) {
                Ok(editor) => editor,
                Err(err) => {
                    println!("failed to get editor");
                    std::process::exit(1);
                }
            };

        editor.bind_sequence(
            rustyline::KeyEvent::new('\x1B', rustyline::Modifiers::empty()),
            rustyline::Cmd::Interrupt,
        );

        loop {
            match events_receiver.recv() {
                Ok(InputEvent::Sigwinch) => print!("Got SIGWINCH\r\n"),
                Ok(InputEvent::DataInput(databuf)) => {
                    print!("Got data: {:?}\r\n", databuf.data().as_bstr());
                    let _ = databuf_sender.send(databuf);
                }
                Ok(InputEvent::TTYInput(Event::Key(Key::Ctrl('c')))) => break,
                Ok(InputEvent::TTYInput(Event::Key(Key::Char(':')))) => {
                    // Need to tell TTY input to stop reading before calling this.
                    let result = editor.readline("Enter command: ");
                    print!("\rGot command: {:?}\r\n", result);
                }
                Ok(InputEvent::TTYInput(event)) => print!("Got tty input: {:?}\r\n", event),
                Ok(InputEvent::DataEOF) => print!("Got EOF\r\n"),
                Err(_) => break,
            }

            let _lock = tty_mutex.lock();
            tty_condvar.notify_one();
        }
    }

    println!("Done");
    std::process::exit(0);
}

enum InputEvent {
    Sigwinch,
    DataInput(Databuf),
    DataEOF,
    TTYInput(Event),
}

fn register_sigwinch_handler(sender: mpsc::Sender<InputEvent>) {
    let (mut sigwinch_read, sigwinch_write) = UnixStream::pair().unwrap();
    // NOTE: This overrides the SIGWINCH handler registered by rustyline.
    // We should maybe get a reference to the existing signal handler
    // and call it when appropriate, but it seems to only be used to handle
    // line wrapping, and it seems to work fine without it.
    signal_hook::low_level::pipe::register(SIGWINCH, sigwinch_write);
    thread::spawn(move || {
        let mut buf = [0];
        loop {
            // Ignore return error;
            let _ = sigwinch_read.read_exact(&mut buf);
            sender.send(InputEvent::Sigwinch);
        }
    });
}

fn get_text_input(
    sender: mpsc::Sender<InputEvent>,
    databuf_receiver: mpsc::Receiver<Databuf>,
) -> std::io::Result<()> {
    // let mut input = fs::File::open("examples/small.json")?;
    let mut input = std::io::stdin();

    thread::spawn(move || loop {
        let mut databuf = databuf_receiver.recv().unwrap();
        databuf.read(&mut input).unwrap();
        if databuf.data().is_empty() {
            sender.send(InputEvent::DataEOF);
            break;
        } else {
            sender.send(InputEvent::DataInput(databuf));
        }
    });

    Ok(())
}

struct Databuf {
    buf: Vec<u8>,
    content_size: usize,
}

impl Databuf {
    pub fn new(capacity: usize) -> Databuf {
        Databuf {
            buf: vec![0; capacity],
            content_size: 0,
        }
    }

    pub fn read<R>(&mut self, reader: &mut R) -> std::io::Result<()>
    where
        R: Read,
    {
        self.content_size = 0;
        self.content_size = reader.read(&mut self.buf)?;
        Ok(())
    }

    pub fn data(&self) -> &[u8] {
        &self.buf[0..self.content_size]
    }
}

fn get_tty_input(
    sender: mpsc::Sender<InputEvent>,
    mutex: Arc<std::sync::Mutex<()>>,
    condvar: Arc<std::sync::Condvar>,
) {
    let mut tty_events = termion::get_tty().unwrap().events();

    thread::spawn(move || {
        // Due to the implementation of termion's [events] function, which reads
        // a minimum of two bytes so that it can detect solitary ESC presses,
        // if you copy and paste text starting with ':' (or containing a ':' at
        // even index technically...), rustyline won't see the first character
        // after the ':' (but will see everything else), and then once the command
        // is entered, the first character after the ':' will be processed here
        // and sent as a key _after_ the command has been entered, i.e., the input
        // will be received out of order.
        //
        // This is not expected to be a common problem.
        while let Some(Ok(event)) = tty_events.next() {
            sender.send(InputEvent::TTYInput(event));
            let lock = mutex.lock().unwrap();
            condvar.wait(lock);
        }
    });
}
