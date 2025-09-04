//! Person identifiers and profile types for social network simulation.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_data::digraph::NodeId;

use std::fmt;

// Person identifier in the social network, allowing a population size of 4.294B.
new_type!(PersonId, u32, Clone, Copy, Debug, Eq, Hash, PartialEq);

impl NodeId for PersonId {}

impl fmt::Display for PersonId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

/// Profile types for population segments.
#[derive(Clone, Debug, Copy, PartialEq, Eq, Hash)]
pub enum ProfileType {
    Connected,
    Isolated,
    Standard,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_person_id_display() {
        let id = PersonId(12345);
        assert_eq!(format!("{}", id), "0x3039"); // 12345 in hex
    }
}
