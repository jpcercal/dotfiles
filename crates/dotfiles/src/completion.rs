use anyhow::Result;
use clap::Parser;
use clap_complete::{generate, Shell};
use std::io;

#[derive(Parser, Debug)]
pub struct CompletionArgs {
    /// Shell to generate completions for
    #[arg(value_enum)]
    pub shell: Shell,
}

pub fn run(args: CompletionArgs) -> Result<()> {
    let mut cmd = crate::cli_for_completion();
    let name = cmd.get_name().to_string();
    generate(args.shell, &mut cmd, name, &mut io::stdout());
    Ok(())
}
