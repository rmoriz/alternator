use crate::error::MediaError;
use std::path::{Path, PathBuf};
use tempfile::{NamedTempFile, TempPath};
use tokio::fs;

/// A helper for managing temporary files with automatic cleanup
#[derive(Debug)]
#[allow(dead_code)]
pub struct TempFile {
    temp_path: Option<TempPath>,
}

#[allow(dead_code)]
impl TempFile {
    /// Create a new temporary file
    pub fn new() -> Result<Self, MediaError> {
        let temp_file = NamedTempFile::new().map_err(|e| {
            MediaError::ProcessingFailed(format!("Failed to create temp file: {e}"))
        })?;

        let temp_path = temp_file.into_temp_path();
        tracing::debug!("Created temp file: {:?}", temp_path);

        Ok(Self {
            temp_path: Some(temp_path),
        })
    }

    /// Create a new temporary file with a specific suffix
    pub fn with_suffix(suffix: &str) -> Result<Self, MediaError> {
        let temp_file = NamedTempFile::with_suffix(suffix).map_err(|e| {
            MediaError::ProcessingFailed(format!(
                "Failed to create temp file with suffix '{suffix}': {e}"
            ))
        })?;

        let temp_path = temp_file.into_temp_path();
        tracing::debug!(
            "Created temp file with suffix '{}': {:?}",
            suffix,
            temp_path
        );

        Ok(Self {
            temp_path: Some(temp_path),
        })
    }

    /// Create a new temporary file with a specific prefix
    pub fn with_prefix(prefix: &str) -> Result<Self, MediaError> {
        let temp_file = NamedTempFile::with_prefix(prefix).map_err(|e| {
            MediaError::ProcessingFailed(format!(
                "Failed to create temp file with prefix '{prefix}': {e}"
            ))
        })?;

        let temp_path = temp_file.into_temp_path();
        tracing::debug!(
            "Created temp file with prefix '{}': {:?}",
            prefix,
            temp_path
        );

        Ok(Self {
            temp_path: Some(temp_path),
        })
    }

    /// Get the path to the temporary file
    pub fn path(&self) -> &Path {
        self.temp_path.as_ref().unwrap().as_ref()
    }

    /// Get the path as a PathBuf (for ownership)
    pub fn path_buf(&self) -> PathBuf {
        self.temp_path.as_ref().unwrap().to_path_buf()
    }

    /// Write data to the temporary file asynchronously
    pub async fn write_data(&self, data: &[u8]) -> Result<(), MediaError> {
        fs::write(self.path(), data).await.map_err(|e| {
            MediaError::ProcessingFailed(format!("Failed to write data to temp file: {e}"))
        })?;

        tracing::debug!("Wrote {} bytes to temp file: {:?}", data.len(), self.path());
        Ok(())
    }

    /// Read data from the temporary file asynchronously
    pub async fn read_data(&self) -> Result<Vec<u8>, MediaError> {
        let data = fs::read(self.path()).await.map_err(|e| {
            MediaError::ProcessingFailed(format!("Failed to read data from temp file: {e}"))
        })?;

        tracing::debug!(
            "Read {} bytes from temp file: {:?}",
            data.len(),
            self.path()
        );
        Ok(data)
    }

    /// Write a string to the temporary file asynchronously
    pub async fn write_string(&self, content: &str) -> Result<(), MediaError> {
        self.write_data(content.as_bytes()).await
    }

    /// Read a string from the temporary file asynchronously
    pub async fn read_string(&self) -> Result<String, MediaError> {
        let data = self.read_data().await?;
        String::from_utf8(data).map_err(|e| {
            MediaError::ProcessingFailed(format!("Failed to decode UTF-8 from temp file: {e}"))
        })
    }

    /// Copy data from another file to this temporary file
    pub async fn copy_from_path(&self, source_path: &Path) -> Result<(), MediaError> {
        let data = fs::read(source_path).await.map_err(|e| {
            MediaError::ProcessingFailed(format!("Failed to read source file for copy: {e}"))
        })?;

        self.write_data(&data).await?;
        tracing::debug!(
            "Copied data from {:?} to temp file {:?}",
            source_path,
            self.path()
        );
        Ok(())
    }

    /// Get the file size in bytes
    pub async fn size(&self) -> Result<u64, MediaError> {
        let metadata = fs::metadata(self.path()).await.map_err(|e| {
            MediaError::ProcessingFailed(format!("Failed to get temp file metadata: {e}"))
        })?;

        Ok(metadata.len())
    }

    /// Keep the temporary file (prevent automatic cleanup)
    pub fn keep(mut self) -> PathBuf {
        let path = self.path_buf();
        if let Some(temp_path) = self.temp_path.take() {
            let _ = temp_path.keep();
        }
        path
    }

    /// Check if the file exists
    pub fn exists(&self) -> bool {
        self.temp_path.as_ref().unwrap().exists()
    }

