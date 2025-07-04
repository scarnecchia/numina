//! Human types - Partner and Conversant distinction

use serde::{Deserialize, Serialize};

/// User/Partner identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserId(pub i64);

impl UserId {
    pub fn new(id: i64) -> Self {
        Self(id)
    }
}

// Future additions:
// - Partner struct (owns a constellation)
// - Conversant struct (interacts with partner's agents)
// - Methods for determining partner vs conversant context
// - Privacy boundaries between partners
