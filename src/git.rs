use git2::{build::CheckoutBuilder, ObjectType, Oid, Repository};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileHash(String);
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommitHash(String);

pub fn hash_file<P: AsRef<Path>>(file: P) -> FileHash {
    FileHash(
        Oid::hash_file(ObjectType::Blob, file)
            .expect("Couldn't hash object")
            .to_string(),
    )
}

pub struct Repo {
    inner: Repository,
}

impl Repo {
    pub fn open() -> Result<Self, git2::Error> {
        Ok(Self {
            inner: Repository::open_from_env()?,
        })
    }

    pub fn is_file_dirty(&self, file: &PathBuf) -> Result<bool, git2::Error> {
        Ok(!self.inner.status_file(file.as_path())?.is_empty())
    }

    pub fn head_commit_hash(&self) -> Result<CommitHash, git2::Error> {
        Ok(CommitHash(
            self.inner.head()?.peel_to_commit()?.id().to_string(),
        ))
    }

    pub fn checkout_file_from<'a>(
        &self,
        paths: impl IntoIterator<Item = &'a String>,
        commit: &CommitHash,
    ) -> Result<(), git2::Error> {
        let object = self.inner.find_object(
            Oid::from_str(&commit.0).expect("Couldn't parse Oid"),
            Some(ObjectType::Commit),
        )?;
        let mut checkout = CheckoutBuilder::new();
        for path in paths {
            checkout.path(path);
        }
        self.inner.checkout_tree(&object, Some(&mut checkout))?;

        Ok(())
    }
}
