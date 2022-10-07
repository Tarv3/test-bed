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

#[derive(Clone)]
pub struct ProcessBar {
    pub bar: ProgressBar,
    ident: String,
    stdout: Arc<AtomicBool>,
    stderr: Arc<AtomicBool>,
    status: Arc<Mutex<ProcessState>>,
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

            status: Arc::new(Mutex::new(ProcessState::Running)),
            ident,
            stdout: Arc::new(AtomicBool::new(false)),
            stderr: Arc::new(AtomicBool::new(false)),
        };
        output.update_prefix();

        output
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

    pub fn update_prefix(&self) {
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
        self.bar.set_prefix(prefix);
    }

    pub fn set_message(&self, msg: String) {
        let status = &*self.status.lock().unwrap();

        match status {
            ProcessState::Running => {}
            _ => return,
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

    pub fn run(&mut self, multibar: &MultiProgress, args: usize) -> io::Result<()> {
        let mut ident = self.command.clone();
        let args = args.min(self.args.len());

        for i in 0..args {
            let arg = &self.args[i];
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

    pub fn wait_or_terminate(&mut self, wait: Option<Duration>, shutdown: &Shutdown) {
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

            if let Err(e) = writer.write_all(&bytes) {
                println!("Write Failed {}: {}", path, e);
                break;
            }
            writer.flush().ok();
        }
    });

    Ok(())
}

fn read_output<'a, R: BufRead>(reader: &mut R, buf: &'a mut Vec<u8>) -> io::Result<&'a [u8]> {
    buf.clear();

    let find_delimiters = |bytes: &[u8]| -> Option<(usize, usize)> {
        let newline = b'\n';
        let c_return = b'\r';

        let first = memchr::memchr2(newline, c_return, bytes)?;
        let mut end = first + 1;

        while end < bytes.len() {
            if bytes[end] == newline || bytes[end] == c_return {
                break;
            }
            end += 1;
        }

        Some((first, end))
    };

    loop {
        let (done, used) = {
            let available = match reader.fill_buf() {
                Ok(n) => n,
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };

            match find_delimiters(available) {
                Some((start, end)) => {
                    buf.extend_from_slice(&available[..start]);
                    (true, end)
                }
                None => {
                    buf.extend_from_slice(available);
                    (false, available.len())
                }
            }
        };

        reader.consume(used);

        if done || used == 0 {
            return Ok(buf);
        }
    }
}

fn spawn_progress_writer<R: Read + Send>(reader: R, bar: ProcessBar)
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut bytes = vec![];

        while let Ok(bytes) = read_output(&mut reader, &mut bytes) {
            let value = String::from_utf8_lossy(bytes);
            bar.set_message(value.to_string());
        }
    });
}
