use tokio::io::{self, BufReader, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Command, ChildStdin};
use tokio::sync::mpsc::{Sender, Receiver, channel};
use tokio::task;
use std::process::Stdio;
use std::io::{stdin, Stdin};
use std::io::Read as stdRead;

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
                    return;
                }
            }
            if let Err(_) = game_stdin.flush().await {
                return;
            }
        } else {
            return;
        }
    }
}

// this task reads keystrokes from the user/terminal input,
// and relays them to the handler for the game input, via mpsc channel
fn user_input_handler(mut user_stdin: Stdin, tx: Sender<u8>) {
    let mut buffer: [u8; 4096] = [0; 4096];

    while let Ok(length) = user_stdin.read(&mut buffer) {
        if length == 0 { return; }
        let mut index = 0;
        while index < length {
            if let Err(_) = tx.blocking_send(buffer[index]) {
                return;
            } else {
                index += 1;
            }
        }
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

    // set up channel for handler of input TO game
    let (tx, rx) = channel(100);
    task::spawn(game_input_handler(rx, child_stdin));

    // set up handler for I/O input from terminal/user
    let mut stdin = stdin();
    task::spawn_blocking(move || user_input_handler(stdin, tx.clone()));

    // the 'main' task simply reads data from the child's standard output,
    // relays it to our parent stdout *and* our game watcher task
    let mut reader = BufReader::new(child_stdout);
    let mut buffer: [u8; 4096] = [0; 4096];

    // Ensure the child process is spawned in the runtime so it can
    // make progress on its own while we await for any output.
    task::spawn(async move {
        let status = child.wait().await
            .expect("child process encountered an error");

        println!("child status was: {}", status);
    });

    let mut stdout = io::stdout();
    while let length = reader.read(&mut buffer).await? {
        if length == 0 { break } // EOF
        let mut cursor = 0;
        while cursor < length {
            cursor += stdout.write(&buffer[cursor .. length]).await?;
        }
        stdout.flush().await?;
    }

    Ok(())
}
