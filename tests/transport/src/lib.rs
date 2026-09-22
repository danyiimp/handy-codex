// Compile the production HTTP modules directly without the Tauri/audio engines.
// This stub contains only the settings fields consumed by llm_client.
pub mod settings {
    #[derive(Clone, Debug)]
    pub struct PostProcessProvider {
        pub id: String,
        pub label: String,
        pub base_url: String,
        pub allow_base_url_edit: bool,
        pub models_endpoint: Option<String>,
        pub supports_structured_output: bool,
    }
}

#[path = "../../../src-tauri/src/managers/codex_asr.rs"]
pub mod codex_asr;

pub mod managers {
    pub use crate::codex_asr;
}

#[path = "../../../src-tauri/src/llm_client.rs"]
pub mod llm_client;
