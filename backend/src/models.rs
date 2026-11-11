use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetsPage {
    pub client: Client,
    pub notice: String,
    pub funding: FundingSummary,
    pub next_steps: Vec<NextStep>,
    #[serde(default)]
    pub columns: Vec<String>,
    pub categories: Vec<AssetCategory>,
    pub total_value: i64,
    pub total_note: String,
    pub footer: Footer,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Client {
    pub name: String,
    #[serde(rename = "coClient")]
    pub co_client: String,
    #[serde(rename = "reportTitle")]
    pub report_title: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingSummary {
    pub percent: f64,
    pub verified: usize,
    pub left_to_verify: usize,
    pub not_fundable: usize,
    pub not_funded_per_client: usize,
    pub fundable_total: usize,
    pub footnote: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NextStep {
    pub title: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AssetCategory {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: String,
    #[serde(default)]
    pub record_id: String,
    #[serde(default)]
    pub category_value: String,
    #[serde(default)]
    pub editable_fields: HashMap<String, String>,
    pub name: String,
    pub details: String,
    pub value: i64,
    pub status: AssetStatus,
    pub status_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_help: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub excluded_from_total: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetStatus {
    Verified,
    NeedsVerification,
    Undecided,
    NotFundable,
    NotFunded,
}

impl AssetStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Verified => "Verified",
            Self::NeedsVerification => "Needs to Be Verified",
            Self::Undecided => "Undecided",
            Self::NotFundable => "Not Fundable",
            Self::NotFunded => "Not Funded Per Client",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Footer {
    pub firm: String,
    pub address: String,
    pub phone: String,
    pub links: Vec<String>,
}
