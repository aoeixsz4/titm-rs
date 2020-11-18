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
use std::fmt;
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
pub struct Position {
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

#[derive(Clone)]
pub struct ScrSize {
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
    size: ScrSize,
    charset: CharMap,
    mapg0: CharMap,
    mapg1: CharMap,
    mode: EntryMode,
    escape: Vec<u8>,
    grid: Vec<Vec<char>>,
    savestate: ScreenState
}
#[derive(Debug)]
pub enum ScrErr {
    BoxInvalid(usize, usize),
    BrokenBorder,
    OutofBounds(usize, usize)
}

impl fmt::Display for ScrErr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ScrErr::BoxInvalid(row, col) => write!(f, "row {}, col {} is not a valid box start position", row, col),
            ScrErr::BrokenBorder => write!(f, "box borders broken/incomplete"),
            ScrErr::OutofBounds(row, col) => write!(f, "row {}, col {} is out of bounds", row, col)
        }
    }
}

impl error::Error for ScrErr {}

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

pub trait ExtractLines {
    fn get_size(&self) -> (usize, usize);
    fn get_char_at_pos(&self, row: usize, col: usize) -> Result<char>;

    fn rest_null_ln(&self, row: usize, col: usize) -> bool {
        let (height, width) = self.get_size();

        assert!(row <= height);
        let mut index = col;
        while index <= width {
            if self.get_char_at_pos(row, index).unwrap() != '\0'
                && self.get_char_at_pos(row, index).unwrap() != ' ' {
                return false;
            }
            index += 1;
        }
        true
    }

    fn get_lines(&self) -> Result<Vec<String>> {
        let (height, width) = self.get_size();
        let mut row= 1;
        let mut out = Vec::new();

        while row <= height {
            let mut col = 1;
            let mut line = String::new();
            while col <= width {
                let c = self.get_char_at_pos(row, col)?;
                match c {
                    '\0'|' ' => {
                        if self.rest_null_ln(row, col) {
                            break;
                        } else {
                            line.push(' ');
                        }
                    },
                    _ => line.push(c)
                }
                col += 1;
            }
            out.push(line);
            row += 1;
        }

        Ok(out)
    }
}

#[derive(Clone)]
pub struct SubWindow {
    origin: Position,
    size: ScrSize,
    grid: Vec<Vec<char>>
}

impl ExtractLines for SubWindow {
    fn get_size(&self) -> (usize, usize) {
        (self.size.height, self.size.width)
    }

    fn get_char_at_pos(&self, row: usize, col: usize) -> Result<char> {
        if row <= self.size.height && col <= self.size.width {
            Ok(self.grid[row-1][col-1])
        } else {
            Err(Box::new(ScrErr::OutofBounds(row, col)))
        }
    }
}

pub trait ExtractWindows {
    fn get_size(&self) -> (usize, usize);
    fn get_char_at_pos(&self, row: usize, col: usize) -> Result<char>;

    fn contains_curses_windows(&self) -> bool {
        let mut scan = Position { y: 1, x: 1 };
        let (height, width) = self.get_size();

        while scan.y <= height {
            match self.get_char_at_pos(scan.y, scan.x) {
                Ok('┘') => return true,
                Ok('┐') => return true,
                Ok('┌') => return true,
                Ok('└') => return true,
                Ok('─') => return true,
                Ok('│') => return true,
                Ok(_) => {
                    scan.x += 1;
                    if scan.x > width {
                        scan.y += 1;
                        scan.x = 1;
                    }
                },
                Err(_) => return false
            }
        }

        false
    }

