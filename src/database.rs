use super::git::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
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

    pub fn set_current_environment_state(
        &mut self,
        name: String,
        mut env: DeployState,
    ) -> Result<(), DatabaseError> {
        let any_dirty = env.files.values().any(|f| f.dirty);
        env.any_dirty = any_dirty;
        if let Some(state) = self.state.environments.get_mut(&name) {
            std::mem::swap(&mut state.current, &mut env);
            state.history.push_front(env);
        } else {
            self.state.environments.insert(
                name,
                EnvironmentState {
                    current: env,
                    history: VecDeque::new(),
                },
            );
        }
        self.persist()
    }

    fn persist(&self) -> Result<(), DatabaseError> {
        use std::io::Write;
        let mut file = File::create(&self.state_file)?;
        file.write_all(&serde_yaml::to_vec(&self.state)?)?;
        Ok(())
    }

    pub fn current_environment_state(&self, env: &String) -> Option<&DeployState> {
        self.state.environments.get(env).map(|env| &env.current)
    }

    pub fn get_target_propagated_state(
        &self,
        env: &String,
        propagated_from: &String,
    ) -> Option<&DeployState> {
        match (
            self.state.environments.get(env),
            self.state.environments.get(propagated_from),
        ) {
            (Some(env), Some(from)) => {
                if let Some(from_head) = env.current.propagated_head.as_ref() {
                    if from_head == &from.current.head_commit {
                        Some(&from.current)
                    } else {
                        let (last_idx, _) = from
                            .history
                            .iter()
                            .enumerate()
                            .find(|(_, state)| &state.head_commit == from_head)
                            .expect("Couldn't find state in history");
                        if last_idx >= 1 {
                            Some(&from.history[last_idx - 1])
                        } else {
                            Some(&from.current)
                        }
                    }
                } else {
                    Some(&from.current)
                }
            }
            (None, Some(state)) => Some(&state.current),
            _ => None,
        }
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
    current: DeployState,
    history: VecDeque<DeployState>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeployState {
    pub head_commit: CommitHash,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub propagated_head: Option<CommitHash>,
    #[serde(skip_serializing_if = "is_false")]
    #[serde(default)]
    any_dirty: bool,
    #[serde(default)]
    pub files: BTreeMap<String, FileState>,
}

impl DeployState {
    pub fn new(head_commit: CommitHash) -> Self {
        Self {
            head_commit,
            propagated_head: None,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub from_commit: Option<CommitHash>,
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
