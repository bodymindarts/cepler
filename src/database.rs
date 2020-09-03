use super::git::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
use thiserror::Error;

pub struct Database {
    state: DbState,
    state_file: String,
}

impl Database {
    pub fn open(file: String) -> Result<Self, DatabaseError> {
        let path = Path::new(&file);
        let state = if path.exists() {
            let file = File::open(path)?;
            let reader = BufReader::new(file);

            DbState::from_reader(reader)?
        } else {
            DbState::default()
        };
        Ok(Self {
            state,
            state_file: file,
        })
    }

    pub fn set_environment_state(
        &mut self,
        name: String,
        mut env: EnvironmentState,
    ) -> Result<(), DatabaseError> {
        let any_dirty = env.files.values().any(|f| f.dirty);
        env.any_dirty = any_dirty;
        self.state.environments.insert(name, env);
        self.persist()
    }

    fn persist(&self) -> Result<(), DatabaseError> {
        use std::io::Write;
        let mut file = File::create(&self.state_file)?;
        file.write_all(&serde_yaml::to_vec(&self.state)?)?;
        Ok(())
    }

    pub fn environment(&self, env: &String) -> Option<&EnvironmentState> {
        self.state.environments.get(env)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DbState {
    environments: BTreeMap<String, EnvironmentState>,
}

impl DbState {
    fn from_reader(reader: impl Read) -> Result<Self, DatabaseError> {
        let state = serde_yaml::from_reader(reader)?;
        Ok(state)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EnvironmentState {
    pub head_commit: CommitHash,
    #[serde(skip_serializing_if = "is_false")]
    #[serde(default)]
    any_dirty: bool,
    #[serde(default)]
    pub files: BTreeMap<String, FileState>,
}

impl EnvironmentState {
    pub fn new(head_commit: CommitHash) -> Self {
        Self {
            head_commit,
            any_dirty: false,
            files: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileState {
    pub file_hash: FileHash,
    #[serde(skip_serializing_if = "is_false")]
    #[serde(default)]
    pub dirty: bool,
}

fn is_false(b: &bool) -> bool {
    !b
}

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("Could not read the database file: '{0}'")]
    UnknownFormat(#[from] serde_yaml::Error),
    #[error("Could not open the database file: '{0}'")]
    CouldNotOpen(#[from] std::io::Error),
    #[error("Error interfacing with git: '{0}'")]
    GitError(String),
}

impl From<git2::Error> for DatabaseError {
    fn from(err: git2::Error) -> Self {
        DatabaseError::GitError(err.message().to_string())
    }
}
