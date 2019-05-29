#![warn(clippy::all)]
#![allow(dead_code)]
#![allow(unused_imports)]
use failure::Error;
use git2::DiffDelta;
use git2::Odb;
use git2::Oid;
use git2::{Commit, Delta, ObjectType, Patch, Repository, Status, Tree};
use regex::Regex;
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug)]
pub struct GitLogConfig {
    /// include merge commits in file stats - usually excluded by `git log` - see https://stackoverflow.com/questions/37801342/using-git-log-to-display-files-changed-during-merge
    include_merges: bool,
}

pub const DEFAULT_GIT_LOG_CONFIG: GitLogConfig = GitLogConfig {
    include_merges: false,
};

#[derive(Debug, Serialize)]
pub struct GitLog {
    entries: Vec<GitLogEntry>,
}

/// simplified user info - based on git2::Signature but using blanks not None for now.
/// TODO: consider using None - let the UI decide how to handle?
#[derive(Debug, Serialize, PartialEq)]
pub struct User {
    name: String,
    email: String,
}

impl User {
    fn new(name: &str, email: &str) -> User {
        User {
            name: name.to_owned(),
            email: email.to_owned(),
        }
    }
}

/// simplified commit log entry
#[derive(Debug, Serialize)]
pub struct GitLogEntry {
    id: String,
    summary: String,
    parents: Vec<String>,
    committer: User,
    commit_time: i64,
    author: User,
    author_time: i64,
    co_authors: Vec<User>,
    file_changes: Vec<FileChange>,
}

/// the various kinds of git change we care about - a serializable subset of git2::Delta
#[derive(Debug, Serialize)]
pub enum CommitChange {
    Add,
    Rename,
    Delete,
    Modify,
    Copied,
}

/// Stats for file changes
#[derive(Debug, Serialize)]
pub struct FileChange {
    file: PathBuf,
    old_file: Option<PathBuf>,
    change: CommitChange,
    lines_added: usize,
    lines_deleted: usize,
}

// WIP:
// /// For each file we just keep a simplified history - what the changes were, by whom, and when.
// #[derive(Debug, Serialize)]
// pub struct FileHistoryEntry {
//     id: String,
//     summary: String,
//     committer: User,
//     commit_time: i64,
//     author: User,
//     author_time: i64,
//     co_authors: Vec<User>,
//     change: CommitChange,
//     lines_added: usize,
//     lines_deleted: usize,
// }

pub fn log(start_dir: &Path, config: Option<GitLogConfig>) -> Result<GitLog, Error> {
    let config = config.unwrap_or(DEFAULT_GIT_LOG_CONFIG);

    let repo = Repository::discover(start_dir)?;

    let workdir = repo
        .workdir()
        .ok_or_else(|| format_err!("bare repository - no workdir"));

    debug!("work dir: {:?}", workdir);

    let odb = repo.odb()?;
    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;

    // TODO: filter by dates! This will get mad on a big history

    let entries: Result<Vec<_>, _> = revwalk
        .map(|oid| summarise_commit(&repo, &odb, oid, &config))
        .collect();

    let entries = entries?.into_iter().flat_map(|e| e).collect();

    Ok(GitLog { entries })
}

fn summarise_commit(
    repo: &Repository,
    odb: &Odb,
    oid: Result<Oid, git2::Error>,
    config: &GitLogConfig,
) -> Result<Option<GitLogEntry>, Error> {
    let oid = oid?;
    let kind = odb.read(oid)?.kind();
    match kind {
        ObjectType::Commit => {
            let commit = repo.find_commit(oid)?;
            debug!("processing {:?}", commit);
            let author = commit.author();
            let committer = commit.committer();
            let author_time = author.when().seconds();
            let commit_time = committer.when().seconds();
            let other_time = commit.time().seconds();
            if commit_time != other_time {
                error!(
                    "Commit {:?} time {:?} != commit time {:?}",
                    commit, other_time, commit_time
                );
            }
            let co_authors = if let Some(message) = commit.message() {
                find_coauthors(message)
            } else {
                Vec::new()
            };

            let commit_tree = commit.tree()?;
            let file_changes = commit_file_changes(&repo, &commit, &commit_tree, config);
            Ok(Some(GitLogEntry {
                id: oid.to_string(),
                summary: commit.summary().unwrap_or("[no message]").to_string(),
                parents: commit.parent_ids().map({ |p| p.to_string() }).collect(),
                committer: signature_to_user(&committer),
                commit_time,
                author: signature_to_user(&author),
                author_time,
                co_authors,
                file_changes,
            }))
        }
        _ => {
            info!("ignoring object type: {}", kind);
            Ok(None)
        }
    }
}

