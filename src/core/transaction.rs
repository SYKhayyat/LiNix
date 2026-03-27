use crate::core::Result;
use async_trait::async_trait;
use std::fmt;
use tracing::{error, info, warn};

/// Represents an operation that can be executed and rolled back
#[async_trait]
pub trait Operation: Send + Sync {
    /// Execute the operation
    async fn execute(&mut self) -> Result<()>;

    /// Rollback the operation (best effort)
    async fn rollback(&mut self) -> Result<()>;

    /// Get a description of this operation
    fn description(&self) -> String;
}

/// A transaction that can execute multiple operations and rollback on failure
pub struct Transaction {
    operations: Vec<Box<dyn Operation>>,
    executed: Vec<usize>,
    description: String,
}

impl Transaction {
    /// Create a new transaction
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            operations: Vec::new(),
            executed: Vec::new(),
            description: description.into(),
        }
    }

    /// Add an operation to the transaction
    pub fn add_operation(&mut self, operation: Box<dyn Operation>) {
        self.operations.push(operation);
    }

    /// Execute all operations
    pub async fn execute(&mut self) -> Result<()> {
        info!("Executing transaction: {}", self.description);

        let op_count = self.operations.len();

        for idx in 0..op_count {
            let operation = &mut self.operations[idx];
            info!("  Operation {}/{}: {}", idx + 1, op_count, operation.description());

            match operation.execute().await {
                Ok(_) => {
                    self.executed.push(idx);
                }
                Err(e) => {
                    error!("Operation failed: {}", e);
                    warn!("Rolling back {} executed operations", self.executed.len());

                    // Rollback in reverse order
                    for &executed_idx in self.executed.iter().rev() {
                        if let Err(rollback_err) = self.operations[executed_idx].rollback().await {
                            error!("Rollback failed for operation {}: {}", executed_idx, rollback_err);
                        }
                    }

                    return Err(crate::core::Error::Transaction(format!(
                        "Transaction '{}' failed: {}",
                        self.description, e
                    )));
                }
            }
        }

        info!("Transaction completed successfully: {}", self.description);
        Ok(())
    }

    /// Get the number of operations
    pub fn len(&self) -> usize {
        self.operations.len()
    }

    /// Check if transaction is empty
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }
}

impl fmt::Display for Transaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Transaction '{}' with {} operations", self.description, self.operations.len())
    }
}

/// Install operation
pub struct InstallOperation {
    backend_name: String,
    packages: Vec<String>,
    executed: bool,
}

impl InstallOperation {
    pub fn new(backend_name: String, packages: Vec<String>) -> Self {
        Self {
            backend_name,
            packages,
            executed: false,
        }
    }
}

#[async_trait]
impl Operation for InstallOperation {
    async fn execute(&mut self) -> Result<()> {
        info!("Installing packages via {}: {:?}", self.backend_name, self.packages);
        self.executed = true;
        Ok(())
    }

    async fn rollback(&mut self) -> Result<()> {
        if !self.executed {
            return Ok(());
        }

        warn!("Rolling back installation via {}: {:?}", self.backend_name, self.packages);
        self.executed = false;
        Ok(())
    }

    fn description(&self) -> String {
        format!("Install {} packages via {}", self.packages.len(), self.backend_name)
    }
}

/// Remove operation
pub struct RemoveOperation {
    backend_name: String,
    packages: Vec<String>,
    executed: bool,
}

impl RemoveOperation {
    pub fn new(backend_name: String, packages: Vec<String>) -> Self {
        Self {
            backend_name,
            packages,
            executed: false,
        }
    }
}

#[async_trait]
impl Operation for RemoveOperation {
    async fn execute(&mut self) -> Result<()> {
        info!("Removing packages via {}: {:?}", self.backend_name, self.packages);
        self.executed = true;
        Ok(())
    }

    async fn rollback(&mut self) -> Result<()> {
        if !self.executed {
            return Ok(());
        }

        warn!("Rolling back removal via {}: {:?}", self.backend_name, self.packages);
        self.executed = false;
        Ok(())
    }

    fn description(&self) -> String {
        format!("Remove {} packages via {}", self.packages.len(), self.backend_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockSuccessOperation;

    #[async_trait]
    impl Operation for MockSuccessOperation {
        async fn execute(&mut self) -> Result<()> {
            Ok(())
        }

        async fn rollback(&mut self) -> Result<()> {
            Ok(())
        }

        fn description(&self) -> String {
            "Mock success operation".to_string()
        }
    }

    struct MockFailOperation;

    #[async_trait]
    impl Operation for MockFailOperation {
        async fn execute(&mut self) -> Result<()> {
            Err(crate::core::Error::Other("Mock failure".to_string()))
        }

        async fn rollback(&mut self) -> Result<()> {
            Ok(())
        }

        fn description(&self) -> String {
            "Mock fail operation".to_string()
        }
    }

    #[tokio::test]
    async fn test_transaction_success() {
        let mut transaction = Transaction::new("test");
        transaction.add_operation(Box::new(MockSuccessOperation));
        transaction.add_operation(Box::new(MockSuccessOperation));

        let result = transaction.execute().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_transaction_rollback() {
        let mut transaction = Transaction::new("test");
        transaction.add_operation(Box::new(MockSuccessOperation));
        transaction.add_operation(Box::new(MockFailOperation));

        let result = transaction.execute().await;
        assert!(result.is_err());
    }
}
