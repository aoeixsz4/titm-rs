// new approach
// we can use script -f in order to duplicate a session
// i.e. say user is rxvt-unicode at /dev/pts/4, and we
// have created a /dev/pts/5 that we own and control
// we run script -f /dev/pts/5 on the shell running on
// /dev/pts/4, and then run script -f /dev/pts/4 on our own
// (or possibly do what script would do, directly - i think
// it is essentially routing everything pts/4 to pts/5 and
// vice-versa)

// actually script is not designed in such a way that is ideal for our purposes
// instead, we should create our own pts in which to run ssh/nethack,
// while relaying I/O streams to the pts/tty within which we were called

// it also isn't possible to have something like cat /dev/pts/n >/dev/pts/m
// running in the background - 

extern crate nix;
extern crate termion;

use std::error;
use std::io::{Write, stdin, stdout, stderr, Stderr};
use std::process::Command;
use std::thread;
use std::time::Duration;
use nix::unistd;
use nix::unistd::read as raw_read;
use nix::unistd::write as raw_write;
use nix::pty::{forkpty, Winsize};
//use nix::pty::termios::Termios;
use termion::raw::IntoRawMode;
use termion::input::TermReadEventsAndRaw;
use termion::{terminal_size, terminal_size_pixels};

type Result<T> = std::result::Result<T, Box<dyn error::Error>>;

#[derive(Clone)]
struct Position {
    x: usize,
    y: usize
}

#[derive(Clone)]
enum CharMap {
    IsoStandard,
    UTF8,
    VTGraphics,
    Null,
    User
}


struct Size {
    width: usize,
    height: usize
}


#[derive(Clone)]
pub enum CSISubType {
    Parameters,
    Intermediates
}

#[derive(Clone)]
pub enum EscType {
    Simple,
    CSI(CSISubType),
    OSC,
    EsqOSC
}

#[derive(Clone)]
pub enum EntryMode {
    Normal,
    UTF8(usize),
    Escape(EscType)
}

struct ScreenState {
    cursor: Position,
    charset: CharMap,
    mapg0: CharMap,
    mapg1: CharMap
}

struct GameScreen {
    cursor: Position,
    size: Size,
    charset: CharMap,
    mapg0: CharMap,
    mapg1: CharMap,
    mode: EntryMode,
    escape: Vec<u8>,
    grid: Vec<Vec<char>>,
    savestate: ScreenState
}



fn convert_vt(text: char) -> char {
    match text {
        'a' => '▒',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'q' => '─',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        '~' => '·',
        _ => text
    }
}

impl GameScreen {
    pub fn new(width: usize, height: usize) -> Self {
        let grid = vec![vec![0 as char; width]; height];
        let cursor = Position {
            x: 0,
            y: 0
        };
        let size = Size {
            width: width,
            height: height
        };

        GameScreen {
            cursor,
            size,
            mode: EntryMode::Normal,
            escape: Vec::new(),
            charset: CharMap::IsoStandard,
            mapg0: CharMap::IsoStandard,
            mapg1: CharMap::IsoStandard,
            grid,
            savestate: ScreenState {
                cursor: Position { x: 0, y: 0 },
                charset: CharMap::IsoStandard,
                mapg0: CharMap::IsoStandard,
                mapg1: CharMap::IsoStandard
            }
        }
    }

    pub fn update(&mut self, raw_bytes: &[u8]) {
        for byte in raw_bytes.iter() {
            match (self.mode.clone(), byte) {
                (EntryMode::Normal, 0x0a) => self.line_feed(),
                (EntryMode::Normal, 0x0d) => self.carriage_return(),
                (EntryMode::Normal, 0x08) => self.backspace(),
                (EntryMode::Normal, 0x1b) => {
                    self.escape.push(*byte);
                    self.mode = EntryMode::Escape(EscType::Simple);
                },
                (EntryMode::Normal, _) => self.put_char(*byte as char),

                (EntryMode::Escape(_), _) => self.put_esc_code(*byte),
                (_, _) => ()
            }
        }
    }

