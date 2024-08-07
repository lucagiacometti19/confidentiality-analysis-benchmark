#[derive(Debug, Clone)]
pub(crate) struct ConfidentialityStats {
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

pub mod confidentiality_analysis_helper;
pub mod quantitative_analysis;