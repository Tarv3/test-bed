use std::{
    collections::HashMap,
    error::Error,
    fs::File,
    fs::OpenOptions,
    io,
    io::BufWriter,
    io::Read,
    io::Write,
    path::Path,
    process::Child,
    process::Command,
    process::ExitStatus,
    process::Stdio,
    sync::mpsc::{Receiver, TryRecvError},
    time::Duration,
};

use crate::parser::*;

#[derive(Debug, Clone)]
pub enum Instruction {
    BeginFor { id: String, param: String },
    NextLoop,
    Command(TestCommand),
}

#[derive(Copy, Clone)]
pub struct TimeoutLoop {
    duration: u64,
    sleep: u64,
}

impl TimeoutLoop {
    // Creates a new timeoutloop that will sleep at most 'n' times
    pub fn from_sleep_times(duration: u64, n: u64) -> Self {
        assert!(n > 0);

        let mut sleep = duration / n;

        if duration % n != 0 {
            sleep += 1;
        }

        Self { duration, sleep }
    }

    pub fn wait_loop(&self, mut f: impl FnMut() -> bool) {
        assert!(self.sleep > 0);

        let duration = Duration::from_millis(self.sleep);
        let mut current = 0;

        while current < self.duration {
            // NOTE: If f takes a significant amount of time this will not be remotely accurate
            if f() {
                break;
            }

            current += self.sleep;
            std::thread::sleep(duration);
        }
    }
}

pub enum ProcessStopped {
    Exited(ExitStatus),
    Killed,
}

pub struct Running {
    cmd: String,
    pid_arg: String,
    process: Option<Child>,
    stdout: OutputMap,
    stderr: OutputMap,
}

impl Running {
    pub fn new(
        cmd: &str,
        args: &[String],
        stdout: OutputMap,
        stderr: OutputMap,
    ) -> io::Result<Self> {
        let process =
            Command::new(cmd).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;

        let pid = format!("{}", process.id());

        Ok(Self { cmd: cmd.to_string(), process: Some(process), pid_arg: pid, stdout, stderr })
    }

    pub fn kill(&mut self) -> io::Result<()> {
        if let Some(mut value) = self.process.take() {
            value.kill()?;
        }

        Ok(())
    }

    pub fn handle_output<'a, 'b: 'a>(
        &'b self,
        stdout: &[u8],
        stderr: &[u8],
        stack: &Stack,
        params: &'a HashMap<String, Vec<String>>,
    ) -> Result<(), Box<dyn Error>> {
        let print_out = |data: &[u8]| -> io::Result<()> {
            if data.len() == 0 {
                return Ok(());
            }

            println!("-----------{} StdOut-----------", self.cmd);
            std::io::stderr().write_all(data)?;
            println!("----------------------------");

            Ok(())
        };

        let print_err = |data: &[u8]| -> io::Result<()> {
            if data.len() == 0 {
                return Ok(());
            }

            println!("-----------{} StdErr-----------", self.cmd);
            std::io::stderr().write_all(data)?;
            println!("----------------------------");

            Ok(())
        };

        let get_file = |arg: &'b Arg| -> String {
            match arg {
                Arg::String(value) => value.clone(),
                Arg::Param { index, param, prefix, suffix } => {
                    let param = params.get(param).unwrap();
                    let idx = stack.get_idx(index).unwrap();
                    let param_value = &param[idx];

                    format!("{}{}{}", prefix, param_value, suffix)
                }
                Arg::Pid(_) => panic!("Tried to use PID as file"),
            }
        };

        let create_parent = |path: &str| -> io::Result<()> {
            let path: &Path = path.as_ref();

            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            Ok(())
        };

        match &self.stdout {
            OutputMap::Print => print_out(stdout)?,
            OutputMap::Create(file) => {
                let file = get_file(file);
                create_parent(&file)?;
                let file = File::create(file)?;
                let mut writer = BufWriter::new(file);

                writer.write_all(stdout)?;
                writer.flush()?;
            }
            OutputMap::Append(file) => {
                let file = get_file(file);
                create_parent(&file)?;
                let file = OpenOptions::new().append(true).create(true).open(file)?;
                let mut writer = BufWriter::new(file);

                writer.write_all(stdout)?;
                writer.flush()?;
            }
        }

        match &self.stderr {
            OutputMap::Print => print_err(stderr)?,
            OutputMap::Create(file) => {
                let file = get_file(file);
                create_parent(&file)?;
                let file = File::create(file)?;
                let mut writer = BufWriter::new(file);

                writer.write_all(stderr)?;
                writer.flush()?;
            }
            OutputMap::Append(file) => {
                let file = get_file(file);
                create_parent(&file)?;
                let file = OpenOptions::new().append(true).create(true).open(file)?;
                let mut writer = BufWriter::new(file);

                writer.write_all(stderr)?;
                writer.flush()?;
            }
        }

        Ok(())
    }

    pub fn wait_or_terminate(
        mut self,
        timeout: Option<TimeoutLoop>,
        stack: &Stack,
        params: &HashMap<String, Vec<String>>,
        recv: &Receiver<()>,
    ) -> Result<ProcessStopped, Box<dyn Error>> {
        let mut process = match self.process.take() {
            Some(process) => process,
            None => panic!("Tried to take missing child process"),
        };

        let timeout = match timeout {
            Some(timeout) => timeout,
            None => TimeoutLoop { duration: u64::MAX, sleep: 1000 },
        };

        let mut exit_status = None;
        let mut error = None;

        timeout.wait_loop(|| {
            match recv.try_recv() {
                Ok(_) | Err(TryRecvError::Disconnected) => {
                    error = Some(Box::new(TryRecvError::Disconnected) as Box<dyn Error>);
                    return true;
                }
                _ => {}
            }

            match process.try_wait() {
                Ok(Some(status)) => {
                    exit_status = Some(status);
                    true
                }
                Ok(_) => false,
                Err(e) => {
                    error = Some(Box::new(e));
                    true
                }
            }
        });

        if let Some(err) = error {
            return Err(err);
        }

        if exit_status.is_none() {
            process.kill()?;
            return Ok(ProcessStopped::Killed);
        }

        let mut stdout = vec![];
        let mut stderr = vec![];

        if let Some(mut out) = process.stdout.take() {
            out.read_to_end(&mut stdout)?;
        }

        if let Some(mut err) = process.stderr.take() {
            err.read_to_end(&mut stderr)?;
        }

        self.handle_output(&stdout, &stderr, stack, params)?;

        Ok(ProcessStopped::Exited(exit_status.unwrap()))
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub struct LoopIdx {
    id: String,
    idx: usize,
}

pub struct LoopPoint {
    instruction_idx: usize,
    count: usize,
}

#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub struct ProcessId {
    loop_idx: Stack,
    id: usize,
}

