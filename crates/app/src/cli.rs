use clap::{Parser, ValueEnum};

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Cli {
    #[arg(long, default_value = "all")]
    pub mode: Mode,
    #[arg(long, default_value_t = false)]
    pub rebuild: bool,
    #[arg(long, default_value_t = false)]
    pub rebuild_schema: bool,
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

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn parse_rebuild_schema_flag() {
        let cli = Cli::try_parse_from(["inkstone-app", "--rebuild-schema"]).unwrap();
        assert!(cli.rebuild_schema);
        assert!(!cli.rebuild);
    }
}