    fn put_esc_code(&mut self, byte: u8) {
        if let EntryMode::Escape(esc_type) = self.mode.clone() {
            self.escape.push(byte);

            match (esc_type, byte) {
                // these codes aborb an escape sequence entirely
                (_, 0x18) => self.abort_escape(),
                (_, 0x1a) => self.abort_escape(),

                (EscType::Simple, b'[')
                    => self.mode = EntryMode::Escape(EscType::CSI(CSISubType::Parameters)),
                (EscType::Simple, b']')
                    => self.mode = EntryMode::Escape(EscType::OSC),
                
                // for simple escapes, 0x20 - 0x2f are intermediate, and 0x30 - 0x7e are final
                (EscType::Simple, 0x30 ..= 0x7e)
                    => self.process_escape(),

                (EscType::CSI(_subtype), 0x30 ..= 0x3f)
                    => self.mode = EntryMode::Escape(EscType::CSI(CSISubType::Parameters)),
                (EscType::CSI(_subtype), 0x20 ..= 0x2f)
                    => self.mode = EntryMode::Escape(EscType::CSI(CSISubType::Intermediates)),
                (EscType::CSI(_subtype), 0x40 ..= 0x7e) =>
                    self.process_escape(),

                (EscType::OSC, 0x1b)
                    => self.mode = EntryMode::Escape(EscType::EsqOSC),
                (EscType::EsqOSC, b'\\')
                    => self.process_escape(),

                (_, _) => ()
            }
        }
    }

    fn abort_escape(&mut self) {
        self.mode = EntryMode::Normal;
        self.escape.clear();
    }

    fn process_escape(&mut self) {
        if let EntryMode::Escape(esc_type) = self.mode.clone() {
            match esc_type {
                EscType::Simple => self.simple_escape(),
                EscType::CSI(_subtype) => self.csi_escape(),
                EscType::OSC => (),
                EscType::EsqOSC => (), //self.osc_escape()
            }
        }
        self.mode = EntryMode::Normal;
        self.escape.clear();
    }

    fn simple_escape(&mut self) {
        assert_eq!(self.escape.remove(0), 0x1b);
        if self.escape.is_empty() {
            return; // should error here probably
        }
        match self.escape.remove(0) {
            b'c' => (), // RIS reset
            b'D' => (), // IND linefeed
            b'E' => (), // NEL newline
            b'H' => (), // HTS set tab stop at current column
            b'M' => (), // RI reverse linefeed
            b'Z' => (), // no idea
            // save state (cursor position, 'attributes', and G0/G1 charsets)
            b'7' => {
                self.savestate.cursor = self.cursor.clone();
                self.savestate.charset = self.charset.clone();
                self.savestate.mapg0 = self.mapg0.clone();
                self.savestate.mapg1 = self.mapg1.clone();
            },
            // restore saved state (cursor position, 'attributes', and G0/G1 charsets)
            b'8' => {
                self.cursor = self.savestate.cursor.clone();
                self.charset = self.savestate.charset.clone();
                self.mapg0 = self.savestate.mapg0.clone();
                self.mapg1 = self.savestate.mapg1.clone();
            },
            //b'[' => (), begin CSI sequence, handled elsewhere
            b'%' if ! self.escape.is_empty() => {
                // choose character set
                match self.escape.remove(0) {
                    b'@' => self.charset = CharMap::IsoStandard,
                    b'G' | b'8' => self.charset = CharMap::UTF8,
                    _ => ()
                }
            },
            b'#' => (), // DECALN screen alignment test
            b'(' if ! self.escape.is_empty() => {
                match self.escape.remove(0) {
                    b'B' => self.mapg0 = CharMap::IsoStandard,
                    b'0' => self.mapg0 = CharMap::VTGraphics,
                    b'U' => self.mapg0 = CharMap::Null,
                    b'K' => self.mapg0 = CharMap::User,
                    _ => ()
                }
            },
            b')' if ! self.escape.is_empty() => {
                match self.escape.remove(0) {
                    b'B' => self.mapg1 = CharMap::IsoStandard,
                    b'0' => self.mapg1 = CharMap::VTGraphics,
                    b'U' => self.mapg1 = CharMap::Null,
                    b'K' => self.mapg1 = CharMap::User,
                    _ => ()
                }
            },
            b'>' => (), // set numeric keypad mode
            b'=' => (), // set application keypad mode
            //b']' => (),
            _ => ()
        }
    }

