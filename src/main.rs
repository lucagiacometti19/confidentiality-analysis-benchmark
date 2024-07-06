mod gh_api_search_code_response;
mod gh_api_search_repo_response;
use gh_api_search_code_response::GetCodeResponse;
use gh_api_search_repo_response::GetRepoResponse;

use chrono::{prelude::*, TimeDelta};
use dotenv::dotenv;
use reqwest::{
    header::{self, HeaderMap, HeaderValue},
    Client, Error, StatusCode,
};
use serde_json::error;
use std::{
    collections::{HashMap, HashSet},
    env,
    fs::{self, create_dir_all, OpenOptions},
    io::{copy, stdout, BufRead, BufReader, Read, Write},
    os::fd::{FromRawFd, IntoRawFd},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    str::FromStr,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
    vec,
};
use tempfile::tempfile;
use tokio;
use tokio::spawn;
use tokio::sync::Semaphore;
use toml::Value;
use walkdir::WalkDir;
use zip::ZipWriter;

extern crate pretty_env_logger;
#[macro_use]
extern crate log;

#[derive(Debug, Clone)]
struct ConfidentialityStats {
    file_path: String,
    modules_analyzed_amount: usize,
    functions_analyzed_amount: usize,
    structs_analyzed_amount: usize,
    total_diags_excluding_deps: usize,
    explicit_flow_via_call_amount: usize,
    explicit_flow_via_return_amount: usize,
    explicit_flow_via_moveto_amount: usize,
    explicit_flow_via_writeref_amount: usize,
    implicit_flow_via_call_amount: usize,
    implicit_flow_via_return_amount: usize,
}

#[tokio::main]
async fn main() {
    // load env variables
    dotenv().ok();
    //env::set_var("RUST_LOG", "info");
    // TODO: make logger prettier
    pretty_env_logger::init();
    // TEST FOR QUANTITATIVE ANALYSIS
    let mut sources = HashSet::new();
    for e_res in WalkDir::new("/home/move/github_crawler/quantitative_analysis_repos")
        .follow_links(false)
        .into_iter()
    {
        if let Ok(entry) = e_res {
            if let Some(ext) = entry.path().extension() {
                if entry.file_type().is_file() && ext.to_str().unwrap() == "move" {
                    let current_path = entry.path().to_str().unwrap().to_owned();
                    sources.insert(current_path);
                }
            }
        }
    }
    dbg!(&sources);
    for source in sources {
        if let Err(e) = quantitative_analysis(&source) {
            println!("{}", e);
        }
    }

    let res =
        collect_confidentiality_results("/home/move/github_crawler/quantitative_analysis_repos");
    if let Err(err) = save_confidentiality_stats(&res) {
        error!("{err}");
    }
    //if let Err(e) = quantitative_analysis(
    //    "/home/move/github_crawler/justinooi-aptos-token-swap-84192dd/sources/token-swap.move",
    //) {
    //    println!("{}", e);
    //}
    return;
    //let res = run_and_collect_confidentiality("/home/move/github_crawler/move_repo");
    //info!(
    //    "Successfully run the analysis on {} / {}",
    //    res.1 - res.0,
    //    res.1
    //);
    //return;

    //let root_dir = "/home/move/github_crawler/move_repo";
    //match run_and_collect_confidentiality(root_dir) {
    //    Err(err) => {
    //        for i in err {
    //            //println!("{i}");
    //        }
    //    }
    //    Ok(()) => (),
    //}

    let client = reqwest::Client::new();
    //let res = run_and_collect_confidentiality("/home/move/github_crawler/move_repo");
    //info!("Successfully run the analysis on {} / {}", res.0, res.1);
    //return;

    // Open the last commit date file
    let mut update_dates = HashMap::new();
    if let Ok(file) = OpenOptions::new().read(true).open("last_commit_dates.txt") {
        // Read line by line
        let reader = BufReader::new(file);
        for line_res in reader.lines() {
            if let Ok(line_str) = line_res {
                let splitted_line: Vec<&str> = line_str.split(',').collect();
                // index 0 is repo name
                let repo_full_name = splitted_line[0].to_owned();
                // index 1 is date
                let update_time = splitted_line[1].to_owned();
                // index 2 is the 'isNative' flag
                // only native move can run the confidentiality analysis currently
                let is_native_move = splitted_line[2] == "true";
                update_dates.insert(repo_full_name, (update_time, is_native_move));
            }
        }
    }

    let repo_links = get_repos_fullname(&client, &update_dates).await;
    // TODO debug only
    //let hs = HashSet::from([("VishnuKMi/Sample-ERC20-token-using-MOVE".to_owned(), NaiveDateTime::default())]);
    //download_repo(&client, &hs).await;

    let mut missing = HashSet::from(repo_links);
    while !missing.is_empty() {
        missing = download_repo(&client, &missing).await;
    }

    let execution_stats = run_and_collect_confidentiality("/home/move/github_crawler/move_repo");
    info!(
        "Successfully run the analysis on {} / {}",
        execution_stats.1 - execution_stats.0,
        execution_stats.1
    );

    let res = collect_confidentiality_results("/home/move/github_crawler/move_repo");
    if let Err(err) = save_confidentiality_stats(&res) {
        error!("{err}");
        return;
    }
    return;
    //return;

    /*
    for (ref repo_full_name, urls) in download_links {
        // Create directory based on repo full name
        match create_dir_all(repo_full_name) {
            Ok(_) => info!("Folder {} created successfully", repo_full_name),
            Err(err) => {
                error!("Couldn't create folder {}", repo_full_name);
                error!("{}", err);
                continue;
            }
        }

        for url in urls {
            // 10 retries with increasing timeout
            let mut retry = 0;
            // wait 200ms between requests
            let mut wait: TimeDelta = TimeDelta::try_milliseconds(200).unwrap();
            while retry < 10 {
                // Sleep before request
                info!("Sleeping for {} ms", wait.num_milliseconds());
                thread::sleep(Duration::from_millis(wait.num_milliseconds() as u64));
                info!("Attempt #{}", retry);

                // TODO: possible files with same name in same repo in different dirs
                // currently not handled, file will be overwritten
                // Get filename from url
                let file_name = match get_filename_from_url(&url) {
                    Some(filename) => filename,
                    None => continue,
                };

                // Open the file for writing with error handling
                let mut file = match OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(format!("{}/{}", repo_full_name, file_name))
                {
                    Ok(file) => file,
                    Err(err) => {
                        error!("Couldn't open or create file for: {}", url);
                        error!("{}", err);
                        continue;
                    }
                };

                // Download raw file content from github
                let downloaded_file = match download_raw_file(&client, &url).await {
                    Ok(file) => {
                        info!("Successfully downloaded file from: {}", url);
                        file
                    }
                    Err(err) => {
                        error!("Download request failed: {}", url);
                        error!("{}", err);
                        error!("Skipping to next file");
                        continue;
                    }
                };

                // Write to local file
                if let Err(err) = write!(file, "{}", downloaded_file) {
                    error!("Couldn't write downloaded file: {}", err);
                    retry += 1;
                    // Wait 2 more seconds next retry
                    wait = wait
                        .checked_add(&TimeDelta::try_seconds(2).unwrap())
                        .unwrap()
                } else {
                    info!("Successfully saved file from: {}", url);
                    // Success, no more retries needed
                    break;
                }
            }
        }
    }

    //let root_dir = "move_repos";
    //run_and_collect_confidentiality(root_dir);
    */
}

