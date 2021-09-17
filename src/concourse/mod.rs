use serde::{Deserialize, Serialize};

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
    private_key: String,
    environment: Option<String>,
    #[serde(default = "default_config_path")]
    config: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Version {
    trigger: String,
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
