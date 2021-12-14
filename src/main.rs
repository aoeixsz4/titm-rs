mod term;
use crate::term::{fork_terminal, TermFork};
use std::error;
use std::str;
use std::i32::MAX;
use std::f64::MAX as MAX_FLOAT;
use std::io::{stdout, Write};
use std::process::Command;
use regex::CaptureLocations;
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

const DIRECTION_GRID: [[&'static str; 3]; 3] = [["y","k","u"],["h",".","l"],["b","j","n"]];

fn get_direction_key(dx: i32, dy: i32) -> &'static str {
    let (x, y) = ((dx/dx.abs())+1, (dy/dy.abs())+1);
    DIRECTION_GRID[y as usize][x as usize]
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

enum Item<'a> {
    Wand(&'a str),
    Strange(&'a str)
}

enum LookFeet<'a> {
    Nothing,
    UpStairs,
    DownStairs,
    Loot(Item<'a>)
}

fn get_token<'a> (locs: &CaptureLocations, s: &'a str, i: usize) -> &'a str {
    let (b, l) = locs.get(i).unwrap();
    &s[b..l]
}

fn get_token_opt<'a> (locs: &CaptureLocations, s: &'a str, i: usize) -> Option<&'a str> {
    if let Some((b, l)) = locs.get(i) {
        Some(&s[b..l])
    } else {
        None
    } 
}

fn parse_look_message<'a> (buf: &'a [u8]) -> Option<LookFeet<'a>> {
    for bytes in buf.rsplitn(10, |c| *c == b'\x1b')
        .find(|s| s.len() >= 5 && &s[0..5] == "[0;1m".as_bytes()) {
        let no_objects_re = Regex::new(
            r"You see no objects here\."
        ).unwrap();
        let s = str::from_utf8(bytes).unwrap();
        
        if no_objects_re.is_match(s) {
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
        let mut locs = re.capture_locations();
        let first_match = re.captures_read(&mut locs, s);
        if first_match.is_none() { return None; }
        if get_token(&locs, s, 0) == "There is a staircase up here." {
            return Some(LookFeet::UpStairs);
        }
        if get_token(&locs, s, 0) == "There is a staircase down here." {
            return Some(LookFeet::DownStairs);
        }
        if get_token(&locs, s, 1) == "You see here" {
            let (item_type, item_description) = if let Some(_) = locs.get(5) {
                (get_token(&locs, s, 4), get_token_opt(&locs, s, 6).unwrap_or(""))
            } else {
                (get_token(&locs, s, 6), get_token_opt(&locs, s, 4).unwrap_or(""))
            };
            if item_type == "wand" {
                return Some(LookFeet::Loot(Item::Wand(item_description)));
            }
            return Some(LookFeet::Loot(Item::Strange(&s[13..])));
        }
    }
    None
}

fn quit_string(stairs: bool) -> &'static str {
    if stairs { "<y   " } else { "# quit\ny   " }
}

fn respond(term: &Term, read_buf: &[u8], have_picked: &mut bool, have_looked: &mut bool, stairs: &mut bool) -> Option<&'static str> {
    let (north, south, east, west) = get_box(term);
    if let Some(feature) = parse_look_message(&read_buf[read_buf.len() - 512 ..]) {
        match feature {
            LookFeet::Loot(item) => {
                match item {
                    Item::Wand(_) => { *have_picked = true; return Some(","); },
                    Item::Strange(_) => ()
                }
            },
            LookFeet::UpStairs => *stairs = true,
            LookFeet::DownStairs => *stairs = false,
            LookFeet::Nothing => *stairs = false
        }
        *have_looked = true;
    }
    if *have_looked {
        if let Some((dy, dx)) = get_wand_vector(term) {
            if dy < north && dy > south && dx < west && dx > east {
                *have_looked = false;
                Some(get_direction_key(dx, dy))
            } else {
                Some(quit_string(*stairs))
            }
        } else {
            Some(quit_string(*stairs))
        }
    } else {
        Some(":")
    }
}

const SHOW_CURSOR_SEQUENCE: &'static str = "\x1b[?25h";

fn main() -> Result<()> {
    match fork_terminal()? {
        TermFork::Parent(pty_reader, mut pty_writer, mut terminal) => {
            let mut stdout = stdout().into_raw_mode().unwrap();
            let mut processor = Processor::new();
            let mut read_buf= [0u8; 4096];
            let mut have_looked = false;
            let mut have_picked = true;
            let mut stairs = false;

            for c in pty_reader {
                processor.advance(&mut terminal, c, &mut stdout);
                read_buf[read_buf.len() - 1] = c;
                stdout.write(&read_buf[read_buf.len() - 1..])?;
                stdout.flush()?;
                if str::from_utf8(&read_buf[read_buf.len() - 6 ..]).unwrap() == SHOW_CURSOR_SEQUENCE {
                    if let Some(out) = respond(&terminal, &read_buf, &mut have_picked, &mut have_looked, &mut stairs) {
                        pty_writer.write(out.as_bytes())?;
                    }
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

