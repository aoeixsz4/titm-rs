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
#![feature(iter_intersperse)]
mod term;
use crate::term::{fork_terminal, TermFork};
use std::error;
use std::i32::MAX;
use std::f64::MAX as MAX_FLOAT;
use std::io::{stdout, stderr, Write};
use std::process::Command;
use terminal_emulator::ansi::Processor;
use terminal_emulator::term::Term;
use termion::raw::IntoRawMode;
use regex::Regex;

type Result<T> = std::result::Result<T, Box<dyn error::Error>>;

fn get_box(term: &Term) -> (i32, i32, i32, i32) {
    let (cursor_posy, cursor_posx) = (*term.cursor().point.line as i32, *term.cursor().point.col as i32);
    let (mut north, mut east, mut south, mut west) = (MAX, -MAX, -MAX, MAX);
    let grid = term.grid();
    for cell in grid.display_iter() {
        let (disty, distx) = (cursor_posy - *cell.line as i32, cursor_posx - *cell.column as i32);
        match cell.c {
            '─' if disty > 0 && disty < north => north = disty,
            '─' if disty < 0 && disty > south => south = disty,
            '│' if distx < 0 && distx > east => east = distx,
            '│' if distx > 0 && distx < west  => west = distx,
            _ => ()
        }
    }
    (north, south, east, west)
}

fn calculate_distance(disty: i32, distx: i32) -> f64 {
    (f64::from(disty).powf(2.0) + f64::from(distx).powf(2.0)).sqrt()
}

fn get_direction(disty: i32, distx: i32) -> char {
    if disty > 0 && distx > 0 {
        'y'
    } else if disty > 0 && distx == 0 {
        'k'
    } else if disty > 0 && distx < 0 {
        'u'
    } else if disty == 0 && distx < 0 {
        'l'
    } else if disty < 0 && distx < 0 {
        'n'
    } else if disty < 0 && distx == 0 {
        'j'
    } else if disty < 0 && distx > 0 {
        'b'
    } else {
        'h'
    }
}

fn get_wand_vector(term: &Term) -> Option<(i32, i32)> {
    let (cursor_posy, cursor_posx) = (*term.cursor().point.line as i32, *term.cursor().point.col as i32);
    let mut distance = MAX_FLOAT;
    let mut result = None;
    let grid = term.grid();
    for cell in grid.display_iter() {
        if cell.c == '/' {
            let (dy, dx) = (cursor_posy - *cell.line as i32, cursor_posx - *cell.column as i32);
            let d = calculate_distance(dy,dx);
            if d < distance {
                distance = d;
                result = Some((dy, dx));
            }
        }
    }
    result
}

fn shift(buf: &mut [u8]) {
    for i in 1 .. buf.len() {
        buf[i-1] = buf[i]
    }
}

enum Item {
    Wand(String),
    Strange(String)
}

enum LookFeet {
    Nothing,
    Stairs,
    Loot(Item)
}

fn parse_look_message(buf: &[u8]) -> Option<LookFeet> {
    buf.rsplitn(10, |c| *c == b'\x1b')
        .find(|s| s.len() >= 5 && &s[0..5] == "[0;1m".as_bytes())
        .map_or(None, |s| {
        let no_objects_re = Regex::new(
            r"You see no objects here\."
        ).unwrap();
        let bytes_vector = s.to_vec();
        let string = String::from_utf8(bytes_vector).unwrap();
        
        let mut stderr = stderr().into_raw_mode().unwrap();
        stderr.write(format!("{}\n", &string).as_bytes());
        if no_objects_re.is_match(&string) {
            return Some(LookFeet::Nothing);
        }
        let re = Regex::new(
            r"(?x)
            (You\ssee\shere|There\sis)\s
            (an?|\d+)\s
            (?:(blessed|cursed|uncursed|holy|unholy)\s)?
            (?:([[:^space:]]+)\s)*
            (?:(of)\s)?
            ([[:^space:]]+)
            (?:\s(named|called)
                \s([[:^space:]]+))?
            (?:\s\(
                (?P<C>\d+)
                : (?P<c>\d+)
            \))?\."
        ).unwrap();
        for cap in re.captures_iter(&string) {
            stderr.write(format!("{}, {}, {}", &cap[0].to_string(), &cap[1].to_string(), &cap[2].to_string()).as_bytes());
            if cap[0].eq("There is a staircase up here.") {
                return Some(LookFeet::Stairs);
            }
            if cap[1].eq("You see here") {
                for i in 0 .. cap.len() {
                    if cap[i].eq("wand") && i + 2 < cap.len() && cap[i+1].eq("of") {
                        return Some(LookFeet::Loot(Item::Wand(cap[i+2].to_string())));
                    } else if cap[i].eq("wand") {
                        return Some(LookFeet::Loot(Item::Wand(cap[i-1].to_string())));
                    }
                }
                return Some(LookFeet::Loot(Item::Strange(cap.iter().skip(2).map(|c|c.unwrap().as_str()).intersperse(" ").collect::<String>())));
            }
        }
        None
    })
}

