use std::{fs::File, io::BufReader, collections::HashMap};
use commands::test_command;
use bed::TestBed;
use serde::Deserialize;

mod commands;
mod bed;

#[derive(Clone, Deserialize)]
struct Config {
    commands: Vec<String>,
    indices: Vec<usize>,
    params: HashMap<String, Vec<String>>
}

fn main() {
    let mut args = std::env::args(); 
    args.next();

    let commands = args.next().unwrap();
    let file = File::open(commands).unwrap();
    let reader = BufReader::new(file);

    let Config { commands, indices, params } = serde_json::from_reader(reader).unwrap();
    let commands = commands.iter().map(|value| test_command(value).unwrap().1).collect::<Vec<_>>();
    let mut test_bed = TestBed::new(indices, params);

    match test_bed.run_all(&commands) {
        Ok(_) => println!("Test Bed complete"),
        Err(e) => {
            println!("ERROR: Test Bed failed: {}", e);
            test_bed.shutdown();
        }
    }
}
