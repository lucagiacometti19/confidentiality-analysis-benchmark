mod confidentiality_analysis_utils;
mod github_api_utils;
mod move_utils;
use confidentiality_analysis_utils::{
    confidentiality_analysis_helper::{
        collect_confidentiality_results, run_and_collect_confidentiality_par,
        save_confidentiality_stats,
    },
    quantitative_analysis::run_confidentiality_quantitative_analysis,
};
use github_api_utils::{
    github_api_helper::{download_repo, get_repos_fullname},
    last_commit_dates::get_update_dates,
};

use dotenv::dotenv;
use std::collections::HashSet;

use tokio;
use walkdir::WalkDir;

extern crate pretty_env_logger;
#[macro_use]
extern crate log;

#[tokio::main]
async fn main() {
    // load env variables
    dotenv().ok();
    // TODO: make logger prettier
    pretty_env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    match args[1].as_str() {
        "crawler" => {
            let client = reqwest::Client::new();

            // Last commit date file
            let update_dates = get_update_dates().unwrap();
            let repo_links = get_repos_fullname(&client, &update_dates).await;

            let mut missing = HashSet::from(repo_links);
            let mut retries = 0;
            while !missing.is_empty() {
                if retries < 5 {
                    missing = download_repo(&client, &missing).await;
                    retries += 1;
                } else {
                    error!("Failed to download the following repos:\n{:#?}", missing);
                    break;
                }
            }
        }
        "confidentiality_analysis" => {
            let execution_stats = run_and_collect_confidentiality_par(
                std::env::var("MOVE_REPOS_DIR")
                    .expect("Quantitative analysis repos directory not set in the .env")
                    .as_str(),
            )
            .await;
            info!(
                "Successfully run the analysis on {} / {}",
                execution_stats.1 - execution_stats.0,
                execution_stats.1
            );

            let res = collect_confidentiality_results(
                std::env::var("MOVE_REPOS_DIR")
                    .expect("Quantitative analysis repos directory not set in the .env")
                    .as_str(),
            );
            if let Err(err) = save_confidentiality_stats(&res) {
                error!("{err}");
                return;
            }
        }
        "quantitative_confidentiality_analysis" => {
            let mut sources = HashSet::new();
            for e_res in WalkDir::new(
                std::env::var("QUANTITATIVE_ANALYSIS_REPOS_DIR")
                    .expect("Quantitative analysis repos directory not set in the .env"),
            )
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
            // @todo debug only
            // ------------------------------------
            use std::time::Instant;
            let now = Instant::now();
            // ------------------------------------
            for source in sources {
                if let Err(e) = run_confidentiality_quantitative_analysis(&source).await {
                    error!("{e}");
                }
            }
            // @todo: ------------------------------------
            let elapsed = now.elapsed();
            println!("Elapsed: {:.2?}", elapsed);
            // ------------------------------------

            let res = collect_confidentiality_results(
                std::env::var("QUANTITATIVE_ANALYSIS_REPOS_DIR")
                    .expect("Quantitative analysis repos directory not set in the .env")
                    .as_str(),
            );
            if let Err(err) = save_confidentiality_stats(&res) {
                error!("{err}");
            }
        }
        _ => error!("Unrecognized argument {}, closing...", args[1]),
    }
}
