use crate::cli::{Config, DatasetConfig, DatasetSourceConfig};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;

#[derive(Serialize, Deserialize, Debug)]
struct Dataset {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DatasetList {
    #[serde(rename = "sqd-network-datasets")]
    sqd_network_datasets: Vec<Dataset>,
}

async fn fetch_remote_file(url: &str) -> anyhow::Result<DatasetList> {
    tracing::debug!("Fetching remote file from {}", url);

    let response = reqwest::get(url).await?;
    let text = response.text().await?;

    let parser =
        serde_yaml::from_str(&text).with_context(|| format!("failed to parse dataset {}", url));

    Ok(parser?)
}

async fn load_local_file(url: &str) -> anyhow::Result<DatasetList> {
    let full_path = url.replace("file:/", "");

    tracing::debug!("Loading local file from {}", full_path);

    let file = File::open(full_path.clone())
        .with_context(|| format!("failed to open file {}", full_path))?;
    let reader = BufReader::new(file);

    let parser = serde_yaml::from_reader(reader)
        .with_context(|| format!("failed to parse dataset {}", full_path));

    Ok(parser?)
}

pub async fn datasets_load(config: &Config) -> anyhow::Result<Vec<DatasetConfig>> {
    let predefined = config.available_datasets.clone();
    let serve: Option<&str> = config.sqd_network.as_ref().map(|s| s.serve.as_ref());

    if let Ok(Some(file)) = load_file(config).await {
        tracing::debug!(
            "File loaded, {} datasets found",
            file.sqd_network_datasets.len()
        );

        if serve.as_deref() == Some("none") {
            return Ok(predefined);
        }

        let loaded = file
            .sqd_network_datasets
            .iter()
            // .filter(|n| {
            //     let exist = defined.iter().find(|d| {
            //         d.data_sources
            //             .iter()
            //             .find(|y| y.kind == "sqd_network" && y.name_ref === )
            //             .is_some()
            //     });
            //
            //     exist.is_some()
            // })
            .map(|d| DatasetConfig {
                slug: d.name.clone(),
                aliases: None,
                data_sources: vec![DatasetSourceConfig {
                    kind: "sqd_network".into(),
                    name_ref: d.name.clone(),
                    id: d.id.clone(),
                }],
            })
            .collect();

        // FIXME merge with predefined

        Ok(loaded)
    } else {
        tracing::warn!("File loaded with error");

        Ok(predefined)
    }
}

pub async fn load_file(config: &Config) -> anyhow::Result<Option<DatasetList>> {
    if let Some(url) = config
        .sqd_network
        .as_ref()
        .and_then(|n| n.datasets.as_ref())
    {
        if url.starts_with("file://") {
            load_local_file(&url).await.map(|r| Some(r))
        } else {
            fetch_remote_file(&url).await.map(|r| Some(r))
        }
    } else {
        Ok(None)
    }
}
