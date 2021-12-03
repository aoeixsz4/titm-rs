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
extern crate terminal_emulator;
mod term;
use crate::term::{fork_terminal, TermFork};
use std::error;
use std::i64::MAX;
use std::io::{stdin, stdout, stderr, Write, Stderr};
use std::process::Command;
use std::thread;
//use std::time::Duration;
use terminal_emulator::ansi::Processor;
use terminal_emulator::term::Term;
use termion::raw::IntoRawMode;
use termion::input::TermReadEventsAndRaw;

type Result<T> = std::result::Result<T, Box<dyn error::Error>>;

fn get_box(term: &Term) -> (i64, i64, i64, i64) {
    let (cursor_posy, cursor_posx) = (*term.cursor().point.line as i64, *term.cursor().point.col as i64);
    let (mut north, mut east, mut south, mut west) = (MAX, -MAX, -MAX, MAX);
    let grid = term.grid();
    for cell in grid.display_iter() {
        let (disty, distx) = (cursor_posy - *cell.line as i64, cursor_posx - *cell.column as i64);
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

fn inspect_grid(term: &Term, stderr: &mut Stderr) -> Option<(i64, i64)> {
    let (cursor_posy, cursor_posx) = (*term.cursor().point.line as i64, *term.cursor().point.col as i64);
    let grid = term.grid();
    for cell in grid.display_iter() {
        let mut buf = [0; 8];
        stderr.write(cell.c.encode_utf8(&mut buf).as_bytes());
        if *cell.column == 0 {
            buf[0] = '\n' as u8;
            stderr.write(&buf[..1]);
        }
    }
    None
}

fn shift(buf: &mut [u8]) {
    for i in 1 .. buf.len() {
        buf[i-1] = buf[i]
    }
}

fn main() -> Result<()> {

    match fork_terminal()? {
        TermFork::Parent(pty_reader, mut pty_writer, mut terminal) => {
            let mut stdout = stdout().into_raw_mode().unwrap();
            let mut stderr = stderr().into_raw_mode().unwrap();
            let stdin = stdin();
            let mut processor = Processor::new();

            // spawn a background thread to deal with the input
            
            let _join_handler = thread::spawn(move || {
                // loop over events on the term input,(_eventkey, bytevec)
                // forward keys to child process
                for event in stdin.events_and_raw() {
                    if let Ok((_event, byte_vector)) = event {
                        pty_writer.write(&byte_vector);
                        pty_writer.flush();                        
                    }
                    //thread::sleep(Duration::from_millis(50));
                }
            });

            // would like to abstract this raw_read stuff a bit,
            // and just have a bytes iterator coming in, and being
            // passed to processor.advance()
            let mut buf: [u8; 4096] = [ 0; 4096 ];
            for c in pty_reader {
                // do stuff with received byte
                processor.advance(&mut terminal, c, &mut stdout);
                buf[buf.len() - 1] = c;
                //stderr.write(&buf);
                //stderr.flush();
                stdout.write(&buf[buf.len() - 1..]);
                stdout.flush();
                match String::from_utf8(buf[buf.len() - 6 ..].to_vec()).unwrap().as_str() {
                    "\x1b[?25h" => {
                        inspect_grid(&terminal, &mut stderr);
                        let (north, south, east, west) = get_box(&terminal);
                        stderr.write(format!("box distances: N {}, S {}, E {}, W {}", north, south, east, west).as_bytes());
                        stderr.flush();
                    },
                    _ => ()
                }
                shift(&mut buf);
                //thread::sleep(Duration::from_millis(50));
            }

            Ok(())
        },
        TermFork::Child => {
            // Child process just exec `tty`
            //Command::new("tty").status().expect("could not execute tty");
            //Command::new("stty").arg("-a").status().expect("could not execute stty -a");
            Command::new("nethack").status().expect("could not execute local nethack");
            //Command::new("ssh").arg("hdf").status().expect("could not execute local nethack");
            //Command::new("sh").status().expect("could not execute shell");
            Ok(())
        }
    }
}

