use apex_hal::prelude::ErrorCode;
use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HealthEvent{
  pub error: Option<ErrorCode>,
  pub msg: String,
}