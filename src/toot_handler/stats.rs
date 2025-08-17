/// Statistics about toot processing
#[allow(dead_code)] // Stats struct for API completeness
#[derive(Debug, Clone)]
pub struct ProcessingStats {
    pub processed_toots_count: usize,
}