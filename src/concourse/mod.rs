use crate::{config::*, repo::*, workspace::StateId};
use anyhow::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub mod check;
pub mod ci_in;
pub mod ci_out;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ResourceConfig {
    #[serde(default)]
    params: Option<OutParams>,
    source: Source,
    version: Option<Version>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Source {
    uri: String,
    branch: String,
    gates_branch: Option<String>,
    gates_file: Option<String>,
    private_key: String,
    environment: Option<String>,
    #[serde(default = "bool::default")]
    ignore_queue: bool,
    #[serde(default = "default_config_path")]
    config: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Version {
    trigger: String,
    version: Option<u32>,
}
impl From<StateId> for Version {
    fn from(id: StateId) -> Self {
        Self {
            trigger: id.head_commit,
            version: Some(id.version),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct OutParams {
    repository: String,
    environment: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DiffElem {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct ResourceData {
    version: Version,
    metadata: Vec<DiffElem>,
}

fn default_config_path() -> String {
    "cepler.yml".to_string()
}

fn get_gate(
    gates_file: Option<&String>,
    gates_branch: Option<&String>,
    env: &str,
    repo: &Repo,
) -> Result<Option<String>> {
    let gates = match (gates_file, gates_branch) {
        (Some(gates_file), Some(gates_branch)) => {
            if let Some(file) =
                repo.get_file_from_branch(gates_branch, Path::new(&gates_file), |bytes| {
                    GatesConfig::from_reader(bytes)
                })?
            {
                Ok(Some(file))
            } else {
                Err(anyhow!("Couldn't read gates file"))
            }
        }
        (Some(gates_file), _) => Ok(Some(GatesConfig::from_file(gates_file)?)),

        (_, Some(_)) => Err(anyhow!("Missing gates_file in source")),
        _ => Ok(None),
    }?;

    let gate = if let Some(gates) = gates {
        gates.get_gate(env)?
    } else {
        None
    };
    Ok(gate)
}
