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
use std::io::{Read, Write, stdin, stdout};
use std::process::{Command, Stdio};
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

fn main() -> Result<()> {
    let (ws_col, ws_row) = terminal_size()?;
    let (ws_ypixel, ws_xpixel) = terminal_size_pixels()?;
    let win_size = Winsize {
        ws_row,
        ws_col,
        ws_xpixel,
        ws_ypixel 
    };
    let fork = forkpty(Some(&win_size), None)?;

    if fork.fork_result.is_parent() {
        let mut stdout = stdout().into_raw_mode().unwrap();
        let stdin = stdin();
        let mut raw_fd = fork.master;

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
        
        let input_handler = thread::spawn(move || {
            // loop over events on the term input,(_eventkey, bytevec)
            // forward keys to child process
            for event in stdin.events_and_raw() {
                if let Ok((_event, byte_vector)) = event {
                    raw_write(raw_fd, &byte_vector);
                    unistd::fsync(raw_fd);
                }
                thread::sleep(Duration::from_millis(100));
            }
        });

        // continue reading, and copy raw to our stdout
        loop {
            let mut buffer: [u8; 4096] = [0; 4096];
            match raw_read(raw_fd, &mut buffer) {
                Ok(n) => {
                    if n == 0 { break }
                    write!(stdout, "{}", String::from_utf8_lossy(&mut buffer[..n]))?;
                    stdout.flush();
                },
                Err(e) => {
                    //println!("error reading output sent to {}: {}", our_pty.unwrap(), e);
                    println!("error reading output sent our tty: {}", e);
                    return Err(Box::new(e));
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
    } else {
        // Child process just exec `tty`
        //Command::new("tty").status().expect("could not execute tty");
        //Command::new("stty").arg("-a").status().expect("could not execute stty -a");
        //Command::new("nethack").status().expect("could not execute local nethack");
        Command::new("ssh").arg("hdf").status().expect("could not execute local nethack"); 

    }
    Ok(())
}
