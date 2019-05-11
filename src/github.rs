// Extremely helpful was this: https://jsdw.me/posts/rust-asyncawait-preview/

use crate::error::*;
use hubcaps::search::SearchIssuesOptions;
use hubcaps::{self, Credentials};
use hyper;
use hyper_tls;
use std::env;
use tokio::await;
use tokio::prelude::*;
use tokio_async_await::compat::backward::Compat;
use url;

// TODO(sirver): This state of async/await only allowed static references or owning data. So there
// is lots of cloning going on here.

#[derive(Debug)]
pub struct Branch {
    pub repo: Repo,
    pub name: String,
}

impl Branch {
    fn from_label(repo_name: &str, label: &str) -> Self {
        let mut it = label.split(":");
        let owner = it.next().unwrap().to_string();
        let name = it.next().unwrap().to_string();
        Branch {
            repo: Repo {
                owner: owner,
                name: repo_name.to_string(),
            },
            name,
        }
    }
}

#[derive(Debug)]
pub struct PullRequest {
    // Repo where this PR is opened, e.g. "SirVer/UltiSnips"
    pub target: Branch,
    pub source: Branch,
    pub number: i32,
    pub author_login: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Repo {
    pub owner: String,
    pub name: String,
}

type Github = hubcaps::Github<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>;

// bug fixed version from hubcaps: http://lessis.me/hubcaps/src/hubcaps/search/mod.rs.html#229-235
pub fn repo_tuple(repository_url: &str) -> (String, String) {
    // split the last two elements off the repo url path
    let parsed = url::Url::parse(&repository_url).unwrap();
    let mut path = parsed.path().split('/').collect::<Vec<_>>();
    path.reverse();
    (path[1].to_owned(), path[0].to_owned())
}

async fn fetch_pr(
    github: Github,
    repo: Repo,
    number: u64,
) -> hubcaps::Result<(Repo, hubcaps::pulls::Pull)> {
    let res = await!(github
        .repo(repo.owner.to_string(), repo.name.to_string())
        .pulls()
        .get(number)
        .get())?;
    Ok((repo, res))
}

async fn find_assigned_pr_info(
    github: Github,
    login: String,
) -> hubcaps::Result<Vec<(Repo, hubcaps::pulls::Pull)>> {
    let mut search = github.search().issues().iter(
        format!("is:pr is:open archived:false assignee:{}", login,),
        &SearchIssuesOptions::builder().per_page(25).build(),
    );

    let mut futures = vec![];
    while let Some(Ok(result)) = await!(search.next()) {
        let (owner, name) = repo_tuple(&result.repository_url);
        let repo = Repo { owner, name };
        futures.push(Compat::new(fetch_pr(github.clone(), repo, result.number)));
    }

    let mut results = vec![];
    for rv in await!(tokio::prelude::future::join_all(futures))? {
        results.push(rv);
    }
    Ok(results)
}

async fn find_login_name(github: Github) -> hubcaps::Result<String> {
    Ok(await!(github.users().authenticated())?.login)
}

async fn run(github: Github) -> hubcaps::Result<Vec<(Repo, hubcaps::pulls::Pull)>> {
    let login = await!(find_login_name(github.clone()))?;
    let res = await!(find_assigned_pr_info(github.clone(), login))?;
    Ok(res)
}

pub fn find_assigned_prs(repo: Option<&Repo>) -> Result<Vec<PullRequest>> {
    let token = env::var("GITHUB_TOKEN")?;

    let repo = repo.map(|r| r.clone());
    let (tx, rx) = ::std::sync::mpsc::channel();
    let tx = ::std::sync::Mutex::new(tx);
    tokio::run_async(
        async move {
            let github = Github::new("SirVer_giti/unspecified", Some(Credentials::Token(token)));
            let mut prs = await!(run(github.clone())).expect("run() did not succeed.");
            prs.sort_by_key(|(_, pr)| pr.number);

            let new_result = prs
                .iter()
                .filter(|(pr_repo, _)| match repo {
                    None => true,
                    Some(ref r) => pr_repo == r,
                })
                .map(|(pr_repo, pr)| PullRequest {
                    source: Branch::from_label(&pr_repo.name, &pr.head.label),
                    target: Branch::from_label(&pr_repo.name, &pr.base.label),
                    number: pr.number as i32,
                    author_login: pr.user.login.clone(),
                    title: pr.title.clone(),
                })
                .collect::<Vec<_>>();
            tx.lock().unwrap().send(new_result).unwrap();
        },
    );

    Ok(rx.recv().unwrap())
}

pub fn create_pr(repo: &Repo) -> Result<()> {
    let token = env::var("GITHUB_TOKEN")?;

    let repo_clone = repo.clone();
    tokio::run_async(
        async move {
            let github = Github::new("SirVer_giti/unspecified", Some(Credentials::Token(token)));

    let pull_options = hubcaps::pulls::PullOptions {
        title: "My assume PR".to_string(),
        head: "master".to_string(),
        base: "SirVer:open_prs".to_string(),
        body: Some(" Lorem ipsum dolor sit amet, consectetur adipisicing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.".to_string()),
    };
    let foo = await!(github
        .repo(repo_clone.owner.to_string(), repo_clone.name.to_string())
        .pulls().create(&pull_options));
        println!("#sirver foo: {:#?}", foo);
        });
    Ok(())

}

pub fn get_pr(repo: &Repo, pr_id: i32) -> Result<PullRequest> {
    let token = env::var("GITHUB_TOKEN")?;

    let (tx, rx) = ::std::sync::mpsc::channel();
    let tx = ::std::sync::Mutex::new(tx);
    let repo_clone = repo.clone();
    tokio::run_async(
        async move {
            let github = Github::new("SirVer_giti/unspecified", Some(Credentials::Token(token)));
            let (_, pr) = await!(fetch_pr(github, repo_clone, pr_id as u64))
                .expect("fetch_pr did not complete.");
            tx.lock().unwrap().send(pr).unwrap();
        },
    );

    let pr = rx.recv().unwrap();
    println!("#sirver pr: {:#?}", pr);

    Ok(PullRequest {
        source: Branch::from_label(&repo.name, &pr.head.label),
        target: Branch::from_label(&repo.name, &pr.base.label),
        number: pr.number as i32,
        author_login: pr.user.login.clone(),
        title: pr.title.clone(),
    })
}
