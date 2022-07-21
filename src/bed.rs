use std::{
    collections::HashMap,
    fs::OpenOptions,
    io,
    io::Read,
    io::{BufRead, BufReader, Write},
    io::{BufWriter, ErrorKind},
    path::Path,
    process::Child,
    process::Command,
    process::Stdio,
    str::{from_utf8, from_utf8_unchecked},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

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

fn spawn_file_writer<R: Read + Send, P>(reader: R, path: P, append: bool) -> std::io::Result<()>
where
    R: Read + Send + 'static,
    P: AsRef<Path>,
{
    let create_parent = |path: &Path| -> io::Result<()> {
        let path: &Path = path.as_ref();

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        Ok(())
    };

    let path = path.as_ref();
    create_parent(path)?;
    let file = match append {
        true => OpenOptions::new().append(append).create(true).open(path)?,
        false => OpenOptions::new().write(true).create(true).open(path)?,
    };

    let mut writer = BufWriter::new(file);
    let path = path.as_os_str().to_string_lossy().to_string();

    std::thread::spawn(move || {
        let buf = BufReader::new(reader);

        for line in buf.lines() {
            let mut line = match line {
                Ok(line) => line,
                Err(_) => break,
            };

            line.retain(|value| value != '\r');

            if let Err(e) = writer.write_all(line.as_bytes()) {
                println!("Write Failed {}: {}", path, e);
                break;
            }

            if let Err(e) = writer.write_all(&['\n' as u8]) {
                println!("Write Failed {}: {}", path, e);
                break;
            }

            writer.flush().ok();
        }
    });

    Ok(())
}

fn spawn_progress_writer<R: Read + Send>(reader: R, bar: ProgressBar)
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut buf = BufReader::new(reader);
        let pattern = ['\n', '\r'];
        let mut bytes = vec![];

        loop {
            let next_bytes = match buf.fill_buf() {
                Ok(next) => next,
                Err(e) if e.kind() == ErrorKind::Interrupted => {
                    continue;
                }
                _ => break,
            };

            if next_bytes.len() == 0 {
                continue;
            }

            bytes.extend_from_slice(next_bytes);
            let consumed = next_bytes.len();
            buf.consume(consumed);

            loop {
                let (mut remove, error, str) = match from_utf8(&bytes) {
                    Ok(str) => (bytes.len(), None, str),
                    Err(invalid) => {
                        let valid = invalid.valid_up_to();
                        // SAFETY: We know that the bytes are valid up until 'valid' so this is safe
                        let str = unsafe { from_utf8_unchecked(&bytes[..valid]) };

                        (valid, invalid.error_len(), str)
                    }
                };

                let end_of_error = error.map(|value| value + remove);

                match str.rfind(&pattern) {
                    Some(idx) => {
                        remove = idx;
                        let before_pattern = &str[..idx];
                        let line_start = before_pattern.rfind(&pattern).unwrap_or(0);
                        let mut line = before_pattern[line_start..].to_string();
                        line.retain(|val| !pattern.contains(&val));

                        if !line.is_empty() {
                            bar.set_message(line);
                            bar.inc(1);
                        }
                    }
                    // Dont remove anything, we just havent received enough bytes yet
                    None => {
                        if error.is_none() {
                            break;
                        }
                    }
                }

                let remove = end_of_error.unwrap_or(remove);

                if remove == 0 {
                    break;
                }

                let remaining = bytes.len() - remove;

                for i in 0..remaining {
                    let byte = bytes[i + remove];
                    bytes[i] = byte;
                }

                bytes.truncate(remaining);
            }
        }
    });
}

pub struct Running {
    _cmd: String,
    pid_arg: String,
    process: Option<Child>,
    progress: ProgressBar,
}

