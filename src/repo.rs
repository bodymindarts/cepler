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
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

        // Skip libgit2's implicit working-tree checkout. For repos with
        // many files it dominates clone wall-clock time (~2-3 minutes on
        // the volcano-qa state repo) and emits no progress, so it looks
        // like a hang. Callers materialise just the files they actually
        // need via `checkout_paths` (concourse `in`/`check`) or
        // `checkout_head` (CLI), which is orders of magnitude faster than
        // writing every blob in HEAD.
        let mut no_checkout = git2::build::CheckoutBuilder::new();
        no_checkout.dry_run();

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fo);
        builder.branch(&branch);
        builder.with_checkout(no_checkout);
        let inner = builder.clone(&url, Path::new(&dir))?;

        // libgit2 short-circuits both working-tree AND index population
        // when checkout_strategy == GIT_CHECKOUT_NONE (see clone.c
        // `should_checkout`). The index needs to be in sync with HEAD so
        // that a later `index.write_tree()` from `commit_state_file` on
        // the `out` step produces a tree that still includes everything
        // inherited from HEAD — without this step `out` would commit a
        // tree containing only the new state file, deleting the rest of
        // the repo. ResetType::Mixed fills the index from HEAD without
        // touching the working tree.
        {
            let head_commit = inner.head()?.peel_to_commit()?;
            let head_obj = head_commit.into_object();
            inner.reset(&head_obj, ResetType::Mixed, None)?;
        }

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

    /// Fetch the given commits into the local ODB with `depth=1` — just the
    /// trees and blobs reachable from each commit, no history before it.
    /// Used by `Workspace::prepare` to materialise propagated files whose
    /// `from_commit` is recorded in the previous env's state file. Those
    /// commits can sit arbitrarily far back in the deploy branch's history
    /// (5k+ commits for the volcano-qa lana-bank state branch), well past
    /// any reasonable shallow clone depth — progressive deepening would
    /// take many round-trips guessing the right depth, whereas fetching by
    /// OID gets exactly what's needed in one.
    ///
    /// Requires the server to advertise the `tip-oid` or `reachable-oid`
    /// capability — GitHub does both by default; self-hosted servers need
    /// `uploadpack.allow{Any,Reachable}Sha1InWant=true`. If unsupported,
    /// libgit2 surfaces a clear "cannot fetch a specific object from the
    /// remote repository" error.
    ///
    /// No-op when:
    /// - the repo wasn't shallow-cloned (we have no `fetch_credentials`),
    /// - the commits are already in the local ODB.
    pub fn fetch_specific_commits(&self, commits: &[&CommitHash]) -> Result<()> {
        let Some(creds) = self.fetch_credentials.as_ref() else {
            // Full clone — every commit reachable from HEAD is already
            // local. If a caller passes in an unreachable OID we'd
            // surface that via `checkout_file_from`'s own `find_object`,
            // which is the right place to report it.
            return Ok(());
        };
        let mut to_fetch: Vec<String> = Vec::new();
        for hash in commits {
            let oid = Oid::from_str(&hash.0)
                .with_context(|| format!("invalid commit hash: {}", hash.0))?;
            if self
                .inner
                .find_object(oid, Some(ObjectType::Commit))
                .is_err()
            {
                to_fetch.push(hash.0.clone());
            }
        }
        if to_fetch.is_empty() {
            return Ok(());
        }
        eprintln!(
            "Fetching {} commit(s) referenced by propagated state",
            to_fetch.len()
        );
        let callbacks = remote_callbacks(creds.private_key.clone());
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(callbacks);
        fo.depth(1);
        let mut remote = self.inner.find_remote("origin")?;
        let refspec_refs: Vec<&str> = to_fetch.iter().map(|s| s.as_str()).collect();
        remote
            .fetch(&refspec_refs, Some(&mut fo), None)
            .context("Couldn't fetch propagated commits by OID")?;
        Ok(())
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

    /// Selectively materialise specific paths from HEAD into the working
    /// tree. Patterns follow libgit2's fnmatch-style semantics — `*`
    /// inside a single directory component, no `**` recursion — so a
    /// state directory is expressed as `dir/*` (the state files are flat
    /// under it). Used by the concourse `in`/`check` flows after a
    /// no-checkout clone to populate just the files cepler reads from
    /// disk (config + state dir + optional gates file).
    ///
    /// The index is left untouched (`update_index(false)`) since
    /// `Repo::clone` already populated it in full via the Mixed reset —
    /// a partial WT checkout shouldn't be allowed to shrink that record.
    pub fn checkout_paths<I, S>(&self, paths: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut checkout = CheckoutBuilder::new();
        checkout.force();
        checkout.update_index(false);
        let mut any = false;
        for path in paths {
            checkout.path(path.as_ref());
            any = true;
        }
        if !any {
            return Ok(());
        }
        let head_commit = self.inner.head()?.peel_to_commit()?;
        let head_obj = head_commit.into_object();
        self.inner.checkout_tree(&head_obj, Some(&mut checkout))?;
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

    #[test]
    fn clone_skips_working_tree_checkout_but_populates_index() {
        // The whole point of switching `Repo::clone` to a no-checkout
        // clone is that the working tree stays empty (zero per-file
        // syscalls during clone for repos with thousands of files) while
        // the index stays fully in sync with HEAD — otherwise the next
        // `commit_state_file` would build a tree containing only the
        // newly-added state file, deleting everything else inherited
        // from HEAD.
        let base = unique_tmp_dir("clone-no-checkout");
        let bare_path = base.join("remote.git");
        let seed_path = base.join("seed");
        let dest_path = base.join("dest");
        let bare_url = bare_path.to_str().unwrap().to_string();

        // Build a remote with a few tracked files in HEAD.
        Repository::init_bare(&bare_path).unwrap();
        let seed = Repository::init(&seed_path).unwrap();
        seed.remote("origin", &bare_url).unwrap();
        std::fs::write(seed_path.join("a.txt"), "alpha").unwrap();
        std::fs::write(seed_path.join("b.txt"), "beta").unwrap();
        std::fs::create_dir_all(seed_path.join("nested")).unwrap();
        std::fs::write(seed_path.join("nested/c.txt"), "gamma").unwrap();
        stage_and_commit(&seed, "seed commit");
        let branch = seed.head().unwrap().shorthand().unwrap().to_string();
        push_branch(&seed, &branch);

        let conf = GitConfig {
            url: bare_url.clone(),
            branch: branch.clone(),
            gates_branch: None,
            private_key: String::new(),
            dir: dest_path.to_str().unwrap().to_string(),
            depth: None,
        };
        let repo = Repo::clone(conf).expect("clone should succeed");

        // Working tree is empty — the only entry under `dest/` should be
        // the `.git` directory.
        let mut wd_entries: Vec<String> = std::fs::read_dir(&dest_path)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        wd_entries.sort();
        assert_eq!(
            wd_entries,
            vec![".git".to_string()],
            "no-checkout clone must leave the working tree empty"
        );

        // Index is fully populated — every blob from HEAD's tree should
        // be in the index so a downstream `write_tree()` would still
        // produce the HEAD tree (plus any additions).
        let mut index = repo.inner.index().unwrap();
        let entry_paths: std::collections::BTreeSet<String> = (0..index.len())
            .map(|i| String::from_utf8(index.get(i).unwrap().path).expect("utf8 index path"))
            .collect();
        let expected: std::collections::BTreeSet<String> = ["a.txt", "b.txt", "nested/c.txt"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(
            entry_paths, expected,
            "index must include every HEAD entry after the Mixed-reset",
        );

        // `write_tree` from the just-cloned index should reproduce HEAD's
        // tree exactly — this is what makes `commit_state_file` safe.
        let written = index.write_tree_to(&repo.inner).unwrap();
        let head_tree = repo
            .inner
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .tree()
            .unwrap()
            .id();
        assert_eq!(
            written, head_tree,
            "index.write_tree() must reproduce HEAD tree post-clone",
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn checkout_paths_materialises_only_requested_files() {
        // The concourse `in`/`check` flows pair a no-checkout clone with
        // a targeted `checkout_paths` for just the files cepler reads
        // from disk. Verify that the helper writes exactly those paths
        // (single files + glob patterns) and nothing else.
        let base = unique_tmp_dir("checkout-paths");
        let bare_path = base.join("remote.git");
        let seed_path = base.join("seed");
        let dest_path = base.join("dest");
        let bare_url = bare_path.to_str().unwrap().to_string();

        Repository::init_bare(&bare_path).unwrap();
        let seed = Repository::init(&seed_path).unwrap();
        seed.remote("origin", &bare_url).unwrap();
        std::fs::write(seed_path.join("cepler.yml"), "deployment: demo").unwrap();
        std::fs::create_dir_all(seed_path.join(".cepler/demo")).unwrap();
        std::fs::write(seed_path.join(".cepler/demo/staging.state"), "v1").unwrap();
        std::fs::write(seed_path.join(".cepler/demo/prod.state"), "v2").unwrap();
        std::fs::write(seed_path.join("other.txt"), "should stay unchecked-out").unwrap();
        stage_and_commit(&seed, "seed");
        let branch = seed.head().unwrap().shorthand().unwrap().to_string();
        push_branch(&seed, &branch);

        let conf = GitConfig {
            url: bare_url.clone(),
            branch: branch.clone(),
            gates_branch: None,
            private_key: String::new(),
            dir: dest_path.to_str().unwrap().to_string(),
            depth: None,
        };
        let repo = Repo::clone(conf).expect("clone");

        repo.checkout_paths(["cepler.yml", ".cepler/demo/*"])
            .expect("checkout_paths");

        let on_disk = |p: &str| dest_path.join(p).is_file();
        assert!(on_disk("cepler.yml"), "config must be materialised");
        assert!(
            on_disk(".cepler/demo/staging.state"),
            "state files matching the glob must be materialised"
        );
        assert!(on_disk(".cepler/demo/prod.state"));
        assert!(
            !on_disk("other.txt"),
            "files not matching any requested path must stay unchecked-out"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn fetch_specific_commits_is_noop_when_already_in_odb_or_no_credentials() {
        // libgit2's local transport doesn't support OID-in-want (and
        // doesn't even support shallow fetches in unit tests), so we
        // can't end-to-end exercise the actual fetch round-trip here.
        // What we *can* assert is that the two no-op paths bail out
        // cleanly — those are the load-bearing invariants for callers:
        // `Workspace::prepare` calls this unconditionally, and it must
        // not error on full clones or on already-cached commits.
        let base = unique_tmp_dir("fetch-specific-noop");
        let bare_path = base.join("remote.git");
        let work_path = base.join("work");
        let bare_url = bare_path.to_str().unwrap().to_string();

        Repository::init_bare(&bare_path).unwrap();
        let seed = Repository::init(&work_path).unwrap();
        seed.remote("origin", &bare_url).unwrap();
        std::fs::write(work_path.join("file.txt"), "v1").unwrap();
        let oid = stage_and_commit(&seed, "seed");
        let branch = seed.head().unwrap().shorthand().unwrap().to_string();
        push_branch(&seed, &branch);

        let conf = GitConfig {
            url: bare_url.clone(),
            branch: branch.clone(),
            gates_branch: None,
            private_key: String::new(),
            dir: base.join("clone").to_str().unwrap().to_string(),
            depth: None,
        };
        let repo = Repo::clone(conf).expect("clone");

        // Full clone path: `fetch_credentials` is None even when the
        // OID isn't in the ODB. Should still return Ok (the bare
        // `find_object` in `checkout_file_from` is the right place to
        // report unreachable OIDs on full clones).
        assert!(repo.fetch_credentials.is_none());
        let fabricated = CommitHash("0000000000000000000000000000000000000001".to_string());
        repo.fetch_specific_commits(&[&fabricated])
            .expect("no-creds path must be a no-op");

        // All-in-ODB path: passing a hash we already have must not
        // trigger any remote interaction.
        let local_hash = CommitHash(oid.to_string());
        repo.fetch_specific_commits(&[&local_hash])
            .expect("already-in-odb path must be a no-op");

        std::fs::remove_dir_all(&base).ok();
    }
}
