#[macro_use]
extern crate pest_derive;

use std::fs;
mod bed;
mod parser;

use parser::parse_test_bed;

fn main() {
    let mut args = std::env::args();
    args.next();

    let commands = args.next().unwrap();
    let file = fs::read_to_string(commands).unwrap();
    let mut test_bed = parse_test_bed(&file);
    test_bed.run()
}
