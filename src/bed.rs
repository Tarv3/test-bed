use std::{
    collections::HashMap, error::Error, fs::File, fs::OpenOptions, io, io::BufWriter, io::Read,
    io::Write, path::Path, process::Child, process::Command, process::ExitStatus, process::Stdio,
    time::Duration,
};

use crate::commands::*;

#[derive(Copy, Clone)]
pub struct TimeoutLoop {
    duration: u64,
    sleep: u64,
}

impl TimeoutLoop {
    pub fn new(duration: u64, sleep: u64) -> Self {
        Self { duration, sleep }
    }

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
    pub fn new(cmd: &str, args: &[&str], stdout: OutputMap, stderr: OutputMap) -> io::Result<Self> {
        let process = Command::new(cmd)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let pid = format!("{}", process.id());

        Ok(Self {
            cmd: cmd.to_string(),
            process: Some(process),
            pid_arg: pid,
            stdout,
            stderr,
        })
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
        indices: &'a [usize],
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

        let get_file = |arg: &'b Arg| -> Option<&'a str> {
            match arg {
                Arg::String(value) => Some(value.as_str()),
                Arg::Param { index, param } => indices
                    .get(*index)
                    .map(|idx| params.get(param).map(|value| value[*idx].as_str()))
                    .flatten(),
                Arg::Pid(_) => panic!("Tried to use PID as file")
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
                let file = get_file(file).unwrap();
                create_parent(file)?;
                let file = File::create(file)?;
                let mut writer = BufWriter::new(file);

                writer.write_all(stdout)?;
                writer.flush()?;
            }
            OutputMap::Append(file) => {
                let file = get_file(file).unwrap();
                create_parent(file)?;
                let file = OpenOptions::new().append(true).create(true).open(file)?;
                let mut writer = BufWriter::new(file);

                writer.write_all(stdout)?;
                writer.flush()?;
            }
        }

        match &self.stderr {
            OutputMap::Print => print_err(stderr)?,
            OutputMap::Create(file) => {
                let file = get_file(file).unwrap();
                create_parent(file)?;
                let file = File::create(file)?;
                let mut writer = BufWriter::new(file);

                writer.write_all(stderr)?;
                writer.flush()?;
            }
            OutputMap::Append(file) => {
                let file = get_file(file).unwrap();
                create_parent(file)?;
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
        indices: &[usize],
        params: &HashMap<String, Vec<String>>,
    ) -> Result<ProcessStopped, Box<dyn Error>> {
        let mut process = match self.process.take() {
            Some(process) => process,
            None => panic!("Tried to take missing child process"),
        };

        let timeout = match timeout {
            Some(timeout) => timeout,
            None => {
                let output = process.wait_with_output()?;
                self.handle_output(&output.stdout, &output.stderr, indices, params)?;
                return Ok(ProcessStopped::Exited(output.status));
            }
        };

        let mut exit_status = None;
        let mut error = None;

        timeout.wait_loop(|| match process.try_wait() {
            Ok(Some(status)) => {
                exit_status = Some(status);
                true
            }
            Ok(_) => false,
            Err(e) => {
                error = Some(e);
                true
            }
        });

        if let Some(err) = error {
            return Err(Box::new(err));
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

        self.handle_output(&stdout, &stderr, indices, params)?;

        Ok(ProcessStopped::Exited(exit_status.unwrap()))
    }
}

pub struct TestBed {
    map: HashMap<usize, Running>,
    current_indices: Vec<usize>,
    max_indices: Vec<usize>,
    params: HashMap<String, Vec<String>>,
}

impl TestBed {
    pub fn new(max_indices: Vec<usize>, params: HashMap<String, Vec<String>>) -> Self {
        let current_indices = max_indices.iter().map(|_| 0).collect::<Vec<_>>();

        Self {
            map: HashMap::new(),
            current_indices,
            max_indices,
            params,
        }
    }

    pub fn shutdown(&mut self) {
        for (id, mut proc) in self.map.drain() {
            match proc.kill() {
                Ok(_) => println!("Killed: {}", id),
                Err(e) => println!("Failed to Kill {}: {}", id, e),
            }
        }
    }

    pub fn kill(&mut self, id: usize) -> io::Result<()> {
        println!("Killing {}", id);

        match self.map.remove(&id) {
            Some(mut value) => value.kill(),
            None => Ok(()),
        }
    }

    pub fn spawn(
        &mut self,
        id: usize,
        cmd: &str,
        args: &[Arg],
        stdout: OutputMap,
        stderr: OutputMap,
    ) -> io::Result<()> {
        println!("Spawning {}", id);

        let args = args
            .iter()
            .map(|arg| match arg {
                Arg::String(value) => value.as_str(),
                Arg::Param { index, param } => {
                    let current = self.current_indices.get(*index).cloned().unwrap_or(0);
                    self.params
                        .get(param)
                        .map(|values| values.get(current))
                        .flatten()
                        .map(|value| value.as_str())
                        .unwrap_or("")
                },
                Arg::Pid(id) => match self.map.get(id) {
                    Some(value) => value.pid_arg.as_str(),
                    None => "_".into()
                }
            })
            .collect::<Vec<_>>();
        let running = Running::new(cmd, &args, stdout, stderr)?;

        let previous = self.map.insert(id, running);

        if let Some(mut proc) = previous {
            println!("WARN: Process {} overwritten", id);

            proc.kill()?;
        }

        Ok(())
    }

    pub fn wait_or_terminate(
        &mut self,
        id: usize,
        timeout: Option<(u64, u64)>,
    ) -> Result<(), Box<dyn Error>> {
        println!("Waiting for: {}", id);

        let proc = match self.map.remove(&id) {
            Some(proc) => proc,
            None => return Ok(()),
        };

        let timeout = timeout
            .map(|(duration, sleep_times)| TimeoutLoop::from_sleep_times(duration, sleep_times));

        match proc.wait_or_terminate(timeout, &self.current_indices, &self.params)? {
            ProcessStopped::Exited(status) => {
                println!("Process {}, Exit Success: {}", id, status.success())
            }
            ProcessStopped::Killed => println!("WARN: Process {} Kill due to wait timeout", id),
        }

        Ok(())
    }

    fn sleep(ms: u64) {
        println!("Sleeping: {}", ms);
        std::thread::sleep(Duration::from_millis(ms));
    }

    pub fn run_commands(&mut self, commands: &[TestCommand]) -> Result<(), Box<dyn Error>> {
        for command in commands.iter() {
            match command {
                TestCommand::Kill(id) => self.kill(*id)?,
                TestCommand::Spawn {
                    id,
                    command,
                    args,
                    stdout,
                    stderr,
                } => {
                    self.spawn(*id, &command, &args[..], stdout.clone(), stderr.clone())?;
                }
                TestCommand::Sleep(ms) => TestBed::sleep(*ms),
                TestCommand::WaitFor { id, timeout } => self.wait_or_terminate(*id, *timeout)?,
            }
        }

        Ok(())
    }

    pub fn increment_indices(&mut self) -> bool {
        if self.current_indices.len() == 0 {
            return false;
        }

        let len = self.current_indices.len();

        let mut i = 0;
        let mut current = &mut self.current_indices[i];
        let mut max = self.max_indices[i];

        *current += 1;

        while *current >= max {
            i += 1;

            if i >= len {
                break;
            }

            *current = 0;
            current = &mut self.current_indices[i];
            max = self.max_indices[i];

            *current += 1;
        }

        let last = &mut self.current_indices.last().unwrap();
        let last_max = &mut self.max_indices.last().unwrap();

        last < last_max
    }

    pub fn run_all(&mut self, commands: &[TestCommand]) -> Result<(), Box<dyn Error>> {
        let mut i = 0;
        println!("------------ Test {} ------------", i);
        self.run_commands(commands)?;

        i += 1;

        while self.increment_indices() {
            println!("------------ Test {} ------------", i);
            i += 1;
            self.run_commands(commands)?;
            self.shutdown();
        }

        Ok(())
    }
}
