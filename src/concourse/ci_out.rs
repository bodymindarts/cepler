use super::*;
use crate::{config::Config, repo::*, workspace::Workspace};
use anyhow::*;
use std::{io, path};

pub fn exec(origin: &str) -> Result<()> {
    eprintln!("Recording resource - cepler v{}", clap::crate_version!());
    let ResourceConfig { source, params, .. }: ResourceConfig =
        serde_json::from_reader(io::stdin()).context("Deserializing stdin")?;
    let out_params = params.unwrap();
    std::env::set_current_dir(path::Path::new(&format!(
        "{}/{}",
        origin, out_params.repository
    )))?;

    let conf = GitConfig {
        url: source.uri,
        branch: source.branch.clone(),
        private_key: source.private_key,
        dir: origin.to_string(),
    };
    let config = Config::from_file(&source.config)?;
    let environment = out_params.environment.ok_or(()).or({
        source
            .environment
            .ok_or_else(|| anyhow!("Environment not specified in source"))
    })?;
    let mut ws = Workspace::new(source.config)?;
    let env = config
        .environments
        .get(&environment)
        .context(format!("Environment '{}' not found in config", environment))?;
    let (trigger, diff) = ws.record_env(env, true, true, Some(conf))?;
    println!(
        "{}",
        serde_json::to_string(&ResourceData {
            version: Version { trigger },
            metadata: diff
                .into_iter()
                .map(|diff| DiffElem {
                    name: diff.ident.inner(),
                    value: diff
                        .current_state
                        .map(|state| state.to_string())
                        .unwrap_or_else(String::new)
                })
                .collect()
        })?
    );
    Ok(())
}
