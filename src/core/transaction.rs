use crate::core::{PackageManager, Result, Error};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::{info, warn, error};

#[async_trait]
pub trait Operation: Send + Sync {
    async fn execute(&self) -> Result<()>;
    async fn rollback(&self) -> Result<()>;
    fn description(&self) -> String;
}

pub struct PackageOperation {
    pub manager: Arc<dyn PackageManager>,
    pub packages: Vec<String>,
    pub is_install: bool,
    pub sudo: bool,
}

#[async_trait]
impl Operation for PackageOperation {
    async fn execute(&self) -> Result<()> {
        if self.is_install { self.manager.install(&self.packages, self.sudo).await }
        else { self.manager.remove(&self.packages, self.sudo).await }
    }

    async fn rollback(&self) -> Result<()> {
    // First, remove the failed package
    let _ = self.manager.remove(&self.packages, self.sudo).await;
    // NEW: Tell the system to sweep up any leftover "orphaned" files that were brought in
    if self.manager.supports_orphan_cleanup() {
        let _ = self.manager.clean_orphans(self.sudo).await;
    }
    Ok(())
}

    fn description(&self) -> String {
        format!("{} [{}] via {}", if self.is_install { "Install" } else { "Remove" }, self.packages.join(", "), self.manager.name())
    }
}

pub struct Transaction {
    operations: Vec<Box<dyn Operation>>,
    completed: Vec<usize>,
}

impl Transaction {
    pub fn new() -> Self { Self { operations: Vec::new(), completed: Vec::new() } }
    pub fn add(&mut self, op: Box<dyn Operation>) { self.operations.push(op); }

    pub async fn execute(&mut self) -> Result<()> {
        for (i, op) in self.operations.iter().enumerate() {
            info!("Transaction Step: {}", op.description());
            match op.execute().await {
                Ok(_) => self.completed.push(i),
                Err(e) => {
                    error!("Step failed: {}. Initiating rollback of successful steps...", e);
                    for &idx in self.completed.iter().rev() {
                        let _ = self.operations[idx].rollback().await;
                    }
                    return Err(Error::Transaction(e.to_string()));
                }
            }
        }
        Ok(())
    }
}