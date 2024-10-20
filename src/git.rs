use crate::diffbase;
use crate::diffbase::MergeRequest;
use crate::dispatch::{communicate, dispatch_to, run_command, run_editor};
use crate::Error;
use crate::Result;
use crate::{github, gitlab};
use chrono::{Local, NaiveDate, TimeZone};
use git2;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str;
use tokio::try_join;
use webbrowser;

/// Calls git merge and checks if the merge was successful.
pub fn merge(branch: &str, repo: &git2::Repository) -> Result<()> {
    run_command(&["git", "merge", branch])?;
    if repo.state() != git2::RepositoryState::Clean {
        return Err(Error::general(
            "git merge did not complete cleanly.".to_string(),
        ));
    }
    Ok(())
}

pub fn get_main_branch() -> String {
    let out = String::from_utf8(
        communicate(&["git", "symbolic-ref", "refs/remotes/origin/HEAD"])
            .unwrap()
            .stdout,
    )
    .unwrap();
    let line = out
        .lines()
        .next()
        .expect("No HEAD branch for remote 'origin'");
    line.trim().split('/').last().unwrap().to_string()
}

/// Parses git's configuration and extracts all aliases that do not shell out. Returns (key, value)
/// representations.
pub fn get_aliases() -> HashMap<String, String> {
    let mut rv = HashMap::new();
    let config = git2::Config::open_default().unwrap();
    let mut entries = config.entries(Some("alias.*")).unwrap();
    while let Some(entry_or_err) = entries.next() {
        let entry = entry_or_err.unwrap();
        // We only need to understand aliases for git commands (like checkout, branch) and so on.
        // We will never care for stuff that shells out.
        if entry.name().unwrap().trim().starts_with('!') {
            continue;
        }
        // name is alias.<alias>, so we trim the first 6 characters.
        rv.insert(
            entry.name().unwrap()[6..].to_string(),
            entry.value().unwrap().to_string(),
        );
    }
    rv
}

/// Returns the names of all local branches.
pub fn get_all_local_branch_names(repo: &git2::Repository) -> Result<HashSet<String>> {
    Ok(get_all_local_branches(repo)?.keys().cloned().collect())
}

#[derive(Debug)]
pub struct BranchInfo {
    pub upstream: Option<String>,
}

/// Returns some limited information about all local branches.
pub fn get_all_local_branches(repo: &git2::Repository) -> Result<HashMap<String, BranchInfo>> {
    let mut results = HashMap::new();
    for entry in repo.branches(Some(git2::BranchType::Local))? {
        let (branch, _) = entry?;
        let upstream = if let Ok(upstream) = branch.upstream() {
            Some(upstream.name()?.unwrap().to_string())
        } else {
            None
        };
        let name = branch.name()?.unwrap().to_string();
        results.insert(name, BranchInfo { upstream });
    }
    Ok(results)
}

#[derive(Debug, PartialEq, Eq)]
struct GitHubRepository<'a> {
    remote: &'a Remote,
}

impl<'a> GitHubRepository<'a> {
    fn owner_and_project(&self) -> &str {
        const GITHUB_HTTPS: &str = "https://github.com/";
        self.remote
            .url
            .trim_start_matches(GITHUB_HTTPS)
            .rsplit(':')
            .nth(0)
            .unwrap()
    }

    pub fn owner(&self) -> &str {
        self.owner_and_project().rsplit_once('/').unwrap().0
    }

