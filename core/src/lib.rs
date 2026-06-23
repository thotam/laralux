//! Laragon Linux core: GUI-independent service orchestration.

pub mod paths;
pub mod bin;
pub mod config;
pub mod service;
pub mod process;
pub mod orchestrator;
pub mod sites;
pub mod site_registry;
pub mod hosts;
pub mod ssl;
pub mod privileged;
pub mod sync;
pub mod setup;
pub mod scaffold;

pub use config::Config;
pub use orchestrator::{Orchestrator, ServiceStatus};
pub use paths::LaragonPaths;
pub use process::RealSpawner;
pub use service::registry::build_services;
pub use service::{ServiceKind, ServiceState};
pub use privileged::{PkexecPrivileged, Privileged, SudoPrivileged};
pub use sites::{list_all_sites, scan_sites, Site, SiteSource};
pub use site_registry::{RegisteredSite, SiteRegistry, RegistryError};
pub use ssl::MkcertIssuer;
pub use sync::sync_sites;
pub use setup::{detect as detect_components, run_setup, Component, ComponentStatus, CurlDownloader, SetupReport};
pub use scaffold::{CommandRunner, RealCommandRunner, SiteTemplate, ScaffoldError, create_site, CreateReport};
