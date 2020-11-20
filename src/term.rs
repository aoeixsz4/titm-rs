extern crate termion;
extern crate nix;
use std::error;
use std::io::{Read, Error as ioErr, ErrorKind as ioErrKind, Result as ioResult};
use std::os::unix::io::RawFd;
use nix::unistd;
use nix::pty::Winsize;
use terminal_emulator::term::SizeInfo;
use termion::{terminal_size, terminal_size_pixels};

type Result<T> = std::result::Result<T, Box<dyn error::Error>>;

pub struct PtyReader {
    buffer: [u8; 4096],
    bounds: (usize, usize),
    fd: RawFd
}

impl PtyReader {
    pub fn new(fd: RawFd) -> Self {
        PtyReader {
            buffer: [0; 4096],
            bounds: (0, 0),
            fd
        }
    }

    pub fn len(&self) -> usize {
        self.bounds.1 - self.bounds.0
    }

    pub fn raw_read(&mut self) -> ioResult<usize> {
        let n = unistd::read(self.fd, &mut self.buffer[self.bounds.1 ..])
            // the map_err() bit allows us to convert to the correct
            // error type berfore applying ?
            .map_err(|e| ioErr::new(ioErrKind::Other, e))?;
        self.bounds.1 += n;
        Ok(n)
    }
}

impl Read for PtyReader {
    fn read(&mut self, dest_buf: &mut [u8]) -> ioResult<usize> {
        if self.len() == 0 && self.raw_read()? == 0 {
            return Ok(0);
        }

        let len = if dest_buf.len() < self.len() {
            dest_buf.len()
        } else {
            self.len()
        };

        for i in 0 .. len {
            dest_buf[i] = self.buffer[self.bounds.0 + i];
        }
        self.bounds.0 += len;
        Ok(len)
    }
}

impl Iterator for PtyReader {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        // using the iterator form means any I/O error will silently
        // be treated as EOF and just return None
        if self.len() == 0 && self.raw_read().unwrap_or(0) == 0 {
            None
        } else {
            let c = self.buffer[self.bounds.0];
            self.bounds.0 += 1;
            Some(c)
        }
    }
}

pub fn get_winsize() -> Result<Winsize> {
    let (ws_col, ws_row) = terminal_size()?;
    let (ws_xpixel, ws_ypixel) = terminal_size_pixels()?;
    Ok(Winsize {
        ws_row,
        ws_col,
        ws_xpixel,
        ws_ypixel
    })
}

pub fn sizeinfo_from(win_size: Winsize) -> SizeInfo {
    let width = win_size.ws_xpixel as f32;
    let height = win_size.ws_ypixel as f32;
    let cell_width = width / (win_size.ws_col as f32);
    let cell_height = height / (win_size.ws_row as f32);
    SizeInfo {
        width,
        height,
        cell_width,
        cell_height,
        // not sure how to get correct values for padding or DPI
        padding_x: 0.0,
        padding_y: 0.0,
        dpr: 90.0
    }
}