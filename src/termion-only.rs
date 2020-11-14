extern crate termion;
use std::thread;
use std::process::{Stdio, Command, ChildStdin, ChildStdout};
use std::io::{self, Read, Write, Stdout, stdout, Stdin, stdin};
use termion::{async_stdin, terminal_size};
use termion::raw::{RawTerminal, IntoRawMode};
use termion::AsyncReader as TermReader;
use termion::input::TermReadEventsAndRaw;
use termion::clear;

fn main() -> Result<(), io::Error> {
    let stdin = async_stdin();
    let mut stdout = stdout();//.into_raw_mode().unwrap();

    //// clear screen
    //write!(stdout, "{}", clear::All);
    
    // terminal size is what
    //let (width, height) = terminal_size().unwrap();
    //write!(stdout, "terminal size: {} by {}", width, height);
    //stdout.flush();

    let mut cmd = Command::new("nethack");
    //cmd.arg("hfe");
    // Specify that we want the command's standard output piped back to us.
    // By default, standard input/output/error will be inherited from the
    // current process (for example, this means that standard input will
    // come from the keyboard and standard output/error will go directly to
    // the terminal if this process is invoked from the command line).
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.stdin(Stdio::piped());

    let mut child = cmd.spawn()
        .expect("failed to spawn command");

    // set up I/O handlers for child process running game
    let mut child_stdout = child.stdout.take()
        .expect("child did not have a handle to stdout");
    //let mut child_stderr = child.stderr.take()
    //    .expect("child did not have a handle to stderr");
    let mut child_stdin = child.stdin.take()
        .expect("child did not have a handle to stderr");

    // in order to disable input line-buffering, we need
    // to enable raw-mode on stdout
    // this probably means we have to use a load of
    // spawn_blocking() calls for sending output to the screen :/
    // may also be possible to use a blocking thread for all I/O
    // going towards the output... will think about this




    // spawn a background thread to deal with the input
    let input_handler = thread::spawn(move || {
        // loop over events on the term input,(_eventkey, bytevec)
        // forward keys to child process
        for event in stdin.events_and_raw() {
            if let Ok((_event, byte_vector)) = event {
                child_stdin.write_all(&byte_vector);
                child_stdin.flush();
            }
        }
    });

    // main loop 
    loop {
        let mut buffer: [u8; 4096] = [0; 4096];
        
        match child_stdout.read(&mut buffer) {
            Ok(n) if n > 0  => {
                write!(stdout, "{}", String::from_utf8_lossy(&buffer[..n]));
                stdout.flush();
            },
            Ok(_)           => break,
            Err(e)          => {
                println!("got error {} on child_stdout", e);
                break;
            }
        };
    }

    // wait on completion of input handler thread
    let input_handler_result = input_handler.join();

    // Ensure the child process is spawned in the runtime so it can
    // make progress on its own while we await for any output.
    let status = child.wait()
            .expect("child process encountered an error");
    if status.success() {
        println!("child was successful!");
    } else {
        println!("child returned unsuccessful :(");
    }

    Ok(())
}
