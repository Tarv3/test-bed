use std::collections::HashMap;

use pest::{iterators::Pair, Parser};

use crate::bed::{Instruction, TestBed};

#[derive(Parser)]
#[grammar = "grammar.pest"]
pub struct TestBedParser;

pub fn parse_test_bed(file: &str) -> TestBed {
    let ast = TestBedParser::parse(Rule::test_bed, &file).unwrap();
    let mut params = HashMap::new();
    let mut instructions = vec![];

    for value in ast {
        match value.as_rule() {
            Rule::parameters => {
                params = match parse_parameters(value) {
                    Ok(params) => params,
                    Err(e) => panic!("Multiple params with name {}", e),
                }
            }
            Rule::commands => {
                parse_instructions(value, &mut instructions);
            }
            _ => {}
        }
    }

    let test_bed = TestBed::new(params, instructions);
    test_bed
}

fn parse_parameters(pair: Pair<Rule>) -> Result<HashMap<String, Vec<String>>, String> {
    let mut map = HashMap::new();
    let inner_rules = pair.into_inner();
    for parameter in inner_rules {
        let mut values = vec![];
        let mut parameter = parameter.into_inner();
        let name = parameter.next().unwrap();
        let array = parameter.next().unwrap().into_inner();

        for value in array {
            let value = value.into_inner().next().unwrap();
            values.push(value.as_str().to_string());
        }

        let name = name.as_str().to_string();

        if map.contains_key(&name) {
            return Err(name);
        }

        map.insert(name, values);
    }

    Ok(map)
}

fn parse_instructions(pair: Pair<Rule>, instructions: &mut Vec<Instruction>) {
    let mut expressions = vec![];

    let inner = pair.into_inner();

    for value in inner {
        expressions.push(parse_expression(value, instructions));
    }
}

fn parse_expression(pair: Pair<Rule>, instructions: &mut Vec<Instruction>) {
    let inner = pair.into_inner().next().unwrap();

    match inner.as_rule() {
        Rule::for_loop => {
            parse_for_loop(inner, instructions);
        }
        Rule::command => {
            parse_command(inner, instructions);
        }
        _ => {
            unreachable!("{}", inner)
        }
    }
}

fn parse_for_loop(pair: Pair<Rule>, instructions: &mut Vec<Instruction>) {
    let mut inner = pair.into_inner();
    let idx_id = inner.next().unwrap().as_str().into();
    let param_id = inner.next().unwrap().as_str().into();

    instructions.push(Instruction::BeginFor { id: idx_id, param: param_id });

    for value in inner {
        parse_expression(value, instructions);
    }

    instructions.push(Instruction::NextLoop);
}

fn parse_command(pair: Pair<Rule>, instructions: &mut Vec<Instruction>) {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::kill => instructions.push(Instruction::Command(parse_kill(inner))),
        Rule::sleep => instructions.push(Instruction::Command(parse_sleep(inner))),
        Rule::wait_for => instructions.push(Instruction::Command(parse_wait_for(inner))),
        Rule::spawn => instructions.push(Instruction::Command(parse_spawn(inner))),
        _ => unreachable!("{}", inner),
    }
}

fn parse_kill(pair: Pair<Rule>) -> TestCommand {
    let mut inner = pair.into_inner();
    let id = inner.next().unwrap().as_str().parse().unwrap();

    TestCommand::Kill(id)
}

fn parse_sleep(pair: Pair<Rule>) -> TestCommand {
    let mut inner = pair.into_inner();
    let ms = inner.next().unwrap().as_str().parse().unwrap();

    TestCommand::Sleep(ms)
}

fn parse_wait_for(pair: Pair<Rule>) -> TestCommand {
    let mut inner = pair.into_inner();
    let id = inner.next().unwrap().as_str().parse().unwrap();
    let mut timeout = None;

    if let Some(ms) = inner.next() {
        let retries = inner.next().unwrap();

        timeout = Some((ms.as_str().parse().unwrap(), retries.as_str().parse().unwrap()))
    }
    TestCommand::WaitFor { id, timeout }
}

fn parse_wait_all(pair: Pair<Rule>) -> TestCommand {
    let mut inner = pair.into_inner();
    let mut timeout = None;

    if let Some(ms) = inner.next() {
        let retries = inner.next().unwrap();

        timeout = Some((ms.as_str().parse().unwrap(), retries.as_str().parse().unwrap()))
    }
    TestCommand::WaitAll(timeout)
}

fn parse_spawn(pair: Pair<Rule>) -> TestCommand {
    let mut inner = pair.into_inner();
    let id = inner.next().unwrap().as_str().parse().unwrap();

    let mut stdout = OutputMap::default();
    let mut stderr = OutputMap::default();
    let program;
    let mut args = vec![];

    loop {
        let next = inner.next().unwrap();

        match next.as_rule() {
            Rule::stdout_map => {
                let inner = next.into_inner().next().unwrap();
                stdout = OutputMap::parse(inner);
            }
            Rule::stderr_map => {
                let inner = next.into_inner().next().unwrap();
                stderr = OutputMap::parse(inner);
            }
            Rule::program => {
                let inner = next.into_inner();
                program = inner.as_str().into();
                break;
            }
            _ => unreachable!(),
        }
    }

    for value in inner {
        let arg = Arg::parse(value);
        args.push(arg);
    }

    TestCommand::Spawn { id, command: program, args, stdout, stderr }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TestCommand {
    Kill(usize),
    Spawn { id: usize, command: String, args: Vec<Arg>, stdout: OutputMap, stderr: OutputMap },
    Sleep(u64),
    WaitFor { id: usize, timeout: Option<(u64, u64)> },
    WaitAll(Option<(u64, u64)>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    String(String),
    Param { index: String, param: String, prefix: String, suffix: String },
    Pid(usize),
}

impl Arg {
    pub fn parse(pair: Pair<Rule>) -> Self {
        let inner_rules = pair.into_inner();

        let mut prefix = None;
        let mut template = None;
        let mut suffix = None;

        for value in inner_rules {
            match value.as_rule() {
                Rule::prefix => {
                    prefix = Some(value.as_str().into());
                }
                Rule::template => {
                    let mut inner = value.into_inner();
                    let index = inner.next().unwrap().as_str().into();
                    let param = inner.next().unwrap().as_str().into();

                    template = Some((index, param));
                }
                Rule::suffix => {
                    suffix = Some(value.as_str().into());
                }
                Rule::pid => {
                    let inner = value.into_inner();
                    let id = inner.as_str().parse().unwrap();

                    return Arg::Pid(id);
                }
                _ => unreachable!(),
            }
        }

        match (prefix, template, suffix) {
            (Some(prefix), None, None) => Arg::String(prefix),
            (prefix, Some((index, param)), suffix) => Arg::Param {
                index,
                param,
                prefix: prefix.unwrap_or(String::new()),
                suffix: suffix.unwrap_or(String::new()),
            },
            _ => unreachable!(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum OutputMap {
    Print,
    Create(Arg),
    Append(Arg),
}

impl OutputMap {
    pub fn parse(pair: Pair<Rule>) -> Self {
        let inner = pair.into_inner().next().unwrap();

        match inner.as_rule() {
            Rule::append => {
                let inner = inner.into_inner().next().unwrap();
                let arg = Arg::parse(inner);

                OutputMap::Append(arg)
            }
            Rule::print => OutputMap::Print,
            Rule::arg => {
                let arg = Arg::parse(inner);
                OutputMap::Create(arg)
            }
            _ => unreachable!("{}", inner),
        }
    }
}

impl Default for OutputMap {
    fn default() -> Self {
        Self::Print
    }
}