    fn csi_escape(&mut self) {
        assert_eq!(self.escape.remove(0), 0x1b);
        assert_eq!(self.escape.remove(0), b'[');
        if let Some(final_char) = self.escape.pop() {
            let arg_str = String::from_utf8_lossy(&self.escape);
            let mut args = Vec::new();
            for substring in arg_str.split(';') {
                if let Ok(n_arg) = substring.parse::<usize>() {
                    args.push(n_arg);
                } else if substring.len() == 0 {
                    // this will happen if e.g. \033[;5H is the escape
                    // that would mean ypos 1, xpos 5
                    // or if \033[16;H was given as esc, it means ypos 16, xpos 1
                    // tho our array is 0-indexed and that will be corrected elsewhere
                    args.push(1);
                }
            }
            while args.len() < 2 {
                args.push(1);
            }
            match final_char {
                b'A' => self.cursor_up(args[0]),
                b'B' => self.cursor_down(args[0]),
                b'C' => self.cursor_forward(args[0]),
                b'D' => self.cursor_back(args[0]),
                b'E' => self.cursor_next_line(args[0]),
                b'F' => self.cursor_prev_line(args[0]),
                b'G' => self.cursor_set_column(args[0]),
                b'd' => self.cursor_set_row(args[0]),
                // the esc sequence is \033[<line>;<column>H
                b'H' | b'f' => self.cursor_set_position(args[0], args[1]),
                b'J' => self.erase_display(args[0]),
                b'K' => self.erase_inline(args[0]),
                b'm' => (), // ignore colours
                _ => ()
            }
        }
    }

    pub fn dump(&self, out: &mut Stderr) {
        for j in 0 .. self.size.height {
            for i in 0 .. self.size.width {
                if self.grid[j][i] == '\0' {
                    write!(out, " ");
                } else {
                    write!(out, "{}", self.grid[j][i]);
                }
            }
            write!(out, "\n");
        }
    }

    fn line_feed(&mut self) {
        self.cursor_down(1);
    }

    fn carriage_return(&mut self) {
        self.cursor_set_column(1);
    }

    fn backspace(&mut self) {
        if self.cursor.x != 0 {
            self.cursor_back(1);
            self.put_char(' ');
            self.cursor_back(1);
        }
    }

    fn cursor_up(&mut self, count: usize) {
        self.cursor.y -= count;
    }

    fn cursor_down(&mut self, count: usize) {
        self.cursor.y += count;

        if self.cursor.y >= self.size.height {
            self.cursor.y = self.size.height - 1;
        }
    }

    fn cursor_forward(&mut self, count: usize) {
        self.cursor.x += count;
        if self.cursor.x >= self.size.width {
            self.cursor.x = self.size.width - 1;
        }
    }

    fn cursor_back(&mut self, count: usize) {
        self.cursor.x -= count;
    }

    fn cursor_next_line(&mut self, count: usize) {
        self.cursor.y += count;
        if self.cursor.y >= self.size.height {
            self.cursor.y = self.size.height - 1;
        }
        self.cursor.x = 0;
    }

    fn cursor_prev_line(&mut self, count: usize) {
        self.cursor.y -= count;
        self.cursor.x = 0;
    }

    fn cursor_set_column(&mut self, pos: usize) {
        self.cursor.x = pos - 1;    // terminal indexing starts at 1
    }

    fn cursor_set_row(&mut self, pos: usize) {
        self.cursor.y = pos - 1;    // terminal indexing starts at 1
    }

    fn cursor_set_position(&mut self, ypos: usize, xpos: usize) {
        self.cursor.x = xpos - 1;    // terminal indexing starts at 1
        self.cursor.y = ypos - 1;

        if self.cursor.x >= self.size.width {
            self.cursor.x = self.size.width - 1;
        }
        if self.cursor.y >= self.size.height {
            self.cursor.y = self.size.height - 1;
        }
    }

    fn erase_display(&mut self, mode: usize) {
        match mode {
            1 => {
                // clear from cursor to beginning of screen
                for j in 0 .. self.cursor.y - 1 {
                    self.clear_line(j);
                }
                for i in 0 .. self.cursor.x {
                    self.grid[self.cursor.y][i] = '\0';
                }
            },
            2 | 3 => {
                for j in 0 .. self.size.height {
                    self.clear_line(j);
                }
                self.cursor.x = 0;
                self.cursor.y = 0;
            },
            _ => {
                for i in self.cursor.x .. self.size.width {
                    self.grid[self.cursor.y][i] = '\0';
                }
                for j in self.cursor.y + 1 .. self.size.height {
                    self.clear_line(j);
                }
            }
        }
    }

