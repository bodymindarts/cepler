use super::git::*;
use glob::*;
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
    state_dir: &'static str,
}

pub const STATE_DIR: &str = ".cepler";

impl Database {
    pub fn open() -> Result<Self, DatabaseError> {
        let mut state = DbState::default();
        let dir = STATE_DIR;
        if Path::new(&dir).is_dir() {
            for path in glob(&format!("{}/*.state", dir))? {
                let path = path?;
                if let Some(name) = path.as_path().file_stem() {
                    let file = File::open(&path)?;
                    let reader = BufReader::new(file);
                    state.environments.insert(
                        name.to_str().expect("Convert name").to_string(),
                        EnvironmentState::from_reader(reader)?,
                    );
                }
            }
        }

        Ok(Self {
            state,
            state_dir: dir,
        })
    }

    pub fn set_current_environment_state(
        &mut self,
        name: String,
        mut env: DeployState,
    ) -> Result<String, DatabaseError> {
        let any_dirty = env.files.values().any(|f| f.dirty);
        env.any_dirty = any_dirty;
        let ret = format!("{}/{}.state", self.state_dir, &name);
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
        self.persist()?;
        Ok(ret)
    }

    fn persist(&self) -> Result<(), DatabaseError> {
        use std::fs;
        use std::io::Write;
        let _ = fs::remove_dir_all(&self.state_dir);
        fs::create_dir(&self.state_dir)?;
        for (name, env) in self.state.environments.iter() {
            let mut file = File::create(&format!("{}/{}.state", self.state_dir, name))?;
            file.write_all(&serde_yaml::to_vec(&env)?)?;
        }
        Ok(())
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

#[derive(Debug, Serialize, Deserialize)]
pub struct EnvironmentState {
    current: DeployState,
    #[serde(skip_serializing_if = "VecDeque::is_empty")]
    #[serde(default)]
    history: VecDeque<DeployState>,
}

impl EnvironmentState {
    fn from_reader(reader: impl Read) -> Result<Self, DatabaseError> {
        let state = serde_yaml::from_reader(reader)?;
        Ok(state)
    }
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
    pub from_commit: CommitHash,
    pub message: String,
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
    #[error("{0}")]
    PatternError(#[from] PatternError),
    #[error("{0}")]
    GlobError(#[from] GlobError),
}

impl From<git2::Error> for DatabaseError {
    fn from(err: git2::Error) -> Self {
        DatabaseError::GitError(err.message().to_string())
    }
}
