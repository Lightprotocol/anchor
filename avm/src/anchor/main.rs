use light_avm as avm;
use std::{env, fs, process::Command};

fn main() -> anyhow::Result<()> {
    let args = env::args().skip(1).collect::<Vec<String>>();

    let version = avm::current_version()
        .map_err(|_e| anyhow::anyhow!("Anchor version not set. Please run `avm use latest`."))?;

    let binary_path = avm::version_binary_path(&version);
    if fs::metadata(&binary_path).is_err() {
        anyhow::bail!(
            "anchor-cli {} not installed. Please run `avm use {}`.",
            version,
            version
        );
    }
    let exit = Command::new(binary_path)
        .args(args)
        .spawn()?
        .wait_with_output()
        .expect("Failed to run anchor-cli");

    if !exit.status.success() {
        std::process::exit(exit.status.code().unwrap_or(1));
    }

    Ok(())
}
