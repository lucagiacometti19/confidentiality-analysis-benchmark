pub mod github_api_helper;
pub mod last_commit_dates;

mod search_code_api_response {
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
}

mod search_repo_api_response {
    #[derive(serde::Deserialize, Debug)]
    pub(crate) struct Repo {
        pub(crate) url: String,
        #[serde(rename = "nameWithOwner")]
        pub(crate) full_name: String,
        pub(crate) description: Option<String>,
        #[serde(rename = "pushedAt")]
        pub(crate) last_update: String,
    }

    #[derive(serde::Deserialize, Debug)]
    pub(crate) struct GetRepoResponse {
        pub(crate) data: ResponseData,
    }
    #[derive(serde::Deserialize, Debug)]
    pub(crate) struct ResponseData {
        pub(crate) search: SearchResponse,
    }
    #[derive(serde::Deserialize, Debug)]
    pub(crate) struct SearchResponse {
        #[serde(rename = "nodes")]
        pub(crate) repositories: Vec<Repo>,
        #[serde(rename = "pageInfo")]
        pub(crate) page_info: GithubPageInfo,
    }
    #[derive(serde::Deserialize, Debug)]
    pub(crate) struct GithubPageInfo {
        #[serde(rename = "endCursor")]
        pub(crate) end_cursor: String,
        #[serde(rename = "hasNextPage")]
        pub(crate) has_next_page: bool,
        //#[serde(skip)]
        //start_cursor: String,
        //#[serde(skip)]
        //has_previous_page: bool,
    }
}
