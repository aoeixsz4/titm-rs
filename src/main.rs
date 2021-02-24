extern crate nix;
extern crate termion;
extern crate terminal_emulator;
mod term;
use crate::term::{fork_terminal, TermFork};
use std::error;
use std::io::{stdin, stdout, stderr, Write};
use std::process::Command;
use std::thread;
//use std::time::Duration;
use terminal_emulator::ansi::Processor;
use termion::raw::IntoRawMode;
use termion::input::TermReadEventsAndRaw;

type Result<T> = std::result::Result<T, Box<dyn error::Error>>;

fn main() -> Result<()> {
    match fork_terminal()? {
        TermFork::Parent(pty_reader, mut pty_writer, mut terminal) => {
            let mut stdout = stdout().into_raw_mode().unwrap();
            let stdin = stdin();
            let mut stderr = stderr();

            // spawn a background thread to deal with the input
            let _join_handler = thread::spawn(move || {
                for event in stdin.events_and_raw() {
                    if let Ok((_event, byte_vector)) = event {
                        pty_writer.write(&byte_vector);
                        pty_writer.flush();                        
                    }
                    //thread::sleep(Duration::from_millis(50));
                }
            });

            let mut processor = Processor::new();
            for c in pty_reader {
                processor.advance(&mut terminal, c, &mut stdout);
                let buf = [c];
                stdout.write_all(&buf);
                stderr.write_all(&buf);
            }
        },
        TermFork::Child => {
            Command::new("ipbt")
                        .arg("-u")
                        .arg("--utf8-linedraw")
                        .arg("2020-12-23.12:10:09.ttyrec")
                        .status().expect("could not execute tty");
        }
    }
    Ok(())
}

