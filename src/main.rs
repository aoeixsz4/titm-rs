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

extern crate pty;

use std::io::{self, Read, Write, Result};
use std::process::{Command};

use pty::fork::Fork;

fn main() -> Result<()> {
  let fork = Fork::from_ptmx().unwrap();

  if let Some(mut terminal) = fork.is_parent().ok() {
    // Read output via PTY master
    let mut output = String::new();

    let our_pty = match terminal.read_to_string(&mut output) {
        Ok(_nread) => {
            println!("child tty is: {}", output.trim());
            Some(output.trim())
        },
        Err(e)     => {
            panic!("read error: {}", e);
            // unreachable expression - I don't fool the compiler :D None
        }
    };

    // continue reading, and copy raw to our stdout
    loop {
        let mut buffer: [u8; 4096] = [0; 4096];
        match terminal.read(&mut buffer) {
            Ok(n) => {
                io::stdout().write_all(&mut buffer[..n])?;
            },
            Err(e) => {
                println!("error reading output sent to {}: {}", our_pty.unwrap(), e);
                return Err(e);
            }
        }
    }
  }
  else {
    // Child process just exec `tty`
    Command::new("tty").status().expect("could not execute tty"); 
  }
  Ok(())
}
