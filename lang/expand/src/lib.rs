use std::{ffi::OsString, fs, path::PathBuf, process::Stdio};

use anyhow::Result;

pub fn expand_program(
    root: PathBuf,
    package_name: &str,
    version: &str,
    expansions_path: Option<PathBuf>,
    cargo_args: &[String],
) -> Result<Vec<u8>> {
    let target_dir_arg = match expansions_path {
        Some(ref expansions_path) => {
            let mut target_dir_arg = OsString::from("--target-dir=");
            target_dir_arg.push(expansions_path.join("expand-target"));
            Some(target_dir_arg)
        }
        None => None,
    };

    let mut cmd = std::process::Command::new("cargo");
    let cmd = cmd.arg("expand");
    if let Some(target_dir_arg) = target_dir_arg {
        cmd.arg(target_dir_arg);
    }
    let exit = cmd
        .current_dir(root)
        .arg(&format!("--package={package_name}"))
        .args(cargo_args)
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| anyhow::format_err!("{}", e.to_string()))?;
    if !exit.status.success() {
        eprintln!("'cargo expand' failed. Perhaps you have not installed 'cargo-expand'? https://github.com/dtolnay/cargo-expand#installation");
        std::process::exit(exit.status.code().unwrap_or(1));
    }

    if let Some(ref expansions_path) = expansions_path {
        let program_expansions_path = expansions_path.join(package_name);
        fs::create_dir_all(&program_expansions_path)?;

        // let version = cargo.version();
        let time = chrono::Utc::now().to_string().replace(' ', "_");
        let file_path = program_expansions_path.join(format!("{package_name}-{version}-{time}.rs"));
        fs::write(&file_path, &exit.stdout)
            .map_err(|e| anyhow::format_err!("{}", e.to_string()))?;

        println!(
            "Expanded {} into file {}\n",
            package_name,
            file_path.to_string_lossy()
        );
    }

    Ok(exit.stdout)
}
