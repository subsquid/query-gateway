use std::{str::FromStr, time::Duration};

use sqd_messages::Range;

use super::DatasetId;

#[derive(Debug, Clone)]
pub struct ClientRequest {
    pub dataset_id: DatasetId,
    pub query: ParsedQuery,
    pub buffer_size: usize,
    pub max_chunks: Option<usize>,
    pub chunk_timeout: Duration,
    pub timeout_quantile: f32,
    pub request_multiplier: usize,
    pub retries: usize,
}

#[derive(Debug, Clone)]
pub struct ParsedQuery {
    json: serde_json::Value,
    first_block: u64,
    last_block: Option<u64>,
}

impl ParsedQuery {
    pub fn from_string(query: &str) -> Result<Self, anyhow::Error> {
        let json: serde_json::Value = serde_json::from_str(query)?;
        let first_block = json
            .get("fromBlock")
            .and_then(|v| v.as_u64())
            .ok_or(anyhow::anyhow!("fromBlock is required"))?;
        let last_block = json.get("toBlock").and_then(|v| v.as_u64());
        Ok(Self {
            json,
            first_block,
            last_block,
        })
    }

    pub fn first_block(&self) -> u64 {
        self.first_block
    }

    pub fn last_block(&self) -> Option<u64> {
        self.last_block
    }

    pub fn with_range(&self, range: &Range) -> String {
        let mut json = self.json.clone();
        json["fromBlock"] = serde_json::Value::from(range.begin);
        json["toBlock"] = serde_json::Value::from(range.end);
        serde_json::to_string(&json).expect("Couldn't serialize query")
    }

    pub fn intersect_with(&self, range: &Range) -> Option<Range> {
        let begin = std::cmp::max(range.begin, self.first_block as u32);
        let end = if let Some(last_block) = self.last_block {
            std::cmp::min(range.end, last_block as u32)
        } else {
            range.end
        };
        (begin <= end).then_some(Range { begin, end })
    }
}

impl FromStr for ParsedQuery {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_string(s)
    }
}
