//! Laragon Linux core: GUI-independent service orchestration.

pub mod paths;
pub mod config;
pub mod service;
pub mod process;
pub mod orchestrator;
pub mod sites;

pub use config::Config;
pub use orchestrator::Orchestrator;
pub use paths::LaragonPaths;
pub use process::RealSpawner;
pub use service::registry::build_services;
pub use service::ServiceKind;