fn signature_to_user(signature: &git2::Signature) -> User {
    User {
        name: signature.name().unwrap_or("[invalid name]").to_owned(),
        email: signature.email().unwrap_or("[invalid email]").to_owned(),
    }
}

fn find_coauthors(message: &str) -> Vec<User> {
    lazy_static! {
        static ref CO_AUTH_LINE: Regex = Regex::new(r"(?m)^\s*Co-authored-by:(.*)$").unwrap();
        static ref CO_AUTH_ANGLE_BRACKETS: Regex = Regex::new(r"^(.*)<([^>]+)>$").unwrap();
    }

    CO_AUTH_LINE
        .captures_iter(message)
        .map(|capture_group| {
            let co_author_text = &capture_group[1];
            if let Some(co_author_bits) = CO_AUTH_ANGLE_BRACKETS.captures(co_author_text) {
                User::new(
                    co_author_bits.get(1).unwrap().as_str().trim(),
                    co_author_bits.get(2).unwrap().as_str().trim(),
                )
            } else if co_author_text.contains('@') {
                // no angle brackets, but an @
                User::new("", co_author_text.trim())
            } else {
                User::new(co_author_text.trim(), "")
            }
        })
        .collect()
}

fn commit_file_changes(
    repo: &Repository,
    commit: &Commit,
    commit_tree: &Tree,
    config: &GitLogConfig,
) -> Vec<FileChange> {
    if commit.parent_count() == 0 {
        info!("Commit {} has no parent", commit.id());

        scan_diffs(&repo, &commit_tree, None, &commit, None).expect("Can't scan for diffs")
    } else if commit.parent_count() > 1 && !config.include_merges {
        debug!(
            "Not showing file changes for merge commit {:?}",
            commit.id()
        );
        Vec::new()
    } else {
        commit
            .parents()
            .flat_map(|parent| {
                debug!("Getting changes for parent {:?}:", parent);
                let parent_tree = parent.tree().expect("can't get parent tree");
                scan_diffs(
                    &repo,
                    &commit_tree,
                    Some(&parent_tree),
                    &commit,
                    Some(&parent),
                )
                .expect("Can't scan for diffs")
            })
            .collect()
    }
}

fn scan_diffs(
    repo: &Repository,
    commit_tree: &Tree,
    parent_tree: Option<&Tree>,
    commit: &Commit,
    parent: Option<&Commit>,
) -> Result<Vec<FileChange>, Error> {
    let mut diff = repo.diff_tree_to_tree(parent_tree, Some(&commit_tree), None)?;
    diff.find_similar(None)?;
    let file_changes = diff
        .deltas()
        .enumerate()
        .filter_map(|(delta_index, delta)| {
            // can we / should we get bytes for binary changes?  Adds show as 0 lines.
            let patch =
                Patch::from_diff(&diff, delta_index).expect("can't get a patch from a diff");
            let (_, lines_added, lines_deleted) = if let Some(patch) = patch {
                patch
                    .line_stats()
                    .expect("Couldn't get line stats from a patch")
            } else {
                warn!("No patch possible diffing {:?} -> {:?}", commit, parent);
                (0, 0, 0)
            };
            summarise_delta(delta, lines_added, lines_deleted)
        });
    Ok(file_changes.collect())
}