fn handle_response_headers(response_headers: &HeaderMap, response_status: StatusCode) -> TimeDelta {
    let mut primary_rate_limit_wait: TimeDelta = TimeDelta::try_milliseconds(500).unwrap();
    let mut secondary_rate_limit_wait: TimeDelta = TimeDelta::try_milliseconds(500).unwrap();

    let primary_rate_limit =
        if let Some(header_value) = response_headers.get("x-ratelimit-remaining") {
            header_value
                .to_str()
                .unwrap_or_else(|e| {
                    panic!(
                        "Couldn't parse 'x-ratelimit-remaining' header to string.\n{}",
                        e
                    )
                })
                .parse::<u32>()
                .unwrap_or_else(|e| {
                    panic!(
                        "Couldn't parse 'x-ratelimit-remaining' header to i32.\n{}",
                        e
                    )
                })
        } else {
            0
        };

    let primary_rate_limit_reset_timestamp =
        if let Some(header_value) = response_headers.get("x-ratelimit-reset") {
            header_value
                .to_str()
                .unwrap_or_else(|e| {
                    panic!(
                        "Couldn't parse 'x-ratelimit-remaining' header to string.\n{}",
                        e
                    )
                })
                .parse::<i64>()
                .unwrap_or_else(|e| {
                    panic!(
                        "Couldn't parse 'x-ratelimit-remaining' header to i32.\n{}",
                        e
                    )
                })
        } else {
            Utc::now().timestamp()
        };

    let reset = Utc
        .timestamp_opt(primary_rate_limit_reset_timestamp, 0)
        .single()
        .unwrap_or_else(|| panic!("Couldn't parse 'x-ratelimit-reset' to datetime"));

    if primary_rate_limit == 0 {
        // need to wait for primary_rate_limit_reset - compute how much we should wait + 1 secs
        primary_rate_limit_wait = (reset - Utc::now())
            .checked_add(&TimeDelta::try_seconds(1).unwrap())
            .unwrap_or_else(|| panic!("TimeDeltas add failed"));
    } else if response_headers.contains_key("retry-after") {
        // most likely caused by secondary rate limits, retry after timestamp
        let retry_after_timestamp = response_headers
            .get("retry-after")
            .unwrap()
            .to_str()
            .unwrap_or_else(|e| panic!("Couldn't parse 'retry-after' header to string.\n{}", e))
            .parse::<i64>()
            .unwrap_or_else(|e| panic!("Couldn't parse 'retry-after' header to i64.\n{}", e));

        let retry_after = Utc
            .timestamp_opt(retry_after_timestamp, 0)
            .single()
            .unwrap_or_else(|| panic!("Couldn't parse 'retry-after' to datetime"));

        secondary_rate_limit_wait = (retry_after - Utc::now())
            .checked_add(&TimeDelta::try_seconds(1).unwrap())
            .unwrap_or_else(|| panic!("TimeDeltas add failed"));
    } else if !response_status.is_success() {
        // most likely caused by secondary rate limits
        // since the header 'retry-after' is missing, double the wait and retry
        secondary_rate_limit_wait = secondary_rate_limit_wait
            .checked_add(&secondary_rate_limit_wait)
            .unwrap_or_else(|| panic!("TimeDeltas add failed"));
    }

    // return the max between the 2 delays
    primary_rate_limit_wait.max(secondary_rate_limit_wait)
}

fn append_last_commit_date_to_file(
    repo: &String,
    date: &String,
    usable: &bool,
) -> Result<(), String> {
    // Open the file for writing with error handling
    let mut file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open("last_commit_dates.txt")
    {
        Ok(file) => file,
        Err(err) => return Err(err.to_string()),
    };

    // Write to local file
    if let Err(err) = write!(file, "{},{},{}\n", repo, date, usable) {
        return Err(err.to_string());
    } else {
        Ok(())
    }
}

