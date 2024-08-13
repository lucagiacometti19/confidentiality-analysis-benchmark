use std::{
    collections::HashMap,
    fs::{read, OpenOptions},
    io::Write,
    os::fd::{FromRawFd, IntoRawFd},
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread::available_parallelism,
    usize,
};

use tokio::{spawn, sync::Semaphore};
use walkdir::WalkDir;

use super::ConfidentialityStats;

const APTOS_MOVE_CLI_EXECUTABLE_REL_PATH: &str = "resources/aptos";
const CONFIDENTIALITY_ANALYSIS_STATS_FILE_REL_PATH: &str =
    "output/confidentiality-analysis-stats.txt";

// This fn needs to be as generic as possible. Quantitative analysis uses it
// differently from standard confidentiality analysis, by passing the single
// move src file as root_dir: this is needed to generate and remove intermediate files
// efficiently. Keep this in mind when modifying it.
// @todo: remove the following
// currently realying on .env var just for prettier prints
pub async fn run_and_collect_confidentiality_par(root_dir: &str) -> (i32, i32) {
    let fails = Arc::new(Mutex::new(0));
    let total = Arc::new(Mutex::new(0));
    let parallelism = available_parallelism().unwrap().get();
    let semaphore = Arc::new(Semaphore::new(parallelism));
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    let wlkdir = WalkDir::new(root_dir).follow_links(false).into_iter();
    // @todo: size_hint returns (0, None) when running quantitative analysis
    // since root_dir is a single file
    //Find a better way to estimate the
    // amount of elements in the walkdir iterator.
    let wlkdir_size_upper_bound = if wlkdir.size_hint().1.is_some() {
        wlkdir.size_hint().1.unwrap().to_string()
    } else {
        String::from("?")
    };
    let mut current_index = 0;
    for e_res in wlkdir {
        current_index += 1;
        if let Ok(entry) = e_res {
            let c_total = total.clone();
            let c_fails = fails.clone();
            let c_wlkdir_size_upper_bound = wlkdir_size_upper_bound.clone();
            let base_path = root_dir.rsplit_once('/').unwrap_or_default().0;
            dbg!(root_dir);
            let c_base_path = base_path.to_owned();
            // acquire semaphore
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            tasks.push(spawn(async move {
                if let Some(ext) = entry.path().extension() {
                    if entry.file_type().is_file() && ext.to_str().unwrap() == "move" {
                        *c_total.lock().unwrap() += 1;
                        let current_path = entry.path().to_str().unwrap().to_owned();
                        // removing base path, more readable
                        let pretty_current_path = current_path
                            .split_once(&c_base_path)
                            .unwrap_or(("_", "?"))
                            .1;

                        if let Some(p) = Path::new(&current_path).parent() {
                            // Open the file for writing with error handling
                            let output_file = format!(
                                "{}/{}.txt",
                                p.to_str().unwrap(),
                                Path::new(&current_path)
                                    .file_stem()
                                    .unwrap()
                                    .to_str()
                                    .unwrap()
                            );
                            match OpenOptions::new()
                                .create(true)
                                .write(true)
                                .truncate(true)
                                .open(&output_file)
                            {
                                Ok(file) => {
                                    info!(
                                        "Running confidentiality analysis for {}",
                                        pretty_current_path
                                    );
                                    // @todo: currently suppressing stderr, looks cleaner when running
                                    // & there isn't a good reason to NOT do this :D
                                    if !Command::new(APTOS_MOVE_CLI_EXECUTABLE_REL_PATH)
                                        .args([
                                            "move",
                                            "prove",
                                            "--run-confidentiality",
                                            "--package-dir",
                                            &current_path,
                                            "-f",
                                            Path::new(&current_path)
                                                .file_name()
                                                .unwrap()
                                                .to_str()
                                                .unwrap(),
                                        ])
                                        .stdout(unsafe { Stdio::from_raw_fd(file.into_raw_fd()) })
                                        .stderr(Stdio::null())
                                        .status()
                                        .expect("failed to execute process")
                                        .success()
                                    {
                                        error!(
                                            "Execution of confidentiality analysis for {} failed",
                                            pretty_current_path
                                        );
                                        *c_fails.lock().unwrap() += 1;
                                    } else {
                                        // @todo: success - debug only
                                        info!(
                                            "Success {}|{}",
                                            current_index, c_wlkdir_size_upper_bound
                                        );
                                    }
                                }
                                Err(err) => {
                                    error!("Cannot open/create {:?}", output_file);
                                    error!("{:?}", err);
                                }
                            };
                        } else {
                            error!(
                                "{pretty_current_path} is the root directory or an invalid path"
                            );
                        };
                    }
                }
                drop(permit);
            }));
        }
    }
    for req in tasks {
        req.await.unwrap();
    }

    (
        Arc::try_unwrap(fails)
            .expect("Lock on 'fails' still has multiple owners")
            .into_inner()
            .expect("Mutex cannot be locked"),
        Arc::try_unwrap(total)
            .expect("Lock on 'total' still has multiple owners")
            .into_inner()
            .expect("Mutex cannot be locked"),
    )
}

