use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edition {
    #[default]
    #[serde(rename = "0.1")]
    V0_1,

    #[serde(rename = "0.2")]
    V0_2,
}

impl fmt::Display for Edition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Edition::V0_1 => f.write_str("0.1"),
            Edition::V0_2 => f.write_str("0.2"),
        }
    }
}

impl FromStr for Edition {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "0.1" => Ok(Edition::V0_1),
            "0.2" => Ok(Edition::V0_2),
            other => Err(format!(
                "unsupported edition `{other}`; expected `0.1` or `0.2`"
            )),
        }
    }
}
