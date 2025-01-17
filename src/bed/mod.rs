use std::{
    io::{Seek, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

use indicatif::{MultiProgress, ProgressDrawTarget};

use crate::program::{Executable, Object, ProgramState, VarNameId, VarNames, VariableAccessError};

use self::{
    commands::Command,
    iters::IterProgress,
    process::ProcessInfo,
    templates::{yield_value, TemplateBuilder, TemplateCommand},
};

pub mod commands;
pub mod expr;
pub mod iters;
pub mod process;
pub mod templates;

pub const SLEEP_TIME: Duration = Duration::from_millis(100);

pub struct TestBed<'source> {
    pub templates: TemplateBuilder<'source>,
    pub var_names: VarNames,

    pub spawn_limit: Option<usize>,
    pub processes: Vec<ProcessInfo>,
    pub iters: Vec<(VarNameId, IterProgress)>,
    pub multibar: MultiProgress,

    progress_file: Option<std::fs::File>,
}

impl<'source> TestBed<'source> {
    pub fn new(
        template_output: PathBuf,
        template_includes: Vec<PathBuf>,
        var_names: VarNames,
    ) -> Self {
        let templates = TemplateBuilder::new(template_output, template_includes);
        let progress = MultiProgress::with_draw_target(ProgressDrawTarget::stdout());

        let progress_file = std::env::var("BED_PROGRESS").ok().map(|file| {
            match std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .open(&file)
            {
                Ok(file) => file,
                Err(e) => {
                    panic!("Failed to create file `{file}`: {e}");
                }
            }
        });

        Self {
            templates,
            var_names,
            spawn_limit: None,
            processes: vec![],
            iters: vec![],
            multibar: progress,
            progress_file,
        }
    }

    pub fn reset(&mut self, shutdown: &crate::program::Shutdown) {
        self.wait_all(None, 0, shutdown);
        self.processes.clear();
        self.spawn_limit = None;
        self.multibar = MultiProgress::with_draw_target(ProgressDrawTarget::stdout());
    }

    fn wait_all(
        &mut self,
        wait: Option<u64>,
        remaining: usize,
        shutdown: &crate::program::Shutdown,
    ) {
        let duration = wait.unwrap_or(u64::MAX);
        let duration = Duration::from_millis(duration);
        let now = Instant::now();
        let mut kill = false;
        let remaining = remaining.max(1);

        while self.processes.len() >= remaining && now.elapsed() < duration {
            if shutdown.is_shutdown() {
                kill = true;
                break;
            }
            let mut i = 0;

            while i < self.processes.len() {
                if self.processes[i].try_wait() {
                    self.processes.swap_remove(i);
                    continue;
                }
                i += 1;
            }

            std::thread::sleep(SLEEP_TIME);
        }

        if kill {
            <Self as Executable<Command>>::shutdown(self);
        }
    }

    fn write_progress(&mut self) {
        let Some(file) = &mut self.progress_file else {
            return;
        };
        let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        file.seek(std::io::SeekFrom::Start(0))
            .expect("Failed to seek file");

        let mut written = 0;

        for (_, value) in self.iters.iter() {
            written += value
                .write_summary(&mut *file)
                .expect("Failed to write to file");
            file.write_all(&[b'\n']).expect("Failed to write new line");
            written += 1;
        }

        if written < len as usize {
            let zeros = vec![b' '; len as usize - written];
            file.write_all(&zeros).expect("Failed to write zeros");
        }
    }
}

impl<'source> Executable<Command> for TestBed<'source> {
    fn shutdown(&mut self) {
        for mut value in self.processes.drain(..) {
            value.kill();
        }

        for (_, value) in self.iters.drain(..) {
            value.finish();
        }
    }

    fn finish(&mut self, _: &mut ProgramState, shutdown: &crate::program::Shutdown) {
        self.wait_all(None, 0, shutdown);

        for (_, value) in self.iters.drain(..) {
            value.finish();
        }
    }

    fn execute(
        &mut self,
        command: &Command,
        stack: &mut ProgramState,
        shutdown: &crate::program::Shutdown,
    ) -> Result<(), VariableAccessError> {
        match command {
            Command::LimitSpawn(limit) => self.spawn_limit = Some(*limit),
            Command::Sleep(millis) => {
                let duration = Duration::from_millis(*millis);
                let start = std::time::Instant::now();

                while start.elapsed() < duration {
                    if shutdown.is_shutdown() {
                        break;
                    }
                    std::thread::sleep(SLEEP_TIME);
                }
            }
            Command::Spawn(spawn) => {
                if let Some(limit) = self.spawn_limit {
                    if self.processes.len() >= limit {
                        self.wait_all(None, limit, shutdown);
                    }
                }

                self.iters.iter().for_each(|value| value.1.update());
                self.write_progress();

                let mut process = spawn.evaluate(stack)?;
                if let Err(e) = process.run(self.iters.len(), &self.multibar) {
                    self.multibar
                        .println(&format!("Failed to spawn {}: {e}", process.command))
                        .ok();
                    return Ok(());
                }

                self.processes.push(process);
            }
            Command::WaitAll(timeout) => {
                self.wait_all(*timeout, 0, shutdown);
            }
        }

        Ok(())
    }

    fn set_iter(&mut self, iter_var: VarNameId, idx: usize, var: &Object) {
        let len = match var {
            Object::Counter(counter) => counter.len(),
            Object::List(vec) => vec.len(),
            _ => 0,
        };
        let len = len as u64;
        let bar = match self.iters.iter_mut().find(|(id, _)| *id == iter_var) {
            Some((_, bar)) => bar,
            None => {
                let name = self.var_names.evaluate(iter_var).unwrap_or("Unknown");
                let bar = IterProgress::new(name.into(), len, &self.multibar);
                self.iters.push((iter_var, bar));
                &mut self.iters.last_mut().unwrap().1
            }
        };

        bar.set(idx as u64);

        match var {
            Object::Struct(value) => {
                bar.set_message(&value.base);
            }
            Object::List(list) => {
                if let Some(Object::Struct(value)) = list.get(idx) {
                    bar.set_message(&value.base);
                }
            }
            Object::Counter(counter) => bar.set_message(&format!("{}", counter.idx())),
            _ => {}
        }
    }
}

impl<'source> Executable<TemplateCommand> for TestBed<'source> {
    fn shutdown(&mut self) {}

    fn finish(&mut self, _: &mut ProgramState, _: &crate::program::Shutdown) {}

    fn execute(
        &mut self,
        command: &TemplateCommand,
        state: &mut ProgramState,
        _: &crate::program::Shutdown,
    ) -> Result<(), VariableAccessError> {
        let err = match command {
            TemplateCommand::BuildAssign { output, object } => {
                match object.evaluate(state, &mut self.templates, &self.var_names) {
                    Ok(object) => {
                        state.insert_var(*output, object, None);
                        return Ok(());
                    }
                    Err(templates::TemplateBuildError::VariableError(e)) => return Err(e),
                    Err(e) => e,
                }
            }

            TemplateCommand::Yield { output, object } => {
                match object.evaluate(state, &mut self.templates, &self.var_names) {
                    Ok(object) => {
                        yield_value(*output, object, state);
                        return Ok(());
                    }
                    Err(templates::TemplateBuildError::VariableError(e)) => return Err(e),
                    Err(e) => e,
                }
            }
        };

        println!("{err}\n");
        return Ok(());
    }
}