impl Running {
    pub fn new(
        progress: ProgressBar,
        cmd: &str,
        args: &[String],
        stdout: OutputMap,
        stderr: OutputMap,
        stack: &Stack,
        params: &HashMap<String, Vec<String>>,
    ) -> Option<Self> {
        let mut process = Command::new(cmd);
        process.args(args);
        process.stdout(Stdio::piped());
        process.stderr(Stdio::piped());

        let mut arg_string = String::new();

        for arg in args.iter() {
            arg_string.push_str(arg);
            arg_string.push(' ');
        }

        progress.set_prefix(cmd.to_string());
        progress.set_message(arg_string);

        let mut process = match process.spawn() {
            Ok(process) => process,
            Err(e) => {
                progress.set_message(format!("ERROR: {e}"));
                return None;
            }
        };

        let get_file = |arg: &Arg| -> Option<String> {
            match arg {
                Arg::String(value) => Some(value.clone()),
                Arg::Param {
                    index,
                    param,
                    prefix,
                    suffix,
                } => {
                    let param = params.get(param).unwrap();
                    let idx = stack.get_idx(index).unwrap();
                    let param_value = &param[idx];

                    Some(format!("{}{}{}", prefix, param_value, suffix))
                }
                Arg::Pid(_) => {
                    progress.set_message("Tried to use PID as file output");
                    return None;
                }
            }
        };

        let out = process.stdout.take().unwrap();
        match stdout {
            OutputMap::Create(arg) => {
                if let Some(file) = get_file(&arg) {
                    spawn_file_writer(out, &file, false).ok();
                }
            }
            OutputMap::Append(arg) => {
                if let Some(file) = get_file(&arg) {
                    spawn_file_writer(out, &file, true).ok();
                }
            }
            _ => {
                spawn_progress_writer(out, progress.clone());
            }
        }

        let out = process.stderr.take().unwrap();
        match stderr {
            OutputMap::Create(arg) => {
                if let Some(file) = get_file(&arg) {
                    spawn_file_writer(out, &file, false).ok();
                }
            }
            OutputMap::Append(arg) => {
                if let Some(file) = get_file(&arg) {
                    spawn_file_writer(out, &file, true).ok();
                }
            }
            _ => spawn_progress_writer(out, progress.clone()),
        }

        let pid = format!("{}", process.id());

        Some(Self {
            _cmd: cmd.to_string(),
            process: Some(process),
            pid_arg: pid,
            progress,
        })
    }

    pub fn kill(&mut self) {
        if let Some(mut process) = self.process.take() {
            match process.kill() {
                Ok(_) => self.progress.finish_with_message("Killed"),
                Err(e) => self
                    .progress
                    .finish_with_message(format!("Failed to Kill: {e}")),
            }

            self.progress.inc(1);
        }
    }