    fn follow_border(&self, row: usize, col: usize) -> Result<ScrSize> {
        // first follow along the top
        let mut scan = Position { y: row, x: col };
        let mut box_size = ScrSize { height: 0, width: 0 };
        let (height, width) = self.get_size();

        let top_left = self.get_char_at_pos(scan.y, scan.x)?;

        if top_left != '┌' {
            return Err(Box::new(ScrErr::BoxInvalid(scan.y, scan.x)));
        }

        let mut forward= true;
        let mut border_char = self.get_char_at_pos(scan.y, scan.x)?;
        let mut prev_border_char;
        loop {
            prev_border_char = border_char;
            match prev_border_char {
                '┌' if forward => {
                    scan.x += 1;
                    if scan.x > width {
                        return Err(Box::new(ScrErr::OutofBounds(scan.y, scan.x)));
                    }
                    border_char = self.get_char_at_pos(scan.y, scan.x)?;
                    if ! (border_char == '─' || border_char == '┐') {
                        return Err(Box::new(ScrErr::BrokenBorder));
                    }
                },
                '─' if forward => {
                    scan.x += 1;
                    if scan.x > width {
                        return Err(Box::new(ScrErr::OutofBounds(scan.y, scan.x)));
                    }
                    border_char = self.get_char_at_pos(scan.y, scan.x)?;
                    if ! (border_char == '─' || border_char == '┐') {
                        return Err(Box::new(ScrErr::BrokenBorder));
                    }
                },
                '┐' if forward => {
                    scan.y += 1;
                    if scan.y > height {
                        return Err(Box::new(ScrErr::OutofBounds(scan.y, scan.x)));
                    }
                    border_char = self.get_char_at_pos(scan.y, scan.x)?;
                    if ! (border_char == '│' || border_char == '┘') {
                        return Err(Box::new(ScrErr::BrokenBorder));
                    }
                },
                '│' if forward => {
                    scan.y += 1;
                    if scan.y > height {
                        return Err(Box::new(ScrErr::OutofBounds(scan.y, scan.x)));
                    }
                    border_char = self.get_char_at_pos(scan.y, scan.x)?;
                    if ! (border_char == '│' || border_char == '┘') {
                        return Err(Box::new(ScrErr::BrokenBorder));
                    }
                },
                '┘' if forward => {
                    forward = false;

                    // save box size
                    box_size.height = (scan.y - row) - 1;
                    box_size.width = (scan.x - col) - 1;

                    scan.x -= 1;
                    if scan.x < 1 {
                        return Err(Box::new(ScrErr::OutofBounds(scan.y, scan.x)));
                    }
                    border_char = self.get_char_at_pos(scan.y, scan.x)?;
                    if ! (border_char == '─' || border_char == '└') {
                        return Err(Box::new(ScrErr::BrokenBorder));
                    }
                },
                '─' if ! forward => {
                    scan.x -= 1;
                    if scan.x < 1 {
                        return Err(Box::new(ScrErr::OutofBounds(scan.y, scan.x)));
                    }
                    border_char = self.get_char_at_pos(scan.y, scan.x)?;
                    if ! (border_char == '─' || border_char == '└') {
                        return Err(Box::new(ScrErr::BrokenBorder));
                    }
                },
                '└' if ! forward => {
                    scan.y -= 1;
                    if scan.y < 1 {
                        return Err(Box::new(ScrErr::OutofBounds(scan.y, scan.x)));
                    }
                    border_char = self.get_char_at_pos(scan.y, scan.x)?;
                    if ! (border_char == '│' || border_char == '┌') {
                        return Err(Box::new(ScrErr::BrokenBorder));
                    }
                },
                '│' if ! forward => {
                    scan.y -= 1;
                    if scan.y < 1 {
                        return Err(Box::new(ScrErr::OutofBounds(scan.y, scan.x)));
                    }
                    border_char = self.get_char_at_pos(scan.y, scan.x)?;
                    if ! (border_char == '│' || border_char == '┌') {
                        return Err(Box::new(ScrErr::BrokenBorder));
                    }
                },
                '┌' if ! forward => {
                    if scan.y == row && scan.x == col {
                        return Ok(box_size);
                    }
                },
                _ => return Err(Box::new(ScrErr::BrokenBorder))
            }
        }
    }

    fn copy_data(&self, origin: &Position, size: &ScrSize, dest_grid: &mut Vec<Vec<char>>) -> Result<()> {
        let mut scan = Position { y: 0, x: 0 };

        while scan.y < size.height {
            dest_grid[scan.y][scan.x] = self.get_char_at_pos(origin.y + scan.y, origin.x + scan.x)?;
            scan.x += 1;
            if scan.x >= size.width {
                scan.x = 0;
                scan.y += 1;
            }
        }

        Ok(())
    }