#[derive(Clone, PartialEq, Eq, Debug, Hash, Default)]
pub struct Stack(pub Vec<LoopIdx>);

impl Stack {
    pub fn get_idx(&self, loop_id: &str) -> Option<usize> {
        for value in self.0.iter().rev() {
            if value.id == loop_id {
                return Some(value.idx);
            }
        }

        None
    }
}

pub struct TestBed {
    map: HashMap<ProcessId, Running>,
    params: HashMap<String, Vec<String>>,
    loop_points: Vec<LoopPoint>,
    stack: Stack,
    instructions: Vec<Instruction>,
    instruction_idx: usize,
    shutdown_signal: Receiver<()>,
}

impl Drop for TestBed {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl TestBed {
    pub fn new(
        params: HashMap<String, Vec<String>>,
        instructions: Vec<Instruction>,
        shutdown_signal: Receiver<()>,
    ) -> Self {
        Self {
            map: HashMap::new(),
            params,
            stack: Stack::default(),
            loop_points: vec![],
            instructions,
            instruction_idx: 0,
            shutdown_signal,
        }
    }

    pub fn shutdown(&mut self) {
        for (id, mut proc) in self.map.drain() {
            match proc.kill() {
                Ok(_) => println!("Killed: {:?}", id),
                Err(e) => println!("Failed to Kill {:?}: {}", id, e),
            }
        }
    }

    pub fn kill(&mut self, id: ProcessId) -> io::Result<()> {
        println!("Killing {:?}", id);

        match self.map.remove(&id) {
            Some(mut value) => value.kill(),
            None => Ok(()),
        }
    }

    pub fn get_id(&self, id: usize) -> ProcessId {
        ProcessId { loop_idx: self.stack.clone(), id }
    }

    pub fn spawn(
        &mut self,
        id: usize,
        cmd: &str,
        args: &[Arg],
        stdout: OutputMap,
        stderr: OutputMap,
    ) -> io::Result<()> {
        let id = self.get_id(id);
        println!("Spawning {:?}", id);

        let args = args
            .iter()
            .map(|arg| match arg {
                Arg::String(value) => value.clone(),
                Arg::Param { index, param, prefix, suffix } => {
                    let param = self.params.get(param).unwrap();
                    let idx = self.stack.get_idx(index).unwrap();
                    let param_value = &param[idx];

                    format!("{}{}{}", prefix, param_value, suffix)
                }
                Arg::Pid(id) => {
                    let id = ProcessId { loop_idx: self.stack.clone(), id: *id };
                    match self.map.get(&id) {
                        Some(value) => value.pid_arg.clone(),
                        None => "_".into(),
                    }
                }
            })
            .collect::<Vec<_>>();
        let running = Running::new(cmd, &args, stdout, stderr)?;
        let previous = self.map.insert(id.clone(), running);

        if let Some(mut proc) = previous {
            println!("WARN: Process {:?} overwritten", id);

            proc.kill()?;
        }

        Ok(())
    }

