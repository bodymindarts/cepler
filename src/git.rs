use git2::{build::CheckoutBuilder, ObjectType, Oid, Repository};
use glob::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileHash(String);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

    pub fn head_files(&self, filters: &[String]) -> impl Iterator<Item = PathBuf> + '_ {
        let mut opts = MatchOptions::new();
        opts.require_literal_leading_dot = true;
        let files: Vec<_> = filters
            .iter()
            .map(move |files| glob_with(&files, opts).expect("Couldn't resolve glob"))
            .flatten()
            .map(|res| res.expect("Couldn't list file"))
            .collect();
        let repo = Self::open().expect("Couldn't re-open repo");
        files.into_iter().filter_map(move |file| {
            if repo.is_trackable_file(&file) {
                Some(file)
            } else {
                None
            }
        })
    }

    fn is_trackable_file(&self, file: &PathBuf) -> bool {
        let path = file.as_path();
        if self.inner.status_file(path).is_err() {
            return false;
        }
        !self
            .inner
            .status_should_ignore(path)
            .expect("Cannot check ignore status")
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
        path: &str,
        commit: &CommitHash,
    ) -> Result<(), git2::Error> {
        let object = self.inner.find_object(
            Oid::from_str(&commit.0).expect("Couldn't parse Oid"),
            Some(ObjectType::Commit),
        )?;
        let mut checkout = CheckoutBuilder::new();
        checkout.force();
        checkout.path(path);
        self.inner.checkout_tree(&object, Some(&mut checkout))?;

        Ok(())
    }

    pub fn checkout_head(&self) -> Result<(), git2::Error> {
        let mut checkout = CheckoutBuilder::new();
        checkout.force();
        Ok(self.inner.checkout_head(Some(&mut checkout))?)
    }
}
