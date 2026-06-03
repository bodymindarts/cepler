use super::config::{MATCH_OPTIONS, default_scope};
use anyhow::{Context, Result};
use git2::{
    BranchType, CertificateCheckStatus, Commit, Cred, MergeOptions, Object, ObjectType, Oid,
    PushOptions, RebaseOptions, RemoteCallbacks, Repository, ResetType, Signature, TreeWalkMode,
    TreeWalkResult, build::CheckoutBuilder,
};
use glob::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    fmt,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileHash(String);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommitHash(String);
impl fmt::Display for CommitHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.chars().take(7).collect::<String>())
    }
}
impl CommitHash {
    pub fn inner(self) -> String {
        self.0
    }

    pub fn to_short_ref(&self) -> String {
        self.0.chars().take(7).collect()
    }
}

pub fn hash_file<P: AsRef<Path>>(file: P) -> Option<FileHash> {
    let path = file.as_ref();
    if path.is_file() {
        Some(FileHash(
            Oid::hash_file(ObjectType::Blob, path)
                .expect("Couldn't hash object")
                .to_string(),
        ))
    } else {
        None
    }
}

pub struct GitConfig {
    pub url: String,
    pub branch: String,
    pub gates_branch: Option<String>,
    pub private_key: String,
    pub dir: String,
    /// Shallow-clone depth. `None` (or `Some(0)`) means a full clone, matching
    /// the concourse git-resource's `depth` source param semantics.
    pub depth: Option<i32>,
}

pub struct Repo {
    inner: Repository,
    gate: Option<Oid>,
    /// Credentials + branch info kept around so we can deepen a shallow clone
    /// on-demand when a history walk reaches the shallow boundary. Only the
    /// clone path on the `in` step populates this; `Repo::open` (used by `out`)
    /// leaves it `None` since the workspace there came from `in` and shouldn't
    /// need further deepening.
    fetch_credentials: Option<FetchCredentials>,
}

#[derive(Clone)]
pub struct FetchCredentials {
    branch: String,
    private_key: String,
}