    /// Persist the temporary file to a permanent location
    pub fn persist<P: AsRef<Path>>(&self, new_path: P) -> Result<(), MediaError> {
        let path_ref = new_path.as_ref();
        let current_path = self.temp_path.as_ref().unwrap().to_path_buf();

        // Copy the file to the new location
        std::fs::copy(&current_path, &new_path).map_err(|e| {
            MediaError::ProcessingFailed(format!("Failed to copy temp file to new location: {e}"))
        })?;

        tracing::debug!("Persisted temp file to {:?}", path_ref);
        Ok(())
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if let Some(ref temp_path) = self.temp_path {
            if temp_path.exists() {
                tracing::debug!("Cleaning up temp file: {:?}", temp_path.as_ref() as &Path);
            }
        }
    }
}

/// A helper for managing multiple temporary files
#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct TempFileManager {
    temp_files: Vec<TempFile>,
}

#[allow(dead_code)]
impl TempFileManager {
    /// Create a new temp file manager
    pub fn new() -> Self {
        Self {
            temp_files: Vec::new(),
        }
    }

    /// Create a new temporary file and add it to the manager
    pub fn create_temp_file(&mut self) -> Result<&TempFile, MediaError> {
        let temp_file = TempFile::new()?;
        self.temp_files.push(temp_file);
        Ok(self.temp_files.last().unwrap())
    }

    /// Create a new temporary file with suffix and add it to the manager
    pub fn create_temp_file_with_suffix(&mut self, suffix: &str) -> Result<&TempFile, MediaError> {
        let temp_file = TempFile::with_suffix(suffix)?;
        self.temp_files.push(temp_file);
        Ok(self.temp_files.last().unwrap())
    }

    /// Create a new temporary file with prefix and add it to the manager
    pub fn create_temp_file_with_prefix(&mut self, prefix: &str) -> Result<&TempFile, MediaError> {
        let temp_file = TempFile::with_prefix(prefix)?;
        self.temp_files.push(temp_file);
        Ok(self.temp_files.last().unwrap())
    }

    /// Get the number of managed temporary files
    pub fn len(&self) -> usize {
        self.temp_files.len()
    }

    /// Check if the manager has any temporary files
    pub fn is_empty(&self) -> bool {
        self.temp_files.is_empty()
    }

    /// Get a reference to a temporary file by index
    pub fn get(&self, index: usize) -> Option<&TempFile> {
        self.temp_files.get(index)
    }

    /// Clear all temporary files (they will be cleaned up when dropped)
    pub fn clear(&mut self) {
        self.temp_files.clear();
    }

    /// Keep all temporary files (prevent automatic cleanup) and return their paths
    pub fn keep_all(mut self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for temp_file in self.temp_files.drain(..) {
            // Keep the temp file using the TempFile::keep method
            paths.push(temp_file.keep());
        }
        paths
    }
}

/// Utility functions for common temporary file operations
#[allow(dead_code)]
pub mod utils {
    use super::*;

    /// Create a temporary file, write data to it, and return the TempFile
    pub async fn create_temp_file_with_data(data: &[u8]) -> Result<TempFile, MediaError> {
        let temp_file = TempFile::new()?;
        temp_file.write_data(data).await?;
        Ok(temp_file)
    }

    /// Create a temporary file with a specific suffix and write data to it
    pub async fn create_temp_file_with_data_and_suffix(
        data: &[u8],
        suffix: &str,
    ) -> Result<TempFile, MediaError> {
        let temp_file = TempFile::with_suffix(suffix)?;
        temp_file.write_data(data).await?;
        Ok(temp_file)
    }

    /// Create a temporary file and write a string to it
    pub async fn create_temp_file_with_string(content: &str) -> Result<TempFile, MediaError> {
        let temp_file = TempFile::new()?;
        temp_file.write_string(content).await?;
        Ok(temp_file)
    }

    /// Create a temporary file with suffix and write a string to it
    pub async fn create_temp_file_with_string_and_suffix(
        content: &str,
        suffix: &str,
    ) -> Result<TempFile, MediaError> {
        let temp_file = TempFile::with_suffix(suffix)?;
        temp_file.write_string(content).await?;
        Ok(temp_file)
    }

    /// Process data through a temporary file (useful for external command processing)
    pub async fn process_with_temp_file<F, Fut, T>(
        data: &[u8],
        processor: F,
    ) -> Result<T, MediaError>
    where
        F: FnOnce(&TempFile) -> Fut,
        Fut: std::future::Future<Output = Result<T, MediaError>>,
    {
        let temp_file = create_temp_file_with_data(data).await?;
        processor(&temp_file).await
    }

