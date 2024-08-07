use crate::github_api_utils::{search_code_api_response, search_repo_api_response};

use super::last_commit_dates::append_last_commit_date_to_file;
use chrono::{NaiveDateTime, TimeDelta, TimeZone, Utc};
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, StatusCode};
use std::{
    collections::{HashMap, HashSet},
    fs::{create_dir_all, File, OpenOptions},
    io::{copy, Read, Write},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tokio::{spawn, sync::Semaphore};

/// Gets the link to the next page of the github graphql query result
fn get_next_page_from_response_header(link_header: Option<&HeaderValue>) -> Option<String> {
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

/// Creates and sets standard github api request headers:
/// 1. **authorization**, using the api key from the enviroment variable *GH_API_KEY*.
/// 2. **X-GitHub-Api-Version**, github api version.
/// 3. **user-agent**, using the enviroment variable *GH_API_USER_AGENT*.
fn get_requests_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        format!(
            "Bearer {}",
            std::env::var("GH_API_KEY")
                .expect("Github API token not found, set 'GH_API_KEY' in the .env")
        )
        .parse()
        .unwrap(),
    );
    headers.insert("X-GitHub-Api-Version", "2022-11-28".parse().unwrap());
    headers.insert(
        "user-agent",
        format!(
            "{}",
            std::env::var("GH_API_USER_AGENT").expect("Set 'GH_API_USER_AGENT' in the .env")
        )
        .parse()
        .unwrap(),
    );
    headers
}

/// Uses github response headers to handle ratelimits:
/// returns the amount of time to wait before the next request.
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

/// Checks inside the repo zip for the move.toml file.  
/// If the move.toml contains any reference to sui | starcoin | 0lnetwork the function
/// returns false, since these move flavours cannot be parsed by the aptos move cli
/// custom executable currently used.
fn can_be_executed(repo_zip: &File) -> bool {
    let mut can_be_executed = true;
    let mut archive = zip::ZipArchive::new(repo_zip).unwrap();
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

/// Unzips the repo_zip if *usable* (see fn can_be_executed) into the specified path.  
/// Returns a result with the following integer error codes:  
/// 1. The repo_zip is not usable move (see fn can_be_executed).
/// 2. Couldn't get the name/path of a file inside the zip (see zip::read::ZipFile::enclosed_name)
/// 3. Couldn't create directory to unzip content (see fs::create_dir_all)
/// 4. Couldn't create output file for content extraction. (see fs::OpenOptions::open)
/// 5. Couldn't copy zipped file content to output file (see io::copy)
fn unzip_move_repo_in_path(repo_zip: File, path: &String) -> Result<(), usize> {
    if can_be_executed(&repo_zip) {
        let mut archive = zip::ZipArchive::new(repo_zip).unwrap();
        for i in 0..archive.len() {
            let mut current = archive.by_index(i).unwrap();
            // get relative path
            let rel_outpath = match current.enclosed_name() {
                Some(p) => p,
                None => {
                    error!("Couldn't get relative path of current content");
                    return Err(2);
                }
            };
            // produce abs path from base path
            let current_path = format!("{}/{}", path, rel_outpath.to_str().unwrap());

            // @todo: Do I really want to do this? May break some repos perhaps
            // do not extract content to - or create - the build directory
            if rel_outpath.components().any(|comp| {
                return comp.as_os_str().to_str().unwrap().to_ascii_lowercase() == "build";
            }) {
                continue;
            }

            if current.is_dir() {
                // create dirs
                if let Err(err) = create_dir_all(&current_path) {
                    error!("Couldn't create folder {}", current_path);
                    error!("{}", err);
                    return Err(3);
                }
            } else if current.is_file() && rel_outpath.extension().unwrap() != "Identifier" {
                // skip zone identifier file, they just store whether the file was downloaded
                // from the internet.
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
                        return Err(4);
                    }
                };

                if let Err(err) = copy(&mut current, &mut outfile) {
                    error!("Couldn't extract from temp file to {}", current_path);
                    error!("{err}");
                    return Err(5);
                }
            }
        }
        return Ok(());
    } else {
        return Err(1);
    }
}

// @todo: parallelization?
/// Returns the urls of move source code files from the specified hashset of repos.  
/// The download links are *raw.githubusercontent* links.
pub async fn get_url_of_move_content_from_repos(
    client: &Client,
    repos: &HashSet<String>,
) -> HashMap<String, Vec<String>> {
    // <a, b> -> a is the repo identifier, b is the vec containing all the raw download
    // links of the move content in the specified repo.
    let mut raw_file_url: HashMap<String, Vec<String>> = HashMap::new();

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
            // request when requesting last page
            let link = match next_page_link {
                Some(ref link) => link.to_owned(),
                None => format!("https://api.github.com/search/code?q=language:move+repo:{repo}"),
            };

            let headers = get_requests_headers();
            let response = client.get(link).headers(headers).send().await.unwrap();

            // handle headers for ratelimits and status code
            let rate_limit_wait = handle_response_headers(response.headers(), response.status());

            if response.status().is_success() {
                info!("Request successful: {}", response.status());

                // Check for next page
                next_page_link = get_next_page_from_response_header(response.headers().get("link"));

                // Get current page files
                let response_body = response.text().await.unwrap();
                let get_code_response: search_code_api_response::GetCodeResponse =
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

                    // @todo: does this happen at all?
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

/// IF update_dates txt is present, this will only return repos that need to be
/// updated (usable move only) or downloaded for the first time (not necessarily move native).
/// Otherwise it will return ALL move repos with the last commit date associated.
///
/// **TLDR**: the return value is a set of repos that need to be downloaded and their last commit date.
pub async fn get_repos_fullname(
    client: &Client,
    update_dates: &HashMap<String, (String, bool)>,
) -> HashSet<(String, NaiveDateTime)> {
    let mut repo_links: HashSet<(String, NaiveDateTime)> = HashSet::new();

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
            let headers = get_requests_headers();
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
            let rate_limit_wait = handle_response_headers(response.headers(), response.status());

            if response.status().is_success() {
                info!("Request successful: {}", response.status());

                let response_body = response.text().await.unwrap();

                let get_repo_response: search_repo_api_response::GetRepoResponse =
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

/// Downloads + unzips repositories from `repos` and updates last commit date txt.  
/// Currently allows up to 3 downloads concurrently, change it based on computer specs
/// and github api key ratelimits.
pub async fn download_repo(
    client: &Client,
    repos: &HashSet<(String, NaiveDateTime)>,
) -> HashSet<(String, NaiveDateTime)> {
    let current_idx = Arc::new(Mutex::new(0));
    let fails = Arc::new(Mutex::new(HashSet::new()));
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
            let headers = get_requests_headers();
            let response = c_client.get(url).headers(headers).send().await.unwrap();

            // handle headers for ratelimits and status code
            let rate_limit_wait = handle_response_headers(response.headers(), response.status());

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

                if let Err(code) = unzip_move_repo_in_path(tmpfile, &parent_path) {
                    if code == 1 {
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
                    }
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
