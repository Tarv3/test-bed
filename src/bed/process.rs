use std::{
    fs::OpenOptions,
    io::{self, BufRead, BufReader, BufWriter, ErrorKind, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use console::Term;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::program::Shutdown;

use super::{commands::OutputMap, SLEEP_TIME};

#[derive(Debug)]
pub enum ProcessState {
    Running,
    Killed,
    Error(io::Error),
    Failed(Option<i32>),
    Finished,
}

#[derive(Clone, Copy)]
struct BarUsage {
    truncated: bool,
    prefix: usize,
    message: usize,
}

impl Default for BarUsage {
    fn default() -> Self {
        Self {
            truncated: false,
            prefix: Default::default(),
            message: Default::default(),
        }
    }
}

#[derive(Clone)]
pub struct ProcessBar {
    pub bar: ProgressBar,
    usage: Arc<Mutex<BarUsage>>,
    ident: String,
    stdout: Arc<AtomicBool>,
    stderr: Arc<AtomicBool>,
    status: Arc<Mutex<ProcessState>>,
    term: Term,
}

impl ProcessBar {
    pub fn new(multibar: &MultiProgress, ident: String) -> Self {
        let bar = ProgressBar::new_spinner();
        bar.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner} {prefix:.bold.dim} {wide_msg}")
                .unwrap(),
        );
        let bar = multibar.add(bar);

        let output = Self {
            bar,
            usage: Arc::new(Mutex::new(BarUsage::default())),
            status: Arc::new(Mutex::new(ProcessState::Running)),
            ident,
            stdout: Arc::new(AtomicBool::new(false)),
            stderr: Arc::new(AtomicBool::new(false)),
            term: Term::stdout(),
        };
        let available = output.term_cols();
        let prefix = output.prepare_prefix();
        {
            let mut usage = output.usage.lock().unwrap();
            output.update_prefix(available, prefix, &mut usage);
        }

        output
    }

    pub fn extra_space(&self) -> usize {
        3 // spinner + space + space
    }

    pub fn inc(&self, delta: u64) {
        self.bar.inc(delta);
    }

    pub fn set_stdout(&self, value: bool) {
        self.stdout.store(value, Ordering::Release);
    }

    pub fn set_stderr(&self, value: bool) {
        self.stderr.store(value, Ordering::Release);
    }

    pub fn prepare_prefix(&self) -> String {
        let stdout = self.stdout.load(Ordering::Acquire);
        let stderr = self.stderr.load(Ordering::Acquire);

        let mut prefix = String::new();
        if stdout {
            prefix.push_str("!stdout");
        }

        if stderr {
            if stdout {
                prefix.push_str(" ");
            }
            prefix.push_str("!stderr");
        }

        if stdout || stderr {
            prefix.push_str(": ");
        }

        prefix.push_str(&self.ident);

        prefix
    }

    fn term_cols(&self) -> usize {
        let extra = self.extra_space();
        let (_, cols) = self.term.size();
        (cols as usize).max(extra) - extra
    }

    fn update_prefix(&self, available: usize, mut prefix: String, usage: &mut BarUsage) {
        let next_usage = usage.message + prefix.len();
        usage.prefix = prefix.len();

        if next_usage > available {
            usage.truncated = true;

            match usage.message >= available {
                true => {
                    usage.prefix = 0;
                    prefix.truncate(0);
                }
                false => {
                    usage.prefix = available - usage.message;
                    prefix.truncate(usage.prefix);
                }
            }
        } else {
            usage.truncated = false;
        }

        self.bar.set_prefix(prefix);
    }

    fn update_message(&self, available: usize, message_len: usize, usage: &mut BarUsage) {
        usage.message = message_len;
        let next_usage = usage.message + usage.prefix;

        let mut refresh = next_usage > available;
        refresh |= next_usage < available && usage.truncated;

        if refresh {
            let prefix = self.prepare_prefix();
            self.update_prefix(available, prefix, usage);
        }
    }

    pub fn set_message(&self, msg: String) {
        {
            let status = &*self.status.lock().unwrap();

            match status {
                ProcessState::Running => {}
                _ => return,
            }
        }

        let available = self.term_cols();

        {
            let mut usage = self.usage.lock().unwrap();
            self.update_message(available, msg.len(), &mut usage);
        }

        self.bar.set_message(msg);
        self.bar.inc(1);
    }

    pub fn set_state(&self, state: ProcessState) {
        match state {
            ProcessState::Running => return,
            _ => {}
        }

        let message = format!("{:?}", state);
        *self.status.lock().unwrap() = state;
        let available = self.term_cols();

        {
            let mut usage = self.usage.lock().unwrap();
            self.update_message(available, message.len(), &mut usage);
        }

        self.bar.finish_with_message(message);
    }
}

pub struct ProcessInfo {
    pub command: String,
    pub args: Vec<String>,
    pub stdout: OutputMap<PathBuf>,
    pub stderr: OutputMap<PathBuf>,
    pub running: Option<ProcessStatus>,
}

impl ProcessInfo {
    pub fn new(command: String) -> Self {
        Self {
            command,
            args: vec![],
            stdout: OutputMap::Print,
            stderr: OutputMap::Print,
            running: None,
        }
    }

    pub fn add_args(&mut self, args: impl IntoIterator<Item = String>) -> &mut Self {
        self.args.extend(args.into_iter());
        self
    }