    pub fn wait_or_terminate(mut self, timeout: Option<TimeoutLoop>, shutdown: &Arc<AtomicBool>) {
        let mut process = match self.process.take() {
            Some(process) => process,
            None => panic!("Tried to take missing child process"),
        };

        let timeout = match timeout {
            Some(timeout) => timeout,
            None => TimeoutLoop {
                duration: u64::MAX,
                sleep: 1000,
            },
        };

        let mut exit_status = None;
        let mut error = None;

        timeout.wait_loop(|| {
            if shutdown.load(Ordering::Relaxed) {
                return true;
            }

            self.progress.inc(1);

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
            process.kill().ok();

            self.progress
                .finish_with_message(format!("Error while waiting: {err}"));
        }

        match exit_status {
            Some(status) => match status.success() {
                true => self.progress.finish_with_message("Success"),
                false => self
                    .progress
                    .finish_with_message(format!("Failed: {:?}", status.code())),
            },
            None => self.kill(),
        }

        self.progress.inc(1);
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

impl std::fmt::Display for ProcessId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[")?;
        let mut iter = self.loop_idx.0.iter();

        if let Some(first) = iter.next() {
            write!(f, "{}({})", first.id, first.idx)?;
        }

        for value in iter {
            write!(f, ", {}({})", value.id, value.idx)?;
        }

        write!(f, "]: {}", self.id)
    }
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
    pub shutdown_signal: Arc<AtomicBool>,

    progress: MultiProgress,
    status: ProgressBar,
}

impl Drop for TestBed {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl TestBed {
    pub fn new(params: HashMap<String, Vec<String>>, instructions: Vec<Instruction>) -> Self {
        let status = ProgressBar::new_spinner();
        status.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner} {wide_msg}")
                .unwrap(),
        );

        let progress = MultiProgress::with_draw_target(ProgressDrawTarget::stdout());
        // let progress = MultiProgress::with_draw_target(ProgressDrawTarget::hidden());
        let status = progress.add(status);
        status.enable_steady_tick(std::time::Duration::from_millis(500));

        Self {
            map: HashMap::new(),
            params,
            stack: Stack::default(),
            loop_points: vec![],
            instructions,
            instruction_idx: 0,
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            progress,
            status,
        }
    }

    pub fn shutdown(&mut self) {
        for (_, mut proc) in self.map.drain() {
            proc.kill();
        }
    }

    pub fn kill(&mut self, id: ProcessId) {
        if let Some(mut value) = self.map.remove(&id) {
            value.kill();
        }
    }

    pub fn new_spinner(&mut self) -> ProgressBar {
        let bar = ProgressBar::new_spinner();
        bar.set_style(
            // ProgressStyle::default_spinner().template(&format!("{{spinner}} {cmd}: {{wide_msg}}")),
            ProgressStyle::default_spinner()
                .template("{spinner} {prefix:.bold.dim} {wide_msg}")
                .unwrap(),
        );

        self.progress.add(bar)
    }

    pub fn get_id(&self, id: usize) -> ProcessId {
        ProcessId {
            loop_idx: self.stack.clone(),
            id,
        }
    }

    pub fn spawn(
        &mut self,
        id: usize,
        cmd: &str,
        args: &[Arg],
        stdout: OutputMap,
        stderr: OutputMap,
    ) {
        let id = self.get_id(id);
        self.status.set_message(format!("Spawning {id}"));
        self.status.inc(1);
        let spinner = self.new_spinner();

        let args = args
            .iter()
            .map(|arg| match arg {
                Arg::String(value) => value.clone(),
                Arg::Param {
                    index,
                    param,
                    prefix,
                    suffix,
                } => {
                    let param = self.params.get(param).unwrap();
                    let idx = self.stack.get_idx(index).unwrap();
                    let param_value = &param[idx];

                    format!("{}{}{}", prefix, param_value, suffix)
                }
                Arg::Pid(id) => {
                    let id = ProcessId {
                        loop_idx: self.stack.clone(),
                        id: *id,
                    };
                    match self.map.get(&id) {
                        Some(value) => value.pid_arg.clone(),
                        None => "_".into(),
                    }
                }
            })
            .collect::<Vec<_>>();

        let running = Running::new(
            spinner.clone(),
            cmd,
            &args,
            stdout,
            stderr,
            &self.stack,
            &self.params,
        );
        spinner.inc(1);

        let running = match running {
            Some(running) => running,
            None => return,
        };

        let previous = self.map.insert(id.clone(), running);

        if let Some(mut proc) = previous {
            println!("WARN: Process {:?} overwritten", id);
            proc.kill();
        }
    }

    pub fn wait_or_terminate(&mut self, id: usize, timeout: Option<(u64, u64)>) {
        let id = self.get_id(id);
        self.status.set_message(format!("Waiting for: {id}"));
        self.status.inc(1);

        let proc = match self.map.remove(&id) {
            Some(proc) => proc,
            None => return,
        };

        let timeout = timeout
            .map(|(duration, sleep_times)| TimeoutLoop::from_sleep_times(duration, sleep_times));

        proc.wait_or_terminate(timeout, &self.shutdown_signal);
    }

    pub fn wait_all(&mut self, timeout: Option<(u64, u64)>) {
        self.status.set_message("Waiting All");
        self.status.inc(1);
        let timeout = timeout
            .map(|(duration, sleep_times)| TimeoutLoop::from_sleep_times(duration, sleep_times));

        for (_, proc) in self.map.drain() {
            proc.wait_or_terminate(timeout, &self.shutdown_signal);
        }
    }

    fn sleep(&self, ms: u64) {
        self.status.set_message("Sleeping");
        std::thread::sleep(Duration::from_millis(ms));
    }

    pub fn run_command(&mut self, command: &TestCommand) {
        match command {
            TestCommand::Kill(id) => {
                let id = self.get_id(*id);
                self.kill(id);
            }
            TestCommand::Spawn {
                id,
                command,
                args,
                stdout,
                stderr,
            } => {
                self.spawn(*id, &command, &args[..], stdout.clone(), stderr.clone());
            }
            TestCommand::Sleep(ms) => self.sleep(*ms),
            TestCommand::WaitFor { id, timeout } => self.wait_or_terminate(*id, *timeout),
            TestCommand::WaitAll(timeout) => self.wait_all(*timeout),
        }
    }

    pub fn next(&mut self) -> bool {
        if self.instruction_idx >= self.instructions.len() {
            return false;
        }

        let next_instruction = self.instructions[self.instruction_idx].clone();

        match next_instruction {
            Instruction::BeginFor { id, param } => {
                let count = match self.params.get(&param) {
                    Some(value) => value.len(),
                    None => panic!("No param named {}", param),
                };

                self.instruction_idx += 1;
                let point = LoopPoint {
                    instruction_idx: self.instruction_idx,
                    count,
                };
                let idx = LoopIdx {
                    id: id.clone(),
                    idx: 0,
                };
                self.loop_points.push(point);
                self.stack.0.push(idx);
            }
            Instruction::NextLoop => {
                let point = self.loop_points.pop();
                let idx = self.stack.0.pop();

                if point.is_none() || idx.is_none() {
                    return false;
                }

                let point = point.unwrap();
                let mut idx = idx.unwrap();

                idx.idx += 1;

                if idx.idx < point.count {
                    self.instruction_idx = point.instruction_idx;
                    self.loop_points.push(point);
                    self.stack.0.push(idx);
                } else {
                    self.instruction_idx += 1;
                }
            }
            Instruction::Command(command) => {
                self.instruction_idx += 1;
                self.run_command(&command);
            }
        }

        true
    }

    pub fn run(mut self) {
        loop {
            if self.shutdown_signal.load(Ordering::Relaxed) {
                break;
            }

            if !self.next() {
                break;
            }
        }

        self.shutdown();
        // self.progress.fi
        // self.progress.join().unwrap();
    }
}
