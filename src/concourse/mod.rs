use serde::{Deserialize, Serialize};

mod gen;

pub mod check;
pub mod ci_in;
pub mod ci_out;
pub use gen::ConcourseGen;

#[derive(Debug, Deserialize)]
struct ResourceConfig {
    #[serde(default)]
    params: Option<OutParams>,
    source: Source,
    version: Option<Version>,
}
#[derive(Debug, Deserialize)]
struct Source {
    uri: String,
    branch: String,
    private_key: String,
    environment: String,
    #[serde(default = "default_config_path")]
    config: String,
}
#[derive(Debug, Deserialize, Serialize)]
struct Version {
    deployment_no: String,
}

#[derive(Debug, Deserialize)]
struct OutParams {
    repo: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct InReturn {
    version: Version,
}

fn default_config_path() -> String {
    "cepler.yml".to_string()
}
