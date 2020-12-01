use std::{fs::File, io::BufReader, collections::HashMap};
use commands::{TestBed, TestCommand};
use serde::Deserialize;

mod commands;

#[derive(Clone, Deserialize)]
struct Config {
    commands: Vec<TestCommand>,
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
    let mut test_bed = TestBed::new(indices, params);

    match test_bed.run_all(&commands) {
        Ok(_) => println!("Test Bed complete"),
        Err(e) => {
            println!("ERROR: Test Bed failed: {}", e);
            test_bed.shutdown();
        }
    }
}