pub fn run_and_collect_confidentiality_sync(root_dir: &str) -> (i32, i32) {
    let mut fails = 0;
    let mut total = 0;
    let wlkdir = WalkDir::new(root_dir).follow_links(false).into_iter();
    // @todo: size_hint returns (0, None) most of the times. Find a better way to estimate the
    // amount of elements in the walkdir iterator.
    let wlkdir_size_upper_bound = if wlkdir.size_hint().1.is_some() {
        wlkdir.size_hint().1.unwrap().to_string()
    } else {
        String::from("?")
    };
    let mut current_index = 0;
    for e_res in wlkdir {
        current_index += 1;
        if let Ok(entry) = e_res {
            if let Some(ext) = entry.path().extension() {
                if entry.file_type().is_file() && ext.to_str().unwrap() == "move" {
                    total += 1;
                    let current_path = entry.path().to_str().unwrap().to_owned();
                    // removing base path, more readable
                    let base_path = root_dir.rsplit_once('/').unwrap_or_default().0;
                    let pretty_current_path =
                        current_path.split_once(base_path).unwrap_or(("_", "?")).1;
                    if let Some(p) = Path::new(&current_path).parent() {
                        // Open the file for writing with error handling
                        let output_file = format!(
                            "{}/{}.txt",
                            p.to_str().unwrap(),
                            Path::new(&current_path)
                                .file_stem()
                                .unwrap()
                                .to_str()
                                .unwrap()
                        );
                        match OpenOptions::new()
                            .create(true)
                            .write(true)
                            .truncate(true)
                            .open(&output_file)
                        {
                            Ok(file) => {
                                info!(
                                    "Running confidentiality analysis for {}",
                                    pretty_current_path
                                );
                                if !Command::new(APTOS_MOVE_CLI_EXECUTABLE_REL_PATH)
                                    .args([
                                        "move",
                                        "prove",
                                        "--run-confidentiality",
                                        "--package-dir",
                                        &current_path,
                                        "-f",
                                        Path::new(&current_path)
                                            .file_name()
                                            .unwrap()
                                            .to_str()
                                            .unwrap(),
                                    ])
                                    .stdout(unsafe { Stdio::from_raw_fd(file.into_raw_fd()) })
                                    .status()
                                    .expect("failed to execute process")
                                    .success()
                                {
                                    error!(
                                        "Execution of confidentiality analysis for {} failed",
                                        pretty_current_path
                                    );
                                    fails += 1;
                                } else {
                                    // @todo: success - debug only
                                    info!("Success {}|{}", current_index, wlkdir_size_upper_bound);
                                }
                            }
                            Err(err) => {
                                error!("Cannot open/create {:?}", output_file);
                                error!("{:?}", err);
                            }
                        };
                    } else {
                        error!("{pretty_current_path} is the root directory or an invalid path");
                    };
                }
            }
        }
    }
    (fails, total)
}

