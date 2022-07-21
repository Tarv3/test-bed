#[macro_use]
extern crate pest_derive;

use std::{fs, sync::mpsc::channel};
mod bed;
mod parser;

use parser::parse_test_bed;

fn main() {
    let mut args = std::env::args();
    args.next();

    let commands = args.next().unwrap();
    let file = fs::read_to_string(commands).unwrap();
    let test_bed = parse_test_bed(&file);
    let shutdown = test_bed.shutdown_signal.clone();
    let (send, recv) = channel();
    let send_clone = send.clone();

    ctrlc::set_handler(move || {
        if shutdown.swap(true, std::sync::atomic::Ordering::Relaxed) {
            send_clone.send(()).ok();
        }
    })
    .unwrap();

    std::thread::spawn(move || {
        test_bed.run();
        send.send(()).ok();
    });

    recv.recv().unwrap();
}