impl Repo {
    pub fn clone(
        GitConfig {
            url,
            branch,
            private_key,
            dir,
            depth,
            ..
        }: GitConfig,
    ) -> Result<Self> {
        let callbacks = remote_callbacks(private_key.clone());
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(callbacks);
        let shallow = depth.filter(|d| *d > 0);
        if let Some(d) = shallow {
            fo.depth(d);
        }

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fo);
        builder.branch(&branch);
        let inner = builder.clone(&url, Path::new(&dir))?;
        let fetch_credentials = shallow.map(|_| FetchCredentials {
            branch,
            private_key,
        });
        Ok(Self {
            inner,
            gate: None,
            fetch_credentials,
        })
    }

    pub fn pull(
        &self,
        GitConfig {
            branch,
            gates_branch,
            private_key,
            ..
        }: GitConfig,
    ) -> Result<()> {
        let callbacks = remote_callbacks(private_key);
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(callbacks);
        let mut remote = self.inner.find_remote("origin")?;
        let mut branches = vec![branch.clone()];
        if let Some(gates) = gates_branch {
            branches.push(gates);
        }
        remote.fetch(&branches, Some(&mut fo), None)?;
        let suffix = format!("/{}", branch);
        let remote_head = remote
            .list()?
            .iter()
            .find(|head| head.name().ends_with(&suffix))
            .context("Cannot find head")?;
        let object = self
            .inner
            .find_object(remote_head.oid(), Some(ObjectType::Commit))?;
        self.inner.reset(&object, ResetType::Hard, None)?;
        Ok(())
    }

    pub fn push(&self, config: GitConfig) -> Result<bool> {
        const MAX_PUSH_ATTEMPTS: usize = 5;
        let GitConfig {
            branch,
            private_key,
            ..
        } = config;

        let mut attempt = 0;
        loop {
            attempt += 1;
            // Skip the pre-push fetch on the first attempt: the workspace was
            // freshly cloned/pulled milliseconds ago by `in`/`check`, so
            // refreshing origin again is wasted bandwidth in the happy path.
            // On a `NotFastForward` retry we fall back to fetching so the
            // rebase has up-to-date upstream.
            let refresh_remote = attempt > 1;
            match self.try_push(&branch, &private_key, refresh_remote) {
                Ok(pushed) => return Ok(pushed),
                Err(e) => {
                    let non_fast_forward = e
                        .downcast_ref::<git2::Error>()
                        .map(|git_err| git_err.code() == git2::ErrorCode::NotFastForward)
                        .unwrap_or(false);
                    if non_fast_forward && attempt < MAX_PUSH_ATTEMPTS {
                        eprintln!(
                            "Push rejected because the remote moved; re-fetching and retrying ({}/{})",
                            attempt, MAX_PUSH_ATTEMPTS
                        );
                        std::thread::sleep(std::time::Duration::from_millis(200 * attempt as u64));
                        continue;
                    }
                    return Err(e).context("Couldn't push to remote");
                }
            }
        }
    }

    fn try_push(&self, branch: &str, private_key: &str, refresh_remote: bool) -> Result<bool> {
        let mut remote = self.inner.find_remote("origin")?;
        if refresh_remote {
            let callbacks = remote_callbacks(private_key.to_string());
            let mut fo = git2::FetchOptions::new();
            fo.remote_callbacks(callbacks);
            remote
                .fetch(&[branch], Some(&mut fo), None)
                .context("Couldn't fetch origin")?;
        }

        let annotated_head = self
            .inner
            .reference_to_annotated_commit(&self.inner.head()?)
            .context("Couldn't find head reference")?;

        let head_commit = match self.inner.find_branch(branch, BranchType::Local) {
            Ok(local) if local.is_head() => annotated_head,
            _ => {
                let branch_ref = self
                    .inner
                    .branch_from_annotated_commit(branch, &annotated_head, true)
                    .context("Could not create local branch")?;
                self.inner.reference_to_annotated_commit(branch_ref.get())?
            }
        };

        let remote_ref = self
            .inner
            .resolve_reference_from_short_name(&format!("origin/{}", branch))
            .context("Couldn't resolve remote branch")?;
        let remote_commit = self
            .inner
            .reference_to_annotated_commit(&remote_ref)
            .context("Couldn't get remote commit")?;

        let mut rebase_options = RebaseOptions::new();
        let mut merge_options = MergeOptions::new();
        merge_options.fail_on_conflict(true);
        rebase_options.merge_options(merge_options);
        let mut rebase = self.inner.rebase(
            Some(&head_commit),
            Some(&remote_commit),
            None,
            Some(&mut rebase_options),
        )?;
        let sig = Signature::now("Cepler", "bot@cepler.dev")?;

        let mut n_applied = 0;
        while let Some(_) = rebase.next() {
            let res = rebase.commit(None, &sig, None);
            if matches!(res.as_ref(), Err(e) if e.code() == git2::ErrorCode::Applied) {
                continue;
            }
            n_applied += 1;
            res.context("Couldn't commit rebase")?;
        }
        rebase.finish(None).context("Couldn't finish rebase")?;

        if n_applied > 0 {
            let mut push_options = PushOptions::new();
            push_options.remote_callbacks(remote_callbacks(private_key.to_string()));
            let refname = head_commit
                .refname()
                .context("Annotated commit has no reference name")?;
            remote.push(
                &[format!("{}:{}", refname, refname)],
                Some(&mut push_options),
            )?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn open(gate: Option<String>) -> Result<Self> {
        let inner = Repository::open_from_env()?;
        let gate = if let Some(gate) = gate {
            let commit = Oid::from_str(&gate).context("Gate is not a valid commit hash")?;
            inner
                .find_commit(commit)
                .context("Gate commit doesn't exist")?;
            Some(commit)
        } else {
            None
        };
        Ok(Self {
            inner,
            gate,
            fetch_credentials: None,
        })
    }

    /// Borrow the credentials a shallow `Repo::clone` was given. Workspace
    /// methods that re-open the repo via `Repo::open` use this to carry the
    /// deepen-on-demand capability across the boundary.
    pub fn fetch_credentials(&self) -> Option<&FetchCredentials> {
        self.fetch_credentials.as_ref()
    }

    pub fn with_fetch_credentials(mut self, creds: Option<FetchCredentials>) -> Self {
        self.fetch_credentials = creds;
        self
    }

    /// Returns true when the local repo is a shallow clone and `commit` sits
    /// exactly on the shallow boundary — i.e. its real parents exist on the
    /// remote but were not fetched locally.
    fn is_shallow_boundary(&self, commit: Oid) -> bool {
        if !self.inner.is_shallow() {
            return false;
        }
        let shallow_path = self.inner.path().join("shallow");
        let Ok(contents) = std::fs::read_to_string(&shallow_path) else {
            return false;
        };
        let needle = commit.to_string();
        contents.lines().any(|line| line.trim() == needle)
    }

    /// Deepen the shallow clone by re-fetching with a larger depth. Returns
    /// `true` if the boundary actually moved (i.e. new history is now available),
    /// `false` when we have no credentials, the repo isn't shallow, or the
    /// fetch failed.
    fn deepen(&self, new_depth: i32) -> bool {
        let Some(creds) = self.fetch_credentials.as_ref() else {
            return false;
        };
        if !self.inner.is_shallow() {
            return false;
        }
        let callbacks = remote_callbacks(creds.private_key.clone());
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(callbacks);
        fo.depth(new_depth);
        let Ok(mut remote) = self.inner.find_remote("origin") else {
            return false;
        };
        eprintln!(
            "Hit shallow boundary; deepening clone to depth {}",
            new_depth
        );
        remote
            .fetch(&[creds.branch.as_str()], Some(&mut fo), None)
            .is_ok()
    }

    pub fn commit_state_file(&self, scope: &str, file_name: String) -> Result<()> {
        let path = Path::new(&file_name);
        let mut index = self.inner.index()?;
        index.add_path(path)?;
        let oid = index.write_tree()?;
        let tree = self.inner.find_tree(oid)?;
        let sig = Signature::now("Cepler", "bot@cepler.io")?;

        let head_commit = self.inner.head().unwrap().peel_to_commit().unwrap();
        let msg = if scope != default_scope() {
            format!(
                "ci(cepler): Updated '{}' state in '{}'",
                scope,
                path.file_stem().unwrap().to_str().unwrap()
            )
        } else {
            format!(
                "ci(cepler): Updated '{}' state",
                path.file_stem().unwrap().to_str().unwrap()
            )
        };
        self.inner
            .commit(Some("HEAD"), &sig, &sig, &msg, &tree, &[&head_commit])?;
        let mut checkout = CheckoutBuilder::new();
        checkout.path(path);
        self.inner.checkout_index(None, Some(&mut checkout))?;
        Ok(())
    }

    fn gate_files_matching<'a>(
        &self,
        globs: &'a [Pattern],
        ignore_files: &'a [Pattern],
    ) -> impl Iterator<Item = PathBuf> + 'a {
        let ignore = move |file: &Path| {
            ignore_files
                .iter()
                .any(|p| p.matches_path_with(file, MATCH_OPTIONS))
        };
        let includes = move |file: &Path| {
            globs
                .iter()
                .any(|p| p.matches_path_with(file, MATCH_OPTIONS))
        };
        let mut paths = Vec::new();
        self.all_files(self.gate_commit_hash(), |_, path| {
            if !ignore(path) && includes(path) {
                paths.push(path.to_path_buf())
            }
            Ok(())
        })
        .expect("Couldn't list gate files");
        paths.into_iter()
    }

    pub fn all_files<F>(&self, commit: CommitHash, mut f: F) -> Result<()>
    where
        F: FnMut(FileHash, &Path) -> Result<()>,
    {
        let commit = Oid::from_str(&commit.0).expect("Couldn't parse commit hash");
        let commit = self.inner.find_commit(commit)?;
        let tree = commit.tree().context("Couldn't resolve tree")?;
        let mut ret = Ok(());
        tree.walk(TreeWalkMode::PreOrder, |dir, entry| {
            let path_name = format!("{}{}", dir, entry.name().expect("Entry has no name"));
            let path = Path::new(&path_name);
            if !matches!(entry.kind(), Some(ObjectType::Blob)) {
                return TreeWalkResult::Ok;
            }
            if let Err(e) = f(FileHash(entry.id().to_string()), path) {
                ret = Err(e);
                return TreeWalkResult::Abort;
            }
            TreeWalkResult::Ok
        })?;
        ret
    }

    fn is_trackable_file(&self, file: &Path) -> bool {
        if self.inner.status_file(file).is_err() {
            return false;
        }
        !self
            .inner
            .status_should_ignore(file)
            .expect("Cannot check ignore status")
    }

    pub fn gate_commit_hash(&self) -> CommitHash {
        CommitHash(self.gate_oid().to_string())
    }

    pub fn head_commit_summary(&self) -> Result<(CommitHash, String)> {
        let commit = self.inner.head().unwrap().peel_to_commit().unwrap();
        Ok((
            CommitHash(commit.id().to_string()),
            commit.summary().expect("Couldn't get summary").to_string(),
        ))
    }

    pub fn checkout_file_from(&self, path: &str, commit: &CommitHash) -> Result<()> {
        let object = self.inner.find_object(
            Oid::from_str(&commit.0).expect("Couldn't parse Oid"),
            Some(ObjectType::Commit),
        )?;
        let mut checkout = CheckoutBuilder::new();
        checkout.force();
        checkout.path(path);
        checkout.update_index(false);
        self.inner.checkout_tree(&object, Some(&mut checkout))?;

        Ok(())
    }

    pub fn checkout_gate(
        &self,
        globs: &[Pattern],
        ignore_files: &[Pattern],
        clean: bool,
    ) -> Result<()> {
        let mut checkout = CheckoutBuilder::new();
        checkout.force();
        checkout.update_index(false);
        let mut path_added = false;
        for path in self.gate_files_matching(globs, ignore_files) {
            path_added = true;
            checkout.path(path);
        }

        for path in glob("**/*").expect("List all files") {
            let path = path.expect("Get file");
            if self.is_trackable_file(&path) {
                let path = path.as_path();
                let check = |p: &glob::Pattern| {
                    p.matches_path_with(
                        path,
                        glob::MatchOptions {
                            case_sensitive: true,
                            require_literal_separator: true,
                            require_literal_leading_dot: true,
                        },
                    )
                };
                if !ignore_files.iter().any(check)
                    && path.is_file()
                    && (clean || globs.iter().any(check))
                {
                    std::fs::remove_file(path).expect("Couldn't remove file");
                }
            }
        }
        if path_added {
            self.inner
                .checkout_tree(&self.gate_object(), Some(&mut checkout))
                .expect("Couldn't checkout");
        }
        Ok(())
    }

    pub fn checkout_head(&self) -> Result<()> {
        let mut checkout = CheckoutBuilder::new();
        checkout.force();
        checkout.update_index(false);
        self.inner.checkout_head(Some(&mut checkout))?;
        Ok(())
    }

    pub fn walk_commits_before<F>(&self, commit: CommitHash, mut cb: F) -> Result<()>
    where
        F: FnMut(CommitHash) -> Result<bool>,
    {
        let commit = Oid::from_str(&commit.0).expect("Couldn't parse commit hash");
        let mut commit = self.inner.find_commit(commit)?;
        let mut set = HashSet::new();
        let mut queue = VecDeque::new();
        set.insert(commit.id());
        self.deepen_if_needed(&mut commit);
        for parent in commit.parents() {
            if set.insert(parent.id()) {
                queue.push_back(parent);
            }
        }
        loop {
            if queue.is_empty() {
                break;
            }
            let mut commit = queue.pop_front().unwrap();
            if !cb(CommitHash(commit.id().to_string()))? {
                break;
            }
            self.deepen_if_needed(&mut commit);
            for parent in commit.parents() {
                if set.insert(parent.id()) {
                    queue.push_back(parent);
                }
            }
        }
        Ok(())
    }

    /// If `commit` sits on the shallow boundary, repeatedly double the clone
    /// depth (re-fetching) until either parents become visible, deepening
    /// fails, or we hit a sanity cap. Re-binds `commit` after each fetch so its
    /// `.parents()` reflects the new history.
    fn deepen_if_needed<'a>(&'a self, commit: &mut Commit<'a>) {
        const INITIAL_DEPTH: i32 = 100;
        const MAX_DEPTH: i32 = 100_000;
        if !self.is_shallow_boundary(commit.id()) {
            return;
        }
        let mut next_depth = INITIAL_DEPTH;
        while self.is_shallow_boundary(commit.id()) && next_depth <= MAX_DEPTH {
            if !self.deepen(next_depth) {
                return;
            }
            // Re-bind so parents() sees the freshly fetched history.
            if let Ok(refreshed) = self.inner.find_commit(commit.id()) {
                *commit = refreshed;
            }
            next_depth = next_depth.saturating_mul(2);
        }
    }

    pub fn find_last_changed_commit(
        &self,
        file: &Path,
        from_commit: CommitHash,
    ) -> Result<(CommitHash, String)> {
        let commit = Oid::from_str(&from_commit.0).expect("Couldn't parse commit hash");
        let commit = self.inner.find_commit(commit)?;
        let target = commit
            .tree()
            .context("Couldn't resolve tree")?
            .get_path(file)
            .context("Trying to record uncommitted file")?;
        let mut set = HashSet::new();
        let mut queue = VecDeque::new();
        set.insert(commit.id());
        queue.push_back(commit);

        loop {
            let mut commit = queue.pop_front().unwrap();
            self.deepen_if_needed(&mut commit);
            let mut go = false;
            for parent in commit.parents() {
                if let Ok(tree) = parent.tree().expect("Couldn't get tree").get_path(file) {
                    let eq = tree.id() == target.id();
                    if eq && set.insert(parent.id()) {
                        queue.push_back(parent);
                    }
                    go = go || eq;
                }
            }
            if !go || queue.is_empty() {
                return Ok((
                    CommitHash(commit.id().to_string()),
                    commit.summary().expect("Couldn't get summary").to_string(),
                ));
            }
        }
    }

    pub fn get_file_content<F, T>(&self, commit: CommitHash, file: &Path, f: F) -> Result<Option<T>>
    where
        F: Fn(&[u8]) -> Result<T>,
    {
        let commit = Oid::from_str(&commit.0).expect("Couldn't parse commit hash");
        let commit = self.inner.find_commit(commit)?;
        self.get_file_from_commit(commit, file, f)
    }

    fn get_file_from_commit<F, T>(&self, commit: Commit, file: &Path, f: F) -> Result<Option<T>>
    where
        F: Fn(&[u8]) -> Result<T>,
    {
        let tree = commit.tree().context("Couldn't resolve tree")?;
        let target = if let Ok(target) = tree.get_path(file) {
            target
        } else {
            return Ok(None);
        };
        let object = target
            .to_object(&self.inner)
            .context("Couldn't create object")?;
        let blob = object.peel_to_blob().context("Couldn't peel to blob")?;
        Ok(Some(f(blob.content())?))
    }

    fn gate_commit(&self) -> Commit<'_> {
        if let Some(gate) = self.gate {
            self.inner.find_commit(gate).unwrap()
        } else {
            self.inner.head().unwrap().peel_to_commit().unwrap()
        }
    }

    fn gate_oid(&self) -> Oid {
        self.gate_commit().id()
    }

    fn gate_object(&self) -> Object<'_> {
        self.inner
            .find_object(self.gate_oid(), Some(ObjectType::Commit))
            .unwrap()
    }

    pub fn get_file_from_branch<F, T>(&self, name: &str, file: &Path, f: F) -> Result<Option<T>>
    where
        F: Fn(&[u8]) -> Result<T>,
    {
        let branch = if let Ok(branch) = self.inner.find_branch(name, BranchType::Local) {
            branch
        } else {
            self.inner
                .find_branch(&format!("origin/{}", name), BranchType::Remote)
                .context("Couldn't find branch")?
        };

        self.get_file_from_commit(branch.into_reference().peel_to_commit()?, file, f)
    }
}