/// IF update_dates txt is present, this will only return repos that need to be
/// updated (usable move only) or downloaded for the first time (not necessarily move native).
/// Otherwise it will return ALL move repos with the last commit date associated.
///
/// TLDR: the return value is a set of repos that need to be downloaded and their last commit date.
async fn get_repos_fullname(
    client: &Client,
    update_dates: &HashMap<String, (String, bool)>,
) -> HashSet<(String, NaiveDateTime)> {
    let mut repo_links: HashSet<(String, NaiveDateTime)> = HashSet::new();

    // wait 1 second between requests
    let mut rate_limit_wait: TimeDelta = TimeDelta::try_milliseconds(100).unwrap();

    let mut size_range_start = 0.;
    let mut size_range_end = 5.;
    let mut keep_searching = true;

    while keep_searching {
        let mut has_next_page = true;
        let mut end_cursor = "null".to_string();

        // size range for search query
        let size_query = if size_range_start < 50. {
            format!("{}..{}", size_range_start, size_range_end)
        } else {
            keep_searching = false; // last search query
            format!(">={}", size_range_start)
        };

        info!(
            "Getting repositories that match the following size query: {}",
            size_query
        );

        while has_next_page {
            // post request headers
            let mut headers = header::HeaderMap::new();
            headers.insert(
                "authorization",
                format!(
                    "Bearer {}",
                    std::env::var("gh_api_token")
                        .expect("Github API token not found, set 'gh_api_token' in the .env")
                )
                .parse()
                .unwrap(),
            );
            headers.insert("X-GitHub-Api-Version", "2022-11-28".parse().unwrap());
            headers.insert("user-agent", "move_repo_benchmark/1.0".parse().unwrap());

            // post request body & send
            let response = client
            .post("https://api.github.com/graphql")
            .headers(headers)
            .json(&serde_json::json!(
                {
                    "query": "query
                            {
                                search(query: \"language:move size:#sizeRange\" type: REPOSITORY first: 100 after: #cursor)
                                {
                                    nodes
                                    {
                                        ... on Repository
                                        {
                                            url,
                                            nameWithOwner,
                                            description,
                                            pushedAt
                                        }
                                    }
                                    pageInfo
                                    {
                                        endCursor
                                        startCursor
                                        hasNextPage
                                        hasPreviousPage
                                    }
                                }
                            }"
                            .replace("#cursor", &end_cursor)
                            .replace("#sizeRange", size_query.as_str())
                }
            ))
            .send()
            .await
            .unwrap();

            // handle headers for ratelimits and status code
            rate_limit_wait = handle_response_headers(response.headers(), response.status());

            if response.status().is_success() {
                info!("Request successful: {}", response.status());

                let response_body = response.text().await.unwrap();

                let get_repo_response: GetRepoResponse =
                    serde_json::from_str(&response_body).unwrap_or_else(|e| panic!("{}", e));

                for repo in get_repo_response.data.search.repositories {
                    // Parse current commit date
                    let current_update_utc = NaiveDateTime::parse_from_str(
                        &repo.last_update.trim(),
                        "%Y-%m-%dT%H:%M:%S Z",
                    )
                    .unwrap();

                    if let Some((last_update_string, usable)) = update_dates.get(&repo.full_name) {
                        if *usable {
                            // Repo is in last commit date file and is a usable move project, parse last update date to UTC
                            let last_update_utc = NaiveDateTime::parse_from_str(
                                &last_update_string.trim(),
                                "%Y-%m-%dT%H:%M:%SZ",
                            )
                            .unwrap();
                            // Save only if there has been an update
                            if last_update_utc < current_update_utc {
                                warn!(
                                    "Repo {} has a new commit, getting the updated files",
                                    repo.full_name
                                );
                                /* match append_last_commit_date_to_file(
                                    &repo.full_name,
                                    &repo.last_update,
                                ) {
                                    Ok(_) => info!(
                                        "Added {} to last commit date file",
                                        repo.full_name.to_owned()
                                    ),
                                    Err(err) => {
                                        error!(
                                            "Couldn't add {} to last commit date file",
                                            repo.full_name
                                        );
                                        error!("{}", err);
                                    }
                                } */
                                repo_links.insert((repo.full_name, current_update_utc));
                            }
                        }
                    } else {
                        // Repo is not in last commit date file, add it with last commit date
                        /* match append_last_commit_date_to_file(&repo.full_name, &repo.last_update) {
                            Ok(_) => info!(
                                "Added {} to last commit date file",
                                repo.full_name.to_owned()
                            ),
                            Err(err) => {
                                error!("Couldn't add {} to last commit date file", repo.full_name);
                                error!("{}", err);
                            }
                        } */
                        repo_links.insert((repo.full_name, current_update_utc));
                    }
                }

                has_next_page = get_repo_response.data.search.page_info.has_next_page;
                end_cursor = format!("\"{}\"", get_repo_response.data.search.page_info.end_cursor);
            } else {
                error!("Request failed: {}", response.status())
            }

            // sleep before next request
            info!("Sleeping for {} ms", rate_limit_wait.num_milliseconds());
            thread::sleep(Duration::from_millis(
                rate_limit_wait.num_milliseconds() as u64
            ));
        }

        // modify search size range
        size_range_start = size_range_end;
        size_range_end += 5.;
    }

    info!(
        "Found {:?} repositories updated, downloading new content",
        repo_links.len()
    );
    repo_links
}

fn parse_link_header(link_header: Option<&HeaderValue>) -> Option<String> {
    if link_header.is_some() {
        let links: Vec<&str> = link_header.unwrap().to_str().unwrap().split(',').collect();
        match links
            .into_iter()
            .find(|&link| link.contains("rel=\"next\""))
        {
            Some(next_page_link) => {
                // remove first and last char, works with multi-byte unicode characters - just in case
                let mut link = next_page_link.split(';').collect::<Vec<&str>>()[0]
                    .trim()
                    .chars();
                link.next();
                link.next_back();
                Some(link.as_str().to_owned())
            }
            None => None,
        }
    } else {
        None
    }
}

async fn get_repos_move_content_url(
    client: &Client,
    repos: &HashSet<String>,
) -> HashMap<String, Vec<String>> {
    let mut raw_file_url: HashMap<String, Vec<String>> = HashMap::new();
    // wait 500ms between requests
    let mut rate_limit_wait: TimeDelta = TimeDelta::try_milliseconds(500).unwrap();

    // Loop trought all repositories
    for repo in repos {
        let mut next_page_link = Some(format!(
            "https://api.github.com/search/code?q=language:move+repo:{repo}&per_page=100"
        ));
        let mut file_count = 1;
        let mut files_found = 0;
        // Loop for gh api pagination, max 1000 results per search, if file count is 0 skip
        while files_found < file_count && raw_file_url.len() < 1001 && file_count != 0 {
            files_found = raw_file_url
                .entry(repo.to_owned())
                .or_insert(Vec::new())
                .len();

            // per_page={x} seems to not work properly in the last page, fallback to default
            // request when requesting last pages
            let link = match next_page_link {
                Some(ref link) => link.to_owned(),
                None => format!("https://api.github.com/search/code?q=language:move+repo:{repo}"),
            };

            let mut headers = header::HeaderMap::new();
            headers.insert(
                "authorization",
                format!(
                    "Bearer {}",
                    std::env::var("gh_api_token")
                        .expect("Github API token not found, set 'gh_api_token' in the .env")
                )
                .parse()
                .unwrap(),
            );
            headers.insert("X-GitHub-Api-Version", "2022-11-28".parse().unwrap());
            headers.insert("user-agent", "move_repo_benchmark/1.0".parse().unwrap());

            let response = client.get(link).headers(headers).send().await.unwrap();

            // handle headers for ratelimits and status code
            rate_limit_wait = handle_response_headers(response.headers(), response.status());

            if response.status().is_success() {
                info!("Request successful: {}", response.status());

                // Check for next page
                next_page_link = parse_link_header(response.headers().get("link"));

                // Get current page files
                let response_body = response.text().await.unwrap();
                let get_code_response: GetCodeResponse =
                    serde_json::from_str(&response_body).unwrap_or_else(|e| panic!("{}", e));

                if file_count == 1 {
                    file_count = get_code_response.len;
                }

                for file in get_code_response.files {
                    let file_path = file.path;
                    let ref_id = match file.url.find("?ref=") {
                        Some(index) => &file.url[index + 5..],
                        None => "",
                    };

                    // TODO: does this happen at all?
                    if ref_id.is_empty() {
                        error!("Couldn't find ref id, skipping {repo}");
                        // Either way this line should not be needed
                        next_page_link = None;
                        continue;
                    }

                    info!("Found raw download link for {repo}");

                    let raw_download_url =
                        format!("https://raw.githubusercontent.com/{repo}/{ref_id}/{file_path}");
                    raw_file_url
                        .entry(repo.to_owned())
                        .or_insert(Vec::new())
                        .push(raw_download_url);
                }
            } else {
                error!("Request failed: {}", response.status());
            }

            // sleep before next request
            info!("Sleeping for {} ms", rate_limit_wait.num_milliseconds());
            thread::sleep(Duration::from_millis(
                rate_limit_wait.num_milliseconds() as u64
            ))
        }
    }
    raw_file_url
}

/// Downloads + unzips repositories from `repos` and updates last commit date txt
async fn download_repo(
    client: &Client,
    repos: &HashSet<(String, NaiveDateTime)>,
) -> HashSet<(String, NaiveDateTime)> {
    let current_idx = Arc::new(Mutex::new(0));
    let fails = Arc::new(Mutex::new(HashSet::new()));
    // wait 200ms between requests
    let mut rate_limit_wait: TimeDelta = TimeDelta::try_milliseconds(200).unwrap();
    let semaphore = Arc::new(Semaphore::new(3));
    let mut requests: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    // Loop trought all repositories
    let repos_len = repos.len();
    let c_repos = repos.clone();
    for (repo, last_commit_date) in c_repos {
        let c_client = client.clone();
        let c_current_idx = current_idx.clone();
        let c_fails = fails.clone();
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        requests.push(spawn(async move {
            info!(
                "[{} | {}] Downloading {} zip",
                *c_current_idx.lock().unwrap(),
                repos_len,
                repo
            );
            *c_current_idx.lock().unwrap() += 1;

            let url = format!("https://api.github.com/repos/{}/zipball", repo);
            let mut headers = header::HeaderMap::new();
            headers.insert(
                "authorization",
                format!(
                    "Bearer {}",
                    std::env::var("gh_api_token")
                        .expect("Github API token not found, set 'gh_api_token' in the .env")
                )
                .parse()
                .unwrap(),
            );
            headers.insert("X-GitHub-Api-Version", "2022-11-28".parse().unwrap());
            headers.insert("user-agent", "move_repo_benchmark/1.0".parse().unwrap());
            let response = c_client.get(url).headers(headers).send().await.unwrap();

            // handle headers for ratelimits and status code
            rate_limit_wait = handle_response_headers(response.headers(), response.status());

            if response.status().is_success() {
                // save zip to temp file
                let mut tmpfile = tempfile::tempfile().unwrap();
                if let Err(err) = tmpfile.write(&mut response.bytes().await.unwrap()) {
                    error!("Couldn't save {} to temporary file, skipping", repo);
                    error!("{err}");
                    drop(permit);
                    return;
                }
                info!("Downloaded {} zip", repo);
                let parent_path = format!(
                    "/home/move/github_crawler/move_repo/{}",
                    repo.split_once('/').unwrap().0
                );

                // Create directory based on repo full name
                /* match create_dir_all(format!(
                    "/home/move/github_crawler/move_repo/{}",
                    repo.split_once('/').unwrap().0
                )) {
                    Ok(_) => info!("Folder {} created successfully", repo),
                    Err(err) => {
                        error!("Couldn't create folder {}", repo);
                        error!("{}", err);
                        continue;
                    }
                } */

                //let extracted_content = decompress_zip(&tmp).unwrap();

                // Save zip
                /* let repo_path = format!("/home/move/github_crawler/move_repo/{}.zip", repo);
                if let Err(err) = fs::write(&repo_path, tmp) {
                    error!("Couldn't write downloaded file: {}", err);
                    println!("{repo_path}");
                    continue;
                } else {
                    info!("Successfully saved zip of {}", repo);
                } */

                if let Err(_) = unzip_move_repo(tmpfile, &parent_path) {
                    // This type of move cannot be exectured by aptos cli, add this to last commit date txt
                    match append_last_commit_date_to_file(
                        &repo,
                        &last_commit_date.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                        &false,
                    ) {
                        Ok(_) => info!("Added {} to last commit date file", repo.to_owned()),
                        Err(err) => {
                            error!("Couldn't add {} to last commit date file", repo);
                            error!("{}", err);
                        }
                    }
                    error!("Repo {} is not native move, skipping", repo);
                } else {
                    // This is native move, add this to last commit date txt
                    match append_last_commit_date_to_file(
                        &repo,
                        &last_commit_date.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                        &true,
                    ) {
                        Ok(_) => info!("Added {} to last commit date file", repo.to_owned()),
                        Err(err) => {
                            error!("Couldn't add {} to last commit date file", repo);
                            error!("{}", err);
                        }
                    }
                    info!("Unzipped and saved repo {} in {}", repo, parent_path);
                    // todo: Do I really want to do this? May break some repos I think
                    // remove build directory and content
                    //for entry_res in WalkDir::new(&parent_path).follow_links(false).into_iter() {
                    //    if let Ok(entry) = entry_res {
                    //        let p = entry.path();
                    //        if p.is_dir() && p.file_name().unwrap().to_ascii_lowercase() == "build" {
                    //            println!("Removing directory: {:?}", parent_path);
                    //            if let Err(err) = remove_dir_all(p) {
                    //                error!("Couldn't remove build directory from {}", repo);
                    //                error!("{err}");
                    //            }
                    //        }
                    //    }
                    //}
                }
            } else {
                error!("Request failed: {}", response.status());
                c_fails
                    .lock()
                    .unwrap()
                    .insert((repo.to_owned(), last_commit_date.to_owned()));
            }

            // sleep before next request
            info!("Sleeping for {} ms", rate_limit_wait.num_milliseconds());
            thread::sleep(Duration::from_millis(
                rate_limit_wait.num_milliseconds() as u64
            ));
            drop(permit);
        }));
    }

    for req in requests {
        req.await.unwrap();
    }
    Arc::try_unwrap(fails)
        .expect("Lock still has multiple owners")
        .into_inner()
        .expect("Mutex cannot be locked")
    //fails.lock().unwrap()
}

async fn download_raw_file(client: &Client, url: &String) -> Result<String, StatusCode> {
    let response = client.get(url).send().await.unwrap();

    if response.status().is_success() {
        Ok(response.text().await.unwrap())
    } else {
        Err(response.status())
    }
}

fn get_filename_from_url(url: &String) -> Option<String> {
    // Split the URL by '/' and get the last element (filename)
    let mut parts = url.rsplit('/');

    // Handle empty or single-element strings
    if let Some(last) = parts.next() {
        return Some(last.to_string());
    }
    None
}

fn run_and_collect_confidentiality(root_dir: &str) -> (i32, i32) {
    //let mut failed_executions = Vec::new();
    let mut fails = 0;
    let mut total = 0;
    for e_res in WalkDir::new(root_dir).follow_links(false).into_iter() {
        if let Ok(entry) = e_res {
            if let Some(ext) = entry.path().extension() {
                if entry.file_type().is_file() && ext.to_str().unwrap() == "move" {
                    total += 1;
                    let current_path = entry.path().to_str().unwrap().to_owned();
                    let cp_path = Path::new(&current_path);
                    let cp_filename = cp_path.file_name().unwrap().to_str().unwrap();
                    let cp_filestem = cp_path.file_stem().unwrap().to_str().unwrap();
                    if let Some(p) = cp_path.parent() {
                        // Open the file for writing with error handling
                        let output_file = format!("{}/{cp_filestem}.txt", p.to_str().unwrap());
                        match OpenOptions::new()
                            .create(true)
                            .write(true)
                            .truncate(true)
                            .open(&output_file)
                        {
                            Ok(file) => {
                                info!(
                                    "Running confidentiality analysis for {} - {}",
                                    current_path, cp_filename
                                );
                                if !Command::new("/home/move/github_crawler/aptos")
                                    .args([
                                        "move",
                                        "prove",
                                        "--run-confidentiality",
                                        "--package-dir",
                                        &current_path,
                                        "-f",
                                        cp_filename,
                                    ])
                                    .stdout(unsafe { Stdio::from_raw_fd(file.into_raw_fd()) })
                                    .status()
                                    .expect("failed to execute process")
                                    .success()
                                {
                                    error!(
                                        "Execution of confidentiality analysis for {} failed",
                                        current_path
                                    );
                                    fails += 1;
                                } else {
                                    /* if let Err(e) = fs::write(output_file, b"\nsuccess") {
                                        error!(
                                            "Couldn't append success flag to analysis output: {}",
                                            e
                                        );
                                        // todo debug only
                                        exit(1);
                                    } */
                                }
                            }
                            Err(err) => {
                                error!("Cannot open/create {:?}", output_file);
                                error!("{:?}", err);
                                //failed_executions.push(current_path)
                            }
                        };
                    } else {
                        error!("{current_path} is the root directory or an invalid path");
                        //failed_executions.push(current_path)
                    };
                }
            }
        }
    }
    (fails, total)
}

fn unzip_move_repo(tmp_zip_file: fs::File, base_path: &String) -> Result<(), ()> {
    if can_be_executed(&tmp_zip_file) {
        let mut archive = zip::ZipArchive::new(tmp_zip_file).unwrap();
        for i in 0..archive.len() {
            let mut current = archive.by_index(i).unwrap();
            // get relative path and produce abs path from base path
            let rel_outpath = match current.enclosed_name() {
                Some(p) => p,
                None => {
                    error!("Couldn't get relative path of current content");
                    continue;
                }
            };
            let current_path = format!("{}/{}", base_path, rel_outpath.to_str().unwrap());
            if current.is_dir() {
                // create dirs
                if let Err(err) = create_dir_all(&current_path) {
                    error!("Couldn't create folder {}", current_path);
                    error!("{}", err);
                    continue;
                }
            } else if current.is_file() && rel_outpath.extension().unwrap() == "Identifier" {
                // skip zone identifier file, they are just noise
                // extract and save file to current_path
                let mut outfile = match OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(&current_path)
                {
                    Ok(file) => file,
                    Err(err) => {
                        error!("Couldn't open or create file: {}", current_path);
                        error!("{}", err);
                        continue;
                    }
                };

                if let Err(err) = copy(&mut current, &mut outfile) {
                    error!("Couldn't extract from temp file to {}", current_path);
                    error!("{err}");
                }
            }
        }
        return Ok(());
    }
    Err(())
}

//fn unzip_move_repo(tmp_zip_file: fs::File, base_path: &String) -> Result<(), Vec<String>> {
/* for e_res in WalkDir::new(root_dir).follow_links(false).into_iter() {
    if let Ok(entry) = e_res {
        let current_path = Path::new(entry.path().to_str().unwrap());
        println!("{:?}", current_path);
        if let Some(p) = current_path.parent() {
            if entry.file_type().is_file() {
                // create parent directory if it doesn't exist
                // TODO: do i need this check?
                if !p.exists() {
                    create_dir_all(&p).unwrap();
                }
            } else if entry.file_type().is_dir() {
                // create directory
                create_dir_all(current_path).unwrap();
            }
        }
    }
} */

//let mut DUMMY_TOML = "[package]
//name = \"DUMMY\"
//version = \"0.0.0\"
//
//[dependencies]
//MoveStdlib = { git = \"https://github.com/move-language/move.git\", subdir = \"language/move-stdlib\", rev = \"main\" }
//MoveNursery = { git = \"https://github.com/move-language/move.git\", subdir = \"language/move-stdlib/nursery\", rev = \"main\" }
//
//[addresses]
//std = \"0x1\"
//".to_string();

// TODO: think of a legit return logic
// Don't unzip non native move
//let (code, toml_paths) = can_be_executed(&tmp_zip_file);
//if code == 0 {
//    // non native move, just exit
//    return Err(Vec::new());
//}
//let mut archive = zip::ZipArchive::new(tmp_zip_file).unwrap();
//if code == 2 {
//    // use dummy toml in toml_paths and manually unzip the rest
//    for i in 0..archive.len() {
//        let mut current = archive.by_index(i).unwrap();
//        // get relative path and produce abs path from base path
//        let rel_outpath = match current.enclosed_name() {
//            Some(p) => p,
//            None => {
//                error!("Couldn't get relative path of current content");
//                continue;
//            }
//        };
//        let current_path = format!("{}/{}", base_path, rel_outpath.to_str().unwrap());
//        if current.is_dir() {
//            // create dirs
//            if let Err(err) = create_dir_all(&current_path) {
//                error!("Couldn't create folder {}", current_path);
//                error!("{}", err);
//                continue;
//            }
//        } else if current.is_file() {
//            let mut outfile = match OpenOptions::new()
//                .create(true)
//                .write(true)
//                .open(&current_path)
//            {
//                Ok(file) => file,
//                Err(err) => {
//                    error!("Couldn't open or create file: {}", current_path);
//                    error!("{}", err);
//                    continue;
//                }
//            };
//            let mut found = false;
//            for (p, address) in &toml_paths {
//                if p == rel_outpath {
//                    // the current file is the package manifest to change with the dummy toml
//                    found = true;
//                    let add_string = &address.join("\n");
//                    // push addresses to dummy toml
//                    let dummy = format!("{DUMMY_TOML}{add_string}");
//                    if let Err(err) = copy(&mut dummy.as_bytes(), &mut outfile) {
//                        error!("Couldn't copy dummy toml to {}", current_path);
//                        error!("{err}");
//                    }
//                }
//            }
//            if !found {
//                // extract and save file to current_path
//                if let Err(err) = copy(&mut current, &mut outfile) {
//                    error!("Couldn't extract from temp file to {}", current_path);
//                    error!("{err}");
//                }
//            }
//        }
//    }
//    return Ok(());
//} else if code == 3 {
//    // no toml, place one dummy toml in root directory
//    archive.extract(&base_path);
//    let toml_path = format!("{}/{}", base_path, toml_paths[0].0.to_str().unwrap());
//    let mut outfile = match OpenOptions::new().create(true).write(true).open(&toml_path) {
//        Ok(file) => file,
//        Err(err) => {
//            error!("Couldn't open or create file: {:?}", toml_path);
//            error!("{}", err);
//            return Err(Vec::new());
//        }
//    };
//    if let Err(err) = copy(&mut DUMMY_TOML.as_bytes(), &mut outfile) {
//        error!("Couldn't copy dummy toml to {:?}", toml_path);
//        error!("{err}");
//        return Err(Vec::new());
//    }
//    return Ok(());
//} else {
//    // native move, just extract all the content
//    archive.extract(&base_path);
//    return Ok(());
//}

//let mut fails = Vec::new();
/*     for i in 0..archive.len() {
    let mut current = archive.by_index(i).unwrap();
    // get relative path and produce abs path from base path
    let rel_outpath = match current.enclosed_name() {
        Some(p) => p,
        None => {
            error!("Couldn't get relative path of current content");
            fails.push(base_path.to_owned());
            continue;
            //return Err("Couldn't find current zip content relative path".to_owned())
        }
    };
    let current_path = format!("{}/{}", base_path, rel_outpath.to_str().unwrap());
    if current.is_dir() {
        // create dirs
        if let Err(err) = create_dir_all(&current_path) {
            error!("Couldn't create folder {}", current_path);
            error!("{}", err);
            fails.push(base_path.to_owned());
        }
    } else if current.is_file() {
        // extract and save file to current_path
        let mut outfile = match OpenOptions::new()
            .create(true)
            .write(true)
            .open(&current_path)
        {
            Ok(file) => file,
            Err(err) => {
                error!("Couldn't open or create file: {}", current_path);
                error!("{}", err);
                fails.push(base_path.to_owned());
                continue;
            }
        };
        if let Err(err) = copy(&mut current, &mut outfile) {
            error!("Couldn't extract from temp file to {}", current_path);
            error!("{err}");
            fails.push(base_path.to_owned());
        }
    }
} */
//}

//fn can_be_executed(tmp_zip_file: &fs::File) -> (u8, Vec<(PathBuf, Vec<String>)>) {
//    // 0: non native/aptos
//    // 1: native,
//    // 2: native but manifest has local deps, use dummy toml,
//    // 3: native but no toml, use dummy,
//    let mut is_native: u8 = 1;
//    let mut sources_dir_path = PathBuf::from_str("Move.toml").unwrap();
//    let mut paths_to_remove = vec![];
//    let mut found_toml = false;
//    let mut archive = zip::ZipArchive::new(tmp_zip_file).unwrap();
//    // First iteration to check the move.toml file for native move (no Sui/Aptos/...)
//    for i in 0..archive.len() {
//        let mut current = archive.by_index(i).unwrap();
//        let rel_outpath = match current.enclosed_name() {
//            Some(p) => p.to_owned(),
//            None => {
//                error!("Couldn't get relative path of current content");
//                continue;
//            }
//        };
//        if current.is_file() {
//            if rel_outpath.file_name().unwrap().to_ascii_lowercase() == "move.toml" {
//                found_toml = true;
//                let mut toml = String::new();
//                if let Err(err) = current.read_to_string(&mut toml) {
//                    error!("Couldn't read move.toml");
//                    error!("{}", err);
//                    continue;
//                }
//                // check for different move flavours or deps to local directory
//                if toml.to_ascii_lowercase().contains("sui")
//                    || toml.to_ascii_lowercase().contains("aptos")
//                    || toml.to_ascii_lowercase().contains("starcoin")
//                    || toml.to_ascii_lowercase().contains("0lnetwork")
//                {
//                    is_native = 0;
//                    break;
//                } else if toml.to_ascii_lowercase().contains("local") {
//                    // assume it's a local reference to move stdlib and nursery
//                    // return this move.toml path to replace it with the dummy manifest later on
//                    is_native = 2;
//                    let mut addresses = vec![];
//                    let parsed_toml: Value = toml::from_str(&toml).unwrap();
//                    // [addresses] section
//                    if let Some(addresses_table) = parsed_toml.get("addresses") {
//                        for (key, val) in addresses_table.as_table().unwrap() {
//                            if key != "std" {
//                                let add = format!("{key} = {}", val.to_string());
//                                addresses.push(add);
//                            }
//                        }
//                    } else {
//                        println!("No [addresses] section found in the TOML data.");
//                    }
//                    paths_to_remove.push((rel_outpath, addresses));
//                }
//            }
//        } else {
//            if rel_outpath.file_name().unwrap().to_ascii_lowercase() == "sources" {
//                sources_dir_path = rel_outpath.parent().unwrap().to_owned().join("Move.toml");
//            }
//        }
//    }
//    if found_toml {
//        (is_native, paths_to_remove)
//    } else {
//        // TODO: may want to create a dummy move.toml to run the prover on these kind of repos
//        // no move.toml found, consider this as native move and use dummy toml
//        (3, vec![(sources_dir_path, vec![])])
//    }
//}

// currently executes both native & aptos sui via the aptos cli & prover
fn can_be_executed(tmp_zip_file: &fs::File) -> bool {
    let mut can_be_executed = true;
    let mut archive = zip::ZipArchive::new(tmp_zip_file).unwrap();
    // First iteration to check the move.toml file for native move (no Sui/Starcoin/...)
    for i in 0..archive.len() {
        let mut current = archive.by_index(i).unwrap();
        let rel_outpath = match current.enclosed_name() {
            Some(p) => p.to_owned(),
            None => {
                error!("Couldn't get relative path of current content");
                continue;
            }
        };
        if current.is_file() {
            if rel_outpath.file_name().unwrap().to_ascii_lowercase() == "move.toml" {
                let mut toml = String::new();
                if let Err(err) = current.read_to_string(&mut toml) {
                    error!("Couldn't read move.toml");
                    error!("{}", err);
                    continue;
                }
                // check for different move flavours or deps to local directory
                if toml.to_ascii_lowercase().contains("sui")
                    || toml.to_ascii_lowercase().contains("starcoin")
                    || toml.to_ascii_lowercase().contains("0lnetwork")
                {
                    can_be_executed = false;
                    break;
                }
            }
        }
    }
    can_be_executed
}

fn collect_confidentiality_results(root_dir: &str) -> HashMap<String, Vec<ConfidentialityStats>> {
    let mut analysis_output: HashMap<String, Vec<ConfidentialityStats>> = HashMap::new();
    let total = WalkDir::new(root_dir)
        .follow_links(false)
        .into_iter()
        .count();
    let mut current = 0;
    for e_res in WalkDir::new(root_dir).follow_links(false).into_iter() {
        current += 1;
        info!("{} | {}", current, total);
        if let Ok(entry) = e_res {
            if let Some(ext) = entry.path().extension() {
                if entry.file_type().is_file() {
                    if ext.to_str().unwrap() == "txt" {
                        // parse the current txt
                        let bytes = fs::read(entry.path()).unwrap_or_default();
                        // replaces not valid utf-8 with REPLACEMENT_CHARACTER �
                        let content = String::from_utf8_lossy(bytes.as_slice());
                        // content empty most likely means failed analysis execution, skip this file
                        if !content.is_empty()
                            && content
                                .lines()
                                .nth(content.lines().count().checked_sub(2).unwrap_or(0))
                                .unwrap()
                                .to_ascii_lowercase()
                                .contains("success")
                        {
                            let filepath = entry.path().to_str().unwrap();
                            let mut module_count = 0_usize;
                            let mut function_count = 0_usize;
                            let mut struct_count = 0_usize;
                            let mut total_diags = 0_usize;
                            let mut explicit_flow_via_ret_diag_count = 0_usize;
                            let mut explicit_flow_via_call_diag_count = 0_usize;
                            let mut explicit_flow_via_moveto_diag_count = 0_usize;
                            let mut explicit_flow_via_writeref_diag_count = 0_usize;
                            let mut implicit_flow_via_ret_diag_count = 0_usize;
                            let mut implicit_flow_via_call_diag_count = 0_usize;
                            // this should be good enough to get the first line of each diagnostic
                            let diags: Vec<(usize, &str)> = content
                                .lines()
                                .enumerate()
                                .filter(|&(_, line)| {
                                    line.starts_with("warning:") || line.starts_with("Analyzing")
                                })
                                .collect();
                            for (idx, line) in diags {
                                if line.starts_with("Analyzing") {
                                    // this line is not a diag, get module/fn/struct count
                                    let mut tmp = vec![];
                                    for part in line.split_whitespace() {
                                        if let Ok(res) = part.parse::<usize>() {
                                            tmp.push(res);
                                        }
                                    }
                                    (module_count, function_count, struct_count) =
                                        (tmp[0], tmp[1], tmp[2]);
                                    continue;
                                }
                                if content.lines().nth(idx + 1).unwrap().contains("/.move/")
                                    || !content.lines().nth(idx + 1).unwrap().contains(
                                        entry.path().file_stem().unwrap().to_str().unwrap(),
                                    )
                                {
                                    // this mean that the diag is reporting a leak from deps bytecode or another source
                                    // do not count it
                                    continue;
                                }
                                // otherwise it's a diag, parse it
                                total_diags += 1;
                                if line.contains("Explicit") {
                                    if line.contains("via call") {
                                        explicit_flow_via_call_diag_count += 1;
                                    } else if line.contains("via return") {
                                        explicit_flow_via_ret_diag_count += 1;
                                    } else if line.contains("via moveTo") {
                                        explicit_flow_via_moveto_diag_count += 1;
                                    } else if line.contains("via write ref") {
                                        explicit_flow_via_writeref_diag_count += 1;
                                    }
                                } else if line.contains("Implicit") {
                                    if line.contains("via call") {
                                        implicit_flow_via_call_diag_count += 1;
                                    } else if line.contains("via return") {
                                        implicit_flow_via_ret_diag_count += 1;
                                    }
                                }
                            }
                            info!(
                                "{} \n- modules: {}\n- functions: {}\n- structs: {}\n- number of diagnostics: {} \n- expl flow via ret: {} \n- expl flow via call: {} \n- expl flow via moveTo: {} \n- expl flow via writeRef: {} \n- impl flow via ret: {} \n- impl flow via call: {}",
                                filepath,
                                module_count,
                                function_count,
                                struct_count,
                                total_diags,
                                explicit_flow_via_ret_diag_count,
                                explicit_flow_via_call_diag_count,
                                explicit_flow_via_moveto_diag_count,
                                explicit_flow_via_writeref_diag_count,
                                implicit_flow_via_ret_diag_count,
                                implicit_flow_via_call_diag_count
                            );
                            let path_split: Vec<&str> = filepath.split("_spec_").collect();
                            let base_path = if path_split.len() >= 2 {
                                path_split[path_split.len() - 2].to_owned()
                            } else {
                                filepath.split(".txt").nth(0).unwrap().to_owned()
                            };
                            if let Some(conf_stats) = analysis_output.get_mut(&base_path) {
                                // get mut reference to inner vec and push new stats
                                conf_stats.push(ConfidentialityStats {
                                    file_path: filepath.to_owned(),
                                    modules_analyzed_amount: module_count,
                                    functions_analyzed_amount: function_count,
                                    structs_analyzed_amount: struct_count,
                                    total_diags_excluding_deps: total_diags,
                                    explicit_flow_via_return_amount:
                                        explicit_flow_via_ret_diag_count,
                                    explicit_flow_via_call_amount:
                                        explicit_flow_via_call_diag_count,
                                    explicit_flow_via_moveto_amount:
                                        explicit_flow_via_moveto_diag_count,
                                    explicit_flow_via_writeref_amount:
                                        explicit_flow_via_writeref_diag_count,
                                    implicit_flow_via_return_amount:
                                        implicit_flow_via_ret_diag_count,
                                    implicit_flow_via_call_amount:
                                        implicit_flow_via_call_diag_count,
                                })
                            } else {
                                // add the new key with the new stats
                                analysis_output.insert(
                                    base_path,
                                    vec![ConfidentialityStats {
                                        file_path: filepath.to_owned(),
                                        modules_analyzed_amount: module_count,
                                        functions_analyzed_amount: function_count,
                                        structs_analyzed_amount: struct_count,
                                        total_diags_excluding_deps: total_diags,
                                        explicit_flow_via_return_amount:
                                            explicit_flow_via_ret_diag_count,
                                        explicit_flow_via_call_amount:
                                            explicit_flow_via_call_diag_count,
                                        explicit_flow_via_moveto_amount:
                                            explicit_flow_via_moveto_diag_count,
                                        explicit_flow_via_writeref_amount:
                                            explicit_flow_via_writeref_diag_count,
                                        implicit_flow_via_return_amount:
                                            implicit_flow_via_ret_diag_count,
                                        implicit_flow_via_call_amount:
                                            implicit_flow_via_call_diag_count,
                                    }],
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    analysis_output
}

fn save_confidentiality_stats(
    res: &HashMap<String, Vec<ConfidentialityStats>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Open the file for writing with error handling
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .open("conf-res.txt")?;
    let (
        total_modules,
        total_functions,
        total_structs,
        total_diags,
        expl_ret,
        expl_call,
        expl_moveto,
        expl_writeref,
        impl_ret,
        impl_call,
    ) = res.values().flatten().fold(
        (0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
        |(
            mods,
            fns,
            structs,
            total,
            expl_ret,
            expl_call,
            expl_moveto,
            expl_writeref,
            impl_ret,
            impl_call,
        ),
         current| {
            return (
                mods + current.modules_analyzed_amount,
                fns + current.functions_analyzed_amount,
                structs + current.structs_analyzed_amount,
                total + current.total_diags_excluding_deps,
                expl_ret + current.explicit_flow_via_return_amount,
                expl_call + current.explicit_flow_via_call_amount,
                expl_moveto + current.explicit_flow_via_moveto_amount,
                expl_writeref + current.explicit_flow_via_writeref_amount,
                impl_ret + current.implicit_flow_via_return_amount,
                impl_call + current.implicit_flow_via_call_amount,
            );
        },
    );
    let l = format!("total modules: {} - total functions: {} - total structs: {} - total diagnostics: {} - total expl flow via ret: {} - total expl flow via call: {} - total expl flow via moveTo: {} - total expl flow via writeRef: {} - total impl flow via ret: {} - total impl flow via call: {}", total_modules, total_functions, total_structs, total_diags, expl_ret, expl_call, expl_moveto, expl_writeref, impl_ret, impl_call);
    write!(file, "{l}\n")?;
    let mut line;
    for (base_path, stats) in res {
        write!(
            file,
            "Confidentiality analysis stats of repo {}\n",
            base_path
        )?;
        // todo: this is kinda of a performance killer do i really wanna keep this?
        let mut c_stats = stats.clone();
        c_stats.sort_unstable_by_key(|stat| stat.file_path.len());
        for file_stats in c_stats {
            if file_stats.total_diags_excluding_deps == 0 {
                line = format!("{} - no diags", file_stats.file_path);
            } else {
                line = format!(
                    "{} - number of diagnostics: {} - expl flow via ret: {} - expl flow via call: {} - expl flow via moveTo: {} - expl flow via writeRef: {} - impl flow via ret: {} - impl flow via call: {}",
                    file_stats.file_path, file_stats.total_diags_excluding_deps, file_stats.explicit_flow_via_return_amount, file_stats.explicit_flow_via_call_amount, file_stats.explicit_flow_via_moveto_amount, file_stats.explicit_flow_via_writeref_amount, file_stats.implicit_flow_via_return_amount, file_stats.implicit_flow_via_call_amount
                );
            }
            write!(file, "{}\n", line)?;
        }
        write!(file, "{}\n", "-".repeat(20))?;
    }
    Ok(())
}

trait DefaultValue {
    fn default(&self) -> &str;
}

enum MovePrimitiveType {
    Integer,
    Bool,
    Address,
}

impl DefaultValue for MovePrimitiveType {
    fn default(&self) -> &str {
        match *self {
            MovePrimitiveType::Integer => "0",
            MovePrimitiveType::Bool => "true",
            MovePrimitiveType::Address => "@0xCAFE",
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct MoveStruct {
    name: String,
    declaration_span: (usize, usize),
    fields: Vec<MoveField>,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct MoveField {
    name: String,
    _type: String,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct MoveModule {
    address: String,
    name: String,
    declaration_line: usize,
}

#[derive(Debug, PartialEq, Eq, Default)]
struct MoveSourceMetadata {
    modules: HashSet<MoveModule>,
    structs: HashSet<MoveStruct>,
}

impl MoveSourceMetadata {
    fn parse_move_source_metadata(
        code: std::borrow::Cow<str>,
    ) -> Result<MoveSourceMetadata, String> {
        let mut move_source_metadata = MoveSourceMetadata::default();
        let mut current_struct: Option<MoveStruct> = None;
        let mut current_fields: Vec<MoveField> = Vec::new();

        for (idx, line) in code.lines().enumerate() {
            let trimmed_line = line.trim();
            // Check for module definition
            if trimmed_line.starts_with("module") {
                let module_path = trimmed_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .split_once("::")
                    .unwrap();
                move_source_metadata.modules.insert(MoveModule {
                    address: module_path.0.to_owned(),
                    name: module_path.1.to_owned(),
                    declaration_line: idx,
                });
            }
            // Check for struct definition start
            else if trimmed_line.starts_with("struct") || trimmed_line.ends_with("}") {
                if trimmed_line.starts_with("struct") {
                    let struct_name = trimmed_line
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .split("<")
                        .next()
                        .unwrap();
                    current_struct = Some(MoveStruct {
                        name: struct_name.to_string(),
                        declaration_span: (idx, 0),
                        fields: vec![],
                    });
                    current_fields = Vec::new();
                }
                if trimmed_line.ends_with("}") {
                    if let Some(mut move_struct) = current_struct {
                        move_struct.declaration_span.1 = idx;
                        move_struct.fields = current_fields;
                        move_source_metadata.structs.insert(move_struct);
                    }
                    current_struct = None;
                    current_fields = Vec::new();
                }
            } else if !trimmed_line.is_empty()
                && !trimmed_line.starts_with("//")
                && current_struct.is_some()
            {
                // Extract field name and type (heuristic approach)
                let parts = trimmed_line.split_once(":").unwrap();
                let field_name = parts.0.trim().to_string();
                let field_type = if parts.1.trim().ends_with(',') {
                    let mut tmp_chars = parts.1.trim().chars();
                    tmp_chars.next_back();
                    tmp_chars.as_str()
                } else {
                    parts.1.trim()
                };
                current_fields.push(MoveField {
                    name: field_name,
                    _type: field_type.to_owned(),
                });
            }
        }

        if current_struct.is_some() {
            return Err(String::from("Unterminated struct definition"));
        }

        Ok(move_source_metadata)
    }
}

fn create_spec(move_struct: &MoveStruct, selected_field: &MoveField) -> Option<String> {
    let val = match selected_field._type.as_str() {
        "u8" | "u16" | "u32" | "u64" | "u128" | "u256" => MovePrimitiveType::Integer,
        "bool" => MovePrimitiveType::Bool,
        "address" => MovePrimitiveType::Address,
        _ => return None,
    };
    Some(format!(
        "    spec {} {{\n        invariant {} == {};\n    }}\n",
        move_struct.name,
        selected_field.name,
        val.default()
    ))
}

fn quantitative_analysis(source_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fs::read(source_path)?;
    // replaces not valid utf-8 with REPLACEMENT_CHARACTER �
    let content = String::from_utf8_lossy(bytes.as_slice());
    if !content.is_empty() {
        // get structs from move source
        let src_metadata = MoveSourceMetadata::parse_move_source_metadata(content).unwrap();
        // quantitative analysis
        let source_code_path = PathBuf::from_str(source_path).unwrap();
        let mut quantitative_analysis_tmp_file_paths: HashSet<String> = HashSet::new();
        for s in src_metadata.structs {
            //println!("Struct name: {}", s.name);
            for field in &s.fields {
                //let quantitative_analysis_tmp_file_path = PathBuf::from();
                //quantitative_analysis_tmp_file_path.push(PathBuf::from(.))

                //println!("Field {idx}: {} | {}", field.name, field._type);
                let spec_lines: Vec<_> = create_spec(&s, field)
                    .unwrap_or_default()
                    .split("\n")
                    .map(|line| line.to_owned())
                    .collect();
                if spec_lines.len() != 1 {
                    // currently need to modify/hide the file name to prevent matching of the
                    // -f filter option of the move prover. ugly
                    //let mut idk_read_the_comment = source_code_path
                    //    .file_stem()
                    //    .unwrap()
                    //    .to_str()
                    //    .unwrap()
                    //    .chars();
                    //idk_read_the_comment.next_back();
                    let qa_tmp_file_path = format!(
                        "{}/{}_spec_{}_{}.move",
                        source_code_path.parent().unwrap().to_str().unwrap(),
                        source_code_path.file_stem().unwrap().to_str().unwrap(),
                        s.name,
                        field.name
                    );
                    quantitative_analysis_tmp_file_paths.insert(qa_tmp_file_path.clone());

                    // change module name to prevent prover error
                    let mut lines: Vec<_> =
                        bytes.as_slice().lines().collect::<Result<_, _>>().unwrap();
                    for module in &src_metadata.modules {
                        lines[module.declaration_line] = format!(
                            "module {}::{}_spec_{}_{} {{",
                            module.address, module.name, s.name, field.name
                        );
                    }
                    // add the new spec to the lines
                    lines.splice(
                        s.declaration_span.1 + 1..s.declaration_span.1 + 1,
                        spec_lines.clone(),
                    );
                    //println!("Modified Doc:\n{}", lines.join("\n"));
                    let mut quantitative_analysis_tmp_file = match OpenOptions::new()
                        .create(true)
                        .write(true)
                        .open(&qa_tmp_file_path)
                    {
                        Ok(file) => file,
                        Err(err) => {
                            error!("Couldn't open or create file: {}", qa_tmp_file_path);
                            error!("{}", err);
                            continue;
                        }
                    };

                    if let Err(err) = write!(quantitative_analysis_tmp_file, "{}", lines.join("\n"))
                    {
                        error!("Couldn't write to file: {}", qa_tmp_file_path);
                        error!("{err}");
                        continue;
                    }
                }
            }
        }
        // now all quantitative analysis intermediate files have been generated and saved.
        // run confidentiality analysis, then remove intermediate quantitative analysis files
        // todo: this is not optimal placed here. The base source files will be run multiple times.
        // the spec ones will correctly be run here, but all the base ones unchanged will be re-executed
        // currenly fixed by running on parent dir and not on parent.parent dir, not sure it works all times tho
        let execution_stats =
            run_and_collect_confidentiality(source_code_path.parent().unwrap().to_str().unwrap());
        info!(
            "Successfully run the analysis on {} / {}",
            execution_stats.1 - execution_stats.0,
            execution_stats.1
        );

        // remove intermediate quantitative analysis files
        for qa_tmp_path in quantitative_analysis_tmp_file_paths {
            fs::remove_file(&qa_tmp_path)?;
        }
    }
    Ok(())
}
