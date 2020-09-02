use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub environments: HashMap<String, EnvironmentConf>,
    pub hook: HookConf,
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Config, anyhow::Error> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        Self::from_reader(reader)
    }

    fn from_reader(reader: impl Read) -> Result<Config, anyhow::Error> {
        let mut config: Config = serde_yaml::from_reader(reader)?;
        for (name, mut env) in config.environments.iter_mut() {
            env.name = name.clone();
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
pub struct EnvironmentConf {
    #[serde(default)]
    name: String,
    #[serde(default)]
    head_files: Vec<String>,
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