fn main() -> Result<()> {

    match fork_terminal()? {
        TermFork::Parent(pty_reader, mut pty_writer, mut terminal) => {
            let mut stdout = stdout().into_raw_mode().unwrap();
            let mut stderr = stderr().into_raw_mode().unwrap();
            let mut processor = Processor::new();

            let mut read_buf: [u8; 4096] = [ 0; 4096 ];
            let mut unicode_buf: [u8; 6] = [ 0; 6 ];
            let mut have_looked = false;
            let mut stairs = false;

            for c in pty_reader {
                processor.advance(&mut terminal, c, &mut stdout);
                read_buf[read_buf.len() - 1] = c;
                stdout.write(&read_buf[read_buf.len() - 1..])?;
                stdout.flush()?;
                match String::from_utf8(read_buf[read_buf.len() - 6 ..].to_vec()).unwrap().as_str() {
                    "\x1b[?25h" => {
                        let (north, south, east, west) = get_box(&terminal);
                        let at_feet = parse_look_message(&read_buf[read_buf.len() - 512 ..]);
                        if let Some(feature) = at_feet {
                            match feature {
                                LookFeet::Loot(item) => {
                                    match item {
                                        Item::Wand(s) => {
                                            pty_writer.write(",".as_bytes())?;
                                            stderr.write(format!("picked up a {} wand!\n", s).as_bytes())?;
                                            stderr.flush()?;
                                        },
                                        Item::Strange(s) => {
                                            stderr.write(format!("found strange {} on ground!\n", s).as_bytes())?;
                                            stderr.flush()?;
                                        }
                                    }
                                },
                                LookFeet::Stairs => stairs = true,
                                LookFeet::Nothing => stairs = false
                            }
                            have_looked = true;
                        }
                        if have_looked {
                            stderr.flush()?;
                            if let Some((dy, dx)) = get_wand_vector(&terminal) {
                                if dy < north && dy > south && dx < west && dx > east {
                                    stderr.write(format!("wand at location: {}, {}\n", dy, dx).as_bytes())?;
                                    stderr.flush()?;
                                    pty_writer.write(get_direction(dy, dx).encode_utf8(&mut unicode_buf).as_bytes())?;
                                    have_looked = false;
                                } else {
                                    if stairs {
                                        pty_writer.write("<y   ".as_bytes())?;
                                    } else {
                                        pty_writer.write("# quit\ny   ".as_bytes())?;
                                    }
                                }
                            } else {
                                if stairs {
                                    pty_writer.write("<y   ".as_bytes())?;
                                } else {
                                    pty_writer.write("# quit\ny   ".as_bytes())?;
                                }
                            }
                        } else {
                            pty_writer.write(':'.encode_utf8(&mut unicode_buf).as_bytes())?;
                        }
                    },
                    _ => ()
                }
                pty_writer.flush()?;
                shift(&mut read_buf);
            }

            Ok(())
        },
        TermFork::Child => {
            Command::new("nethack").status().expect("could not execute local nethack");
            Ok(())
        }
    }
}

