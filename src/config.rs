use anyhow::*;
use glob::*;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub environments: HashMap<String, EnvironmentConfig>,
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path).context("Couldn't open config file")?;
        let reader = BufReader::new(file);

        Self::from_reader(reader)
    }

    pub fn from_reader(reader: impl Read) -> Result<Self> {
        let mut config: Config = serde_yaml::from_reader(reader)?;
        let all_environments: HashSet<String> = config.environments.keys().cloned().collect();
        for (name, mut env) in config.environments.iter_mut() {
            env.name = name.clone();
            if let Some(previous) = env.propagated_from.as_ref() {
                if !all_environments.contains(previous) {
                    return Err(anyhow!("Previous environment '{}' not defined", previous));
                }
            }
        }

        Ok(config)
    }
}

#[derive(Debug)]
pub struct GatesConfig {
    gates: HashMap<String, String>,
}

impl GatesConfig {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path).context("Couldn't open gates file")?;
        let reader = BufReader::new(file);

        Self::from_reader(reader)
    }

    pub fn from_reader(reader: impl Read) -> Result<Self> {
        let gates: HashMap<String, String> = serde_yaml::from_reader(reader)?;

        Ok(GatesConfig { gates })
    }

    pub fn get_gate(mut self, env: &str) -> Result<Option<String>> {
        let gate = self
            .gates
            .remove(env)
            .context("Environment is missing in gates file")?;
        if gate == "HEAD" {
            Ok(None)
        } else {
            Ok(Some(gate))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EnvironmentConfig {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "passed")]
    propagated_from: Option<String>,
    #[serde(rename = "propagated")]
    #[serde(default)]
    propagated_files: Vec<String>,
    #[serde(rename = "latest")]
    #[serde(default)]
    head_files: Vec<String>,
}

impl EnvironmentConfig {
    pub fn propagated_from(&self) -> Option<&String> {
        self.propagated_from.as_ref()
    }

    pub fn propagated_file_patterns(&self) -> impl Iterator<Item = glob::Pattern> {
        self.propagated_files
            .to_vec()
            .into_iter()
            .map(|path| glob::Pattern::new(&path).expect("Couldn't compile glob pattern"))
    }

    pub fn propagated_files(&self) -> impl Iterator<Item = PathBuf> {
        let files: Vec<_> = self.propagated_files.to_vec();
        files
            .into_iter()
            .map(|file| glob(&file).expect("Couldn't resolve glob"))
            .flatten()
            .map(|res| res.expect("Couldn't list file"))
    }

    pub fn head_file_patterns(&self) -> impl Iterator<Item = glob::Pattern> {
        self.head_files
            .to_vec()
            .into_iter()
            .map(|path| glob::Pattern::new(&path).expect("Couldn't compile glob pattern"))
    }
}

#[derive(Debug, Deserialize)]
pub struct RepoConfig {
    pub uri: String,
    pub branch: String,
    pub private_key: String,
}

#[cfg(test)]
mod test {
    use super::*;
    use stringreader::*;

    #[test]
    fn deserialize_config() {
        let conf = r#"environments:
  testflight:
    latest:
    - file.yml"#;

        let conf = Config::from_reader(StringReader::new(conf)).unwrap();
        assert!(&conf.environments.get("testflight").unwrap().name == "testflight");
        assert!(
            conf.environments.get("testflight").unwrap().head_files == vec!["file.yml".to_string()]
        )
    }
}
