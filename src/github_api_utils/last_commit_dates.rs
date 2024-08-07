use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
};

const COMMIT_DATES_REL_PATH: &str = "resources/last_commit_dates.txt";

/// Appends a new line to **last_commit_dates.txt** (needs to be placed in root dir).  
/// This file maps the move respos found on github with the *crawler* mode.  
// Each line is "{repo},{date},{usable}\n"
pub fn append_last_commit_date_to_file(
    repo: &String,
    date: &String,
    usable: &bool,
) -> Result<(), String> {
    // Open the file for writing with error handling
    let mut file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(COMMIT_DATES_REL_PATH)
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

pub fn get_update_dates() -> Result<HashMap<String, (String, bool)>, ()> {
    let mut update_dates = HashMap::new();
    if let Ok(file) = OpenOptions::new().read(true).open(COMMIT_DATES_REL_PATH) {
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
        Ok(update_dates)
    } else {
        Err(())
    }
}
