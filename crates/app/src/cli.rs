use clap::{Parser, ValueEnum};

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Cli {
    #[arg(long, default_value = "all")]
    pub mode: Mode,
    #[arg(long, default_value_t = false)]
    pub rebuild: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Mode {
    All,
    Api,
    Worker,
}

impl Mode {
    pub fn run_api(self) -> bool {
        matches!(self, Mode::All | Mode::Api)
    }

    pub fn run_worker(self) -> bool {
        matches!(self, Mode::All | Mode::Worker)
    }
}
