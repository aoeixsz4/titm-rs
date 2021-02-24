# titm-rs
term-in-the-middle is a terminal emulator and wrapper designed to interact with curses programs and the like

change the process::Command call in src/main.rs to choose program to wrap (TODO: cli args would be more user-friendly)
raw terminal output from the wrapped program is sent to your terminal *and* to stderr, so best to run it like this:
 $ cargo run 2>dump
