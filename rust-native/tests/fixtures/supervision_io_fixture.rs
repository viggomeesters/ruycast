//! Deterministic script I/O for the real supervised CLI; test-tools only.
use std::io::{Read, Write};
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [mode] if mode == "success" => println!("literal Ω"),
        [mode] if mode == "exit7" => {
            println!("out");
            eprintln!("err");
            std::io::stdout().flush().unwrap();
            std::io::stderr().flush().unwrap();
            std::process::exit(7);
        }
        [mode] if mode == "signal" => {
            assert_eq!(unsafe { libc::kill(libc::getpid(), libc::SIGTERM) }, 0);
        }
        [mode, literal] if mode == "stdin-literal" => {
            let mut byte = [0];
            let read = std::io::stdin().read(&mut byte).unwrap();
            print!("{}|{literal}", if read == 0 { 1 } else { 0 });
        }
        [mode] if mode == "invalid-bytes" => {
            std::io::stdout().write_all(&[255; 100000]).unwrap();
            std::io::stderr().write_all(&[0; 100000]).unwrap();
        }
        _ => panic!("Unknown owned supervision I/O fixture"),
    }
}
