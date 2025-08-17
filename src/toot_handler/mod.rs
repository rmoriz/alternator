pub mod handler;
pub mod processor;
pub mod coordinator;
pub mod race;
pub mod stats;

// Re-export the main struct for backward compatibility
pub use handler::TootStreamHandler;