pub fn collect_confidentiality_results(
    root_dir: &str,
) -> HashMap<String, Vec<ConfidentialityStats>> {
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
                        let bytes = read(entry.path()).unwrap_or_default();
                        // replaces not valid utf-8 with REPLACEMENT_CHARACTER �
                        let content = String::from_utf8_lossy(bytes.as_slice());
                        let content_lines: Vec<_> = content.lines().collect();
                        // content empty most likely means failed analysis execution, skip this file
                        if !content.is_empty()
                        /* && content
                        .lines()
                        .nth(content.lines().count().checked_sub(2).unwrap_or(0))
                        .unwrap()
                        .to_ascii_lowercase()
                        .contains("success") */
                        && content_lines[content.lines().count().checked_sub(2).unwrap_or(0)]
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
                            let diags: Vec<(usize, String)> = content_lines
                                .iter()
                                .enumerate()
                                .filter(|(_, &line)| {
                                    line.starts_with("warning:") || line.starts_with("Analyzing")
                                })
                                .map(|(i, &line)| (i, line.to_owned()))
                                .collect();
                            for (idx, line) in diags {
                                if line.starts_with("Analyzing") {
                                    // this line is not a diag, get module/fn/struct count
                                    let mut tmp = Vec::new();
                                    for part in line.split_whitespace() {
                                        if let Ok(res) = part.parse::<usize>() {
                                            tmp.push(res);
                                        }
                                    }
                                    (module_count, function_count, struct_count) =
                                        (tmp[0], tmp[1], tmp[2]);
                                    continue;
                                }
                                if content_lines[idx + 1].contains("/.move/")
                                    || !content_lines[idx + 1].contains(
                                        format!(
                                            "/{}.{}:",
                                            entry.path().file_stem().unwrap().to_str().unwrap(),
                                            "move"
                                        )
                                        .as_str(),
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
                            // debug only
                            //info!(
                            //    "{} \n- modules: {}\n- functions: {}\n- structs: {}\n- number of diagnostics: {} \n- expl flow via ret: {} \n- expl flow via call: {} \n- expl flow via moveTo: {} \n- expl flow via writeRef: {} \n- impl flow via ret: {} \n- impl flow via call: {}",
                            //    filepath,
                            //    module_count,
                            //    function_count,
                            //    struct_count,
                            //    total_diags,
                            //    explicit_flow_via_ret_diag_count,
                            //    explicit_flow_via_call_diag_count,
                            //    explicit_flow_via_moveto_diag_count,
                            //    explicit_flow_via_writeref_diag_count,
                            //    implicit_flow_via_ret_diag_count,
                            //    implicit_flow_via_call_diag_count
                            //);
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

/// Used to save confidentiality analysis stats to *conf-res.txt* file
// Add new stats in here
pub fn save_confidentiality_stats(
    res: &HashMap<String, Vec<ConfidentialityStats>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Open the file for writing with error handling
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(CONFIDENTIALITY_ANALYSIS_STATS_FILE_REL_PATH)?;
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
    // first line are general stats
    write!(file, "total modules: {} - total functions: {} - total structs: {} - total diagnostics: {} - total expl flow via ret: {} - total expl flow via call: {} - total expl flow via moveTo: {} - total expl flow via writeRef: {} - total impl flow via ret: {} - total impl flow via call: {}", total_modules, total_functions, total_structs, total_diags, expl_ret, expl_call, expl_moveto, expl_writeref, impl_ret, impl_call)?;
    for (base_path, stats) in res {
        write!(
            file,
            "Confidentiality analysis stats of repo {}\n",
            base_path
        )?;

        //  @todo: this is kinda of a performance killer, do i really wanna keep this?
        let mut c_stats = stats.clone();
        c_stats.sort_unstable_by_key(|stat| stat.file_path.len());

        for file_stats in c_stats {
            let line = if file_stats.total_diags_excluding_deps == 0 {
                format!("{} - no diags", file_stats.file_path)
            } else {
                format!(
                    "{} - number of diagnostics: {} - expl flow via ret: {} - expl flow via call: {} - expl flow via moveTo: {} - expl flow via writeRef: {} - impl flow via ret: {} - impl flow via call: {}",
                    file_stats.file_path, file_stats.total_diags_excluding_deps, file_stats.explicit_flow_via_return_amount, file_stats.explicit_flow_via_call_amount, file_stats.explicit_flow_via_moveto_amount, file_stats.explicit_flow_via_writeref_amount, file_stats.implicit_flow_via_return_amount, file_stats.implicit_flow_via_call_amount
                )
            };
            // general stats
            write!(file, "{}\n", line)?;
        }
        // separator
        write!(file, "{}\n", "-".repeat(50))?;
    }
    Ok(())
}