fn remote_callbacks(key: String) -> RemoteCallbacks<'static> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(move |_url, username_from_url, _allowed_types| {
        Cred::ssh_key_from_memory(username_from_url.unwrap(), None, &key, None)
    });
    // libgit2 1.7+ (pulled in by git2 0.20) rejects unknown SSH host keys
    // with `invalid or unknown remote ssh hostkey; class=Ssh (23); code=-17`
    // unless the caller installs a certificate_check callback. The
    // pre-bump cepler (git2 0.13 / libgit2 1.3.x) silently accepted any
    // host key — concourse pipelines run against a known git remote and
    // the user's SSH private key already authenticates the connection,
    // so restore the prior behaviour by explicitly accepting the host
    // certificate. Returning `CertificatePassthrough` would defer to
    // libgit2's built-in check, which without a known_hosts file at a
    // path libgit2 looks for will hard-reject every host.
    callbacks.certificate_check(|_cert, _host| Ok(CertificateCheckStatus::CertificateOk));
    callbacks
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;

    fn unique_tmp_dir(tag: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        dir.push(format!(
            "cepler-test-{}-{}-{}",
            tag,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn stage_and_commit(repo: &Repository, msg: &str) -> Oid {
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = Signature::now("Test", "test@example.com").unwrap();
        let parents: Vec<Commit> = repo
            .head()
            .ok()
            .and_then(|h| h.peel_to_commit().ok())
            .into_iter()
            .collect();
        let parent_refs: Vec<&Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parent_refs)
            .unwrap()
    }

    fn push_branch(repo: &Repository, branch: &str) {
        let mut remote = repo.find_remote("origin").unwrap();
        remote
            .push(&[format!("refs/heads/{0}:refs/heads/{0}", branch)], None)
            .unwrap();
    }

    #[test]
    fn push_rebases_state_commit_onto_concurrent_remote_commit() {
        let base = unique_tmp_dir("push");
        let bare_path = base.join("remote.git");
        let work_a = base.join("work_a");
        let work_b = base.join("work_b");
        let bare_url = bare_path.to_str().unwrap().to_string();

        Repository::init_bare(&bare_path).unwrap();
        let repo_a = Repository::init(&work_a).unwrap();
        repo_a.remote("origin", &bare_url).unwrap();
        std::fs::write(work_a.join("file.txt"), "v1").unwrap();
        stage_and_commit(&repo_a, "initial commit");
        let branch = repo_a.head().unwrap().shorthand().unwrap().to_string();
        push_branch(&repo_a, &branch);

        let repo_b = Repository::clone(&bare_url, &work_b).unwrap();
        std::fs::write(work_b.join("other.txt"), "concurrent").unwrap();
        let concurrent = stage_and_commit(&repo_b, "concurrent writer commit");
        push_branch(&repo_b, &branch);

        std::fs::write(work_a.join("state.txt"), "cepler state").unwrap();
        stage_and_commit(&repo_a, "ci(cepler): Updated state");

        let repo = Repo {
            inner: repo_a,
            gate: None,
            fetch_credentials: None,
        };
        let config = GitConfig {
            url: bare_url.clone(),
            branch: branch.clone(),
            gates_branch: None,
            private_key: String::new(),
            dir: work_a.to_str().unwrap().to_string(),
            depth: None,
        };

        let pushed = repo.push(config).expect("push should succeed after rebase");
        assert!(pushed, "expected the state commit to be pushed");

        let verify = Repository::clone(&bare_url, base.join("verify")).unwrap();
        let tip = verify.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(tip.summary().unwrap(), "ci(cepler): Updated state");
        assert_eq!(
            tip.parent(0).unwrap().id(),
            concurrent,
            "state commit must be rebased onto the concurrent commit"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn is_shallow_boundary_detects_commits_listed_in_shallow_file() {
        // libgit2's `local` transport doesn't support shallow fetches, so we
        // can't end-to-end-test the deepen path in unit tests (it requires a
        // real ssh/https remote). Instead, simulate the on-disk state a
        // shallow clone leaves behind and verify the boundary detector
        // correctly identifies which commits sit on the shallow edge — that
        // detector is what gates `deepen_if_needed` from looping forever on a
        // genuine root commit.
        let base = unique_tmp_dir("shallow-boundary");
        let work = base.join("work");
        let bare_path = base.join("remote.git");
        let bare_url = bare_path.to_str().unwrap().to_string();

        Repository::init_bare(&bare_path).unwrap();
        let seed = Repository::init(&work).unwrap();
        seed.remote("origin", &bare_url).unwrap();

        std::fs::write(work.join("file.txt"), "v0").unwrap();
        let root_oid = stage_and_commit(&seed, "root");
        let branch = seed.head().unwrap().shorthand().unwrap().to_string();
        push_branch(&seed, &branch);

        std::fs::write(work.join("file.txt"), "v1").unwrap();
        let tip_oid = stage_and_commit(&seed, "tip");
        push_branch(&seed, &branch);

        // Hand-craft `.git/shallow` to declare the tip as a boundary, the way
        // a real `git clone --depth 1` would.
        std::fs::write(seed.path().join("shallow"), format!("{}\n", tip_oid)).unwrap();
        // git2 needs a hint to re-evaluate shallow state — reopen the repo.
        let inner = Repository::open(&work).unwrap();
        assert!(
            inner.is_shallow(),
            "writing .git/shallow should make is_shallow() return true"
        );

        let repo = Repo {
            inner,
            gate: None,
            fetch_credentials: None,
        };

        assert!(
            repo.is_shallow_boundary(tip_oid),
            "tip is listed in .git/shallow and must be detected as a boundary"
        );
        assert!(
            !repo.is_shallow_boundary(root_oid),
            "root is NOT listed in .git/shallow — must not be flagged as a boundary"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn first_push_attempt_skips_remote_fetch() {
        // Build a remote with two commits and a clone that only knows the
        // first one. If `try_push` were still doing a pre-push fetch, our
        // local view would jump forward and the push would succeed.
        // Skipping the fetch means we still think origin is at commit #1,
        // so a non-fast-forward push fails (without ever touching the network).
        let base = unique_tmp_dir("first-push-no-fetch");
        let bare_path = base.join("remote.git");
        let work_a = base.join("work_a");
        let work_b = base.join("work_b");
        let bare_url = bare_path.to_str().unwrap().to_string();

        Repository::init_bare(&bare_path).unwrap();
        let repo_a = Repository::init(&work_a).unwrap();
        repo_a.remote("origin", &bare_url).unwrap();
        std::fs::write(work_a.join("file.txt"), "v1").unwrap();
        stage_and_commit(&repo_a, "initial commit");
        let branch = repo_a.head().unwrap().shorthand().unwrap().to_string();
        push_branch(&repo_a, &branch);

        // Clone B from origin (sees commit #1).
        let repo_b = Repository::clone(&bare_url, &work_b).unwrap();

        // Worker A pushes commit #2 to origin out-of-band.
        std::fs::write(work_a.join("file.txt"), "v2").unwrap();
        stage_and_commit(&repo_a, "out of band commit");
        push_branch(&repo_a, &branch);

        // Worker B records its own cepler state on top of its stale view…
        std::fs::write(work_b.join("state.txt"), "cepler state").unwrap();
        stage_and_commit(&repo_b, "ci(cepler): Updated state");

        // Point an unreachable URL to force a network error if any
        // attempt re-fetches. The first attempt should still succeed at the
        // rebase step (it never fetches) and only the push itself can fail.
        let unreachable = "ssh://invalid.invalid/does-not-exist.git";
        repo_b.remote_set_url("origin", unreachable).unwrap();

        let repo = Repo {
            inner: repo_b,
            gate: None,
            fetch_credentials: None,
        };

        // Single-attempt try_push with refresh_remote = false. Skipping the
        // fetch must not error out before reaching the push (the rebase uses
        // the stale local origin/<branch> ref). The push itself will fail
        // because the URL is bogus, which is fine — we're asserting we made
        // it past the fetch.
        let result = repo.try_push(&branch, "", false);
        let err = result.expect_err("push to unreachable remote should fail");
        let msg = format!("{:#}", err);
        assert!(
            !msg.contains("Couldn't fetch origin"),
            "first attempt must skip pre-push fetch, got: {msg}"
        );

        std::fs::remove_dir_all(&base).ok();
    }
}