    pub fn set_stdout(&mut self, out: OutputMap<PathBuf>) -> &mut Self {
        self.stdout = out;
        self
    }

    pub fn set_stderr(&mut self, out: OutputMap<PathBuf>) -> &mut Self {
        self.stderr = out;
        self
    }

    pub fn run(&mut self, multibar: &MultiProgress) -> io::Result<()> {
        let pat = ['/', '\\'];

        let mut ident = self.command.split(pat).last().unwrap_or("?").to_string();

        for arg in self.args.iter() {
            ident.push(' ');
            ident.push_str(arg);
        }

        let bar = ProcessBar::new(multibar, ident);

        let mut process = Command::new(&self.command);
        process.args(self.args.iter());
        process.stdout(Stdio::piped());
        process.stderr(Stdio::piped());

        let mut spawned = process.spawn()?;
        let stdout = spawned.stdout.take().unwrap();

        match &self.stdout {
            OutputMap::Print => spawn_progress_writer(stdout, bar.clone()),
            OutputMap::Create(file) => {
                if let Err(_) = spawn_file_writer(stdout, file, false) {
                    bar.set_stdout(true);
                }
            }
            OutputMap::Append(file) => {
                if let Err(_) = spawn_file_writer(stdout, file, true) {
                    bar.set_stdout(true);
                }
            }
        }

        let stderr = spawned.stderr.take().unwrap();
        match &self.stderr {
            OutputMap::Print => spawn_progress_writer(stderr, bar.clone()),
            OutputMap::Create(file) => {
                if let Err(_) = spawn_file_writer(stderr, file, false) {
                    bar.set_stderr(true);
                }
            }
            OutputMap::Append(file) => {
                if let Err(_) = spawn_file_writer(stderr, file, true) {
                    bar.set_stderr(true);
                }
            }
        }

        let status = ProcessStatus {
            pid: spawned.id(),
            process: spawned,
            bar,
        };

        self.running = Some(status);

        Ok(())
    }

    pub fn kill(&mut self) {
        if let Some(mut value) = self.running.take() {
            value.kill()
        }
    }

    pub fn try_wait(&mut self) -> bool {
        let process = match self.running.as_mut() {
            Some(process) => process,
            None => return true,
        };

        process.bar.inc(1);
        let status = match process.process.try_wait() {
            Ok(Some(status)) => status,
            Ok(None) => return false,
            Err(e) => {
                process.bar.set_state(ProcessState::Error(e));
                return true;
            }
        };

        match status.success() {
            true => process.bar.set_state(ProcessState::Finished),
            false => process.bar.set_state(ProcessState::Failed(status.code())),
        }

        true
    }

    pub fn _wait_or_terminate(&mut self, wait: Option<Duration>, shutdown: &Shutdown) {
        let mut process = match self.running.take() {
            Some(process) => process,
            None => return,
        };

        let now = std::time::Instant::now();
        let wait = wait.unwrap_or(Duration::from_secs(u64::MAX));
        let mut exit = None;

        while now.elapsed() < wait {
            if shutdown.is_shutdown() {
                process.kill();

                return;
            }

            process.bar.inc(1);

            match process.process.try_wait() {
                Ok(None) => std::thread::sleep(SLEEP_TIME),
                Ok(Some(status)) => {
                    exit = Some(status);
                    break;
                }
                Err(e) => {
                    process.bar.set_state(ProcessState::Error(e));
                    return;
                }
            }
        }

        match exit {
            Some(status) => match status.success() {
                true => process.bar.set_state(ProcessState::Finished),
                false => process.bar.set_state(ProcessState::Failed(status.code())),
            },
            None => {
                process.kill();
            }
        }
    }
}

pub struct ProcessStatus {
    pub process: Child,
    pub pid: u32,
    pub bar: ProcessBar,
}

impl ProcessStatus {
    pub fn kill(&mut self) {
        match self.process.kill() {
            Ok(_) => self.bar.set_state(ProcessState::Killed),
            Err(e) => self.bar.set_state(ProcessState::Error(e)),
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
        let mut reader = BufReader::new(reader);
        let mut bytes = vec![];

        loop {
            let available = match reader.fill_buf() {
                Ok(available) => available,
                Err(_) => break,
            };

            bytes.clear();
            bytes.extend_from_slice(available);
            bytes.retain(|value| *value != b'\r');
            let consumed = available.len();
            reader.consume(consumed);

            if consumed == 0 {
                break;
            }

            if let Err(e) = writer.write_all(&bytes) {
                println!("Write Failed {}: {}", path, e);
                break;
            }
            writer.flush().ok();
        }
    });

    Ok(())
}

fn spawn_progress_writer<R: Read + Send>(reader: R, bar: ProcessBar)
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut bytes = vec![];
        // let mut output = String::new();
        let mut clear = false;

        loop {
            let available = match reader.fill_buf() {
                Ok(n) => n,
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    bar.set_message(format!("Error: {e}"));
                    break;
                }
            };

            let used = available.len();

            if used == 0 {
                break;
            }

            for &byte in available.iter() {
                if byte == b'\n' || byte == b'\r' {
                    clear = true;
                    continue;
                }

                if clear {
                    bytes.clear();
                    clear = false;
                }

                bytes.push(byte);
            }

            reader.consume(used);
            let value = String::from_utf8_lossy(&bytes);
            bar.set_message(value.to_string());
        }
    });
}
