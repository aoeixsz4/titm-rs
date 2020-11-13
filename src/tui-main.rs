extern crate termion;
extern crate tokio;
extern crate tokio_pty_process;
extern crate tui;
use std::process::Command;
use std::io::{Write, Stdout};
use termion::raw::IntoRawMode;
use tokio::io::{self, AsyncRead, AsyncReadExt};
use tokio::task;
//use tokio::prelude::*;
use tokio_pty_process::{AsyncPtyMaster,AsyncPtyMasterReadHalf,AsyncPtyMasterWriteHalf,Child,CommandExt};
use tui::Terminal;
use tui::backend::TermionBackend;

#[tokio::main]
async fn main() -> Result<(), io::Error> {
    let stdout = std::io::stdout().into_raw_mode()?;
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    
    // need to fork an ssh process whose stdin and stdout handles are pipes we control
    // pty crate can do this but isn't async, tokio has an async lib for this
    let pty_handler = AsyncPtyMaster::open()?;
    let mut child_handler = Command::new("ls")
        .arg("-l")
        .arg("-a")
        .spawn_pty_async_raw(&pty_handler)?;
    let (reader, writer) = pty_handler.split();

    //let read_buffer = BufReader::new(read);
    let mut buffer: [u8; 4096];
    loop {
        let mut length = reader.read(&buffer).await?;
        let mut index = 0;
        loop {
            let n_wrote = stdout.write(&buffer[index..(index+length)])?;
            length -= n_wrote;
            index += n_wrote;
            if length == 0 { break }
        }
    }

    Ok(())
}