    fn erase_inline(&mut self, mode: usize) {
        match mode {
            1 => {
                for i in 0 .. self.cursor.x {
                    self.grid[self.cursor.y][i] = '\0';
                }
            },
            2 => {
                self.clear_line(self.cursor.y);
            },
            _ => {
                for i in self.cursor.x .. self.size.width {
                    self.grid[self.cursor.y][i] = '\0';
                }
            }
        }
    }

    fn clear_line(&mut self, line_index: usize) {
        for i in 0 .. self.size.width {
            self.grid[line_index][i] = '\0';
        }
    }

    fn put_char(&mut self, text: char) {
        let post_conversion = match self.mapg0 {
            CharMap::IsoStandard => text,
            CharMap::VTGraphics => convert_vt(text),
            _ => text
        };
        self.grid[self.cursor.y][self.cursor.x] = post_conversion;
        self.cursor.x += 1;
        if self.cursor.x >= self.size.width {
            self.cursor.y += 1;
            self.cursor.x = 0;
        }
        if self.cursor.y >= self.size.height {
            // wrapping instead of scrolling could cause issues...
            self.cursor.y = 0;
        }
    }
}

fn main() -> Result<()> {
    let (ws_col, ws_row) = terminal_size()?;
    let (ws_ypixel, ws_xpixel) = terminal_size_pixels()?;
    let win_size = Winsize {
        ws_row,
        ws_col,
        ws_xpixel,
        ws_ypixel 
    };
    let mut model = GameScreen::new(ws_col as usize, ws_row as usize);
    let fork = forkpty(Some(&win_size), None)?;

    if fork.fork_result.is_parent() {
        let mut stdout = stdout().into_raw_mode().unwrap();
        let stdin = stdin();
        let mut stderr = stderr();
        let raw_fd = fork.master;

        // Read output via PTY master
        //let output = String::new();

        //let our_pty = match terminal.read_to_string(&mut output) {
        //    Ok(_nread) => {
        //        println!("child tty is: {}", output.trim());
        //        Some(output.trim())
        //    },
        //    Err(e)     => {
        //        panic!("read error: {}", e);
        //        // unreachable expression - I don't fool the compiler :D None
        //    }
        //};

        // spawn a background thread to deal with the input
        
        let _input_handler = thread::spawn(move || {
            // loop over events on the term input,(_eventkey, bytevec)
            // forward keys to child process
            for event in stdin.events_and_raw() {
                if let Ok((_event, byte_vector)) = event {
                    raw_write(raw_fd, &byte_vector);
                    unistd::fsync(raw_fd);
                }
                thread::sleep(Duration::from_millis(50));
            }
        });

        // continue reading, and copy raw to our stdout
        //let mut loop_count = 0;
        loop {
            let mut buffer: [u8; 4096] = [0; 4096];
            match raw_read(raw_fd, &mut buffer) {
                Ok(n) => {
                    if n == 0 { break }
                    write!(stdout, "{}", String::from_utf8_lossy(&mut buffer[..n]))?;
                    stdout.flush();
                    //for i in 0 .. n {
                    //    write!(stderr, "{:#x} ", buffer[i]);
                    //}
                    //write!(stderr, "{}", String::from_utf8_lossy(&mut buffer[..n]))?;
                    //stderr.flush();
                    model.update(&buffer[..n]);
                },
                Err(e) => {
                    //println!("error reading output sent to {}: {}", our_pty.unwrap(), e);
                    println!("error reading output sent our tty: {}", e);
                    return Err(Box::new(e));
                }
            }
            thread::sleep(Duration::from_millis(50));
            //write!(stderr, "\nabove was raw, below is a dump of the buffer\n");
            //loop_count += 1;
            //if loop_count >= 5 {
            //    model.dump(&mut stderr);
            //    stderr.flush();
            //    return Ok(());
            //}
        }
    } else {
        // Child process just exec `tty`
        //Command::new("tty").status().expect("could not execute tty");
        //Command::new("stty").arg("-a").status().expect("could not execute stty -a");
        Command::new("nethack").status().expect("could not execute local nethack");
        //Command::new("ssh").arg("hdf").status().expect("could not execute local nethack");
        //Command::new("sh").status().expect("could not execute shell");

    }
    Ok(())
}
