use nom::{
    bytes::complete::is_a,
    bytes::complete::is_not,
    bytes::complete::tag,
    character::complete,
    character::complete::digit1,
    combinator::{eof, map_res},
    sequence::delimited,
    Parser,
};
use serde::{Deserialize, Serialize};
// use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Arg {
    String(String),
    Param { index: usize, param: String },
    Pid(usize),
}

fn arg(mut input: &str) -> nom::IResult<&str, Arg> {
    input = input.trim();
    let value: nom::IResult<&str, &str> =
        delimited(complete::char('['), is_not("]"), complete::char(']'))(input);

    if let Ok((input, inner)) = value {
        eof(input)?;
        let (rem, index) = map_res(digit1, str::parse)(inner)?;
        let (rem, _) = tag("::")(rem)?;
        let (rem, param) = is_not(" ")(rem)?;
        eof(rem)?;

        return Ok((
            rem,
            Arg::Param {
                index,
                param: param.to_string(),
            },
        ));
    }

    let value: nom::IResult<&str, &str> =
        delimited(tag("pid("), is_not(")"), complete::char(')'))(input);

    if let Ok((input, inner)) = value {
        eof(input)?;
        let (rem, index) = map_res(digit1, str::parse)(inner)?;
        eof(rem)?;

        return Ok((
            rem,
            Arg::Pid(index)
        ));
    }

    let (rem, arg) = is_not(" ")(input)?;
    Ok((rem, Arg::String(arg.to_string())))
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum OutputMap {
    Print,
    Create(Arg),
    Append(Arg),
}

impl Default for OutputMap {
    fn default() -> Self {
        Self::Print
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum TestCommand {
    Kill(usize),
    Spawn {
        id: usize,
        command: String,
        args: Vec<Arg>,
        #[serde(default)]
        stdout: OutputMap,
        #[serde(default)]
        stderr: OutputMap,
    },
    Sleep(u64),
    WaitFor {
        id: usize,
        timeout: Option<(u64, u64)>,
    },
}

fn kill(i: &str) -> nom::IResult<&str, TestCommand> {
    let i = i.trim();

    let (i, _) = tag("kill").or(tag("KILL")).parse(i)?;
    let (i, _) = is_a(" ")(i)?;
    let (i, id) = map_res(digit1, str::parse)(i)?;

    eof(i)?;

    Ok((i, TestCommand::Kill(id)))
}

fn sleep(i: &str) -> nom::IResult<&str, TestCommand> {
    let i = i.trim();

    let (i, _) = tag("sleep").or(tag("SLEEP")).parse(i)?;
    let (i, _) = is_a(" ")(i)?;
    let (i, ms) = map_res(digit1, str::parse)(i)?;
    eof(i)?;

    Ok((i, TestCommand::Sleep(ms)))
}

fn wait_duration(i: &str) -> nom::IResult<&str, (u64, u64)> {
    let (i, _) = is_a(" ")(i)?;
    let (i, ms) = map_res(digit1, str::parse)(i)?;
    let (i, _) = is_a(" ")(i)?;
    let (i, sleeps) = map_res(digit1, str::parse)(i)?;

    Ok((i, (ms, sleeps)))
}

fn wait_for(i: &str) -> nom::IResult<&str, TestCommand> {
    let i = i.trim();

    let (i, _) = tag("wait").or(tag("WAIT")).parse(i)?;
    let (i, _) = is_a(" ")(i)?;
    let (mut i, id) = map_res(digit1, str::parse)(i)?;

    let mut timeout = None;

    if let Ok((rem, wait)) = wait_duration(i) {
        i = rem;
        timeout = Some(wait);
    }

    eof(i)?;

    Ok((i, TestCommand::WaitFor { id, timeout }))
}

fn output_map(i: &str) -> nom::IResult<&str, OutputMap> {
    let print: nom::IResult<&str, &str> = tag("print")(i);

    if let Ok((i, _)) = print {
        return Ok((i, OutputMap::Print));
    }

    let append: nom::IResult<&str, &str> =
        delimited(tag("append("), is_not(")"), complete::char(')'))(i);

    if let Ok((i, inner)) = append {
        return Ok((i, OutputMap::Append(arg(inner)?.1)));
    }

    let (i, file) = is_not(" ")(i)?;
    Ok((i, OutputMap::Create(arg(file)?.1)))
}

fn stdout(i: &str) -> nom::IResult<&str, OutputMap> {
    let (i, _) = tag("--stdout=")(i)?;

    output_map(i)
}

fn stderr(i: &str) -> nom::IResult<&str, OutputMap> {
    let (i, _) = tag("--stderr=")(i)?;

    output_map(i)
}

fn spawn(i: &str) -> nom::IResult<&str, TestCommand> {
    let i = i.trim();

    let (i, _) = tag("spawn").or(tag("SPAWN")).parse(i)?;
    let (i, _) = is_a(" ")(i)?;
    let (mut i, id) = map_res(digit1, str::parse)(i)?;
    let mut out = OutputMap::Print;
    let mut err = OutputMap::Print;

    loop {
        let (rem, _) = is_a(" ")(i)?;
        if let Ok((rem, output)) = stdout(rem) {
            out = output;
            i = rem;
            continue;
        }

        if let Ok((rem, output)) = stderr(rem) {
            err = output;
            i = rem;
            continue;
        }

        i = rem;
        break;
    }

    let (i, command) = is_not(" ")(i)?;
    let split = i.split_whitespace();
    let mut args = vec![];

    for value in split {
        let arg = arg(value)?;
        args.push(arg.1);
    }

    Ok((
        "",
        TestCommand::Spawn {
            id,
            command: command.into(),
            args,
            stdout: out,
            stderr: err,
        },
    ))
}

pub fn test_command(i: &str) -> nom::IResult<&str, TestCommand> {
    if let Ok(res) = kill(i) {
        return Ok(res);
    }

    if let Ok(res) = sleep(i) {
        return Ok(res);
    }

    if let Ok(res) = wait_for(i) {
        return Ok(res);
    }

    spawn(i)
}

#[cfg(test)]
mod parse_test {
    use super::{arg, test_command, Arg, OutputMap, TestCommand};

    #[test]
    fn arg_parse() {
        let a = "wow_hello";
        let b = "[1::amazing]";
        let c = "  [1::amazing] ";
        let d = "  pid(0) ";

        let res = arg(a);
        println!("{:?}", res);
        assert!(res == Ok(("", Arg::String("wow_hello".to_string()))));

        let res = arg(b);
        println!("{:?}", res);
        assert!(
            res == Ok((
                "",
                Arg::Param {
                    index: 1,
                    param: "amazing".to_string()
                }
            ))
        );

        let res = arg(c);
        println!("{:?}", res);
        assert!(
            res == Ok((
                "",
                Arg::Param {
                    index: 1,
                    param: "amazing".to_string()
                }
            ))
        );

        let res = arg(d);
        println!("{:?}", res);
        assert!(
            res == Ok((
                "",
                Arg::Pid(0)
            ))
        );
    }

    #[test]
    fn command_test() {
        let spawn = "spawn 1 --stdout=append(idontknow.txt) --stderr=something ./something -f hello amazing [1::another]";
        let res = test_command(spawn);
        println!("{:?}", res);

        assert!(
            res == Ok((
                "",
                TestCommand::Spawn {
                    id: 1,
                    command: "./something".into(),
                    args: vec![
                        Arg::String("-f".into()),
                        Arg::String("hello".into()),
                        Arg::String("amazing".into()),
                        Arg::Param {
                            index: 1,
                            param: "another".into()
                        }
                    ],
                    stdout: OutputMap::Append(Arg::String("idontknow.txt".into())),
                    stderr: OutputMap::Create(Arg::String("something".into()))
                }
            ))
        );

        let spawn = "SPAWN 12 --stdout=append([1::files]) --stderr=append(something) ./something -f hello amazing [1::another] [200::whoknows]";
        let res = test_command(spawn);
        println!("{:?}", res);

        assert!(
            res == Ok((
                "",
                TestCommand::Spawn {
                    id: 12,
                    command: "./something".into(),
                    args: vec![
                        Arg::String("-f".into()),
                        Arg::String("hello".into()),
                        Arg::String("amazing".into()),
                        Arg::Param {
                            index: 1,
                            param: "another".into()
                        },
                        Arg::Param {
                            index: 200,
                            param: "whoknows".into()
                        },
                    ],
                    stdout: OutputMap::Append(Arg::Param {
                        index: 1,
                        param: "files".into()
                    }),
                    stderr: OutputMap::Append(Arg::String("something".into()))
                }
            ))
        );

        let kill = "kill 12";
        let res = test_command(kill);
        println!("{:?}", res);

        assert!(res == Ok(("", TestCommand::Kill(12))));

        let sleep = "sleep 12";
        let res = test_command(sleep);
        println!("{:?}", res);

        assert!(res == Ok(("", TestCommand::Sleep(12))));

        let wait = "wait 12 10000 100";
        let res = test_command(wait);
        println!("{:?}", res);

        assert!(
            res == Ok((
                "",
                TestCommand::WaitFor {
                    id: 12,
                    timeout: Some((10000, 100))
                }
            ))
        );

        let wait = "wait 12";
        let res = test_command(wait);
        println!("{:?}", res);

        assert!(
            res == Ok((
                "",
                TestCommand::WaitFor {
                    id: 12,
                    timeout: None
                }
            ))
        );
    }
}