fn summarise_delta(
    delta: DiffDelta,
    lines_added: usize,
    lines_deleted: usize,
) -> Option<FileChange> {
    match delta.status() {
        Delta::Added => {
            let name = delta.new_file().path().unwrap();
            Some(FileChange {
                file: name.to_path_buf(),
                old_file: None,
                change: CommitChange::Add,
                lines_added,
                lines_deleted,
            })
        }
        Delta::Renamed => {
            let old_name = delta.old_file().path().unwrap();
            let new_name = delta.new_file().path().unwrap();
            Some(FileChange {
                file: new_name.to_path_buf(),
                old_file: Some(old_name.to_path_buf()),
                change: CommitChange::Rename,
                lines_added,
                lines_deleted,
            })
        }
        Delta::Deleted => {
            let name = delta.old_file().path().unwrap();
            Some(FileChange {
                file: name.to_path_buf(),
                old_file: None,
                change: CommitChange::Delete,
                lines_added,
                lines_deleted,
            })
        }
        Delta::Modified => {
            let name = delta.new_file().path().unwrap();
            Some(FileChange {
                file: name.to_path_buf(),
                old_file: None,
                change: CommitChange::Modify,
                lines_added,
                lines_deleted,
            })
        }
        Delta::Copied => {
            let old_name = delta.old_file().path().unwrap();
            let new_name = delta.new_file().path().unwrap();
            Some(FileChange {
                file: new_name.to_path_buf(),
                old_file: Some(old_name.to_path_buf()),
                change: CommitChange::Copied,
                lines_added,
                lines_deleted,
            })
        }
        _ => {
            error!("Not able to handle delta of status {:?}", delta.status());
            None
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
struct GitData {
    authors: Vec<String>,
    last_change: i64,
}

fn parse_file(filename: &Path) -> Result<GitData, Error> {
    let repo = Repository::discover(filename)?;
    let odb = repo.odb()?;
    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;
    let mut authors = HashSet::new();
    for oid in revwalk {
        let oid = oid?;
        let kind = odb.read(oid)?.kind();
        if kind == ObjectType::Commit {
            let commit = repo.find_commit(oid)?;
            let author = commit.author();
            let message = commit.message().unwrap_or("no message");
            println!("scanning: {:?}", message);
            let name: String = author.name().unwrap_or("UNKNOWN AUTHOR").to_string();
            authors.insert(name);
        } else {
            println!("Unexpected Kind {:?}", kind);
        }
    }

    Ok(GitData {
        authors: authors.into_iter().collect(),
        last_change: 0,
    })
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_helpers::*;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn authorless_message_has_no_coauthors() {
        assert_eq!(find_coauthors("do be do be do"), Vec::<User>::new());
    }

    #[test]
    fn can_get_coauthors_from_message() {
        let message = r#"This is a commit message
        not valid: Co-authored-by: fred jones
        Co-authored-by: valid user <valid@thing.com>
        Co-authored-by: <be.lenient@any-domain.com>
        Co-authored-by: bad@user <this isn't really trying to be clever>
        ignore random lines
        Co-authored-by: if there's no at it's a name
        Co-authored-by: if there's an @ it's email@thing.com
        ignore trailing lines
        "#;

        let expected = vec![
            User::new("valid user", "valid@thing.com"),
            User::new("", "be.lenient@any-domain.com"),
            User::new("bad@user", "this isn't really trying to be clever"),
            User::new("if there's no at it's a name", ""),
            User::new("", "if there's an @ it's email@thing.com"),
        ];

        assert_eq!(find_coauthors(message), expected);
    }

    #[test]
    fn can_extract_basic_git_log() -> Result<(), Error> {
        let gitdir = tempdir()?;
        unzip_to_dir(gitdir.path(), "tests/data/git/git_sample.zip")?;
        let git_root = PathBuf::from(gitdir.path()).join("git_sample");

        let git_log = log(&git_root, None)?;

        assert_eq_json_file(&git_log, "./tests/expected/git/git_sample.json");

        Ok(())
    }

    #[test]
    fn git_log_can_include_merge_changes() -> Result<(), Error> {
        let gitdir = tempdir()?;
        unzip_to_dir(gitdir.path(), "tests/data/git/git_sample.zip")?;
        let git_root = PathBuf::from(gitdir.path()).join("git_sample");

        let git_log = log(
            &git_root,
            Some(GitLogConfig {
                include_merges: true,
            }),
        )?;

        assert_eq_json_file(&git_log, "./tests/expected/git/git_sample_with_merges.json");

        Ok(())
    }
}

// run a single test with:
// cargo test -- --nocapture can_extract_basic_git_log | grep -v "running 0 tests" | grep -v "0 passed" | grep -v -e '^\s*$'
