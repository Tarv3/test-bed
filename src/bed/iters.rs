use std::{io::Write, time::Duration};

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

pub struct IterProgress {
    next_idx: u64,
    next_message: String,
    bar: ProgressBar,
}

impl IterProgress {
    pub fn new(name: String, len: u64, multibar: &MultiProgress) -> Self {
        let bar = ProgressBar::new(len);
        bar.set_style(
            ProgressStyle::default_bar()
                .template("{prefix:<10.bold.dim} [{bar}] {pos}/{len} {eta} : {elapsed_precise} : {wide_msg}")
                .unwrap()
                .progress_chars("== "),
        );
        let bar = multibar.add(bar);
        bar.set_prefix(name);

        Self {
            next_idx: 0,
            next_message: String::new(),
            bar,
        }
    }

    pub fn set(&mut self, value: u64) {
        self.next_idx = value;
    }

    pub fn set_message(&mut self, message: &str) {
        self.next_message.clear();
        self.next_message.push_str(message);
    }

    pub fn update(&self) {
        if self.next_idx == 0 {
            self.bar.reset();
        } else {
            self.bar.set_position(self.next_idx);
        }

        self.bar.set_message(self.next_message.clone());
    }

    pub fn finish(&self) {
        self.bar.finish();
    }

    pub fn get_progress(&self) -> (u64, u64) {
        let len = self.bar.length().unwrap_or(0);
        let pos = self.bar.position();

        (pos, len)
    }

    pub fn get_eta(&self) -> Duration {
        self.bar.eta()
    }

    pub fn get_elapsed(&self) -> Duration {
        self.bar.elapsed()
    }

    pub fn get_msg(&self) -> String {
        self.bar.message()
    }

    pub fn write_summary(&self, mut writer: impl Write) -> std::io::Result<usize> {
        let var_name = self.bar.prefix();
        let message = self.get_msg();
        let (pos, len) = self.get_progress();
        let eta = self.get_eta();
        let elapsed = self.get_elapsed();
        let (eta_s, eta_m, eta_h) = seconds_to_smh(eta.as_secs());
        let (elapsed_s, elapsed_m, elapsed_h) = seconds_to_smh(elapsed.as_secs());

        let formatted = format!("[{var_name} = {pos} / {len}] : Eta {eta_h}h:{eta_m}m:{eta_s}s : Elapsed {elapsed_h}h:{elapsed_m}m:{elapsed_s}s : {message}");
        let bytes = formatted.as_bytes();

        writer.write_all(bytes)?;
        Ok(bytes.len())
    }
}

fn seconds_to_smh(seconds: u64) -> (u64, u64, u64) {
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let seconds = seconds % 60;
    let minutes = minutes % 60;

    (seconds, minutes, hours)
}
