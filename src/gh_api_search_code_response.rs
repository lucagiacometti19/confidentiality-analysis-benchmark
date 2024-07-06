#[derive(serde::Deserialize, Debug)]
pub(crate) struct File {
    pub(crate) path: String,
    pub(crate) url: String,
}

#[derive(serde::Deserialize, Debug)]
pub(crate) struct GetCodeResponse {
    #[serde(rename = "items")]
    pub(crate) files: Vec<File>,
    #[serde(rename = "total_count")]
    pub(crate) len: usize,
}
