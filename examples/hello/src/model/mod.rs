use serde::{Deserialize, Serialize};
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename = "camelCase")]
pub struct User {
  #[serde(skip_serializing_if = "Option::is_none")]
  username: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  country: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  mobile: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename = "camelCase")]
pub struct QueryUser {
  #[serde(skip_serializing_if = "Option::is_none")]
  username: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  country: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  mobile: Option<String>,
}
