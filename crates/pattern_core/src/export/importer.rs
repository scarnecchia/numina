//! Agent importer implementation

use crate::Result;

/// Options for importing an agent
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// New name for the imported agent
    pub rename_to: Option<String>,
    
    /// Whether to merge with existing agent
    pub merge_existing: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            rename_to: None,
            merge_existing: false,
        }
    }
}

/// Agent importer
pub struct AgentImporter<C>
where
    C: surrealdb::Connection + Clone,
{
    _db: surrealdb::Surreal<C>,
}

impl<C> AgentImporter<C>
where
    C: surrealdb::Connection + Clone,
{
    /// Create a new importer
    pub fn new(db: surrealdb::Surreal<C>) -> Self {
        Self { _db: db }
    }
    
    // TODO: Implement import_from_car
}