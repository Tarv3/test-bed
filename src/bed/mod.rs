use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use indicatif::{MultiProgress, ProgressDrawTarget};

use crate::program::{Executable, ProgramState, VarNames, Variable};

use self::{
    commands::Command,
    process::ProcessInfo,
    templates::{yield_value, TemplateBuilder, TemplateCommand},
};

pub mod commands;
pub mod expr;
pub mod process;
pub mod templates;

pub const SLEEP_TIME: Duration = Duration::from_millis(100);

pub struct TestBed<'source> {
    pub templates: TemplateBuilder<'source>,
    pub var_names: VarNames,

    pub spawn_limit: Option<usize>,
    pub processes: Vec<ProcessInfo>,
    pub multibar: MultiProgress,
}

impl<'source> TestBed<'source> {
    pub fn new(
        template_output: PathBuf,
        template_includes: Vec<PathBuf>,
        var_names: VarNames,
    ) -> Self {
        let templates = TemplateBuilder::new(template_output, template_includes);
        let progress = MultiProgress::with_draw_target(ProgressDrawTarget::stdout());

        Self {
            templates,
            var_names,
            spawn_limit: None,
            processes: vec![],
            multibar: progress,
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
}

impl<'source> Executable<Command> for TestBed<'source> {
    fn shutdown(&mut self) {
        for mut value in self.processes.drain(..) {
            value.kill();
        }
    }

    fn finish(&mut self, _: &mut ProgramState, shutdown: &crate::program::Shutdown) {
        self.wait_all(None, 0, shutdown);
    }

    fn execute(
        &mut self,
        command: &Command,
        stack: &mut ProgramState,
        shutdown: &crate::program::Shutdown,
    ) {
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

                let mut process = spawn.evaluate(stack);
                if let Err(e) = process.run(&self.multibar) {
                    self.multibar
                        .println(&format!("Failed to spawn {}: {e}", process.command))
                        .ok();
                    return;
                }

                self.processes.push(process);
            }
            Command::WaitAll(timeout) => {
                self.wait_all(*timeout, 0, shutdown);
            }
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
    ) {
        let err = match command {
            TemplateCommand::BuildAssign { output, object } => {
                match object.evaluate(state, &mut self.templates, &self.var_names) {
                    Ok(object) => {
                        state.insert_var(*output, Variable::Object(object), None);
                        return;
                    }
                    Err(e) => e,
                }
            }

            TemplateCommand::Yield { output, object } => {
                match object.evaluate(state, &mut self.templates, &self.var_names) {
                    Ok(object) => {
                        yield_value(*output, object, state);
                        return;
                    }
                    Err(e) => e,
                }
            }
        };

        println!("{err}\n");
        return;
    }
}
