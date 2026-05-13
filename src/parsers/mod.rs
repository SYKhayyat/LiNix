pub mod common;
pub mod apt;
pub mod pacman;
pub mod dnf;
pub mod brew;
pub mod language;
pub mod windows;
pub mod macos;
pub mod nix;
pub mod utils;

use crate::core::Package;

/// The Strategy Interface for all backend output parsing.
/// This allows the Registry to treat every parser as an interchangeable object,
/// supporting the SOLID Open/Closed principle for new package manager support.
pub trait OutputParser: Send + Sync {
    /// Parses a raw string of installed packages into structured Package objects.
    fn parse_installed(&self, output: &str) -> Vec<Package>;
    
    /// Parses raw search results into structured Package objects.
    fn parse_search(&self, output: &str) -> Vec<Package>;
}

/// A Functional Strategy Parser that allows injecting functions as data.
/// Used in backends/registry.rs to configure GenericManagers without 
/// creating dozens of boilerplate structs.
pub struct LambdaParser {
    pub installed_fn: fn(&str) -> Vec<Package>,
    pub search_fn: fn(&str) -> Vec<Package>,
}

impl OutputParser for LambdaParser {
    fn parse_installed(&self, output: &str) -> Vec<Package> {
        (self.installed_fn)(output)
    }
    fn parse_search(&self, output: &str) -> Vec<Package> {
        (self.search_fn)(output)
    }
}