    fn get_subwindows(&self) -> Result<Vec<SubWindow>> {
        let mut scan = Position { y: 1, x: 1 };
        let (height, width) = self.get_size();
        let mut main_corners = Vec::new();
        let mut boxes = Vec::new();

        // search for top-left ┌ box-pieces
        while scan.y <= height {
            if let Ok(c) = self.get_char_at_pos(scan.y, scan.x) {
                if c == '┌' {
                    main_corners.push((scan.y, scan.x));
                }
                scan.x += 1;
                if scan.x > width {
                    scan.y += 1;
                    scan.x = 1;
                }
            }
        }

        // sometimes the message window isn't properly boxed,
        // but we can check whether a section of the screen doesn't
        // have it's own proper boxing
        let mut min_border_row = height;
        let mut min_border_col = width;

        // confirm contiguous boxes & copy text
        for (row, col) in main_corners {
            if let Ok(box_size) = self.follow_border(row, col) {
                let mut grid = vec![vec![0 as char; box_size.width]; box_size.height];
                let origin = Position { y: (row + 1), x: (col + 1) };
                self.copy_data(&origin, &box_size, &mut grid)?;

                boxes.push(SubWindow
                {
                    origin,
                    size: box_size,
                    grid
                });
            }
            if row < min_border_row {
                min_border_row = row;
            }
            if col < min_border_col {
                min_border_col = col;
            }
        }

        if min_border_row > 1 && min_border_col > 1 {
            let size = ScrSize {
                height: min_border_row - 1,
                width: min_border_col - 1
            };
            let mut grid = vec![vec![0 as char; size.width]; size.height];
            let origin = Position {
                y: 1,
                x: 1
            };
            self.copy_data(&origin, &size, &mut grid)?;
            boxes.push(SubWindow
            {
                origin,
                size,
                grid
            });
        }

        Ok(boxes)
    }
}

