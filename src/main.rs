#[macro_use]
extern crate pest_derive;

use std::sync::mpsc::channel;

mod bed;
mod parser;
mod program;

use bed::TestBed;
use parser::parse_test_bed;
use program::{ProgramState, Shutdown, VarNameId};

#[derive(Clone, Debug)]
pub enum ToRun {
    Specific(Vec<Option<VarNameId>>),
    All,
}

fn main() {
    let mut args = std::env::args();
    args.next();

    let commands = args.next().unwrap();
    let mut parsed = parse_test_bed(commands);
    let mut commands = vec![];
    let mut run_all = false;

    while let Some(value) = args.next() {
        if &value == "--all" {
            run_all = true;
            break;
        } else if &value == "." {
            commands.push(None);
            continue;
        }

        let name = parsed.names.replace(&value);
        commands.push(Some(name));
    }

    let command_programs = match commands.is_empty() {
        true => match parsed.commands_program(None) {
            Some(command) => vec![command],
            None => panic!("No default command to run"),
        },
        false => match run_all {
            true => parsed.all_programs(),
            false => {
                let mut programs = vec![];

                for value in commands {
                    match parsed.commands_program(value) {
                        Some(program) => programs.push(program),
                        None => {
                            let name = value.map(|value| parsed.names.evaluate(value)).flatten();
                            panic!("Missing program: {:?}", name);
                        }
                    }
                }

                programs
            }
        },
    };

    let template_programs = parsed.template_program();
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

        for program in command_programs {
            program.run(&mut test_bed, &mut state, &shutdown);
        }

        send.send(()).ok();
    });

    recv.recv().unwrap();
}
