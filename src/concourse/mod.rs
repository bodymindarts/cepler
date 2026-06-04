use crate::{config::*, database::Database, repo::*, workspace::StateId};
use anyhow::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::path::Path;

pub mod check;
pub mod ci_in;
pub mod ci_out;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ResourceConfig<P: DeserializeOwned + Serialize> {
    #[serde(bound = "P: DeserializeOwned")]
    params: Option<P>,
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
    version: Option<String>,
}
impl From<StateId> for Version {
    fn from(id: StateId) -> Self {
        Self {
            trigger: id.head_commit,
            version: Some(id.version.to_string()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct OutParams {
    repository: String,
    environment: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct InParams {
    #[serde(default = "bool_true")]
    prepare: bool,
}

fn bool_true() -> bool {
    true
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

/// Materialise the small set of files cepler itself reads from disk
/// (config + database state directory + optional on-disk gates file)
/// from a freshly no-checkout-cloned repo. Everything else stays
/// unwritten — `Workspace::prepare` / `Workspace::reproduce` selectively
/// checks out env-specific files as a separate step. Returns the parsed
/// config so the caller doesn't have to round-trip through
/// `Config::from_file` afterwards.
fn populate_workspace_metadata(
    repo: &Repo,
    path_to_config: &str,
    gates_file: Option<&String>,
    gates_branch: Option<&String>,
) -> Result<Config> {
    let (head, _) = repo.head_commit_summary()?;

    let t = std::time::Instant::now();
    let config = repo
        .get_file_content(head, Path::new(path_to_config), |bytes| {
            Config::from_reader(bytes)
        })?
        .context(format!(
            "Config file '{}' not found in HEAD",
            path_to_config
        ))?;
    eprintln!(
        "[cepler-perf] read config from HEAD tree: {:.2}s",
        t.elapsed().as_secs_f64()
    );

    let state_dir = Database::state_dir_from_config(&config.scope, path_to_config);
    let mut paths: Vec<String> = vec![
        path_to_config.to_string(),
        // libgit2's checkout pathspec is fnmatch-style — a single `*`
        // matches everything directly inside the state dir, which is
        // flat (one `<env>.state` file per environment).
        format!("{}/*", state_dir),
    ];
    // Only the on-disk gates path needs materialising; when `gates_branch`
    // is set the gates file is read directly from the branch's tree via
    // `repo.get_file_from_branch`, never from disk.
    if let (Some(file), None) = (gates_file, gates_branch) {
        paths.push(file.clone());
    }
    let t = std::time::Instant::now();
    repo.checkout_paths(paths)?;
    eprintln!(
        "[cepler-perf] selective checkout (config + state dir): {:.2}s",
        t.elapsed().as_secs_f64()
    );
    Ok(config)
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