impl GameScreen {
    pub fn new(width: usize, height: usize) -> Self {
        let grid = vec![vec![0 as char; width]; height];
        let cursor = Position {
            x: 0,
            y: 0
        };
        let size = ScrSize {
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

    // some escapes used we do not yet deal with:
    // ^[[1;40r
    // ^[>     ^[=
    // ^[[?1049h
    // ^[[?25l
    // ^[[4l
    // ^[[?7h
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
                b'X' => self.erase_chars(args[0]),
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

    fn erase_chars(&mut self, nr_chars: usize) {
        for i in 0 .. nr_chars {
            self.put_char(' ');
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


impl ExtractWindows for GameScreen {
    fn get_size(&self) -> (usize, usize) {
        (self.size.height, self.size.width)
    }

    fn get_char_at_pos(&self, row: usize, col: usize) -> Result<char> {
        if row <= self.size.height && col <= self.size.width {
            Ok(self.grid[row-1][col-1])
        } else {
            Err(Box::new(ScrErr::OutofBounds(row, col)))
        }
    }
}

//struct NHMap {
//}

#[derive(Debug)]
enum CharLevel {
    XLvl(u32),
    XLvlwExp(u32, u32),
    HD(u32)
}

#[derive(Debug)]
enum Strength {
    Normal(u32),
    Percentile(u32, u32)
}

#[derive(Debug)]
struct AbilityScores {
    strength: Strength,
    dexterity: u32,
    constitution: u32,
    intelligence: u32,
    wisdom: u32,
    charisma: u32
}

#[derive(Debug)]
enum Class {
    Rank(String),
    Polyform(String)
}

#[derive(Debug)]
enum Align {
    Lawful,
    Neutral,
    Chaotic,
    Unaligned
}

#[derive(Debug)]
struct NHStats {
    dlvl: u32,
    gold: u32,
    hp: u32,
    maxhp: u32,
    pw: u32,
    maxpw: u32,
    armour_class: i32,
    level: CharLevel,
    turns: Option<u32>,
    score: Option<u32>,
    ability: AbilityScores,
    align: Align,
    name: String,
    rank: Class
}

impl NHStats {
    fn new() -> Self {
        NHStats {
            dlvl: 1,
            gold: 0,
            hp: 1,
            maxhp: 10,
            pw: 5,
            maxpw: 10,
            armour_class: 10,
            level: CharLevel::XLvl(1),
            turns: None,
            score: None,
            ability: AbilityScores {
                strength: Strength::Normal(10),
                dexterity: 10,
                constitution: 10,
                intelligence: 10,
                wisdom: 10,
                charisma: 10
            },
            align: Align::Unaligned,
            name: String::from("luser"),
            rank: Class::Rank(String::from("windows hacker"))
        }
    }

    fn read_statusline(&mut self, window: &SubWindow) -> Result<()> {
        let mut saved_tokens: Vec<String> = Vec::new();
        for line in window.get_lines()? {
            for token in line.split_whitespace().clone() {
                let split_vec: Vec<&str> = token.splitn(2, ':').collect();
                if split_vec.len() == 1 {
                    saved_tokens.push(split_vec[0].to_string());
                } else {
                    let (field, value) = (split_vec[0], split_vec[1]);
                    match field {
                        "Dlvl" => if let Ok(n) = value.parse::<u32>() {
                            self.dlvl = n;
                        },
                        "$" => if let Ok(n) = value.parse::<u32>() {
                            self.gold = n;
                        },
                        "HP" => {
                            let split_maxhp: Vec<&str> = value.split(|c| c == '(' || c == ')').collect();
                            if split_maxhp.len() >= 2 {
                                if let Ok(n) = split_maxhp[0].parse::<u32>() {
                                    self.hp = n;
                                }
                                if let Ok(n) = split_maxhp[1].parse::<u32>() {
                                    self.maxhp = n;
                                }
                            }
                        },
                        "Pw" => {
                            let split_maxpw: Vec<&str> = value.split(|c| c == '(' || c == ')').collect();
                            if split_maxpw.len() >= 2 {
                                if let Ok(n) = split_maxpw[0].parse::<u32>() {
                                    self.pw = n;
                                }
                                if let Ok(n) = split_maxpw[1].parse::<u32>() {
                                    self.maxpw = n;
                                }
                            }
                        },
                        "AC" => if let Ok(n) = value.parse::<i32>() {
                            self.armour_class = n;
                        },
                        "HD" => if let Ok(n) = value.parse::<u32>() {
                            self.level = CharLevel::HD(n);
                        },
                        "Xp" => {
                            let split_xp: Vec<&str> = value.splitn(2, '/').collect();
                            if split_xp.len() == 1 {
                                if let Ok(n) = split_xp[0].parse::<u32>() {
                                    self.level = CharLevel::XLvl(n);
                                }
                            } else if split_xp.len() == 2 {
                                if let Ok(nxp) = split_xp[0].parse::<u32>() {
                                    if let Ok(xp_points) = split_xp[1].parse::<u32>() {
                                        self.level = CharLevel::XLvlwExp(nxp, xp_points);
                                    }
                                }
                            }
                        },
                        "T" => if let Ok(n) = value.parse::<u32>() {
                            self.turns = Some(n);
                        },
                        "S" => if let Ok(n) = value.parse::<u32>() {
                            self.score = Some(n);
                        },
                        "St" => {
                            let st_split: Vec<&str> = value.splitn(2,'/').collect();
                            if st_split.len() == 1 {
                                if let Ok(n) = st_split[0].parse::<u32>() {
                                    self.ability.strength = Strength::Normal(n);
                                }
                            } else if st_split.len() == 2 {
                                if let Ok(n) = st_split[0].parse::<u32>() {
                                    match st_split[1] {
                                        "**" => self.ability.strength = Strength::Percentile(n, 100),
                                        _ => if let Ok(perc) = st_split[1].parse::<u32>() {
                                            self.ability.strength = Strength::Percentile(n, perc);
                                        }
                                    }
                                }
                            }
                        },
                        "Dx" => if let Ok(n) = value.parse::<u32>() {
                            self.ability.dexterity = n;
                        },
                        "Co" => if let Ok(n) = value.parse::<u32>() {
                            self.ability.constitution = n;
                        },
                        "In" => if let Ok(n) = value.parse::<u32>() {
                            self.ability.intelligence = n;
                        },
                        "Wi" => if let Ok(n) = value.parse::<u32>() {
                            self.ability.wisdom = n;
                        },
                        "Ch" => if let Ok(n) = value.parse::<u32>() {
                            self.ability.charisma = n;
                        },
                        _ => saved_tokens.push(token.to_string())
                    }
                }
            }
        }

        // now with the number stuff out the way we try to do the
        // remaning considerations
        if let Some(last_token) = saved_tokens.pop() {
            match last_token.as_str() {
                "Lawful" => self.align = Align::Lawful,
                "Neutral" => self.align = Align::Neutral,
                "Chaotic" => self.align = Align::Chaotic,
                "Unaligned" => self.align = Align::Unaligned,
                _ => ()
            }
        }

        for i in 0 .. saved_tokens.len() {
            if i >= 1 && saved_tokens[i].as_str() == "the" && i < saved_tokens.len() {
                self.name = saved_tokens[i-1].clone();
                match self.level {
                    CharLevel::HD(_) => self.rank = Class::Polyform(saved_tokens[i+1].clone()),
                    _ =>                self.rank = Class::Rank(saved_tokens[i+1].clone())
                }
            }
        }

        Ok(())
    }
}

type NHInv = Vec<NHInvItem>;

enum ItemClass {
    Weapons,
    Armour,
    Comestibles,
    Wands,
    Rings,
    Potions,
    Scrolls,
    Spellbooks,
    Tools,
    GemsnStones
}

enum BUC {
    Blessed,
    Uncursed,
    Cursed
}

enum WearType {
    Corroded,
    Rusty,
    Burnt,
    Rotted
}

enum WearExtent {
    None,
    Some,
    Very,
    Thoroughly
}

struct Wear {
    e_type: WearType,
    e_extent: WearExtent
}

struct NHInvItem {
    item: ItemClass,
    inventory_letter: char, // strictly speaking A-Z
    beatitude: Option<BUC>,
    erosion: Wear,
    charges: Option<u32>,
    enchantment: Option<u32>,
    fooproofed: bool,
    greased: bool,
    description: String,
    name: String
}

struct NetHackData {
    windows: Vec<SubWindow>,
    //level_map: NHMap,
    inventory: NHInv,
    status: NHStats
}

impl NetHackData {
    pub fn new() -> Self {
        NetHackData {
            windows: Vec::new(),
            inventory: Vec::new(),
            status: NHStats::new()
        }
    }

    pub fn update(&mut self, term: &GameScreen) -> Result<()> {
        self.windows.clear();
        let sub_windows = term.get_subwindows()?;
        for window in sub_windows {
            self.windows.push(window);
        }

        for window in self.windows.clone() {
            let (height, width) = window.get_size();
            write!(stderr(), "win size: {} by {}\n", height, width);
            if height == 2 {
                // statusline!
                self.status.read_statusline(&window);
                write!(stderr(), "{:?}\n", self.status);
            }
        }

        Ok(())
    }

    pub fn debug(&self, stderr: &mut Stderr) {
        let mut window_nr = 1;

        for win in self.windows.clone() {
            write!(stderr, "this is the {}th window\n", window_nr);
            if let Ok(line_vec) = win.get_lines() {
                for line in line_vec {
                    write!(stderr, "{}\n", line);
                }
            }
            window_nr += 1;
        }
    }

    

    //try_inventory(&mut self, window: &SubWindow) -> Result<()> {

    //}
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
    let mut game_term = GameScreen::new(ws_col as usize, ws_row as usize);
    let mut nethack = NetHackData::new();
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
        let mut loop_counter = 0;
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

                    write!(stderr, "{}", String::from_utf8_lossy(&mut buffer[..n]))?;
                    stderr.flush();
                    loop_counter += 1;
                    if loop_counter > 3 {
                        game_term.update(&buffer[..n]);
                        //game_term.dump(&mut stderr);
                        nethack.update(&game_term);
                        //nethack.debug(&mut stderr);
                        loop_counter = 0;
                    }
                },
                Err(e) => {
                    //println!("error reading output sent to {}: {}", our_pty.unwrap(), e);
                    println!("error reading output sent our tty: {}", e);
                    return Err(Box::new(e));
                }
            }
            thread::sleep(Duration::from_millis(50));
            //write!(stderr, "\nabove was raw, below is a dump of the buffer\n");
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

