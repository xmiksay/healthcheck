//! The single serialized mutation path: lock → clone config → change → validate →
//! persist atomically → swap in memory → adjust only the affected task.
//! Any failure before the swap leaves memory, disk and running tasks untouched.

use std::path::{Path, PathBuf};

use anyhow::Context;
use tokio::io::AsyncWriteExt;

use super::{AppState, ServiceEntry};
use crate::config::{Config, Service, Settings};
use crate::error::AppError;
use crate::validate::{validate_service, validate_settings};

impl AppState {
    pub async fn create_service(
        &self,
        id: String,
        service: Service,
    ) -> Result<ServiceEntry, AppError> {
        validate_service(&id, &service)?;
        let _guard = self.write_lock.lock().await;
        let mut config = self.get_config().await;
        if config.services.contains_key(&id) {
            return Err(AppError::Conflict(id));
        }
        config.services.insert(id.clone(), service.clone());
        self.commit(config).await?;
        self.apply_live(&id, Some(&service)).await;
        tracing::info!("Service '{}' created", id);
        Ok(self.entry(&id, service).await)
    }

    /// Replaces the service config; live stats are kept, only its task restarts.
    pub async fn update_service(
        &self,
        id: &str,
        service: Service,
    ) -> Result<ServiceEntry, AppError> {
        let _guard = self.write_lock.lock().await;
        let mut config = self.get_config().await;
        let slot = config
            .services
            .get_mut(id)
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        validate_service(id, &service)?;
        *slot = service.clone();
        self.commit(config).await?;
        self.apply_live(id, Some(&service)).await;
        tracing::info!("Service '{}' updated", id);
        Ok(self.entry(id, service).await)
    }

    /// Flips `enabled` under the write lock, so a concurrent edit is never overwritten.
    pub async fn set_service_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> Result<ServiceEntry, AppError> {
        let _guard = self.write_lock.lock().await;
        let mut config = self.get_config().await;
        let slot = config
            .services
            .get_mut(id)
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        slot.enabled = enabled;
        let service = slot.clone();
        self.commit(config).await?;
        self.apply_live(id, Some(&service)).await;
        tracing::info!("Service '{}' enabled={}", id, enabled);
        Ok(self.entry(id, service).await)
    }

    pub async fn delete_service(&self, id: &str) -> Result<(), AppError> {
        let _guard = self.write_lock.lock().await;
        let mut config = self.get_config().await;
        if config.services.remove(id).is_none() {
            return Err(AppError::NotFound(id.to_string()));
        }
        self.commit(config).await?;
        self.apply_live(id, None).await;
        tracing::info!("Service '{}' deleted", id);
        Ok(())
    }

    /// No task restart: the monitor loops read the globals every cycle.
    pub async fn update_settings(&self, settings: Settings) -> Result<Settings, AppError> {
        validate_settings(&settings)?;
        let _guard = self.write_lock.lock().await;
        let mut config = self.get_config().await;
        config.apply_settings(settings);
        self.commit(config).await?;
        tracing::info!("Global settings updated");
        Ok(self.get_settings().await)
    }

    /// Persists `config` and swaps it in. Caller must hold `write_lock`.
    async fn commit(&self, config: Config) -> Result<(), AppError> {
        let persist = async {
            let yaml = serde_yaml::to_string(&config).context("failed to serialize config")?;
            write_atomic(&self.config_path, yaml.as_bytes()).await
        };
        if let Err(e) = persist.await {
            tracing::error!("Failed to persist configuration: {:#}", e);
            return Err(AppError::Persist(e));
        }
        *self.config.write().await = config;
        Ok(())
    }
}

/// Writes `<path>.tmp` next to the target and renames it over, so a crash never
/// leaves a truncated config behind.
async fn write_atomic(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    // Rename replaces a symlink with a plain file; write next to its target instead.
    let path = &tokio::fs::canonicalize(path)
        .await
        .unwrap_or_else(|_| path.to_path_buf());
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);

    let result = async {
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .with_context(|| format!("failed to create {}", tmp.display()))?;
        // The file holds secrets: keep the original's (typically restrictive) mode.
        if let Ok(meta) = tokio::fs::metadata(path).await {
            file.set_permissions(meta.permissions())
                .await
                .with_context(|| format!("failed to set permissions on {}", tmp.display()))?;
        }
        file.write_all(contents)
            .await
            .with_context(|| format!("failed to write {}", tmp.display()))?;
        file.sync_all()
            .await
            .with_context(|| format!("failed to sync {}", tmp.display()))?;
        tokio::fs::rename(&tmp, path)
            .await
            .with_context(|| format!("failed to rename {} over {}", tmp.display(), path.display()))
    }
    .await;

    if result.is_err() {
        // Best effort: the tmp file may not exist at all.
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::write_atomic;

    #[tokio::test]
    async fn write_atomic_keeps_symlink_and_writes_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.yaml");
        let link = dir.path().join("link.yaml");
        std::fs::write(&target, "old").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        write_atomic(&link, b"new").await.unwrap();

        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    }
}
