use serde::{Deserialize, Serialize};
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use anyhow::Context;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub data_source_url: String,
    pub support_tier: Option<i8>,
    pub data: serde_json::Value
}

impl Default for Provider {
    fn default() -> Self {
        Provider {
            data_source_url: String::from(""),
            support_tier: Some(-1),
            data: serde_json::Value::Array(Vec::new())
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ChainType {
    Substrate,
    EVM,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Dataset {
    pub chain_name: Option<String>,
    pub chain_type: Option<ChainType>,
    pub is_testnet: Option<bool>,
    pub network: String,
    pub providers: Vec<Provider>,

    // EVM related data
    pub chain_id: Option<usize>,

    // Substrate related data
    #[serde(rename = "chainSS58Prefix")]
    pub chain_ss58_prefix: Option<usize>,
    pub genesis_hash: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DatasetList {
    #[serde(rename = "archives")]
    datasets: Vec<Dataset>,
}

impl DatasetList {
    pub fn get_by_path(&self, path: &str) -> Option<&Dataset> {
        self.datasets.iter().find(|d| {
            let found = d.providers.get(0).unwrap();

            found.data_source_url.contains(path)
        })
    }
}

pub async fn load_file_remote(url: &str) -> anyhow::Result<DatasetList> {
    let url = format!("https://storage.googleapis.com/subsquid-public/archives/{}", url);
    tracing::debug!("Loading file remote from {}", url);

    let response = reqwest::get(url).await?;

    Ok(response.json::<DatasetList>().await?)
}

pub async fn load_file(url: &str) -> anyhow::Result<DatasetList> {
    if let Ok(path) = env::var("DATASET_DATA_PATH") {
        let full_path = Path::new(&path).join(url);
        if full_path.exists() {
            tracing::debug!("Loading data from {}", full_path.display());

            let file = File::open(full_path.clone()).with_context(|| format!("failed to open file {}", full_path.display()))?;
            let reader = BufReader::new(file);

            Ok(serde_json::from_reader(reader).with_context(|| format!("failed to parse dataset {}", full_path.display()))?)
        } else {
            tracing::debug!("Local {} copy doesn't exist {}", url, full_path.display());

            load_file_remote(url).await
        }
    } else {
        load_file_remote(url).await
    }
}

pub async fn datasets_info_load() -> anyhow::Result<DatasetList> {
    let config_evm: DatasetList = load_file("evm.json").await?;
    let config_substrate: DatasetList = load_file("substrate.json").await?;

    let all_datasets: Vec<Dataset> = config_evm.datasets.into_iter().map(|mut d| {
        d.chain_type = Option::from(ChainType::EVM);

        d
    }).chain(config_substrate.datasets.into_iter().map(|mut d| {
        d.chain_type = Option::from(ChainType::Substrate);

        d
    })).collect();

    Ok(DatasetList {
        datasets: all_datasets
    })
}