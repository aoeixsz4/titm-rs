extern crate tokio;
extern crate termion;
extern crate log;
use log::{debug, warn};
use tokio::io::{BufReader, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Command, ChildStdin, ChildStdout};
use tokio::sync::mpsc::{Sender, Receiver, channel};
use tokio::task;
use std::process::Stdio;
use std::io::{Read, Write, Stdout, stdout};
use termion::async_stdin;
use termion::raw::{RawTerminal, IntoRawMode};
use termion::AsyncReader as TermReader;

// we'll want a spawned task with an mpsc receiver
// user input will be sent across the channel, and the
// handler will forward it to the child stdin,
// but also other events will trigger certain keystrokes to
// be sent to the child stdin (e.g. scripted scumming events)

async fn game_input_handler(mut rx: Receiver<u8>, mut game_stdin: ChildStdin) {
    let mut buffer: [u8; 4096] = [0; 4096];
    loop {
        let mut write_cursor = 0;
        let mut read_index = 0;

        // block on the first input
        if let Some(key) = rx.recv().await {
            buffer[read_index] = key;
            read_index += 1;

            // check for more until we would block
            while let Ok(key) = rx.try_recv() {
                buffer[read_index] = key;
                read_index += 1;
            }

            // try to write everything
            while write_cursor < read_index {
                if let Ok(n_bytes) = game_stdin.write(&buffer[write_cursor .. read_index]).await {
                    write_cursor += n_bytes;
                } else {
                    warn!("write to child's stdin failed");
                    return;
                }
            }
            if let Err(_) = game_stdin.flush().await {
                warn!("flush child's stdin failed");
                return;
            }
        } else {
            warn!("receiving from channel failed");
            return;
        }
    }
}

fn copy_buf(buffer: &[u8], length: usize) -> Vec<u8> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < length {
        result.push(buffer[index]);
        index += 1;
    }
    result
}

// handler for output to terminal
async fn game_output_handler(mut game_stdout: ChildStdout) {
    let mut buffer: [u8; 4096] = [0; 4096];
    let mut reader = BufReader::new(game_stdout);

    while let Ok(n_read) = reader.read(&mut buffer).await {
        if n_read == 0 { // EOF
            warn!("got EOF on child's output");
            return;
        }
        let vec_copy = copy_buf(&buffer, n_read);
        let mut stdout = stdout().into_raw_mode().unwrap();
        task::spawn_blocking(move || {
            stdout.write(&vec_copy);
            stdout.flush();
        });
    }
}

async fn user_input_handler(mut term_input: TermReader, tx: Sender<u8>) {
    let mut buffer: [u8; 4096] = [0; 4096];

    loop {
        // this may be a bit of a busy loop if we don't
        // occasionally yield - apparently yielding will put
        // us at the back of the queue but we don't necessarily
        // need special waking
        if let Ok(bytes_read) = term_input.read(&mut buffer) {
            if bytes_read == 0 { warn!("EOF on term input"); continue; } // EOF
            let mut index = 0;
            while index < bytes_read {
                if let Err(_) = tx.send(buffer[index]).await {
                    warn!("send to channel failed");
                    return;
                } else {
                    index += 1;
                }
            }
        }
        task::yield_now().await;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::new("ssh");
    cmd.arg("hfe");

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
    let child_stdin = child.stdin.take()
        .expect("child did not have a handle to stderr");

    // in order to disable input line-buffering, we need
    // to enable raw-mode on stdout
    // this probably means we have to use a load of
    // spawn_blocking() calls for sending output to the screen :/
    // may also be possible to use a blocking thread for all I/O
    // going towards the output... will think about this
    let mut stdout = stdout().into_raw_mode().unwrap();

    // set up channel for handler of input TO game
    let (tx, rx) = channel(100);
    task::spawn(game_input_handler(rx, child_stdin));

    // set up handler for terminal input from user
    let mut term_stdin = async_stdin();
    task::spawn(user_input_handler(term_stdin, tx));

    // Ensure the child process is spawned in the runtime so it can
    // make progress on its own while we await for any output.
    task::spawn(async move {
        let status = child.wait().await
            .expect("child process encountered an error");

        println!("child status was: {}", status);
    });

    // set up handler for terminal output, forwarded
    // from game stdout to the main terminal
    game_output_handler(child_stdout).await;

    Ok(())
}