    pub fn wait_or_terminate(
        &mut self,
        id: usize,
        timeout: Option<(u64, u64)>,
    ) -> Result<(), Box<dyn Error>> {
        let id = self.get_id(id);
        println!("Waiting for: {:?}", id);

        let proc = match self.map.remove(&id) {
            Some(proc) => proc,
            None => return Ok(()),
        };

        let timeout = timeout
            .map(|(duration, sleep_times)| TimeoutLoop::from_sleep_times(duration, sleep_times));

        match proc.wait_or_terminate(timeout, &id.loop_idx, &self.params, &self.shutdown_signal)? {
            ProcessStopped::Exited(status) => {
                println!("Process {:?}, Exit Success: {}", id, status.success())
            }
            ProcessStopped::Killed => println!("WARN: Process {:?} Kill due to wait timeout", id),
        }

        Ok(())
    }

    pub fn wait_all(&mut self, timeout: Option<(u64, u64)>) -> Result<(), Box<dyn Error>> {
        let timeout = timeout
            .map(|(duration, sleep_times)| TimeoutLoop::from_sleep_times(duration, sleep_times));

        for (id, proc) in self.map.drain() {
            match proc.wait_or_terminate(
                timeout,
                &id.loop_idx,
                &self.params,
                &self.shutdown_signal,
            )? {
                ProcessStopped::Exited(status) => {
                    println!("Process {:?}, Exit Success: {}", id, status.success())
                }
                ProcessStopped::Killed => {
                    println!("WARN: Process {:?} Kill due to wait timeout", id)
                }
            }
        }

        Ok(())
    }

    fn sleep(ms: u64) {
        println!("Sleeping: {}", ms);
        std::thread::sleep(Duration::from_millis(ms));
    }

    pub fn run_command(&mut self, command: &TestCommand) -> Result<(), Box<dyn Error>> {
        match command {
            TestCommand::Kill(id) => {
                let id = self.get_id(*id);
                self.kill(id)?;
            }
            TestCommand::Spawn { id, command, args, stdout, stderr } => {
                self.spawn(*id, &command, &args[..], stdout.clone(), stderr.clone())?;
            }
            TestCommand::Sleep(ms) => TestBed::sleep(*ms),
            TestCommand::WaitFor { id, timeout } => self.wait_or_terminate(*id, *timeout)?,
            TestCommand::WaitAll(timeout) => self.wait_all(*timeout)?,
        }

        Ok(())
    }

    pub fn next(&mut self) -> Result<bool, Box<dyn Error>> {
        if self.instruction_idx >= self.instructions.len() {
            return Ok(false);
        }

        let next_instruction = self.instructions[self.instruction_idx].clone();

        match next_instruction {
            Instruction::BeginFor { id, param } => {
                let count = match self.params.get(&param) {
                    Some(value) => value.len(),
                    None => panic!("No param named {}", param),
                };

                self.instruction_idx += 1;
                let point = LoopPoint { instruction_idx: self.instruction_idx, count };
                let idx = LoopIdx { id: id.clone(), idx: 0 };
                self.loop_points.push(point);
                self.stack.0.push(idx);
            }
            Instruction::NextLoop => {
                let point = self.loop_points.pop();
                let idx = self.stack.0.pop();

                if point.is_none() || idx.is_none() {
                    return Ok(false);
                }

                let point = point.unwrap();
                let mut idx = idx.unwrap();

                idx.idx += 1;

                if idx.idx < point.count {
                    self.instruction_idx = point.instruction_idx;
                    self.loop_points.push(point);
                    self.stack.0.push(idx);
                }
                else {
                    self.instruction_idx += 1;
                }
            }
            Instruction::Command(command) => {
                self.instruction_idx += 1;
                self.run_command(&command)?;
            }
        }

        Ok(true)
    }

    pub fn run(&mut self) {
        loop {
            match self.shutdown_signal.try_recv() {
                Ok(_) | Err(TryRecvError::Disconnected) => break,
                _ => {}
            }

            match self.next() {
                Ok(false) => break,
                Err(e) => {
                    println!("Error {e}");
                    break;
                }
                _ => {}
            }
        }

        self.shutdown();
    }
}
