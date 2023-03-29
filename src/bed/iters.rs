use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

pub struct IterProgress {
    bar: ProgressBar,
}

impl IterProgress {
    pub fn new(name: String, len: u64, multibar: &MultiProgress) -> Self {
        let bar = ProgressBar::new(len);
        bar.set_style(
            ProgressStyle::default_bar()
                .template("{prefix:<10.bold.dim} [{bar}] {pos}/{len} {eta} : {elapsed_precise}")
                .unwrap()
                .progress_chars("== "),
        );
        let bar = multibar.add(bar);
        bar.set_prefix(name);

        Self { bar }
    }

    pub fn set(&self, value: u64) {
        self.bar.set_position(value);
    }

    pub fn reset(&self) {
        self.bar.reset();
    }

    pub fn finish(&self) {
        self.bar.finish();
    }
}