    pub fn repository(&self) -> github::RepoId {
        let mut name = self.owner_and_project().rsplit('/').nth(0).unwrap();
        if name.ends_with(".git") {
            name = &name[..name.len() - 4];
        }
        github::RepoId {
            owner: self.owner().to_string(),
            name: name.to_string(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct GitLabRepository<'a> {
    remote: &'a Remote,
}

impl<'a> GitLabRepository<'a> {
    fn project(&self) -> &str {
        const GITLAB_HTTPS: &str = "https://gitlab.com/";
        self.remote
            .url
            .trim_start_matches(GITLAB_HTTPS)
            .rsplit(':')
            .nth(0)
            .unwrap()
            .trim_end_matches(".git")
    }
}

#[derive(Debug, PartialEq, Eq)]
enum RepositoryType<'a> {
    GitLab(GitLabRepository<'a>),
    GitHub(GitHubRepository<'a>),
    Unknown,
}

#[derive(Debug, PartialEq, Eq)]
/// Could be git@github.com:SirVer/giti.git.
struct Remote {
    url: String,
}

impl Remote {
    /// The project part of the URL, i.e. for git@github.com:SirVer/giti.git, this would be
    /// 'giti.git'.
    pub fn project(&self) -> &str {
        self.url.rsplit('/').nth(0).unwrap()
    }

    pub fn repository(&self) -> RepositoryType {
        if self.url.contains("github.com") {
            RepositoryType::GitHub(GitHubRepository { remote: self })
        } else if self.url.contains("gitlab.com") {
            RepositoryType::GitLab(GitLabRepository { remote: self })
        } else {
            RepositoryType::Unknown
        }
    }
}

/// Returns a map from origin name to Remote.
fn get_remotes() -> Result<HashMap<String, Remote>> {
    let stdout = String::from_utf8(communicate(&["git", "remote", "-v"])?.stdout).unwrap();
    let mut result = HashMap::new();
    for line in stdout.lines() {
        if line.contains("(push)") {
            continue;
        }
        let mut it = line.split_whitespace();
        let name = it.next().unwrap();
        let origin = Remote {
            url: it.next().unwrap().to_string(),
        };
        result.insert(name.to_string(), origin);
    }
    Ok(result)
}

/// Returns the deleted or modified files in the working directory. This shells out to git
/// directly, because using `libgit2::Repository::statuses`() was very, very slow.
pub fn status() -> Result<(HashSet<PathBuf>, HashSet<PathBuf>)> {
    let mut deleted = HashSet::<PathBuf>::new();
    let mut modified = HashSet::<PathBuf>::new();

    let stdout =
        String::from_utf8(communicate(&["git", "status", "--porcelain", "-uno"])?.stdout).unwrap();
    for line in stdout.lines() {
        let entries = line.trim().splitn(2, ' ').collect::<Vec<_>>();
        match entries[0] {
            "M" => modified.insert(PathBuf::from(entries[1])),
            "D" => deleted.insert(PathBuf::from(entries[1])),
            _ => panic!("Unknow status output from git: '{}'", line),
        };
    }
    Ok((deleted, modified))
}

/// Returns an error if the working directory is dirty.
fn expect_working_directory_clean() -> Result<()> {
    let (deleted, changed) = status()?;
    if deleted.len() + changed.len() == 0 {
        return Ok(());
    }

    let mut error = String::from(
        "You cannot have pending changes for this command. Changed \
         files:\n\n",
    );
    for s in deleted.union(&changed) {
        error.push_str(&format!("  {}\n", s.to_string_lossy()));
    }
    error.push('\n');
    Err(Error::general(error))
}

/// Returns the name of the branch that is currently checked out.
pub fn get_current_branch(repo: &git2::Repository) -> String {
    let head = repo.head().unwrap();
    head.shorthand().unwrap().to_string()
}

#[derive(Debug)]
struct OriginBranch {
    remote: String,
    _branch: String,
}

fn get_origin(local_branch: &str) -> Option<OriginBranch> {
    let remote = match communicate(&["git", "config", &format!("branch.{}.remote", local_branch)]) {
        Ok(out) => str::from_utf8(&out.stdout).unwrap().trim().to_string(),
        Err(_) => return None,
    };

    let _branch = match communicate(&["git", "config", &format!("branch.{}.merge", local_branch)]) {
        Ok(out) => str::from_utf8(&out.stdout)
            .unwrap()
            .trim()
            .trim_start_matches("refs/heads/")
            .to_string(),
        Err(_) => return None,
    };

    if remote.is_empty() || _branch.is_empty() {
        return None;
    }
    Some(OriginBranch { remote, _branch })
}

/// Returns the (added, deleted, modified) files between two treeishs, e.g. branch names.
pub fn get_changed_files(
    repo: &git2::Repository,
    old: &str,
    new: &str,
) -> Result<(HashSet<PathBuf>, HashSet<PathBuf>, HashSet<PathBuf>)> {
    let parent = repo.revparse_single(old)?;
    let current = repo.revparse_single(new)?;
    let merge_base = repo.find_object(repo.merge_base(parent.id(), current.id())?, None)?;

    let mut diff_options = git2::DiffOptions::new();
    diff_options
        .include_ignored(false)
        .include_untracked(false)
        .include_typechange(false)
        .ignore_filemode(true)
        .skip_binary_check(true)
        .enable_fast_untracked_dirs(true);
    let diff = repo.diff_tree_to_tree(
        merge_base.peel(git2::ObjectType::Tree)?.as_tree(),
        current.peel(git2::ObjectType::Tree)?.as_tree(),
        Some(&mut diff_options),
    )?;

    let mut added = HashSet::<PathBuf>::new();
    let mut deleted = HashSet::<PathBuf>::new();
    let mut modified = HashSet::<PathBuf>::new();
    diff.print(git2::DiffFormat::NameStatus, |_delte, _hunk, line| {
        // line is 'A\tfile/path\n'
        let path = PathBuf::from(str::from_utf8(&line.content()[2..]).unwrap().trim());
        match line.content()[0] as char {
            'A' => added.insert(path),
            'D' => deleted.insert(path),
            'M' => modified.insert(path),
            unknown => panic!("Unexpected status char: {}", unknown),
        };
        true
    })?;
    Ok((added, deleted, modified))
}

fn run_clang_format(path: &Path) -> Result<()> {
    dispatch_to(
        "clang-format",
        &[
            "-i",
            "-sort-includes",
            "-style=file",
            "-fallback-style=Google",
            &path.to_string_lossy(),
        ],
    )?;
    Ok(())
}

fn run_buildifier(path: &Path) -> Result<()> {
    dispatch_to("buildifier", &[&path.to_string_lossy()])?;
    Ok(())
}

pub fn handle_fix(args: &[&str], repo: &git2::Repository) -> Result<()> {
    expect_working_directory_clean()?;

    let main_branch = get_main_branch();
    let other_branch = if args.len() == 2 {
        args[1].to_string()
    } else {
        format!("origin/{}", main_branch)
    };

    println!("Fixing modified files compared to {}", other_branch);
    let (added, _, modified) = get_changed_files(repo, &other_branch, &get_current_branch(repo))?;

    let workdir = repo.workdir().unwrap();
    for path in added.union(&modified) {
        if path.file_name().is_none() {
            continue;
        }
        let file_name = path.file_name().unwrap().to_str().unwrap();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        let full_path = workdir.join(path);

        match (file_name, ext) {
            (_, "h") | (_, "cc") | (_, "proto") => run_clang_format(&full_path)?,
            ("BUILD", _) | (_, "BUILD") => run_buildifier(&full_path)?,
            _ => (),
        }
    }

    let changed_files = status()?.1;
    if !changed_files.is_empty() {
        println!("Fixed files:\n");
        for filename in changed_files {
            println!("  {}", filename.to_string_lossy());
        }
        println!();
        dispatch_to("git", &["commit", "-am", "Ran git fix."])?;
    }
    Ok(())
}

pub async fn handle_cleanup(repo: &git2::Repository, dbase: &mut diffbase::Diffbase) -> Result<()> {
    let current_branch = get_current_branch(repo);

    for branch in get_all_local_branch_names(repo)? {
        if branch == current_branch {
            continue;
        }

        if branch.starts_with('|') {
            run_command(&["git", "branch", "-D", &branch])?;
            continue;
        }

        if let Some(merge_request) = dbase.get_merge_request(&branch) {
            let should_delete = match merge_request {
                MergeRequest::GitHub(pr_id) => {
                    let pr = github::get_pr(pr_id).await?;
                    if pr.state == github::PullRequestState::Closed {
                        Some((pr_id.to_string(), branch))
                    } else {
                        None
                    }
                }
                MergeRequest::GitLab(mr_id) => {
                    let gitlab = gitlab::GitLab::new().unwrap();
                    let mr = gitlab.get_mr(&mr_id.project(), mr_id.number()).await?;
                    match mr.state {
                        gitlab::PullRequestState::Closed | gitlab::PullRequestState::Merged => {
                            Some((mr.web_url, mr.source_branch))
                        }
                        gitlab::PullRequestState::Open => None,
                    }
                }
            };

            if let Some((pr_id, branch)) = should_delete {
                let rev = repo.revparse_single(&branch)?;
                println!(
                    "{} is closed. Deleting the branch {} ({}).",
                    pr_id,
                    branch,
                    rev.id()
                );
                run_command(&["git", "branch", "-D", &branch])?;
                continue;
            };
        }
    }

    // Delete branches that have been merged upstream.

    Ok(())
}

pub fn handle_review_push(repo: &git2::Repository) -> Result<()> {
    // branch name will be user/branch_name.
    let full_branch_name = get_current_branch(repo);
    let (user, branch_name) = {
        let mut it = full_branch_name.splitn(2, '/');
        // Slice off the leading '|'
        (&it.next().unwrap()[1..], it.next().unwrap())
    };
    run_command(&[
        "git",
        "push",
        "--force",
        user,
        &format!("HEAD:{}", branch_name),
    ])?;
    Ok(())
}

pub async fn handle_review(
    args: &[&str],
    repo: &git2::Repository,
    dbase: &mut diffbase::Diffbase,
) -> Result<()> {
    let remotes = get_remotes()?;

    let main_branch = get_main_branch();
    let main_origin = get_origin(&main_branch).unwrap();
    let main_remote = &remotes[&main_origin.remote];
    let repo_id = match main_remote.repository() {
        RepositoryType::GitHub(s) => s.repository(),
        _ => {
            return Err(Error::general(
                "Cannot handle 'review' for anything but GitHub Repos currently.".to_string(),
            ))
        }
    };

    if args.len() == 1 {
        let prs = github::find_assigned_prs(Some(&repo_id)).await?;
        if prs.is_empty() {
            println!("No reviews assigned in {}/{}.", repo_id.owner, repo_id.name);
        } else {
            for pr in &prs {
                println!(
                    "#{} by @{}: {} ({}:{})",
                    pr.number, pr.author_login, pr.title, pr.source.repo.owner, pr.source.name
                );
            }
        }
        return Ok(());
    }

    if args.len() != 2 {
        return Err(Error::general(
            "review requires a pull request number or a user/branch_name to review.".into(),
        ));
    }

    expect_working_directory_clean()?;

    if args[1] == "push" {
        return handle_review_push(repo);
    }

    let (source_branch, merge_request) = if let Ok(pr_number) = args[1].parse::<i32>() {
        let pr = github::get_pr(&github::PullRequestId {
            repo: repo_id.clone(),
            number: pr_number,
        })
        .await?;
        let merge_request = MergeRequest::GitHub(pr.id());
        (pr.source, Some(merge_request))
    } else {
        let (user, branch) = {
            let mut it = args[1].splitn(2, ':');
            (it.next().unwrap(), it.next().unwrap())
        };

        let branch = github::Branch {
            repo: github::RepoId {
                owner: user.to_string(),
                name: repo_id.name.clone(),
            },
            name: branch.to_string(),
        };
        (branch, None)
    };

    let owner = if source_branch.repo == repo_id {
        "origin"
    } else {
        &source_branch.repo.owner
    };

    if !remotes.contains_key(owner) {
        run_command(&[
            "git",
            "remote",
            "add",
            owner,
            &format!("git@github.com:{}/{}", owner, main_remote.project()),
        ])?;
    }
    // Since the local_branch name is the remote/branch git also resolves it to the correct remote.
    run_command(&["git", "fetch", owner])?;
    let branch_to_fork = format!("remotes/{}/{}", owner, source_branch.name);
    let local_branch = format!("|{}/{}", owner, source_branch.name);

    if get_all_local_branch_names(repo)?.contains(&local_branch) {
        run_command(&["git", "branch", "-D", &local_branch])?;
    }

    run_command(&["git", "branch", "--track", &local_branch, &branch_to_fork])?;
    if let Some(merge_request) = merge_request {
        dbase.set_merge_request(&local_branch, merge_request);
    }
    checkout(repo, &local_branch)?;
    Ok(())
}

pub fn checkout(repo: &git2::Repository, branch: &str) -> Result<()> {
    run_command(&["git", "checkout", branch])?;
    if !repo.submodules().unwrap().is_empty() {
        run_command(&["git", "submodule", "update", "--init", "--recursive"])?;
    }
    Ok(())
}

pub fn handle_clone(args: &[&str]) -> Result<()> {
    let github_repo_regex =
        regex::Regex::new(r"^[a-zA-Z\d][a-zA-Z\d-]*/[a-zA-Z\d][a-zA-Z\d-]").unwrap();

    let new_args: Vec<_> = args
        .iter()
        .map(|a| {
            if github_repo_regex.is_match(a) {
                format!("git@github.com:{}.git", a)
            } else {
                a.to_string()
            }
        })
        .collect();

    let args_ref: Vec<_> = new_args.iter().map(|s| s as &str).collect();
    dispatch_to("git", &args_ref)?;

    Ok(())
}

pub async fn handle_prs(args: &[&str]) -> Result<()> {
    let mut opts = getopts::Options::new();
    opts.optopt(
        "s",
        "start_date",
        "Use this start date. [today - 21 days].",
        "YYYY-MM-DD",
    );
    opts.optopt(
        "e",
        "end_date",
        "Use this end date. [today - 21 days].",
        "YYYY-MM-DD",
    );

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(err) => {
            let brief = format!("{}\nUsage: g up [options]", err);
            return Err(Error::general(opts.usage(&brief)));
        }
    };

    let today = Local::now();
    let start = match matches.opt_str("start_date") {
        None => today
            .checked_sub_signed(chrono::Duration::days(21))
            .expect("This should not underflow."),
        Some(s) => Local
            .from_local_datetime(
                &NaiveDate::parse_from_str(&s, "%Y-%m-%d")?
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
            )
            .single()
            .unwrap(),
    };
    let end = match matches.opt_str("end_date") {
        None => today,
        Some(s) => Local
            .from_local_datetime(
                &NaiveDate::parse_from_str(&s, "%Y-%m-%d")?
                    .and_hms_opt(23, 59, 59)
                    .unwrap(),
            )
            .single()
            .unwrap(),
    };

    println!(
        "Finding GitHub PRs and GitLab MRs from {} to {}.",
        start.format("%Y-%m-%d"),
        end.format("%Y-%m-%d")
    );

    let (mrs, prs) = try_join!(
        gitlab::find_my_mrs(start, end),
        github::find_my_prs(start, end)
    )?;

    let (mut open_github, mut closed_github) = prs
        .into_iter()
        .partition::<Vec<_>, _>(|pr| pr.state == github::PullRequestState::Open);
    open_github.sort_by_key(|p| (p.target.repo.name.clone(), p.number));
    closed_github.sort_by_key(|p| (p.target.repo.name.clone(), p.number));
    let (mut open_gitlab, mut closed_gitlab) = mrs
        .into_iter()
        .partition::<Vec<_>, _>(|pr| pr.state == gitlab::PullRequestState::Open);
    open_gitlab.sort_by_key(|p| p.web_url.clone());
    closed_gitlab.sort_by_key(|p| p.web_url.clone());

    println!("Closed:");
    for p in closed_github {
        println!("  - [#{} • {}]({})", p.number, p.title, p.id().url());
    }
    for p in closed_gitlab {
        println!("  - [#{} • {}]({})", p.number, p.title, p.web_url);
    }

    println!("\nStill open:");
    for p in open_github {
        println!("  - [#{} • {}]({})", p.number, p.title, p.id().url());
    }
    for p in open_gitlab {
        println!("  - [#{} • {}]({})", p.number, p.title, p.web_url);
    }

    Ok(())
}

pub async fn handle_pr(
    _args: &[&str],
    repo: &git2::Repository,
    dbase: &mut diffbase::Diffbase,
) -> Result<()> {
    let local_branches = get_all_local_branches(repo)?;
    let current_branch = get_current_branch(repo);

    let remotes = get_remotes()?;
    let main_branch = get_main_branch();
    let base_remote = {
        let origin = match get_origin(&main_branch) {
            None => get_origin(&current_branch).ok_or(Error::general(
                "Unable to find origin for merge request.".to_string(),
            ))?,
            Some(o) => o,
        };
        &remotes[&origin.remote]
    };

    if local_branches[&current_branch].upstream.is_none() {
        return Err(Error::general(
            "current branch has no upstream (maybe git push -u?). \
             Cannot open a pull request."
                .into(),
        ));
    }
    // Could be "SirVer/foobar" or "origin/foobar"
    let head_upstream = &local_branches[&current_branch].upstream.clone().unwrap();
    let head_remote = &remotes[head_upstream.split('/').next().unwrap()];

    // expect_working_directory_clean()?;

    if let Some(merge_request) = dbase.get_merge_request(&current_branch) {
        return Err(Error::general(format!(
            "current branch already has the merge request {:?} associated with it. \
             Refuse to open a new pull request.",
            merge_request
        )));
    }

    // Get PR original post message.
    let mut temp_file = tempfile::Builder::new()
        .prefix("COMMIT_EDITMSG")
        .rand_bytes(0)
        .tempfile()?;

    if let Some(msg) = github::get_pull_request_template(repo.workdir().unwrap()) {
        temp_file.write_all(msg.as_bytes())?
    }
    let temp_path = temp_file.into_temp_path();

    run_editor(&temp_path)?;
    let content = ::std::fs::read_to_string(&temp_path)?.trim().to_string();
    let lines: Vec<String> = content.lines().map(|l| l.trim().to_string()).collect();
    if lines.is_empty() {
        return Err(Error::general("No message, no PR.".into()));
    }
    let title = lines[0].to_string();
    let body = if lines.len() > 2 {
        Some(lines[2..].join("\n"))
    } else {
        None
    };

    let url = match base_remote.repository() {
        RepositoryType::GitHub(s) => {
            let repo_id = s.repository();
            // Base to merge from. If it is in the same fork as base, it must not contain the owners name.
            let head = if head_remote == base_remote {
                current_branch.clone()
            } else {
                let owner = match head_remote.repository() {
                    RepositoryType::GitHub(s) => s.owner().to_string(),
                    _ => unreachable!("Head cannot not be GitHub since base is."),
                };
                format!("{}:{}", owner, current_branch)
            };

            let pull_options = hubcaps_ex::pulls::PullOptions {
                title,
                body,
                head,
                base: main_branch,
            };

            let pr = github::create_pr(&repo_id, pull_options).await?.id();
            dbase.set_merge_request(&current_branch, MergeRequest::GitHub(pr.clone()));
            pr.url()
        }
        RepositoryType::GitLab(s) => {
            let gitlab = gitlab::GitLab::new().unwrap();
            let mr = gitlab
                .create_mr(
                    s.project(),
                    &current_branch,
                    &main_branch,
                    &title,
                    &body.unwrap_or("".to_string()),
                )
                .await?;
            dbase.set_merge_request(&current_branch, MergeRequest::GitLab(mr.id()));
            mr.web_url
        }
        RepositoryType::Unknown => unreachable!("PR only implemented for GitLab & GitHub."),
    };

    println!("Opened {}. Opening in web browser.", url);
    let _ = webbrowser::open(&url);

    Ok(())
}

pub fn handle_start(args: &[&str], repo: &git2::Repository) -> Result<()> {
    if args.len() != 2 {
        return Err(Error::general("start requires a branch name.".into()));
    }
    run_command(&["git", "fetch"])?;
    let origin = format!("origin/{}", get_main_branch());
    run_command(&["git", "branch", "--no-track", args[1], &origin])?;
    checkout(repo, args[1])
}

fn replace_aliases<'a>(command: &'a str, git_aliases: &'a HashMap<String, String>) -> Vec<&'a str> {
    if let Some(value) = git_aliases.get(command) {
        return value.split(' ').collect();
    }
    vec![command]
}

pub async fn handle_repository(original_args: &[&str]) -> Result<()> {
    if original_args.is_empty() {
        return dispatch_to("git", original_args);
    }

    let git_aliases = get_aliases();
    let alias_expanded = replace_aliases(original_args[0], &git_aliases);
    let expanded_args: Vec<&str> = alias_expanded
        .iter()
        .chain(original_args[1..].iter())
        .copied()
        .collect();

    // Arguments that are valid without a git repository.
    match expanded_args[0] as &str {
        // Intercepted commands.
        "clone" => return handle_clone(&expanded_args),
        "prs" => return handle_prs(&expanded_args).await,
        _ => (),
    };

    let repo = git2::Repository::discover(".");
    if repo.is_err() {
        return dispatch_to("git", &expanded_args);
    }
    let repo = repo.unwrap();
    let mut dbase = diffbase::Diffbase::new(&repo)?;

    let result = match expanded_args[0] as &str {
        // Intercepted commands.
        "branch" => diffbase::handle_branch(&expanded_args, &repo, &mut dbase),
        "checkout" => diffbase::handle_checkout(&expanded_args, &repo, &mut dbase),
        "cleanup" => handle_cleanup(&repo, &mut dbase).await,
        "down" => diffbase::handle_down(&expanded_args, &repo, &dbase),
        "fix" => handle_fix(&expanded_args, &repo),
        "merge" => diffbase::handle_merge(&expanded_args, &repo, &mut dbase),
        "pullc" => diffbase::handle_pullc(&expanded_args, &repo, &dbase),
        "review" => handle_review(&expanded_args, &repo, &mut dbase).await,
        "start" => handle_start(&expanded_args, &repo),
        "up" => diffbase::handle_up(&expanded_args, &repo, &dbase),
        "pr" => handle_pr(&expanded_args, &repo, &mut dbase).await,

        _ => dispatch_to("git", &expanded_args),
    };

    dbase.write_to_disk()?;
    result
}
