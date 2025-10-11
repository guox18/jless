#[derive(Debug, Copy, Clone)]
pub struct Dimensions {
    pub width: usize,
    pub height: usize,
}

pub fn current() -> Dimensions {
    let Ok((columns, rows)) = termion::terminal_size() else {
        panic!("Unable to get terminal size")
    };

    Dimensions {
        width: columns as usize,
        height: rows as usize,
    }
}
