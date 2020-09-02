use anyhow::anyhow;
use glob::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub environments: HashMap<String, EnvironmentConfig>,
    pub hook: HookConf,
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, anyhow::Error> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        Self::from_reader(reader)
    }

    fn from_reader(reader: impl Read) -> Result<Self, anyhow::Error> {
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

#[derive(Debug, Serialize, Deserialize)]
pub struct HookConf {
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EnvironmentConfig {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "passed")]
    propagated_from: Option<String>,
    #[serde(default)]
    propagated_files: Vec<String>,
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

    pub fn all_files(&self) -> impl Iterator<Item = PathBuf> {
        let files: Vec<_> = self
            .head_files
            .iter()
            .chain(self.propagated_files.iter())
            .cloned()
            .collect();
        files
            .into_iter()
            .map(|file| glob(&file).expect("Couldn't resolve glob"))
            .flatten()
            .map(|res| res.expect("Couldn't list file"))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use stringreader::*;

    #[test]
    fn deserialize_config() {
        let conf = r#"environments:
  testflight:
    head_files:
    - file.yml
hook:
  cmd: "ls""#;

        let conf = Config::from_reader(StringReader::new(conf)).unwrap();
        assert!(&conf.environments.get("testflight").unwrap().name == "testflight");
        assert!(
            conf.environments.get("testflight").unwrap().head_files == vec!["file.yml".to_string()]
        )
    }
}
