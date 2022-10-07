#[macro_use]
extern crate pest_derive;

use std::sync::mpsc::channel;

use bed_v2::TestBed;
use parser_v2::parse_test_bed;
use program::{ProgramState, Shutdown};
// mod bed;
// mod parser;
// mod template;
pub mod bed_v2;
mod parser_v2;
mod program;

// use parser::parse_test_bed;

fn main() {
    let mut args = std::env::args();
    args.next();

    let commands = args.next().unwrap();
    let mut parsed = parse_test_bed(commands);

    let template_programs = parsed.template_program();
    let command_program = parsed.commands_program();
    let globals_program = parsed.globals;
    let mut test_bed = TestBed::new(parsed.output, parsed.includes, parsed.names);

    let shutdown = Shutdown::new();
    let (send, recv) = channel();
    let send_clone = send.clone();
    let shutdown_clone = shutdown.clone();

    ctrlc::set_handler(move || {
        if shutdown_clone.shutdown() {
            send_clone.send(()).ok();
        }
    })
    .unwrap();

    std::thread::spawn(move || {
        let mut state = ProgramState::new();
        globals_program.run(&mut test_bed, &mut state, &shutdown);
        for program in template_programs {
            program.run(&mut test_bed, &mut state, &shutdown);
        }
        command_program.run(&mut test_bed, &mut state, &shutdown);
        send.send(()).ok();
    });

    recv.recv().unwrap();
}