    /// Process data through a temporary file with a specific suffix
    pub async fn process_with_temp_file_suffix<F, Fut, T>(
        data: &[u8],
        suffix: &str,
        processor: F,
    ) -> Result<T, MediaError>
    where
        F: FnOnce(&TempFile) -> Fut,
        Fut: std::future::Future<Output = Result<T, MediaError>>,
    {
        let temp_file = create_temp_file_with_data_and_suffix(data, suffix).await?;
        processor(&temp_file).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temp_file_creation() {
        let temp_file = TempFile::new().unwrap();
        assert!(temp_file.path().exists());
        assert!(temp_file.exists());
    }

    #[test]
    fn test_temp_file_with_suffix() {
        let temp_file = TempFile::with_suffix(".txt").unwrap();
        assert!(temp_file.path().exists());
        assert!(temp_file.path().to_string_lossy().ends_with(".txt"));
    }

    #[test]
    fn test_temp_file_with_prefix() {
        let temp_file = TempFile::with_prefix("test_").unwrap();
        assert!(temp_file.path().exists());
        let path_str = temp_file.path().to_string_lossy();
        assert!(path_str.contains("test_"));
    }

    #[tokio::test]
    async fn test_temp_file_write_read_data() {
        let temp_file = TempFile::new().unwrap();
        let test_data = b"Hello, World!";

        temp_file.write_data(test_data).await.unwrap();
        let read_data = temp_file.read_data().await.unwrap();

        assert_eq!(read_data, test_data);
    }

    #[tokio::test]
    async fn test_temp_file_write_read_string() {
        let temp_file = TempFile::new().unwrap();
        let test_string = "Hello, Rust!";

        temp_file.write_string(test_string).await.unwrap();
        let read_string = temp_file.read_string().await.unwrap();

        assert_eq!(read_string, test_string);
    }

    #[tokio::test]
    async fn test_temp_file_size() {
        let temp_file = TempFile::new().unwrap();
        let test_data = b"Test data for size check";

        temp_file.write_data(test_data).await.unwrap();
        let size = temp_file.size().await.unwrap();

        assert_eq!(size, test_data.len() as u64);
    }

    #[tokio::test]
    async fn test_temp_file_copy_from_path() {
        let source_file = TempFile::new().unwrap();
        let source_data = b"Source data";
        source_file.write_data(source_data).await.unwrap();

        let dest_file = TempFile::new().unwrap();
        dest_file.copy_from_path(source_file.path()).await.unwrap();

        let dest_data = dest_file.read_data().await.unwrap();
        assert_eq!(dest_data, source_data);
    }

    #[test]
    fn test_temp_file_manager() {
        let mut manager = TempFileManager::new();
        assert_eq!(manager.len(), 0);
        assert!(manager.is_empty());

        let _temp1 = manager.create_temp_file().unwrap();
        assert_eq!(manager.len(), 1);
        assert!(!manager.is_empty());

        let _temp2 = manager.create_temp_file_with_suffix(".wav").unwrap();
        assert_eq!(manager.len(), 2);

        // Create temp3 and check its properties before checking manager
        let temp3 = manager.create_temp_file_with_prefix("audio_").unwrap();
        let temp3_path = temp3.path().to_string_lossy();
        assert!(temp3_path.contains("audio_"));
        assert_eq!(manager.len(), 3);

        // Check manager access
        assert!(manager.get(0).is_some());
        assert!(manager.get(2).is_some());
        assert!(manager.get(3).is_none());
    }

    #[tokio::test]
    async fn test_utils_create_temp_file_with_data() {
        let test_data = b"Utility test data";
        let temp_file = utils::create_temp_file_with_data(test_data).await.unwrap();

        let read_data = temp_file.read_data().await.unwrap();
        assert_eq!(read_data, test_data);
    }

    #[tokio::test]
    async fn test_utils_create_temp_file_with_string() {
        let test_string = "Utility test string";
        let temp_file = utils::create_temp_file_with_string(test_string)
            .await
            .unwrap();

        let read_string = temp_file.read_string().await.unwrap();
        assert_eq!(read_string, test_string);
    }

    #[tokio::test]
    async fn test_temp_file_persist() {
        let temp_file = TempFile::new().unwrap();
        let test_data = b"Persist test data";
        temp_file.write_data(test_data).await.unwrap();

        // Create a temp path for persistence
        let persist_path = std::env::temp_dir().join("test_persist.txt");
        temp_file.persist(&persist_path).unwrap();

        // Verify the file was copied
        assert!(persist_path.exists());
        let persisted_data = std::fs::read(&persist_path).unwrap();
        assert_eq!(persisted_data, test_data);

        // Clean up manually
        let _ = std::fs::remove_file(&persist_path);
    }

    #[test]
    fn test_temp_file_manager_keep_all() {
        let mut manager = TempFileManager::new();

        let _temp1 = manager.create_temp_file().unwrap();
        let _temp2 = manager.create_temp_file().unwrap();

        let kept_paths = manager.keep_all();
        assert_eq!(kept_paths.len(), 2);

        // Files should still exist (they are not automatically cleaned up when kept)
        for path in &kept_paths {
            assert!(path.exists());
        }

        // Clean up manually since we kept them
        for path in kept_paths {
            let _ = std::fs::remove_file(&path);
        }
    }
}
