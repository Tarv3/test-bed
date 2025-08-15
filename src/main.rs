#[macro_use]
extern crate pest_derive;

use std::{collections::HashMap, sync::mpsc::channel};

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
    let mut debug = false;

    while let Some(value) = args.next() {
        match value.as_str() {
            "--all" => {
                run_all = true;
                break;
            }
            "--debug" => {
                debug = true;
                continue;
            }
            "." => {
                commands.push(None);
                continue;
            }
            "--" => {
                break;
            }
            x => {
                let id = parsed.names.replace(x);
                commands.push(Some(id));
            }
        }
    }

    let mut params = HashMap::new();

    while let Some(value) = args.next() {
        let mut split = value.split("=");
        let variable = split.next().unwrap();
        let id = match variable.split_once(".") {
            Some((id, property)) => (
                parsed.names.replace(id),
                Some(parsed.names.replace(property)),
            ),
            None => (parsed.names.replace(variable), None),
        };

        let value = match split.next() {
            Some(value) => value,
            None => {
                panic!("Invalid input arg `{value}`, expected <variable>=<value>")
            }
        };

        params.insert(id, program::Object::new(value.to_string()));
    }

    let command_programs = match commands.is_empty() && !run_all {
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
        state.new_scope();

        for ((id, property), value) in params.iter() {
            state.set_var(*id, *property, value.clone()).unwrap();
        }

        globals_program
            .run(&mut test_bed, &mut state, &shutdown)
            .unwrap();
        for (name, program) in template_programs {
            test_bed
                .multibar
                .println(format!("Building `{name}` Template"))
                .ok();

            if debug {
                println!("{program}");
            }
            state.new_scope();
            program.run(&mut test_bed, &mut state, &shutdown).unwrap();
            state.pop_scope();
        }

        for (name, program) in command_programs {
            match name {
                Some(name) => test_bed
                    .multibar
                    .println(format!("Running `{name}` Program"))
                    .ok(),
                None => test_bed
                    .multibar
                    .println(format!("Running Default Program"))
                    .ok(),
            };

            if debug {
                println!("{program}");
            }

            state.new_scope();
            if let Err((line, error)) = program.run(&mut test_bed, &mut state, &shutdown) {
                test_bed.multibar.println(format!(
                    "Error on line {line}: {}",
                    error.display(&test_bed.var_names)
                )).unwrap();
            }
            state.pop_scope();
            test_bed.reset(&shutdown);
        }

        send.send(()).ok();
    });

    recv.recv().unwrap();
}
