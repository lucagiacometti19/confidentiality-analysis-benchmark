use std::{
    collections::HashSet,
    fs::{read, remove_file, OpenOptions},
    io::{BufRead, Write},
    path::PathBuf,
    str::FromStr,
};

use crate::{
    confidentiality_analysis_utils::confidentiality_analysis_helper::run_and_collect_confidentiality_par,
    move_utils::{move_helper::create_spec, move_parser::MoveSourceMetadata},
};

pub async fn run_confidentiality_quantitative_analysis(
    source_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let source_code_path = PathBuf::from_str(source_path).unwrap();
    let quantitative_analysis_tmp_file_paths = generate_intermediate_files(&source_code_path)
        .await
        .unwrap();
    // now all quantitative analysis intermediate files have been generated and saved.
    // run confidentiality analysis
    // @todo: this is not optimal placed here. The base source files will be run multiple times.
    // the spec ones will correctly be run here, but all the base ones unchanged will be re-executed
    // currenly fixed by running on parent dir and not on parent.parent dir, not sure it works all times tho
    let execution_stats =
        run_and_collect_confidentiality_par(source_code_path.parent().unwrap().to_str().unwrap())
            .await;
    info!(
        "Successfully run the analysis on {} / {} for {}",
        execution_stats.1 - execution_stats.0,
        execution_stats.1,
        source_code_path.file_name().unwrap().to_str().unwrap()
    );

    // Then remove intermediate quantitative analysis files
    intermediate_files_cleanup(&quantitative_analysis_tmp_file_paths)?;

    Ok(())
}

async fn generate_intermediate_files(
    source_code_path: &PathBuf,
) -> Result<HashSet<String>, Box<dyn std::error::Error>> {
    let bytes = read(&source_code_path)?;
    // replaces not valid utf-8 with REPLACEMENT_CHARACTER �
    let content = String::from_utf8_lossy(bytes.as_slice());
    let mut quantitative_analysis_tmp_file_paths: HashSet<String> = HashSet::new();
    if !content.is_empty() {
        // get structs from move source
        let src_metadata = MoveSourceMetadata::parse_move_source_metadata(content).unwrap();

        for s in src_metadata.structs {
            for field in &s.fields {
                let spec_lines: Vec<_> = create_spec(&s, field)
                    .unwrap_or_default()
                    .split("\n")
                    .map(|line| line.to_owned())
                    .collect();
                if spec_lines.len() != 1 {
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
    }
    Ok(quantitative_analysis_tmp_file_paths)
}

fn intermediate_files_cleanup(quantitative_analysis_tmp_file_paths: &HashSet<String>) -> Result<(), Box<dyn std::error::Error>> {
    // remove intermediate quantitative analysis files
    for qa_tmp_path in quantitative_analysis_tmp_file_paths {
        remove_file(&qa_tmp_path)?;
    }
    Ok(())
}
