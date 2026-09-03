use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
pub struct SchemaArgs {
    /// Which schema to export
    #[arg(long, default_value = "apps", value_parser = ["apps", "prefs"])]
    pub kind: String,

    /// Also write the schema to schema/<kind>.schema.json in the repo
    #[arg(long)]
    pub write: bool,
}

pub fn run(args: SchemaArgs) -> Result<()> {
    let schema = match args.kind.as_str() {
        "apps" => dotfiles_manifest::schema_json()?,
        "prefs" => dotfiles_prefs::prefs_schema_json()?,
        other => anyhow::bail!("unknown schema kind '{}'", other),
    };
    println!("{}", schema);
    if args.write {
        let path = crate::ctx::Ctx::real(false)
            .dotfiles_dir
            .join("schema")
            .join(format!("{}.schema.json", args.kind));
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, &schema)?;
        eprintln!("written {}", path.display());
    }
    Ok(())
}
